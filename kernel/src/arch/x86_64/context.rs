//! Kernel thread context switch (System V AMD64 ABI).
//!
//! A thread that is not running is described entirely by its saved stack
//! pointer. To the compiler, [`switch`] is an ordinary call, so the caller
//! has already saved every caller-saved register it still needs; `switch`
//! pushes the six callee-saved ones (rbp, rbx, r12-r15) on the old stack,
//! stores rsp, loads the new thread's rsp and pops its six registers. Its
//! `ret` then returns to wherever the new thread called `switch` from or, for
//! a thread that has never run, into `thread_trampoline` (see [`prepare`]).
//!
//! Deliberately not saved:
//! - rflags: every switch happens with interrupts disabled (and the direction
//!   flag clear, as at any call); each thread restores its own interrupt
//!   state when it leaves the critical section the switch returned into.
//! - Segment registers: every kernel thread uses the same flat segments.
//! - FPU/SSE state: the kernel target (x86_64-unknown-none) is soft-float and
//!   never touches it. User threads will need xsave/xrstor here, plus
//!   TSS.rsp0 and the FS/GS bases.

use core::arch::global_asm;
use core::mem::size_of;

global_asm!(
    r#"
    .section .text.thread_switch, "ax", @progbits
    .global thread_switch
thread_switch:
    push rbp
    push rbx
    push r12
    push r13
    push r14
    push r15
    mov [rdi], rsp
    mov rsp, rsi
    pop r15
    pop r14
    pop r13
    pop r12
    pop rbx
    pop rbp
    ret

    /* First code a new thread runs, entered by thread_switch's ret with the
       registers from its FirstRun frame: rbp = 0 ends the frame-pointer
       chain here, r14 is the start function, r12 and r13 its arguments. */
    .global thread_trampoline
thread_trampoline:
    mov rdi, r12
    mov rsi, r13
    call r14
    .global thread_trampoline_return
thread_trampoline_return:
    ud2
"#
);

unsafe extern "sysv64" {
    fn thread_switch(save_rsp: *mut u64, load_rsp: u64);
    static thread_trampoline: u8;
    static thread_trampoline_return: u8;
}

/// What `thread_switch` pops for a thread that has never run, lowest
/// address first.
#[repr(C)]
struct FirstRun {
    r15: u64,
    r14: u64,
    r13: u64,
    r12: u64,
    rbx: u64,
    rbp: u64,
    /// `thread_switch` returns here: `thread_trampoline`.
    ret: u64,
    /// Leaves rsp 16-byte aligned at the trampoline's `call`, as the ABI
    /// requires, and is the zero "return address" above the last frame.
    pad: [u64; 2],
}

// Stack tops are 16-byte aligned, so the frame must be 8 mod 16 bytes long
// for rsp (top - 72 + 56 after six pops and the ret) to be 16-aligned at the
// trampoline's call.
const _: () = assert!(size_of::<FirstRun>() == 72);

/// Write the first-run frame for a new thread at the top of its stack and
/// return the stack pointer to give [`switch`]. The thread starts by calling
/// `start(arg0, arg1)` with the sysv64 convention, interrupts disabled and an
/// empty frame-pointer chain; `start` must never return.
///
/// # Safety
/// `stack_top` is the 16-byte aligned end of a mapped, writable stack that
/// nothing is using, and `start` is the address of an
/// `extern "sysv64" fn(usize, usize) -> !` (or one taking two pointer-sized
/// arguments in the same registers).
pub unsafe fn prepare(stack_top: u64, start: usize, arg0: usize, arg1: usize) -> u64 {
    assert!(stack_top.is_multiple_of(16), "stack top not 16-byte aligned");
    let frame = (stack_top - size_of::<FirstRun>() as u64) as *mut FirstRun;
    let first = FirstRun {
        r15: 0,
        r14: start as u64,
        r13: arg1 as u64,
        r12: arg0 as u64,
        rbx: 0,
        rbp: 0,
        ret: (&raw const thread_trampoline) as u64,
        pad: [0; 2],
    };
    // SAFETY: the caller guarantees the top 72 bytes of the stack are mapped,
    // writable and unused; `frame` is 8-aligned because `stack_top` is.
    unsafe { frame.write(first) };
    frame as u64
}

/// Save the running thread's callee-saved registers and stack pointer to
/// `*save_rsp`, and resume the thread whose stack pointer is `load_rsp`.
/// Returns when some later switch resumes the saved thread.
///
/// # Safety
/// Interrupts are disabled. `save_rsp` is valid for writes. `load_rsp` came
/// from [`prepare`] for a thread that has not run yet, or was saved by
/// `switch` for a thread that is not running; either way its stack is still
/// mapped. Nothing that is still in use lives on the current stack unless
/// the current thread will be resumed.
pub unsafe fn switch(save_rsp: *mut u64, load_rsp: u64) {
    // SAFETY: forwarded from the caller; thread_switch follows the sysv64
    // ABI, preserving every callee-saved register across the call.
    unsafe { thread_switch(save_rsp, load_rsp) }
}

/// The return address `start` sees: the outermost frame of every thread.
pub fn trampoline_return() -> u64 {
    (&raw const thread_trampoline_return) as u64
}
