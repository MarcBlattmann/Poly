//! A self-assembled ring-3 PE that drives the NT thread, synchronisation and
//! APC services end to end (ADR 0065). No Microsoft toolchain or DLLs: the
//! code is position-independent and makes the NT system calls directly, the
//! way `ntdll`'s stubs would.
//!
//! The main thread allocates a shared scratch region and passes its address
//! to the child threads as their argument, so every thread names the same
//! slots. It reports a failed step with `PolyTrace(0xBADBAD, step, status)`
//! and ends with [`FAIL_CODE`]; on success it ends with [`PASS_CODE`].
//!
//! Steps:
//! 1. Allocate scratch; create an auto-reset event and an unowned mutant.
//! 2. `NtCreateThreadEx` a child that acquires the mutant, signals the event
//!    and exits with code `0x77` without releasing the mutant.
//! 3. Block in `NtWaitForSingleObject` on the event until the child signals
//!    it (a cross-thread wake); then block on the child's thread handle until
//!    it exits (a thread object signalled at exit).
//! 4. `NtQueryInformationThread` reports the child's exit code `0x77`.
//! 5. Waiting on the mutant the child abandoned returns
//!    `STATUS_ABANDONED_WAIT_0`.
//! 6. A semaphore and a signalled event satisfy a `WaitAll`
//!    `NtWaitForMultipleObjects`.
//! 7. A user APC queued to this thread runs at `NtTestAlert`, which returns
//!    `STATUS_USER_APC`.
//! 8. A periodic synchronization timer is waited on twice (it re-arms).
//! 9. A spinning child is suspended, `NtGetContextThread` reads its RIP, and
//!    it is resumed and joined.

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_personality_nt::pe_writer::{self, PeImage, PeSection};

pub const PASS_CODE: u8 = 0x5A;
pub const FAIL_CODE: u8 = 0xEE;
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

global_asm!(
    r#"
    .section .rodata.poly_nt_threads, "a", @progbits
    .balign 16
    .global poly_nt_threads_code
poly_nt_threads_code:
    sub rsp, 0x400

    // ----- 1. scratch, event, mutant -------------------------------------
    mov r15d, 1
    mov qword ptr [rsp+0x100], 0
    mov qword ptr [rsp+0x108], 0x10000
    mov r10, -1
    lea rdx, [rsp+0x100]
    xor r8d, r8d
    lea r9, [rsp+0x108]
    mov qword ptr [rsp+0x28], 0x3000
    mov qword ptr [rsp+0x30], 4
    mov eax, 0x40
    syscall
    test eax, eax
    jnz fail
    mov r14, [rsp+0x100]
    mov qword ptr [r14+0x38], 0
    mov qword ptr [r14+0x50], 0

    // NtCreateEvent(&ev, EVENT_ALL_ACCESS, NULL, SynchronizationEvent, FALSE)
    mov r15d, 2
    lea r10, [r14+0x00]
    mov edx, 0x1F0003
    xor r8d, r8d
    mov r9d, 1
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x0A
    syscall
    test eax, eax
    jnz fail

    // NtCreateMutant(&mutant, MUTANT_ALL_ACCESS, NULL, FALSE)
    mov r15d, 3
    lea r10, [r14+0x10]
    mov edx, 0x1F0001
    xor r8d, r8d
    xor r9d, r9d
    mov eax, 0x20
    syscall
    test eax, eax
    jnz fail

    // ----- 2. create child1 ----------------------------------------------
    mov r15d, 4
    lea r10, [r14+0x08]
    mov edx, 0x1FFFFF
    xor r8d, r8d
    mov r9, -1
    lea rax, [rip+child1]
    mov [rsp+0x28], rax
    mov [rsp+0x30], r14
    mov qword ptr [rsp+0x38], 0
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0x82
    syscall
    test eax, eax
    jnz fail

    // ----- 3. wait on the event, then on the child -----------------------
    mov r15d, 5
    mov r10, [r14+0x00]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz fail

    mov r15d, 6
    mov r10, [r14+0x08]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz fail

    // ----- 4. child exit code --------------------------------------------
    mov r15d, 7
    mov r10, [r14+0x08]
    xor edx, edx
    lea r8, [r14+0x58]
    mov r9d, 48
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x86
    syscall
    test eax, eax
    jnz fail
    cmp dword ptr [r14+0x58], 0x77
    jne fail

    // ----- 5. mutant abandoned -------------------------------------------
    mov r15d, 8
    mov r10, [r14+0x10]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    cmp eax, 0x80
    jne fail
    // release it (we now own it)
    mov r10, [r14+0x10]
    xor edx, edx
    mov eax, 0x22
    syscall

    // ----- 6. semaphore + event, WaitAll ---------------------------------
    mov r15d, 9
    lea r10, [r14+0x18]
    mov edx, 0x1F0003
    xor r8d, r8d
    mov r9d, 1
    mov qword ptr [rsp+0x28], 2
    mov eax, 0x24
    syscall
    test eax, eax
    jnz fail

    mov r15d, 10
    lea r10, [r14+0x20]
    mov edx, 0x1F0003
    xor r8d, r8d
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 1
    mov eax, 0x0A
    syscall
    test eax, eax
    jnz fail

    mov r15d, 11
    mov rax, [r14+0x18]
    mov [r14+0x600], rax
    mov rax, [r14+0x20]
    mov [r14+0x608], rax
    mov r10d, 2
    lea rdx, [r14+0x600]
    xor r8d, r8d
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x2E
    syscall
    test eax, eax
    jnz fail

    // ----- 7. APC to self, delivered at NtTestAlert ----------------------
    mov r15d, 12
    mov r10, -2
    lea rdx, [rip+apc_routine]
    mov r8, r14
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x30
    syscall
    test eax, eax
    jnz fail

    mov r15d, 13
    mov eax, 0x31
    syscall
    cmp eax, 0xC0
    jne fail
    cmp qword ptr [r14+0x50], 0xABC
    jne fail

    // ----- 8. periodic timer ---------------------------------------------
    mov r15d, 14
    lea r10, [r14+0x28]
    mov edx, 0x1F0003
    xor r8d, r8d
    mov r9d, 1
    mov eax, 0x28
    syscall
    test eax, eax
    jnz fail

    mov r15d, 15
    mov qword ptr [r14+0x48], -100000
    mov r10, [r14+0x28]
    lea rdx, [r14+0x48]
    xor r8d, r8d
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 10
    mov qword ptr [rsp+0x38], 0
    mov eax, 0x2A
    syscall
    test eax, eax
    jnz fail

    mov r15d, 16
    mov r10, [r14+0x28]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz fail

    mov r15d, 17
    mov r10, [r14+0x28]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz fail

    mov r10, [r14+0x28]
    xor edx, edx
    mov eax, 0x2B
    syscall

    // ----- 9. suspend a spinning child, read its context -----------------
    mov r15d, 18
    lea r10, [r14+0x30]
    mov edx, 0x1FFFFF
    xor r8d, r8d
    mov r9, -1
    lea rax, [rip+child2]
    mov [rsp+0x28], rax
    mov [rsp+0x30], r14
    mov qword ptr [rsp+0x38], 0
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0x82
    syscall
    test eax, eax
    jnz fail

    mov r15d, 19
    mov r10, [r14+0x30]
    xor edx, edx
    mov eax, 0x87
    syscall
    test eax, eax
    jnz fail

    mov r15d, 20
    mov dword ptr [r14+0x90+0x30], 0x00100003
    mov r10, [r14+0x30]
    lea rdx, [r14+0x90]
    mov eax, 0x89
    syscall
    test eax, eax
    jnz fail
    mov rax, [r14+0x90+0xF8]
    test rax, rax
    jz fail

    mov r15d, 21
    mov r10, [r14+0x30]
    xor edx, edx
    mov eax, 0x88
    syscall
    test eax, eax
    jnz fail

    mov qword ptr [r14+0x38], 1

    mov r15d, 22
    mov r10, [r14+0x30]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz fail

    // ----- success -------------------------------------------------------
    mov r10, 0xC0FFEE
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x3000
    syscall
    mov r10, -1
    mov edx, {pass}
    mov eax, 0x80
    syscall
    ud2

fail:
    mov r8d, eax
    mov rdx, r15
    mov r10, 0xBADBAD
    mov eax, 0x3000
    syscall
    mov r10, -1
    mov edx, {fail}
    mov eax, 0x80
    syscall
    ud2

    // child1(scratch in rcx): own the mutant, signal the event, exit 0x77
    // without releasing the mutant (so it is abandoned).
    .balign 16
child1:
    mov r14, rcx
    sub rsp, 0x400
    mov r10, [r14+0x10]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    mov r10, [r14+0x00]
    xor edx, edx
    mov eax, 0x0C
    syscall
    add rsp, 0x400
    mov eax, 0x77
    ret

    // child2(scratch in rcx): spin until [scratch+0x38] is set, then exit.
    .balign 16
child2:
    mov r14, rcx
    sub rsp, 0x400
2:
    mov rax, [r14+0x38]
    test rax, rax
    jz 2b
    add rsp, 0x400
    mov eax, 0x55
    ret

    // apc_routine(scratch in rcx): record that it ran.
    .balign 16
apc_routine:
    mov qword ptr [rcx+0x50], 0xABC
    ret

    .global poly_nt_threads_code_end
poly_nt_threads_code_end:
"#,
    pass = const PASS_CODE as u32,
    fail = const FAIL_CODE as u32,
);

unsafe extern "C" {
    static poly_nt_threads_code: u8;
    static poly_nt_threads_code_end: u8;
}

fn code() -> &'static [u8] {
    let start = &raw const poly_nt_threads_code;
    let end = &raw const poly_nt_threads_code_end;
    // SAFETY: the two symbols bound the code in a read-only section of the
    // kernel image.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

/// The PE executable: one `.text` section, relocatable, a 64 KiB stack.
pub fn image() -> Vec<u8> {
    let mut data = code().to_vec();
    let virtual_size = data.len() as u32;
    data.resize(data.len().max(1), 0);
    PeImage {
        image_base: IMAGE_BASE,
        entry: 0x1000,
        sections: vec![PeSection { name: *b".text\0\0\0", data, virtual_size, characteristics: pe_writer::TEXT }],
        relocations: Vec::new(),
        fixed: false,
        stack_reserve: 0x1_0000,
        stack_commit: 0x4000,
    }
    .write()
}
