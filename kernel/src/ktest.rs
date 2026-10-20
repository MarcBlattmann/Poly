//! In-kernel test harness: tests that need the real machine (CPU exceptions,
//! page tables, devices) live next to the code they test and run inside
//! QEMU on every boot.
//!
//! [`kernel_test!`](crate::kernel_test) defines a test function and puts a
//! [`KernelTest`] record for it into the `.kernel_tests.*` linker sections.
//! The linker script gathers them, sorted by name, between
//! `__kernel_tests_start` and `__kernel_tests_end`, so registering a test
//! needs neither a heap nor a central list. [`run_all`] prints one line per
//! test and a summary line, which `cargo xtask test` parses:
//!
//! ```text
//! ktest: running 5 of 6 tests
//! test selftest::frame_allocator ... FAILED: out of frames
//! test sync::spinlock_excludes_until_guard_dropped ... ok
//! test result: FAILED. 4 passed; 1 failed; 1 filtered out
//! ```
//!
//! Boot with `poly.test=<substring>` to run only the tests whose name
//! contains that substring. Pure logic belongs in a host-tested crate;
//! these are for what only the kernel can check.

use crate::boot_info::BootInfo;
use crate::kprintln;
use core::mem::size_of;
use poly_mem::MemoryMap;

pub type TestResult = Result<(), &'static str>;

/// What a test may look at: the state the kernel has built up by the time
/// the tests run.
#[derive(Debug)]
pub struct TestEnv<'a> {
    pub boot: &'a BootInfo,
    /// Normalised physical memory map.
    pub memory: &'a MemoryMap,
}

/// One registered test. Only [`kernel_test!`](crate::kernel_test) creates these.
#[derive(Debug)]
pub struct KernelTest {
    /// `module_path!()::name`, including the crate name.
    pub path: &'static str,
    pub run: fn(&TestEnv) -> TestResult,
}

impl KernelTest {
    /// Name without the crate prefix, e.g. `sync::spinlock_excludes_until_guard_dropped`.
    pub fn name(&self) -> &'static str {
        self.path.split_once("::").map_or(self.path, |(_, rest)| rest)
    }
}

unsafe extern "C" {
    static __kernel_tests_start: u8;
    static __kernel_tests_end: u8;
}

/// Every test linked into the kernel, sorted by path.
pub fn all() -> &'static [KernelTest] {
    let start = (&raw const __kernel_tests_start).cast::<KernelTest>();
    let end = (&raw const __kernel_tests_end).cast::<KernelTest>();
    let len = (end as usize - start as usize) / size_of::<KernelTest>();
    // SAFETY: the linker script places only `KernelTest` statics (emitted by
    // `kernel_test!`) between the two symbols. Each is 8-aligned with a size
    // that is a multiple of 8, so they are packed back to back with no gaps.
    unsafe { core::slice::from_raw_parts(start, len) }
}

/// Counts reported in the summary line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Summary {
    pub passed: usize,
    pub failed: usize,
    pub filtered_out: usize,
}

/// Run every test, or with `poly.test=<filter>` on the command line, every
/// test whose name contains `filter`.
pub fn run_all(env: &TestEnv) -> Summary {
    let filter = env.boot.arg("poly.test");
    let tests = all();
    let selected = |t: &KernelTest| filter.is_none_or(|f| t.name().contains(f));
    kprintln!("ktest: running {} of {} tests", tests.iter().filter(|t| selected(t)).count(), tests.len());
    let mut s = Summary::default();
    for test in tests {
        if !selected(test) {
            s.filtered_out += 1;
            continue;
        }
        match (test.run)(env) {
            Ok(()) => {
                kprintln!("test {} ... ok", test.name());
                s.passed += 1;
            }
            Err(why) => {
                kprintln!("test {} ... FAILED: {why}", test.name());
                s.failed += 1;
            }
        }
    }
    let verdict = if s.failed == 0 { "ok" } else { "FAILED" };
    kprintln!("test result: {verdict}. {} passed; {} failed; {} filtered out", s.passed, s.failed, s.filtered_out);
    s
}

/// Define an in-kernel test next to the code it tests:
///
/// ```ignore
/// kernel_test! {
///     fn lock_is_exclusive() -> TestResult {
///         // ...
///         Ok(())
///     }
/// }
///
/// kernel_test! {
///     fn sees_usable_memory(env: &TestEnv) -> TestResult {
///         if env.memory.total(RegionKind::Usable) == 0 { return Err("no usable RAM") }
///         Ok(())
///     }
/// }
/// ```
///
/// The test is named after its module path and function name. `TestResult`
/// and `TestEnv` must be written exactly like that; the macro supplies the
/// real types, so they do not need to be imported.
#[macro_export]
macro_rules! kernel_test {
    (@register $name:ident, $run:expr) => {
        const _: () = {
            #[used]
            #[unsafe(link_section = concat!(".kernel_tests.", module_path!(), "::", stringify!($name)))]
            static TEST: $crate::ktest::KernelTest = $crate::ktest::KernelTest {
                path: concat!(module_path!(), "::", stringify!($name)),
                run: $run,
            };
        };
    };
    ($(#[$attr:meta])* fn $name:ident() -> TestResult $body:block) => {
        $(#[$attr])*
        fn $name() -> $crate::ktest::TestResult $body
        $crate::kernel_test!(@register $name, |_| $name());
    };
    ($(#[$attr:meta])* fn $name:ident($env:ident: &TestEnv) -> TestResult $body:block) => {
        $(#[$attr])*
        fn $name($env: &$crate::ktest::TestEnv) -> $crate::ktest::TestResult $body
        $crate::kernel_test!(@register $name, $name);
    };
}
