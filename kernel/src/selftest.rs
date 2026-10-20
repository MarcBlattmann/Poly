//! Built-in boot self-test. Runs every registered in-kernel test (see
//! [`crate::ktest`]) on every boot; `cargo xtask test` boots the kernel in
//! QEMU and checks the result.

use crate::arch::{self, idt::BREAKPOINTS};
use crate::boot_info::BootInfo;
use crate::frames::FRAMES;
use crate::kernel_test;
use crate::ktest::{self, TestEnv, TestResult};
use crate::process::Process;
use crate::syscall;
use core::sync::atomic::Ordering;
use poly_abi::{Outcome, Personality, RFLAGS_CF, SyscallRegs};
use poly_mem::{BuddyAllocator, FreeError, MAX_ORDER, MemoryMap, PAGE_SIZE, RegionKind};

/// Printed on success; the test runner looks for this exact line.
pub const PASS_MARKER: &str = "POLY-SELFTEST: PASS";

/// Run the in-kernel tests (filtered by `poly.test=<substring>`) and print
/// the PASS/FAIL marker. True if no test failed.
pub fn run(boot: &BootInfo, memory: &MemoryMap) -> bool {
    let env = TestEnv { boot, memory };
    let ok = ktest::run_all(&env).failed == 0;
    if ok {
        crate::kprintln!("{PASS_MARKER}");
    } else {
        crate::kprintln!("POLY-SELFTEST: FAIL");
    }
    ok
}

kernel_test! {
    fn cpu_exceptions_return() -> TestResult {
        let before = BREAKPOINTS.load(Ordering::Relaxed);
        arch::cpu::breakpoint();
        if BREAKPOINTS.load(Ordering::Relaxed) == before + 1 { Ok(()) } else { Err("handler did not run") }
    }
}

/// Mixed orders, including the largest, in no particular order.
const FRAME_ORDERS: [usize; 12] = [0, 3, 1, 0, MAX_ORDER, 2, 0, 9, 5, 0, 1, 4];

kernel_test! {
    fn frame_allocator(env: &TestEnv) -> TestResult {
        let memory = env.memory;
        let mut guard = FRAMES.lock();
        let alloc = guard.as_mut().ok_or("frame allocator not initialised")?;
        let start = alloc.free_frames();

        let mut live = [(0u64, 0usize); FRAME_ORDERS.len()];
        for (slot, &order) in live.iter_mut().zip(&FRAME_ORDERS) {
            let f = alloc.allocate(order).ok_or("out of frames")?;
            let size = BuddyAllocator::block_size(order);
            if !f.is_multiple_of(size) {
                return Err("block not aligned to its size");
            }
            let inside_usable =
                memory.regions().iter().any(|r| r.kind == RegionKind::Usable && r.start <= f && f + size <= r.end);
            if !inside_usable {
                return Err("block outside usable RAM");
            }
            *slot = (f, order);
        }
        for (i, &(a, ao)) in live.iter().enumerate() {
            for &(b, bo) in &live[..i] {
                if a < b + BuddyAllocator::block_size(bo) && b < a + BuddyAllocator::block_size(ao) {
                    return Err("blocks overlap");
                }
            }
        }
        let used: u64 = FRAME_ORDERS.iter().map(|&o| 1 << o).sum();
        if alloc.free_frames() != start - used {
            return Err("free count wrong after allocating");
        }

        // Write a pattern to the first and last frame of every block the boot
        // mapping reaches, then read it all back.
        let mut touched = 0;
        for pass in [Pass::Write, Pass::Check] {
            for &(f, order) in &live {
                let last = f + BuddyAllocator::block_size(order) - PAGE_SIZE;
                for frame in [f, last] {
                    if let Some(virt) = arch::phys_to_virt(frame) {
                        pattern(virt as *mut u64, frame, pass)?;
                        touched += 1;
                    }
                }
            }
        }
        if touched == 0 {
            return Err("no allocated frame below the boot mapping limit");
        }

        // Free in a different order than allocated so merges happen out of order.
        for &(f, order) in live.iter().step_by(2).chain(live.iter().skip(1).step_by(2)) {
            alloc.free(f, order).map_err(|_| "free refused a live block")?;
        }
        if alloc.free(live[0].0, live[0].1) != Err(FreeError::NotAllocated) {
            return Err("double free not detected");
        }
        if alloc.free_frames() != start {
            return Err("free count did not return to its starting value");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    Write,
    Check,
}

/// Fill one frame with values derived from its address, or check them.
fn pattern(frame: *mut u64, phys: u64, pass: Pass) -> TestResult {
    let words = (PAGE_SIZE / 8) as usize;
    for i in 0..words {
        let value = phys ^ (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        // SAFETY: `frame` maps a whole 4 KiB frame that this test allocated
        // and still owns, and `i` stays inside it.
        unsafe {
            let p = frame.add(i);
            match pass {
                Pass::Write => p.write_volatile(value),
                Pass::Check if p.read_volatile() != value => return Err("frame did not hold its pattern"),
                Pass::Check => {}
            }
        }
    }
    Ok(())
}

kernel_test! {
    fn acpi_tables() -> TestResult {
        let Some(s) = crate::acpi::summary() else {
            if let Some(phys) = crate::acpi::beyond_boot_mapping() {
                crate::kprintln!("selftest: acpi tables skipped: tables at {phys:#x} need the direct map");
                return Ok(());
            }
            return Err("ACPI tables not read");
        };
        if s.cpus == 0 {
            return Err("no enabled CPU in the MADT");
        }
        if s.io_apics == 0 {
            return Err("no I/O APIC in the MADT");
        }
        Ok(())
    }
}

/// Minimal headers for each format; only the fields detection reads.
const ELF_X86_64: [u8; 64] = {
    let mut h = [0u8; 64];
    h[0] = 0x7f;
    h[1] = b'E';
    h[2] = b'L';
    h[3] = b'F';
    h[4] = 2;
    h[5] = 1;
    h[6] = 1;
    h[18] = 62;
    h
};

const PE_X86_64: [u8; 0x100] = {
    let mut h = [0u8; 0x100];
    h[0] = b'M';
    h[1] = b'Z';
    h[0x3c] = 0x80;
    h[0x80] = b'P';
    h[0x81] = b'E';
    h[0x84] = 0x64;
    h[0x85] = 0x86;
    h[0x98] = 0x0b;
    h[0x99] = 0x02;
    h
};

const MACHO_ARM64: [u8; 32] = {
    let mut h = [0u8; 32];
    h[0] = 0xcf;
    h[1] = 0xfa;
    h[2] = 0xed;
    h[3] = 0xfe;
    h[4] = 0x0c;
    h[7] = 0x01;
    h
};

kernel_test! {
    fn personality_routing() -> TestResult {
        let cases: [(&[u8], Personality, u64); 3] = [
            (&ELF_X86_64, Personality::Linux, 39),
            (&PE_X86_64, Personality::Nt, 0x0055),
            (&MACHO_ARM64, Personality::Xnu, 0x0200_0014),
        ];
        for (pid, (image, expected, nr)) in (1u32..).zip(cases) {
            let p = Process::from_image(pid, image).map_err(|_| "image not recognised")?;
            crate::kprintln!("exec: pid {pid} {:?} -> {}", p.format, p.personality.name());
            if p.personality != expected {
                return Err("wrong personality for format");
            }
            let mut regs = SyscallRegs { rax: nr, ..Default::default() };
            if !matches!(syscall::dispatch(&p, &mut regs), Outcome::Unimplemented { .. }) {
                return Err("stub reported a call as handled");
            }
            let native_error = match expected {
                Personality::Linux => regs.rax == poly_personality_linux::err(poly_personality_linux::ENOSYS),
                Personality::Nt => regs.rax == poly_personality_nt::STATUS_NOT_IMPLEMENTED as u64,
                Personality::Xnu => regs.rax == poly_personality_xnu::ENOSYS && regs.rflags & RFLAGS_CF != 0,
            };
            if !native_error {
                return Err("wrong native error convention");
            }
        }
        Ok(())
    }
}

/// Negative test (`poly.selftest.page_fault`): touch an unmapped address and
/// let the exception path report it. The runner expects a panic, not a pass,
/// and a backtrace naming this function, so it must not be inlined.
#[inline(never)]
pub fn trigger_page_fault() {
    crate::kprintln!("selftest: touching unmapped memory on purpose");
    let p = 0x7fff_0000_0000u64 as *const u64; // canonical, never mapped
    // SAFETY: deliberately invalid; the page fault handler panics.
    let v = unsafe { core::ptr::read_volatile(p) };
    crate::kprintln!("unexpectedly read {v:#x}");
}

kernel_test! {
    /// Negative tests of the harness itself: passes unless the boot asks it
    /// to fail (`poly.selftest.fail`) or panic (`poly.selftest.panic`), so
    /// the runner can check that both are reported.
    fn fails_on_request(env: &TestEnv) -> TestResult {
        if env.boot.has_flag("poly.selftest.panic") {
            panic_on_purpose();
        }
        if env.boot.has_flag("poly.selftest.fail") { Err("failure requested by poly.selftest.fail") } else { Ok(()) }
    }
}

/// Kept out of line so the backtrace check has a frame to look for.
#[inline(never)]
fn panic_on_purpose() -> ! {
    panic!("panic requested by poly.selftest.panic");
}
