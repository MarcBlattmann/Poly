//! Interrupt descriptor table and CPU exception handling.
//!
//! Every vector enters through a small assembly stub that pushes a uniform
//! [`TrapFrame`] and calls [`trap_dispatch`]: vectors 0-31 are CPU
//! exceptions, handled here; 32-255 are external interrupts, IPIs and the
//! local APIC's own (timer, error, spurious), handed to [`super::irq`]. The
//! same frame layout will carry the syscall path later.
//!
//! All 256 entries are interrupt gates, so the CPU clears RFLAGS.IF on entry
//! and handlers never nest. Only the double fault uses an IST stack: an
//! interrupt handler may switch threads (the timer preempts), which needs
//! the trap frame to stay on the interrupted thread's own stack.

use super::gdt::{DOUBLE_FAULT_IST, DescriptorTablePointer, KERNEL_CODE};
use core::arch::{asm, global_asm};
use core::mem::size_of;
use core::sync::atomic::{AtomicU64, Ordering};

/// Saved CPU state, in the order the stubs push it (lowest address first).
#[repr(C)]
#[derive(Debug)]
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

pub const BREAKPOINT: u64 = 3;
pub const DOUBLE_FAULT: u64 = 8;
pub const PAGE_FAULT: u64 = 14;

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
    mov %rsp, %rdi
    cld
    call {dispatch}
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
    options(att_syntax)
);

unsafe extern "C" {
    static isr_stub_table: [u64; 32];
    static irq_stubs: u8;
    static irq_stubs_end: u8;
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
            let ist = if vector as u64 == DOUBLE_FAULT { DOUBLE_FAULT_IST } else { 0 };
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

extern "C" fn trap_dispatch(frame: &mut TrapFrame) {
    if frame.vector >= u64::from(FIRST_INTERRUPT_VECTOR) {
        super::irq::dispatch(frame.vector as u8);
        return;
    }
    match frame.vector {
        BREAKPOINT => {
            BREAKPOINTS.fetch_add(1, Ordering::Relaxed);
            crate::kprintln!("breakpoint at {:#x}", frame.rip);
        }
        v => {
            let name = EXCEPTION_NAMES.get(v as usize).copied().unwrap_or("unknown");
            // A double fault is usually a page fault that could not be
            // delivered, so CR2 still says where that one hit.
            let cr2 = if v == PAGE_FAULT || v == DOUBLE_FAULT { super::cpu::read_cr2() } else { 0 };
            let overflow = StackOverflow([cr2, frame.rsp].into_iter().find_map(super::paging::stack_guard_hit));
            super::backtrace::set_fault_origin(frame.rip, frame.rbp);
            panic!(
                "CPU exception {v} ({name}), error {:#x}, rip {:#x}, rsp {:#x}, cr2 {cr2:#x}{overflow}\n{frame:#x?}",
                frame.error_code, frame.rip, frame.rsp
            );
        }
    }
}

/// Names the stack whose guard page a fault hit, if any.
struct StackOverflow(Option<&'static str>);

impl core::fmt::Display for StackOverflow {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            Some(stack) => write!(f, "\nkernel stack overflow: hit the guard page below the {stack} stack"),
            None => Ok(()),
        }
    }
}
