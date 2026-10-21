//! A self-assembled ring-3 PE that drives the NT file I/O services end to
//! end (ADR 0067). No Microsoft toolchain or DLLs: the code is
//! position-independent and makes the NT system calls the way `ntdll`'s
//! stubs would (the service number in `eax`, arguments in `r10, rdx, r8,
//! r9` then the stack above `0x28`).
//!
//! `rbx` points at a scratch frame; the program reports a failed step with
//! `PolyTrace(0xBADBAD, step, status)` and ends with [`FAIL_CODE`], else
//! ends with [`PASS_CODE`] through `NtTerminateProcess`. Every label is
//! prefixed `nt_file_` so it never clashes with the other test PEs'
//! `global_asm!` (the linker puts them in one object).
//!
//! Steps, all under `C:` (`\Device\PolyFs1`, the VFS's `/nt/c`):
//!
//! 1. `NtCreateFile` a file, `NtWriteFile` 8 bytes, read them back,
//!    `NtClose` (synchronous I/O with a file position).
//! 2. Open the same file in a different case (`FILE.TXT`): case-insensitive
//!    lookup.
//! 3. A second open with a conflicting share mode →
//!    `STATUS_SHARING_VIOLATION`.
//! 4. Create a directory, two files in it, and enumerate it with the
//!    wildcard `*.txt` (`NtQueryDirectoryFile`).
//! 5. An asynchronous (overlapped) `NtReadFile` completing through an event.
//! 6. An asynchronous `NtReadFile` completing through an APC, run at an
//!    alertable `NtDelayExecution` (`STATUS_USER_APC`).
//! 7. A pending read on a `\Device\PolyPipe` stream (a slow device),
//!    cancelled with `NtCancelIoFile` → `STATUS_CANCELLED`.
//! 8. A byte-range lock on one handle makes a conflicting write on another
//!    fail with `STATUS_FILE_LOCK_CONFLICT`, and a conflicting lock with
//!    `STATUS_LOCK_NOT_GRANTED`.
//! 9. `FILE_DELETE_ON_CLOSE`: the file is gone after the handle closes.

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_personality_nt::pe_writer::{self, PeImage, PeSection};

pub const PASS_CODE: u8 = 0x5A;
pub const FAIL_CODE: u8 = 0xEE;
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

global_asm!(
    r#"
    .section .rodata.poly_nt_io, "a", @progbits
    .balign 16
    .global poly_nt_io_code
poly_nt_io_code:
    sub rsp, 0x2000
    mov rbx, rsp

    // ----- 1. create, write, read, close ---------------------------------
    mov r15d, 1
    lea rax, [rip+nt_file_file]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 30
    mov word ptr [rbx+0x152], 30
    call nt_file_build_oa
    // NtCreateFile(&h, 0x1F01FF, &oa, &iosb, 0, FILE_ATTRIBUTE_NORMAL,
    //   FILE_SHARE_ALL, FILE_CREATE, SYNC|NON_DIR, 0, 0)
    lea r10, [rbx+0x100]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0x80
    mov qword ptr [rsp+0x38], 7
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0x60
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0xC0
    syscall
    test eax, eax
    jnz nt_file_fail

    // Write "PolyData" (8 bytes) at the file position.
    mov r15d, 2
    mov rax, 0x61746144796C6F50
    mov [rbx+0x190], rax
    mov qword ptr [rbx+0x180], 0
    mov r10, [rbx+0x100]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x190]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 8
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC3
    syscall
    test eax, eax
    jnz nt_file_fail
    cmp qword ptr [rbx+0x168], 8
    jne nt_file_fail

    // Read 8 bytes at offset 0 and compare.
    mov r15d, 3
    mov qword ptr [rbx+0x198], 0
    mov qword ptr [rbx+0x180], 0
    mov r10, [rbx+0x100]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x198]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 8
    lea rax, [rbx+0x180]
    mov [rsp+0x40], rax
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC2
    syscall
    test eax, eax
    jnz nt_file_fail
    mov rax, [rbx+0x198]
    mov rcx, 0x61746144796C6F50
    cmp rax, rcx
    jne nt_file_fail
    // NtClose(h)
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // ----- 2. mixed-case open --------------------------------------------
    mov r15d, 4
    lea rax, [rip+nt_file_file_uc]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 30
    mov word ptr [rbx+0x152], 30
    call nt_file_build_oa
    // NtOpenFile(&h, FILE_GENERIC_READ, &oa, &iosb, SHARE_ALL, SYNC|NON_DIR)
    lea r10, [rbx+0x100]
    mov edx, 0x120089
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail

    // ----- 3. sharing violation ------------------------------------------
    // h is open GENERIC_READ above with SHARE_ALL; now open for write with
    // only FILE_SHARE_READ: the existing writer-less open shares write, but
    // this open asks write while sharing only read, and the existing open
    // does not share write -> violation.
    mov r15d, 5
    // Reopen the first handle restricting its share to read only.
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall
    lea rax, [rip+nt_file_file]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 30
    mov word ptr [rbx+0x152], 30
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x120089
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 1
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail
    // Second open: GENERIC_WRITE, FILE_SHARE_READ.
    call nt_file_build_oa
    lea r10, [rbx+0x108]
    mov edx, 0x120116
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x170]
    mov qword ptr [rsp+0x28], 1
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    cmp eax, 0xC0000043
    jne nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // ----- 4. directory enumeration with a wildcard ----------------------
    mov r15d, 6
    lea rax, [rip+nt_file_dir]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 18
    mov word ptr [rbx+0x152], 18
    call nt_file_build_oa
    // NtCreateFile(&h, LIST|SYNCHRONIZE, &oa, &iosb, 0, 0, SHARE_ALL,
    //   FILE_CREATE, FILE_DIRECTORY_FILE|SYNC, 0, 0)
    lea r10, [rbx+0x100]
    mov edx, 0x100001
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    mov qword ptr [rsp+0x38], 7
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0x21
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0xC0
    syscall
    test eax, eax
    jnz nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // Two files in the directory.
    mov r15d, 7
    lea rax, [rip+nt_file_dir_a]
    mov r13d, 30
    call nt_file_touch
    lea rax, [rip+nt_file_dir_b]
    mov r13d, 30
    call nt_file_touch

    // Open the directory and enumerate with "*.txt".
    mov r15d, 8
    lea rax, [rip+nt_file_dir]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 18
    mov word ptr [rbx+0x152], 18
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x100001
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x21
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail
    // Pattern UNICODE_STRING "*.txt" at 0x1C0.
    lea rax, [rip+nt_file_star_txt]
    mov [rbx+0x1C8], rax
    mov word ptr [rbx+0x1C0], 10
    mov word ptr [rbx+0x1C2], 10
    // NtQueryDirectoryFile(h, 0,0,0, &iosb, &buf, 0x200,
    //   FileDirectoryInformation(1), single=0, &pattern, restart=1)
    mov r15d, 9
    mov r10, [rbx+0x100]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x400]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 0x200
    mov qword ptr [rsp+0x40], 1
    mov qword ptr [rsp+0x48], 0
    lea rax, [rbx+0x1C0]
    mov [rsp+0x50], rax
    mov qword ptr [rsp+0x58], 1
    mov eax, 0xC6
    syscall
    test eax, eax
    jnz nt_file_fail
    // Exactly one entry (NextEntryOffset 0), FileNameLength 10, first char 'a'.
    cmp dword ptr [rbx+0x400], 0
    jne nt_file_fail
    cmp dword ptr [rbx+0x43C], 10
    jne nt_file_fail
    cmp word ptr [rbx+0x440], 0x61
    jne nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // ----- 5. asynchronous read through an event -------------------------
    mov r15d, 10
    // NtCreateEvent(&ev, 0x1F0003, 0, NotificationEvent, FALSE)
    lea r10, [rbx+0x110]
    mov edx, 0x1F0003
    xor r8d, r8d
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x0A
    syscall
    test eax, eax
    jnz nt_file_fail
    // Open file.txt asynchronously (no SYNC option).
    lea rax, [rip+nt_file_file]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 30
    mov word ptr [rbx+0x152], 30
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x120089
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x40
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail
    // NtReadFile(h, ev, 0, 0, &iosb, &buf, 8, &off0, 0)
    mov r15d, 11
    mov qword ptr [rbx+0x198], 0
    mov qword ptr [rbx+0x180], 0
    mov r10, [rbx+0x100]
    mov rdx, [rbx+0x110]
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x198]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 8
    lea rax, [rbx+0x180]
    mov [rsp+0x40], rax
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC2
    syscall
    cmp eax, 0x103
    jne nt_file_fail
    // NtWaitForSingleObject(ev, FALSE, NULL)
    mov r15d, 12
    mov r10, [rbx+0x110]
    xor edx, edx
    xor r8d, r8d
    mov eax, 0x2D
    syscall
    test eax, eax
    jnz nt_file_fail
    cmp dword ptr [rbx+0x160], 0
    jne nt_file_fail
    cmp qword ptr [rbx+0x168], 8
    jne nt_file_fail
    mov rax, [rbx+0x198]
    mov rcx, 0x61746144796C6F50
    cmp rax, rcx
    jne nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall

    // ----- 6. asynchronous read through an APC ---------------------------
    mov r15d, 13
    lea rax, [rip+nt_file_file]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 30
    mov word ptr [rbx+0x152], 30
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x120089
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x40
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail
    // marker = 0; NtReadFile(h, 0, apc_routine, &marker, &iosb, &buf, 8, &off0, 0)
    mov r15d, 14
    mov qword ptr [rbx+0x118], 0
    mov qword ptr [rbx+0x180], 0
    mov r10, [rbx+0x100]
    xor edx, edx
    lea r8, [rip+nt_file_apc]
    lea r9, [rbx+0x118]
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x198]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 8
    lea rax, [rbx+0x180]
    mov [rsp+0x40], rax
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC2
    syscall
    cmp eax, 0x103
    jne nt_file_fail
    // NtDelayExecution(TRUE, &timeout) with a 3 s cap; the APC cuts it short.
    mov r15d, 15
    mov qword ptr [rbx+0x188], -30000000
    mov r10d, 1
    lea rdx, [rbx+0x188]
    mov eax, 0x32
    syscall
    cmp eax, 0xC0
    jne nt_file_fail
    cmp qword ptr [rbx+0x118], 0xA9C
    jne nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall
    mov r10, [rbx+0x110]
    xor eax, eax
    syscall

    // ----- 7. cancel a pending read on a pipe ----------------------------
    mov r15d, 16
    lea rax, [rip+nt_file_pipe]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 36
    mov word ptr [rbx+0x152], 36
    call nt_file_build_oa
    // NtCreateFile(&pipe, 0x1F01FF, &oa, &iosb, 0, 0, SHARE_ALL,
    //   FILE_OPEN_IF, 0 (async), 0, 0)
    lea r10, [rbx+0x1B0]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    mov qword ptr [rsp+0x38], 7
    mov qword ptr [rsp+0x40], 3
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0xC0
    syscall
    test eax, eax
    jnz nt_file_fail
    // A pending read (no data yet).
    mov r15d, 17
    mov dword ptr [rbx+0x170], 0
    mov r10, [rbx+0x1B0]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x170]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x198]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 8
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC2
    syscall
    cmp eax, 0x103
    jne nt_file_fail
    // NtCancelIoFile(pipe, &iosb_other)
    mov r15d, 18
    mov r10, [rbx+0x1B0]
    lea rdx, [rbx+0x160]
    mov eax, 0xCC
    syscall
    test eax, eax
    jnz nt_file_fail
    // The read's status block now holds STATUS_CANCELLED.
    cmp dword ptr [rbx+0x170], 0xC0000120
    jne nt_file_fail
    mov r10, [rbx+0x1B0]
    xor eax, eax
    syscall

    // ----- 8. byte-range lock conflict -----------------------------------
    mov r15d, 19
    lea rax, [rip+nt_file_file]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 30
    mov word ptr [rbx+0x152], 30
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail
    call nt_file_build_oa
    lea r10, [rbx+0x108]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x170]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    test eax, eax
    jnz nt_file_fail
    // Lock [0,10) exclusive on h (fail immediately).
    mov r15d, 20
    mov qword ptr [rbx+0x1A0], 0
    mov qword ptr [rbx+0x1A8], 10
    mov r10, [rbx+0x100]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x160]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x1A0]
    mov [rsp+0x30], rax
    lea rax, [rbx+0x1A8]
    mov [rsp+0x38], rax
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 1
    mov qword ptr [rsp+0x50], 1
    mov eax, 0xC9
    syscall
    test eax, eax
    jnz nt_file_fail
    // A conflicting lock [5,10) on h2 -> STATUS_LOCK_NOT_GRANTED.
    mov r15d, 21
    mov qword ptr [rbx+0x1A0], 5
    mov r10, [rbx+0x108]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x170]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x1A0]
    mov [rsp+0x30], rax
    lea rax, [rbx+0x1A8]
    mov [rsp+0x38], rax
    mov qword ptr [rsp+0x40], 0
    mov qword ptr [rsp+0x48], 1
    mov qword ptr [rsp+0x50], 1
    mov eax, 0xC9
    syscall
    cmp eax, 0xC0000055
    jne nt_file_fail
    // A write on h2 inside the locked range -> STATUS_FILE_LOCK_CONFLICT.
    mov r15d, 22
    mov qword ptr [rbx+0x180], 0
    mov r10, [rbx+0x108]
    xor edx, edx
    xor r8d, r8d
    xor r9d, r9d
    lea rax, [rbx+0x170]
    mov [rsp+0x28], rax
    lea rax, [rbx+0x190]
    mov [rsp+0x30], rax
    mov qword ptr [rsp+0x38], 4
    lea rax, [rbx+0x180]
    mov [rsp+0x40], rax
    mov qword ptr [rsp+0x48], 0
    mov eax, 0xC3
    syscall
    cmp eax, 0xC0000054
    jne nt_file_fail
    // Unlock and close both.
    mov r15d, 23
    mov qword ptr [rbx+0x1A0], 0
    mov r10, [rbx+0x100]
    lea rdx, [rbx+0x160]
    lea r8, [rbx+0x1A0]
    lea r9, [rbx+0x1A8]
    mov qword ptr [rsp+0x28], 0
    mov eax, 0xCA
    syscall
    test eax, eax
    jnz nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall
    mov r10, [rbx+0x108]
    xor eax, eax
    syscall

    // ----- 9. delete-on-close --------------------------------------------
    mov r15d, 24
    lea rax, [rip+nt_file_doomed]
    mov [rbx+0x158], rax
    mov word ptr [rbx+0x150], 34
    mov word ptr [rbx+0x152], 34
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    mov qword ptr [rsp+0x38], 7
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0x1060
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0xC0
    syscall
    test eax, eax
    jnz nt_file_fail
    mov r10, [rbx+0x100]
    xor eax, eax
    syscall
    // Reopening fails: the file is gone.
    mov r15d, 25
    call nt_file_build_oa
    lea r10, [rbx+0x100]
    mov edx, 0x120089
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 7
    mov qword ptr [rsp+0x30], 0x60
    mov eax, 0xC1
    syscall
    cmp eax, 0xC0000034
    jne nt_file_fail

    // ----- success -------------------------------------------------------
    mov r10, -1
    mov edx, {pass}
    mov eax, 0x80
    syscall
    ud2

    // nt_file_build_oa: OBJECT_ATTRIBUTES at [rbx+0x120], name at [rbx+0x150],
    // OBJ_CASE_INSENSITIVE.
nt_file_build_oa:
    mov dword ptr [rbx+0x120], 48
    mov qword ptr [rbx+0x128], 0
    lea rax, [rbx+0x150]
    mov [rbx+0x130], rax
    mov dword ptr [rbx+0x138], 0x40
    mov qword ptr [rbx+0x140], 0
    mov qword ptr [rbx+0x148], 0
    ret

    // nt_file_touch(rax = name ptr, r13d = name length in bytes): create an
    // empty file and close it. Clobbers the OA/name slots.
nt_file_touch:
    mov [rbx+0x158], rax
    mov [rbx+0x150], r13w
    mov [rbx+0x152], r13w
    call nt_file_build_oa
    lea r10, [rbx+0x108]
    mov edx, 0x1F01FF
    lea r8, [rbx+0x120]
    lea r9, [rbx+0x160]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    mov qword ptr [rsp+0x38], 7
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0x60
    mov qword ptr [rsp+0x50], 0
    mov qword ptr [rsp+0x58], 0
    mov eax, 0xC0
    syscall
    test eax, eax
    jnz nt_file_fail
    mov r10, [rbx+0x108]
    push rax
    xor eax, eax
    syscall
    pop rax
    ret

    // nt_file_apc(rcx = &marker, rdx = IoStatusBlock, r8 = 0): record that
    // it ran.
    .balign 16
nt_file_apc:
    mov qword ptr [rcx], 0xA9C
    ret

nt_file_fail:
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

    .balign 2
nt_file_file:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'f', 'i', 'l'
    .2byte 'e', '.', 't', 'x', 't'
nt_file_file_uc:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'F', 'I', 'L'
    .2byte 'E', '.', 'T', 'X', 'T'
nt_file_dir:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'e', 'd'
nt_file_dir_a:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'e', 'd', 0x5c
    .2byte 'a', '.', 't', 'x', 't'
nt_file_dir_b:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'e', 'd', 0x5c
    .2byte 'b', '.', 'l', 'o', 'g'
nt_file_star_txt:
    .2byte '*', '.', 't', 'x', 't'
nt_file_pipe:
    .2byte 0x5c, 'D', 'e', 'v', 'i', 'c', 'e', 0x5c, 'P', 'o'
    .2byte 'l', 'y', 'P', 'i', 'p', 'e', 0x5c, 'k'
nt_file_doomed:
    .2byte 0x5c, '?', '?', 0x5c, 'C', ':', 0x5c, 'd', 'o', 'o'
    .2byte 'm', 'e', 'd', '.', 't', 'x', 't'
    .global poly_nt_io_code_end
poly_nt_io_code_end:
"#,
    pass = const PASS_CODE as u32,
    fail = const FAIL_CODE as u32,
);

unsafe extern "C" {
    static poly_nt_io_code: u8;
    static poly_nt_io_code_end: u8;
}

fn code() -> &'static [u8] {
    let start = &raw const poly_nt_io_code;
    let end = &raw const poly_nt_io_code_end;
    // SAFETY: the two symbols bound the code in a read-only section of the
    // kernel image.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

/// The PE executable: one `.text` section, relocatable, a 128 KiB stack.
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
        stack_reserve: 0x2_0000,
        stack_commit: 0x8000,
    }
    .write()
}
