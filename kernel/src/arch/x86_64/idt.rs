//! Interrupt descriptor table and CPU exception handling.
//!
//! Every vector enters through a small assembly stub that pushes a uniform
//! [`TrapFrame`] and calls [`trap_dispatch`]: vectors 0-31 are CPU
//! exceptions, handled here; 32-255 are external interrupts, IPIs and the
//! local APIC's own (timer, error, spurious), handed to [`super::irq`]. The
//! `syscall` entry (`syscall.rs`) builds the same frame, and every return
//! goes through the same exit, `trap_return`.
//!
//! All 256 entries are interrupt gates, so the CPU clears RFLAGS.IF on entry
//! and handlers never nest. Every gate has DPL 0, so user mode cannot raise
//! any vector with `INT n` (or `INT3`): that is a #GP, which kills it. An
//! interrupt or exception in ring 3 makes the CPU switch to TSS.rsp0, the
//! running thread's kernel stack. Only three vectors use an IST stack: the
//! double fault (it may come from a kernel stack overflow), and the NMI and
//! machine check, which cannot be masked and may arrive while `syscall` is
//! still on the user's stack. Every other handler, the timer above all, may
//! switch threads, which needs the trap frame on the thread's own stack.
//!
//! # Entry and exit
//!
//! `isr_common` executes `swapgs` when the saved CS is a ring-3 selector, so
//! GS base points at the per-CPU area in the kernel (`syscall.rs`), clears
//! RFLAGS.AC when SMAP is on (an interrupted user copy gets it back with its
//! RFLAGS), and starts the kernel's frame-pointer chain at 0 when it came
//! from ring 3 (a user RBP means nothing to the backtrace). `trap_return`
//! pops the frame and swaps GS back when returning to ring 3.
//!
//! # Exceptions
//!
//! An exception in ring 3 kills the thread (`user.rs`), unless it is a page
//! fault that demand paging resolves. In ring 0 it is a kernel bug and
//! panics, except a page fault on user memory inside a user-copy routine,
//! which brings the page in or resumes at the routine's fixup
//! (`usercopy.rs`), and a #GP from the IRETQ of a return to ring 3, which
//! kills the returning thread.

use super::gdt::{DOUBLE_FAULT_IST, DescriptorTablePointer, KERNEL_CODE, NMI_IST};
use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

/// Saved CPU state, in the order the stubs push it (lowest address first).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TrapFrame {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    pub vector: u64,
    pub error_code: u64,
    // Pushed by the CPU:
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

pub const NMI: u64 = 2;
pub const BREAKPOINT: u64 = 3;
pub const DOUBLE_FAULT: u64 = 8;
pub const GENERAL_PROTECTION: u64 = 13;
pub const PAGE_FAULT: u64 = 14;
pub const MACHINE_CHECK: u64 = 18;

const EXCEPTION_NAMES: [&str; 32] = [
    "divide error",
    "debug",
    "non-maskable interrupt",
    "breakpoint",
    "overflow",
    "bound range exceeded",
    "invalid opcode",
    "device not available",
    "double fault",
    "coprocessor segment overrun",
    "invalid TSS",
    "segment not present",
    "stack-segment fault",
    "general protection fault",
    "page fault",
    "reserved",
    "x87 floating-point exception",
    "alignment check",
    "machine check",
    "SIMD floating-point exception",
    "virtualization exception",
    "control protection exception",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "reserved",
    "hypervisor injection exception",
    "VMM communication exception",
    "security exception",
    "reserved",
];

/// Name of exception vector `v` (0-31).
pub fn exception_name(v: u8) -> &'static str {
    EXCEPTION_NAMES.get(usize::from(v)).copied().unwrap_or("unknown")
}

/// Number of breakpoint exceptions handled; used by the boot self-test.
pub static BREAKPOINTS: AtomicU64 = AtomicU64::new(0);

global_asm!(
    r#"
    .macro ISR_NOERR n
isr_stub_\n:
    pushq $0
    pushq $\n
    jmp isr_common
    .endm

    .macro ISR_ERR n
isr_stub_\n:
    pushq $\n
    jmp isr_common
    .endm

    .section .text.isr, "ax", @progbits
    ISR_NOERR 0
    ISR_NOERR 1
    ISR_NOERR 2
    ISR_NOERR 3
    ISR_NOERR 4
    ISR_NOERR 5
    ISR_NOERR 6
    ISR_NOERR 7
    ISR_ERR   8
    ISR_NOERR 9
    ISR_ERR   10
    ISR_ERR   11
    ISR_ERR   12
    ISR_ERR   13
    ISR_ERR   14
    ISR_NOERR 15
    ISR_NOERR 16
    ISR_ERR   17
    ISR_NOERR 18
    ISR_NOERR 19
    ISR_NOERR 20
    ISR_ERR   21
    ISR_NOERR 22
    ISR_NOERR 23
    ISR_NOERR 24
    ISR_NOERR 25
    ISR_NOERR 26
    ISR_NOERR 27
    ISR_NOERR 28
    ISR_ERR   29
    ISR_ERR   30
    ISR_NOERR 31

    /* Vectors 32-255: no error code. One stub every 16 bytes from
       irq_stubs, so the IDT entry for vector v is irq_stubs + 16 * (v - 32).
       Each is at most 12 bytes: two pushes (2 and 2 or 5 bytes) and a jmp
       (5). The assembler substitutes the current value of irq_stub_vector
       at each use, as it is an absolute symbol. */
    .balign 16
    .global irq_stubs
irq_stubs:
    .set irq_stub_vector, 32
    .rept 224
    .balign 16
    pushq $0
    pushq $irq_stub_vector
    jmp isr_common
    .set irq_stub_vector, irq_stub_vector + 1
    .endr
    .balign 16
    .global irq_stubs_end
irq_stubs_end:

isr_common:
    testb $3, 24(%rsp)          /* saved CS: from ring 3? */
    jz 1f
    swapgs
1:
    push %rax
    push %rbx
    push %rcx
    push %rdx
    push %rsi
    push %rdi
    push %rbp
    push %r8
    push %r9
    push %r10
    push %r11
    push %r12
    push %r13
    push %r14
    push %r15
    cmpb $0, {smap}(%rip)
    je 2f
    clac
2:
    mov %rsp, %rdi
    testb $3, 144(%rsp)         /* saved CS, now above 15 registers */
    jz 3f
    xor %ebp, %ebp
3:
    cld
    call {dispatch}

    /* The one way out of the kernel to an interrupted context or to ring 3
       (the syscall entry and enter_user_frame jump here too): rsp points
       at a TrapFrame. */
    .global trap_return
trap_return:
    pop %r15
    pop %r14
    pop %r13
    pop %r12
    pop %r11
    pop %r10
    pop %r9
    pop %r8
    pop %rbp
    pop %rdi
    pop %rsi
    pop %rdx
    pop %rcx
    pop %rbx
    pop %rax
    add $16, %rsp               /* vector + error code */
    testb $3, 8(%rsp)           /* CS to return to: ring 3? */
    jz 4f
    swapgs
4:
    .global trap_return_iretq
trap_return_iretq:
    iretq

    .section .rodata.isr, "a", @progbits
    .balign 8
    .global isr_stub_table
isr_stub_table:
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad isr_stub_\n
    .endr
"#,
    dispatch = sym trap_dispatch,
    smap = sym super::usercopy::SMAP,
    options(att_syntax)
);

unsafe extern "C" {
    static trap_return_iretq: u8;
    static isr_stub_table: [u64; 32];
    static irq_stubs: u8;
    static irq_stubs_end: u8;
}

/// Address of the IRETQ instruction of the common exit.
pub fn return_iretq() -> u64 {
    (&raw const trap_return_iretq) as u64
}

/// Bytes between the stubs of consecutive vectors from 32 up.
const IRQ_STUB_SIZE: u64 = 16;

/// First vector that is not a CPU exception.
pub const FIRST_INTERRUPT_VECTOR: u8 = 32;

/// Address of the entry stub for vector `vector` (32 or more).
pub fn irq_stub(vector: u8) -> u64 {
    debug_assert!(vector >= FIRST_INTERRUPT_VECTOR);
    (&raw const irq_stubs) as u64 + IRQ_STUB_SIZE * u64::from(vector - FIRST_INTERRUPT_VECTOR)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    ist: u8,
    type_attr: u8,
    offset_mid: u16,
    offset_high: u32,
    _zero: u32,
}

impl IdtEntry {
    const MISSING: IdtEntry =
        IdtEntry { offset_low: 0, selector: 0, ist: 0, type_attr: 0, offset_mid: 0, offset_high: 0, _zero: 0 };

    fn interrupt_gate(handler: u64, ist: u8) -> IdtEntry {
        IdtEntry {
            offset_low: handler as u16,
            selector: KERNEL_CODE,
            ist,
            type_attr: 0x8e, // present, DPL 0, 64-bit interrupt gate
            offset_mid: (handler >> 16) as u16,
            offset_high: (handler >> 32) as u32,
            _zero: 0,
        }
    }
}

static mut IDT: [IdtEntry; 256] = [IdtEntry::MISSING; 256];

/// # Safety
/// Call once on the boot CPU after [`super::gdt::init`].
pub unsafe fn init() {
    // SAFETY: single-threaded early boot; the stub table is static data.
    unsafe {
        let idt = (&raw mut IDT).cast::<IdtEntry>();
        for (vector, &stub) in isr_stub_table.iter().enumerate() {
            let ist = match vector as u64 {
                DOUBLE_FAULT => DOUBLE_FAULT_IST,
                NMI | MACHINE_CHECK => NMI_IST,
                _ => 0,
            };
            idt.add(vector).write(IdtEntry::interrupt_gate(stub, ist));
        }
        let stubs_len = (&raw const irq_stubs_end) as u64 - (&raw const irq_stubs) as u64;
        assert_eq!(stubs_len, IRQ_STUB_SIZE * (256 - u64::from(FIRST_INTERRUPT_VECTOR)), "irq stubs misassembled");
        for vector in FIRST_INTERRUPT_VECTOR..=u8::MAX {
            idt.add(vector.into()).write(IdtEntry::interrupt_gate(irq_stub(vector), 0));
        }
        let ptr = DescriptorTablePointer { limit: (size_of::<[IdtEntry; 256]>() - 1) as u16, base: idt as u64 };
        asm!("lidt [{}]", in(reg) &ptr, options(readonly, nostack, preserves_flags));
    }
}

/// Called by `isr_common` with the frame it pushed. A raw pointer, not a
/// reference: on the way back to ring 3 the thread's personality may
/// rewrite the frame (a signal handler's registers) through
/// `user::set_user_regs`.
extern "C" fn trap_dispatch(frame: *mut TrapFrame) {
    // SAFETY: the stub passes the complete frame it just pushed on this
    // stack; nothing else refers to it during these reads.
    let (vector, from_user) = unsafe { ((*frame).vector, (*frame).in_user_mode()) };
    if vector >= u64::from(FIRST_INTERRUPT_VECTOR) {
        super::irq::set_interrupted_user(from_user);
        super::irq::dispatch(vector as u8);
        if from_user {
            crate::user::return_to_user();
        }
        return;
    }
    // An NMI or machine check that happens to arrive in ring 3 is not the
    // thread's doing (and runs on an IST stack): it is handled, or not, as
    // in ring 0.
    if from_user && !matches!(vector, NMI | MACHINE_CHECK | DOUBLE_FAULT) {
        // Returns only if the thread goes on: a page fault was resolved and
        // the instruction is retried, or a signal handler runs next.
        // SAFETY: as above; the handler gets a copy.
        super::user::exception(unsafe { *frame });
        crate::user::return_to_user();
        return;
    }
    // SAFETY: a fault in ring 0: nothing else refers to the frame, and the
    // handlers below may change it (a user-copy routine's fixup).
    let frame = unsafe { &mut *frame };
    if frame.vector == PAGE_FAULT && super::usercopy::page_fault(frame) {
        return;
    }
    match frame.vector {
        BREAKPOINT => {
            BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
            crate::kprintln!("breakpoint at {:#x}", frame.rip);
        }
        v => {
            if v == GENERAL_PROTECTION {
                super::user::failed_return(frame);
            }
            let name = exception_name(v as u8);
            // A double fault is usually a page fault that could not be
            // delivered, so CR2 still says where that one hit.
            let cr2 = if v == PAGE_FAULT || v == DOUBLE_FAULT { super::cpu::read_cr2() } else { 0 };
            let overflow = StackOverflow([cr2, frame.rsp].into_iter().find_map(|addr| {
                let fixed = super::paging::stack_guard_hit(addr).map(Guard::Fixed);
                fixed.or_else(|| crate::thread::stack_guard_hit(addr).map(Guard::Thread))
            }));
            super::backtrace::set_fault_origin(frame.rip, frame.rbp);
            panic!(
                "CPU exception {v} ({name}), error {:#x}, rip {:#x}, rsp {:#x}, cr2 {cr2:#x}{overflow}\n{frame:#x?}",
                frame.error_code, frame.rip, frame.rsp
            );
        }
    }
}

/// A stack guard a fault hit.
enum Guard {
    /// The guard page below one of the stacks in the kernel image (boot,
    /// double fault, NMI), by name.
    Fixed(&'static str),
    /// The guard below a kernel thread's stack.
    Thread(crate::thread::Overflowed),
}

/// Names the stack whose guard a fault hit, if any.
struct StackOverflow(Option<Guard>);

impl core::fmt::Display for StackOverflow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.0 {
            Some(Guard::Fixed(stack)) => {
                write!(f, "\nkernel stack overflow: hit the guard page below the {stack} stack")
            }
            Some(Guard::Thread(thread)) => {
                write!(f, "\nkernel stack overflow in thread {thread}: hit the guard below its stack")
            }
            None => Ok(()),
        }
    }
}
