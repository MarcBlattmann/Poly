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
//!
//! The shell's `test` command runs them again at runtime with [`run`]: any
//! subset, with the output going to the shell's console. One run at a time
//! ([`RunError::Busy`]), so a test never runs inside itself. Tests marked
//! `#[boot_only]` check the state at boot (they run on the boot thread, or
//! before any driver routed an interrupt) and are left out at runtime.

use crate::boot_info::BootInfo;
use crate::log::Kprint;
use core::fmt::Write;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    /// Only meaningful during the boot self-test (`#[boot_only]`).
    pub boot_only: bool,
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
/// test whose name contains `filter`, printing to the kernel log.
pub fn run_all(env: &TestEnv) -> Summary {
    run(env, env.boot.arg("poly.test"), When::Boot, &mut Kprint).expect("the boot tests run first")
}

/// Whether [`run`] is the boot self-test, which runs `#[boot_only]` tests too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    Boot,
    Runtime,
}

/// Why [`run`] did not run anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunError {
    /// Tests are running already (this is called from inside a test).
    Busy,
}

/// Set while [`run`] runs.
static RUNNING: AtomicBool = AtomicBool::new(false);
/// The test running now: its index in [`all`] plus one, 0 for none.
static CURRENT: AtomicUsize = AtomicUsize::new(0);

/// Whether tests are running (at boot, or from the shell's `test`).
/// Readers of shared devices step aside meanwhile: the keyboard tests read
/// the keyboard themselves.
pub fn running() -> bool {
    RUNNING.load(Ordering::Acquire)
}

/// The name of the test running now, if one is.
pub fn current() -> Option<&'static str> {
    match CURRENT.load(Ordering::Acquire) {
        0 => None,
        i => all().get(i - 1).map(KernelTest::name),
    }
}

/// Run every test whose name contains `filter` (all with `None`), writing
/// the lines `cargo xtask test` parses to `out`. At [`When::Runtime`],
/// `#[boot_only]` tests are left out (counted as filtered out). The tests'
/// own messages still go to the kernel log.
pub fn run(env: &TestEnv, filter: Option<&str>, when: When, out: &mut dyn Write) -> Result<Summary, RunError> {
    if RUNNING.swap(true, Ordering::Acquire) {
        return Err(RunError::Busy);
    }
    let tests = all();
    let matches = |t: &KernelTest| filter.is_none_or(|f| t.name().contains(f));
    let selected = |t: &KernelTest| matches(t) && (when == When::Boot || !t.boot_only);
    let _ = writeln!(out, "ktest: running {} of {} tests", tests.iter().filter(|t| selected(t)).count(), tests.len());
    let boot_only = tests.iter().filter(|t| matches(t) && !selected(t)).count();
    if boot_only > 0 {
        let _ = writeln!(out, "ktest: leaving out {boot_only} tests that only run at boot");
    }
    let mut s = Summary::default();
    for (i, test) in tests.iter().enumerate() {
        if !selected(test) {
            s.filtered_out += 1;
            continue;
        }
        CURRENT.store(i + 1, Ordering::Release);
        let result = (test.run)(env);
        CURRENT.store(0, Ordering::Release);
        let _ = match result {
            Ok(()) => {
                s.passed += 1;
                writeln!(out, "test {} ... ok", test.name())
            }
            Err(why) => {
                s.failed += 1;
                writeln!(out, "test {} ... FAILED: {why}", test.name())
            }
        };
    }
    let verdict = if s.failed == 0 { "ok" } else { "FAILED" };
    let _ = writeln!(
        out,
        "test result: {verdict}. {} passed; {} failed; {} filtered out",
        s.passed, s.failed, s.filtered_out
    );
    RUNNING.store(false, Ordering::Release);
    Ok(s)
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
/// real types, so they do not need to be imported. Write `#[boot_only]`
/// first (before any doc comment) for a test that checks the state at boot
/// and cannot pass when the shell runs it later.
#[macro_export]
macro_rules! kernel_test {
    (@register $name:ident, $run:expr, $boot_only:expr) => {
        const _: () = {
            #[used]
            #[unsafe(link_section = concat!(".kernel_tests.", module_path!(), "::", stringify!($name)))]
            static TEST: $crate::ktest::KernelTest = $crate::ktest::KernelTest {
                path: concat!(module_path!(), "::", stringify!($name)),
                run: $run,
                boot_only: $boot_only,
            };
        };
    };
    (#[boot_only] $(#[$attr:meta])* fn $name:ident() -> TestResult $body:block) => {
        $(#[$attr])*
        fn $name() -> $crate::ktest::TestResult $body
        $crate::kernel_test!(@register $name, |_| $name(), true);
    };
    (#[boot_only] $(#[$attr:meta])* fn $name:ident($env:ident: &TestEnv) -> TestResult $body:block) => {
        $(#[$attr])*
        fn $name($env: &$crate::ktest::TestEnv) -> $crate::ktest::TestResult $body
        $crate::kernel_test!(@register $name, $name, true);
    };
    ($(#[$attr:meta])* fn $name:ident() -> TestResult $body:block) => {
        $(#[$attr])*
        fn $name() -> $crate::ktest::TestResult $body
        $crate::kernel_test!(@register $name, |_| $name(), false);
    };
    ($(#[$attr:meta])* fn $name:ident($env:ident: &TestEnv) -> TestResult $body:block) => {
        $(#[$attr])*
        fn $name($env: &$crate::ktest::TestEnv) -> $crate::ktest::TestResult $body
        $crate::kernel_test!(@register $name, $name, false);
    };
}
