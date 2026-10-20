//! Built-in boot self-test. Runs on every boot; `cargo xtask test` boots the
//! kernel in QEMU and checks the result.

use crate::arch::{self, idt::BREAKPOINTS};
use crate::process::Process;
use crate::syscall;
use core::sync::atomic::Ordering;
use poly_abi::{Outcome, Personality, RFLAGS_CF, SyscallRegs};
use poly_mem::{BumpFrameAllocator, MemoryMap, PAGE_SIZE, RegionKind};

/// Printed on success; the test runner looks for this exact line.
pub const PASS_MARKER: &str = "POLY-SELFTEST: PASS";

type TestResult = Result<(), &'static str>;

pub fn run(memory: &MemoryMap) -> bool {
    let tests: [(&str, &dyn Fn() -> TestResult); 3] = [
        ("cpu exceptions return", &breakpoint),
        ("frame allocator", &|| frames(memory)),
        ("personality routing", &personality_routing),
    ];
    let mut ok = true;
    for (name, test) in tests {
        match test() {
            Ok(()) => crate::kprintln!("selftest: {name} ... ok"),
            Err(why) => {
                crate::kprintln!("selftest: {name} ... FAILED: {why}");
                ok = false;
            }
        }
    }
    if ok {
        crate::kprintln!("{PASS_MARKER}");
    } else {
        crate::kprintln!("POLY-SELFTEST: FAIL");
    }
    ok
}

fn breakpoint() -> TestResult {
    let before = BREAKPOINTS.load(Ordering::Relaxed);
    arch::cpu::breakpoint();
    if BREAKPOINTS.load(Ordering::Relaxed) == before + 1 { Ok(()) } else { Err("handler did not run") }
}

fn frames(memory: &MemoryMap) -> TestResult {
    let mut alloc = BumpFrameAllocator::new(memory);
    let mut last = None;
    for _ in 0..64 {
        let f = alloc.allocate().ok_or("out of frames")?;
        if f % PAGE_SIZE != 0 {
            return Err("unaligned frame");
        }
        if last.is_some_and(|l| f <= l) {
            return Err("frames not increasing");
        }
        let inside_usable =
            memory.regions().iter().any(|r| r.kind == RegionKind::Usable && r.start <= f && f + PAGE_SIZE <= r.end);
        if !inside_usable {
            return Err("frame outside usable RAM");
        }
        last = Some(f);
    }
    Ok(())
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

/// Negative test (`poly.selftest.page_fault`): touch an unmapped address and
/// let the exception path report it. The runner expects a panic, not a pass.
pub fn trigger_page_fault() {
    crate::kprintln!("selftest: touching unmapped memory on purpose");
    let p = 0x7fff_0000_0000u64 as *const u64; // canonical, never mapped
    // SAFETY: deliberately invalid; the page fault handler panics.
    let v = unsafe { core::ptr::read_volatile(p) };
    crate::kprintln!("unexpectedly read {v:#x}");
}
