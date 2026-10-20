//! Tiny user-mode programs for the kernel tests, written in assembly.
//!
//! They live in read-only kernel data (never executed there): a test copies
//! the whole blob, from `user_programs_start` to `user_programs_end`, into
//! a user page and starts a thread at one program's offset in it. Everything
//! is position-independent. Every program ends with a system call that
//! exits, or with an instruction that faults on purpose.
//!
//! The system call numbers are the test hook's own ([`Call`]), not any
//! operating system's: `tests::hook` handles them. One program,
//! [`Program::CallStoreTrap`], makes a call with a number of the caller's
//! choosing and is run with the default hook (the process's personality).

use core::arch::global_asm;

/// System calls the test hook understands (in RAX).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub enum Call {
    /// Record RAX, RDI, RSI, RDX, R10, R8, R9 and return [`RECORD_RESULT`].
    Record = 1,
    /// End the thread with the exit code in RDI.
    Exit = 2,
    /// Let other threads run; returns 0.
    Yield = 3,
    /// Set the FS base to RDI; returns 0, or 1 if refused.
    SetFs = 4,
    /// Resume at RDI instead of after the `syscall`.
    ReturnTo = 5,
    /// Run the test's callback in this thread and exit with its result.
    Callback = 6,
    /// Set the GS base to RDI; returns 0, or 1 if refused.
    SetGs = 7,
    /// Make the return from this call fail in IRETQ
    /// (`arch::user::make_next_return_fail`).
    FailReturn = 8,
}

/// What [`Call::Record`] returns in RAX.
pub const RECORD_RESULT: u64 = 0x5afe_c0de_1234;

/// Exit code of a program that found something wrong.
pub const BAD: usize = 0xbad;

global_asm!(
    r#"
    .section .rodata.user_programs, "a", @progbits
    .balign 16
    .global user_programs_start
user_programs_start:

/* Registers: set the callee-saved registers, make a Record call with six
   known arguments, check that the call left the callee-saved registers,
   RSP and the return address in RCX as they were, then exit with what the
   call returned (or BAD). */
    .global user_prog_registers
user_prog_registers:
    mov rbx, 0x1111
    mov rbp, 0x2222
    mov r12, 0x3333
    mov r13, 0x4444
    mov r14, 0x5555
    mov r15, 0x6666
    mov qword ptr [rsp - 8], rsp
    mov eax, {record}
    mov edi, 0x10
    mov esi, 0x20
    mov edx, 0x30
    mov r10d, 0x40
    mov r8d, 0x50
    mov r9d, 0x60
    syscall
1:
    mov rdi, rax
    lea rdx, [rip + 1b]
    cmp rcx, rdx
    jne 9f
    cmp rbx, 0x1111
    jne 9f
    cmp rbp, 0x2222
    jne 9f
    cmp r12, 0x3333
    jne 9f
    cmp r13, 0x4444
    jne 9f
    cmp r14, 0x5555
    jne 9f
    cmp r15, 0x6666
    jne 9f
    cmp qword ptr [rsp - 8], rsp
    je 8f
9:
    mov edi, {bad}
8:
    mov eax, {exit}
    syscall
    ud2

/* Call with number RDI, store RAX at [RSI], then trap (#UD). */
    .global user_prog_call_store_trap
user_prog_call_store_trap:
    mov rax, rdi
    mov rbx, rsi
    syscall
    mov qword ptr [rbx], rax
    ud2

/* Each of these touches RDI (or does something only ring 0 may), then
   exits with 0 if that did not fault. */
    .global user_prog_load
user_prog_load:
    mov rax, qword ptr [rdi]
    jmp user_exit0
    .global user_prog_store
user_prog_store:
    mov qword ptr [rdi], rax
    jmp user_exit0
    .global user_prog_jump
user_prog_jump:
    jmp rdi
    .global user_prog_int80
user_prog_int80:
    int 0x80
    jmp user_exit0
    .global user_prog_int_timer
user_prog_int_timer:
    int 0xf0
    jmp user_exit0
    .global user_prog_hlt
user_prog_hlt:
    hlt
    jmp user_exit0
    .global user_prog_cli
user_prog_cli:
    cli
    jmp user_exit0
    .global user_prog_read_cr3
user_prog_read_cr3:
    mov rax, cr3
    jmp user_exit0
    .global user_prog_ud2
user_prog_ud2:
    ud2
user_exit0:
    xor edi, edi
    mov eax, {exit}
    syscall
    ud2

/* Run the test's callback in this thread (it exits the thread). */
    .global user_prog_callback
user_prog_callback:
    mov eax, {callback}
    syscall
    ud2

/* Read [RDI], yield ten times, read it again: exit with the value if both
   reads agree, else BAD. */
    .global user_prog_isolation
user_prog_isolation:
    mov rbx, rdi
    mov r12, qword ptr [rbx]
    mov r13, 10
1:
    mov eax, {yield_}
    syscall
    dec r13
    jnz 1b
    mov rdi, qword ptr [rbx]
    cmp rdi, r12
    je 2f
    mov edi, {bad}
2:
    mov eax, {exit}
    syscall
    ud2

/* SSE, MXCSR and the FS/GS bases across context switches. RDI = seed,
   RSI = FS base (GS base is RSI + 8), RDX = the value at FS:0 (GS:0 holds
   RDX + 1). Fill XMM0-15 with seed + n in both halves and MXCSR with a
   seed-dependent rounding mode, yield twenty times, check them all and
   both segment bases; exit 1 if all held, else BAD. */
    .global user_prog_sse
user_prog_sse:
    mov r12, rdi
    mov r13, rdx
    mov rbx, rsi
    mov rdi, rbx
    mov eax, {set_fs}
    syscall
    test rax, rax
    jnz 9f
    lea rdi, [rbx + 8]
    mov eax, {set_gs}
    syscall
    test rax, rax
    jnz 9f
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
    lea rax, [r12 + \n]
    movq xmm\n, rax
    punpcklqdq xmm\n, xmm\n
    .endr
    mov rax, r12
    and eax, 3
    shl eax, 13
    or eax, 0x1f80
    mov r15, rax
    sub rsp, 16
    mov dword ptr [rsp], eax
    ldmxcsr dword ptr [rsp]
    mov r14, 20
1:
    mov eax, {yield_}
    syscall
    dec r14
    jnz 1b
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15
    movdqu xmmword ptr [rsp], xmm\n
    lea rax, [r12 + \n]
    cmp qword ptr [rsp], rax
    jne 9f
    cmp qword ptr [rsp + 8], rax
    jne 9f
    .endr
    stmxcsr dword ptr [rsp]
    mov eax, dword ptr [rsp]
    cmp rax, r15
    jne 9f
    mov rax, qword ptr fs:[0]
    cmp rax, r13
    jne 9f
    mov rax, qword ptr gs:[0]
    dec rax
    cmp rax, r13
    jne 9f
    mov edi, 1
    jmp 8f
9:
    mov edi, {bad}
8:
    mov eax, {exit}
    syscall
    ud2

/* Spin in ring 3 until [RDI] is not 0, then exit with it. Only the
   timer gets the CPU away from it. */
    .global user_prog_spin
user_prog_spin:
    mov rax, qword ptr [rdi]
    test rax, rax
    jz user_prog_spin
    mov rdi, rax
    mov eax, {exit}
    syscall
    ud2

/* Ask to resume at RDI; exit 0x77 if the call came back here instead. */
    .global user_prog_return_to
user_prog_return_to:
    mov eax, {return_to}
    syscall
    mov edi, 0x77
    mov eax, {exit}
    syscall
    ud2
/* Make a FailReturn call; exit 0x77 if it came back. */
    .global user_prog_fail_return
user_prog_fail_return:
    mov eax, {fail_return}
    syscall
    mov edi, 0x77
    mov eax, {exit}
    syscall
    ud2
/* A place to resume at: exit 0x99. */
    .global user_prog_return_target
user_prog_return_target:
    mov edi, 0x99
    mov eax, {exit}
    syscall
    ud2

    .balign 16
    .global user_programs_end
user_programs_end:
"#,
    record = const Call::Record as u64,
    exit = const Call::Exit as u64,
    yield_ = const Call::Yield as u64,
    set_fs = const Call::SetFs as u64,
    set_gs = const Call::SetGs as u64,
    return_to = const Call::ReturnTo as u64,
    callback = const Call::Callback as u64,
    fail_return = const Call::FailReturn as u64,
    bad = const BAD,
);

unsafe extern "C" {
    static user_programs_start: u8;
    static user_programs_end: u8;
    static user_prog_registers: u8;
    static user_prog_call_store_trap: u8;
    static user_prog_load: u8;
    static user_prog_store: u8;
    static user_prog_jump: u8;
    static user_prog_int80: u8;
    static user_prog_int_timer: u8;
    static user_prog_hlt: u8;
    static user_prog_cli: u8;
    static user_prog_read_cr3: u8;
    static user_prog_ud2: u8;
    static user_prog_callback: u8;
    static user_prog_isolation: u8;
    static user_prog_sse: u8;
    static user_prog_spin: u8;
    static user_prog_return_to: u8;
    static user_prog_return_target: u8;
    static user_prog_fail_return: u8;
}

/// The programs, by where they start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    Registers,
    CallStoreTrap,
    Load,
    Store,
    Jump,
    Int80,
    IntTimer,
    Hlt,
    Cli,
    ReadCr3,
    Ud2,
    Callback,
    Isolation,
    Sse,
    Spin,
    ReturnTo,
    ReturnTarget,
    FailReturn,
}

impl Program {
    /// Offset of the program's first instruction in [`blob`].
    pub fn offset(self) -> u64 {
        let start = match self {
            Program::Registers => &raw const user_prog_registers,
            Program::CallStoreTrap => &raw const user_prog_call_store_trap,
            Program::Load => &raw const user_prog_load,
            Program::Store => &raw const user_prog_store,
            Program::Jump => &raw const user_prog_jump,
            Program::Int80 => &raw const user_prog_int80,
            Program::IntTimer => &raw const user_prog_int_timer,
            Program::Hlt => &raw const user_prog_hlt,
            Program::Cli => &raw const user_prog_cli,
            Program::ReadCr3 => &raw const user_prog_read_cr3,
            Program::Ud2 => &raw const user_prog_ud2,
            Program::Callback => &raw const user_prog_callback,
            Program::Isolation => &raw const user_prog_isolation,
            Program::Sse => &raw const user_prog_sse,
            Program::Spin => &raw const user_prog_spin,
            Program::ReturnTo => &raw const user_prog_return_to,
            Program::ReturnTarget => &raw const user_prog_return_target,
            Program::FailReturn => &raw const user_prog_fail_return,
        };
        start as u64 - (&raw const user_programs_start) as u64
    }
}

/// All the programs' code, to be copied into a user page.
pub fn blob() -> &'static [u8] {
    let start = &raw const user_programs_start;
    let len = (&raw const user_programs_end) as usize - start as usize;
    // SAFETY: the assembly above puts all the programs between the two
    // symbols, in one read-only section that is part of the kernel image.
    unsafe { core::slice::from_raw_parts(start, len) }
}
