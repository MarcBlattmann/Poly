//! Kernel backtraces from the frame-pointer chain.
//!
//! The kernel is built with `-C force-frame-pointers=yes`
//! (kernel/.cargo/config.toml), so `rbp` heads a chain of frame records that
//! [`poly_backtrace::FrameWalker`] follows. The panic handler prints it in a
//! fixed format that `cargo xtask` symbolises from the kernel ELF:
//!
//! ```text
//! backtrace:
//!   #0 0xffffffff80106a8c
//!   #1 0xffffffff80100e2f
//! ```
//!
//! Frame #0 is an exact instruction address: the faulting `rip` for a CPU
//! exception, otherwise a point inside the panic path. Every later frame is
//! a return address, i.e. the instruction after a call.

use core::arch::naked_asm;
use core::ops::Range;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_backtrace::{End, FrameWalker};

/// Frames printed at most, so a corrupt chain cannot flood the log.
pub const MAX_DEPTH: usize = 32;

static FAULT_PC: AtomicU64 = AtomicU64::new(0);
static FAULT_FP: AtomicU64 = AtomicU64::new(0);
static PRINTING: AtomicBool = AtomicBool::new(false);

unsafe extern "C" {
    static __boot_stack_bottom: u8;
    static __boot_stack_top: u8;
}

/// Record where a fatal CPU exception hit, so the panic backtrace starts at
/// the faulting instruction rather than inside the exception handler.
pub fn set_fault_origin(pc: u64, fp: u64) {
    FAULT_FP.store(fp, Ordering::Relaxed);
    FAULT_PC.store(pc, Ordering::Relaxed);
}

/// Every stack the kernel runs on today. Kernel threads will add theirs.
fn stacks() -> [Range<u64>; 2] {
    let boot = (&raw const __boot_stack_bottom) as u64..(&raw const __boot_stack_top) as u64;
    [boot, super::gdt::double_fault_stack()]
}

#[repr(C)]
struct Here {
    pc: u64,
    fp: u64,
}

/// The caller's frame pointer and an address inside the caller (where this
/// call returns to). Reading `rbp` with inline asm instead is not reliable:
/// LLVM may move the caller's prologue past the asm (shrink-wrapping), but
/// never past a call, so the frame is set up by the time we get here.
#[unsafe(naked)]
extern "sysv64" fn here() -> Here {
    // SAFETY: leaf code that only reads the return address at [rsp] and rbp
    // into the two return registers (rax, rdx) of a 16-byte sysv64 struct.
    naked_asm!("mov rax, [rsp]", "mov rdx, rbp", "ret")
}

/// Call `f(n, address)` for frame #0 at `pc` and each return address on the
/// chain starting at frame pointer `fp`. Returns why the walk stopped.
fn walk(pc: u64, fp: u64, mut f: impl FnMut(usize, u64)) -> End {
    f(0, pc);
    let read = |addr: u64| {
        // SAFETY: the walker only reads aligned words inside one of
        // `stacks()`, which are the boot stack (.bss) and a static, both
        // always mapped.
        unsafe { (addr as *const u64).read_volatile() }
    };
    let mut walker = FrameWalker::new(fp, &stacks(), MAX_DEPTH, read);
    for (n, ret) in (1..).zip(walker.by_ref()) {
        f(n, ret);
    }
    walker.end().unwrap_or(End::Bottom)
}

/// Print the backtrace for a panic, without taking the console lock. Starts
/// at the faulting instruction if a CPU exception caused the panic.
#[inline(never)]
pub fn print_for_panic() {
    if PRINTING.swap(true, Ordering::Relaxed) {
        return; // panicked while printing a backtrace: do not try again
    }
    let (pc, fp) = match FAULT_PC.load(Ordering::Relaxed) {
        0 => {
            let h = here();
            (h.pc, h.fp)
        }
        pc => (pc, FAULT_FP.load(Ordering::Relaxed)),
    };
    crate::log::emergency_print(format_args!("backtrace:\n"));
    let end = walk(pc, fp, |n, addr| crate::log::emergency_print(format_args!("  #{n} {addr:#018x}\n")));
    if end != End::Bottom {
        crate::log::emergency_print(format_args!("  ({})\n", end.describe()));
    }
}

crate::kernel_test! {
    fn walks_to_kernel_entry() -> TestResult {
        let Here { pc, fp } = here();
        let (mut frames, mut last, mut higher_half) = (0, 0, true);
        let end = walk(pc, fp, |_, addr| {
            higher_half &= last == 0 || last >= super::KERNEL_VMA;
            last = addr;
            frames += 1;
        });
        if end != End::Bottom {
            return Err(end.describe());
        }
        // Inlining merges frames in release builds, but the runner calls the
        // test through a pointer, so there are at least: this test, the
        // runner (or kernel_entry, if it is inlined there) and the boot stub.
        if frames < 3 {
            return Err("fewer frames than calls");
        }
        // The walk must end in the boot stub, which runs at its physical
        // address; every return address before that is in the kernel proper.
        if !higher_half || last >= super::KERNEL_VMA {
            return Err("return addresses do not lead back to the boot stub");
        }
        Ok(())
    }
}
