//! The exception vector table and what the stub does with an exception.
//!
//! `VBAR_EL1` points at the table (Arm ARM D1.10, "Exception vectors"): 16
//! entries of 128 bytes, four groups (current EL with `SP_EL0`, current EL
//! with `SP_ELx`, lower EL AArch64, lower EL AArch32) of four kinds
//! (synchronous, IRQ, FIQ, SError), 2 KiB aligned. Each entry saves a
//! [`TrapFrame`] on the stack in use and calls [`a64_exception_handler`] with
//! the frame and the entry number; `eret` restores it, with whatever
//! `elr`/`spsr` the handler left in the frame.
//!
//! A self-test announces the exception it is about to cause with [`expect`];
//! an exception of that class from the stub's own code is recorded and
//! stepped over. Anything else is a fatal error report.

use crate::semihosting;
use crate::{cpu, kprintln};
use core::arch::global_asm;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_a64::regs::{self, ec};
use poly_a64::semihost::status;

/// The registers an entry saves: `x0..=x30`, then the exception state.
#[repr(C)]
#[derive(Debug)]
pub struct TrapFrame {
    pub x: [u64; 31],
    pub elr: u64,
    pub spsr: u64,
    pub esr: u64,
    pub far: u64,
    _pad: u64,
}

const FRAME_BYTES: usize = 288;
const _: () = assert!(size_of::<TrapFrame>() == FRAME_BYTES);

global_asm!(
    r#"
    .section .text.vectors, "ax"
    .balign 0x800
    .global a64_vectors
a64_vectors:
    .macro a64_ventry id
    .balign 0x80
    sub     sp, sp, #288
    stp     x0, x1, [sp, #0]
    mov     x0, #\id
    b       a64_vector_common
    .endm
    a64_ventry 0
    a64_ventry 1
    a64_ventry 2
    a64_ventry 3
    a64_ventry 4
    a64_ventry 5
    a64_ventry 6
    a64_ventry 7
    a64_ventry 8
    a64_ventry 9
    a64_ventry 10
    a64_ventry 11
    a64_ventry 12
    a64_ventry 13
    a64_ventry 14
    a64_ventry 15

    .balign 4
a64_vector_common:
    // x0 holds the entry number; x0 and x1 are already saved.
    stp     x2, x3, [sp, #16]
    stp     x4, x5, [sp, #32]
    stp     x6, x7, [sp, #48]
    stp     x8, x9, [sp, #64]
    stp     x10, x11, [sp, #80]
    stp     x12, x13, [sp, #96]
    stp     x14, x15, [sp, #112]
    stp     x16, x17, [sp, #128]
    stp     x18, x19, [sp, #144]
    stp     x20, x21, [sp, #160]
    stp     x22, x23, [sp, #176]
    stp     x24, x25, [sp, #192]
    stp     x26, x27, [sp, #208]
    stp     x28, x29, [sp, #224]
    str     x30, [sp, #240]
    mrs     x1, elr_el1
    mrs     x2, spsr_el1
    stp     x1, x2, [sp, #248]
    mrs     x1, esr_el1
    mrs     x2, far_el1
    stp     x1, x2, [sp, #264]
    mov     x1, x0
    mov     x0, sp
    bl      a64_exception_handler
    ldp     x1, x2, [sp, #248]
    msr     elr_el1, x1
    msr     spsr_el1, x2
    ldp     x2, x3, [sp, #16]
    ldp     x4, x5, [sp, #32]
    ldp     x6, x7, [sp, #48]
    ldp     x8, x9, [sp, #64]
    ldp     x10, x11, [sp, #80]
    ldp     x12, x13, [sp, #96]
    ldp     x14, x15, [sp, #112]
    ldp     x16, x17, [sp, #128]
    ldp     x18, x19, [sp, #144]
    ldp     x20, x21, [sp, #160]
    ldp     x22, x23, [sp, #176]
    ldp     x24, x25, [sp, #192]
    ldp     x26, x27, [sp, #208]
    ldp     x28, x29, [sp, #224]
    ldr     x30, [sp, #240]
    ldp     x0, x1, [sp, #0]
    add     sp, sp, #288
    eret
"#
);

unsafe extern "C" {
    static a64_vectors: u8;
}

/// The table's address.
pub fn vectors() -> u64 {
    // Only the address of the symbol is taken.
    &raw const a64_vectors as u64
}

/// The exception class a self-test expects next (0: none; class 0 is
/// "unknown", which no test causes).
static EXPECTED: AtomicU64 = AtomicU64::new(0);
/// Exceptions taken as expected.
static TAKEN: AtomicU64 = AtomicU64::new(0);
/// `ESR_EL1` and `FAR_EL1` of the last expected one.
static LAST_ESR: AtomicU64 = AtomicU64::new(0);
static LAST_FAR: AtomicU64 = AtomicU64::new(0);

/// What an expected exception looked like.
#[derive(Clone, Copy, Debug)]
pub struct Caught {
    /// How many expected exceptions have been taken in total.
    pub count: u64,
    pub esr: u64,
    pub far: u64,
}

/// The next synchronous exception of class `class` from this code is
/// expected: record it and continue after it.
pub fn expect(class: u64) {
    EXPECTED.store(class, Ordering::SeqCst);
}

/// The state recorded by the last expected exception.
pub fn caught() -> Caught {
    Caught {
        count: TAKEN.load(Ordering::SeqCst),
        esr: LAST_ESR.load(Ordering::SeqCst),
        far: LAST_FAR.load(Ordering::SeqCst),
    }
}

/// Called by every vector entry with the saved registers and the entry
/// number (0..16).
#[unsafe(no_mangle)]
extern "C" fn a64_exception_handler(frame: &mut TrapFrame, id: u64) {
    let class = regs::esr_class(frame.esr);
    // Only from the stub's own code on the stack in use (group 1).
    let from_here = id / 4 == 1;
    if regs::vector_is_synchronous(id) && from_here && class != 0 && EXPECTED.load(Ordering::SeqCst) == class {
        EXPECTED.store(0, Ordering::SeqCst);
        LAST_ESR.store(frame.esr, Ordering::SeqCst);
        LAST_FAR.store(frame.far, Ordering::SeqCst);
        TAKEN.fetch_add(1, Ordering::SeqCst);
        // BRK and aborts return to the instruction that caused them (Arm
        // ARM D1.10.1, "Preferred exception return address"); the test's
        // is one 4-byte instruction, so go past it. SVC already returns to
        // the next instruction.
        if class == ec::BRK_AARCH64 || class == ec::DATA_ABORT_SAME {
            frame.elr += 4;
        }
        return;
    }
    kprintln!("KERNEL EXCEPTION: {} {} (vector {id})", regs::vector_source(id), regs::vector_kind(id));
    kprintln!("  class {class:#x}: {}", regs::class_name(class));
    kprintln!("  esr {:#010x}  elr {:#018x}  far {:#018x}  spsr {:#010x}", frame.esr, frame.elr, frame.far, frame.spsr);
    if class == ec::DATA_ABORT_SAME || class == ec::INSTRUCTION_ABORT_SAME {
        kprintln!("  {}", regs::fault_status_name(regs::esr_iss(frame.esr)));
    }
    kprintln!("  sp {:#018x}  lr {:#018x}  x0 {:#018x}", frame as *const TrapFrame as u64, frame.x[30], frame.x[0]);
    semihosting::exit(status::EXCEPTION);
}

/// Whether `VBAR_EL1` points at our table.
pub fn installed() -> bool {
    cpu::vbar_el1() == vectors()
}
