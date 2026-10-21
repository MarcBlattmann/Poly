//! A tiny freestanding x86-64 PE executable, written by Poly's own
//! toolchain (the kernel's assembler and [`poly_personality_nt::pe_writer`]),
//! that calls the NT system services directly from ring 3. No Microsoft
//! toolchain and no Microsoft DLLs are involved.
//!
//! The code is position-independent (RIP-relative), so the image maps at
//! its preferred base with no relocations. Its entry makes a sequence of NT
//! system calls — the x64 convention of `ntdll`'s stubs: the service number
//! in `eax`, the first four arguments in `r10, rdx, r8, r9`, the rest on the
//! stack above 0x28 bytes (shadow space and the return slot), the result an
//! `NTSTATUS` in `eax`. It checks each result and, on the first failure,
//! logs the step and status with `PolyTrace` and ends with
//! [`FAIL_CODE`]; if everything passes it ends with [`PASS_CODE`] through
//! `NtTerminateProcess`. A `kernel_test!` builds the PE, runs it and checks
//! the exit code.
//!
//! What it exercises (the Phase 5 object and memory items):
//!
//! 1. Create a named `Event` in `\BaseNamedObjects`, open it again by name,
//!    duplicate the handle, close every handle; opening by name then fails
//!    with `STATUS_OBJECT_NAME_NOT_FOUND` (the reference counts reached zero
//!    and the object is gone).
//! 2. Open `\` and query the directory.
//! 3. Reserve+commit memory, write it, make it read-only, query it
//!    (`MemoryBasicInformation`), free it.
//! 4. Create a pagefile-backed section, map it twice in this process, write
//!    through one view and read it back through the other (one shared copy),
//!    unmap both.

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_personality_nt::pe_writer::{self, PeImage, PeSection};

/// The exit status the program ends with when every check passed.
pub const PASS_CODE: u8 = 0x5A;
/// The exit status on the first failed check.
pub const FAIL_CODE: u8 = 0xEE;
/// The image's preferred base.
pub const IMAGE_BASE: u64 = 0x1_4000_0000;

global_asm!(
    r#"
    .section .rodata.poly_nt_test, "a", @progbits
    .balign 16
    .global poly_nt_test_code
poly_nt_test_code:
    sub rsp, 0x1000
    mov rbx, rsp

    // ----- 1. named event -------------------------------------------------
    mov r15d, 1
    // UNICODE_STRING at [rbx+0x250] for the event name.
    lea rax, [rip+evt_name]
    mov [rbx+0x258], rax
    mov word ptr [rbx+0x250], 50
    mov word ptr [rbx+0x252], 50
    // OBJECT_ATTRIBUTES at [rbx+0x220].
    call build_oa
    // NtCreateEvent(&OUT_H, EVENT_ALL_ACCESS, &OA, NotificationEvent, FALSE)
    lea r10, [rbx+0x200]
    mov edx, 0x1F0003
    lea r8, [rbx+0x220]
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov eax, 0x0A
    syscall
    test eax, eax
    jnz fail

    // NtOpenEvent(&OUT_H2, EVENT_ALL_ACCESS, &OA)
    mov r15d, 2
    lea r10, [rbx+0x208]
    mov edx, 0x1F0003
    lea r8, [rbx+0x220]
    mov eax, 0x0B
    syscall
    test eax, eax
    jnz fail

    // NtDuplicateObject(-1, OUT_H2, -1, &OUT_DUP, 0, 0, DUPLICATE_SAME_ACCESS)
    mov r15d, 3
    mov r10, -1
    mov rdx, [rbx+0x208]
    mov r8, -1
    lea r9, [rbx+0x210]
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    mov qword ptr [rsp+0x38], 2
    mov eax, 0x01
    syscall
    test eax, eax
    jnz fail

    // NtClose each of the three handles.
    mov r15d, 4
    mov r10, [rbx+0x200]
    mov eax, 0
    syscall
    test eax, eax
    jnz fail
    mov r10, [rbx+0x208]
    mov eax, 0
    syscall
    test eax, eax
    jnz fail
    mov r10, [rbx+0x210]
    mov eax, 0
    syscall
    test eax, eax
    jnz fail

    // NtOpenEvent again: the object is gone (STATUS_OBJECT_NAME_NOT_FOUND).
    mov r15d, 5
    lea r10, [rbx+0x208]
    mov edx, 0x1F0003
    lea r8, [rbx+0x220]
    mov eax, 0x0B
    syscall
    cmp eax, 0xC0000034
    jne fail

    // ----- 2. directory ---------------------------------------------------
    mov r15d, 6
    lea rax, [rip+dir_name]
    mov [rbx+0x258], rax
    mov word ptr [rbx+0x250], 2
    mov word ptr [rbx+0x252], 2
    call build_oa
    // NtOpenDirectoryObject(&OUT_H, DIRECTORY_QUERY|TRAVERSE, &OA)
    lea r10, [rbx+0x200]
    mov edx, 3
    lea r8, [rbx+0x220]
    mov eax, 0x05
    syscall
    test eax, eax
    jnz fail
    // NtQueryDirectoryObject(OUT_H, &QBUF, 0x100, TRUE, TRUE, &QCTX, &QRET)
    mov r15d, 7
    mov qword ptr [rbx+0x2F0], 0
    mov r10, [rbx+0x200]
    lea rdx, [rbx+0x300]
    mov r8d, 0x100
    mov r9d, 1
    mov qword ptr [rsp+0x28], 1
    lea rax, [rbx+0x2F0]
    mov [rsp+0x30], rax
    lea rax, [rbx+0x2F8]
    mov [rsp+0x38], rax
    mov eax, 0x06
    syscall
    test eax, eax
    js fail
    // Close the directory handle.
    mov r10, [rbx+0x200]
    mov eax, 0
    syscall

    // ----- 3. virtual memory ---------------------------------------------
    mov r15d, 8
    mov qword ptr [rbx+0x260], 0
    mov qword ptr [rbx+0x268], 0x2000
    // NtAllocateVirtualMemory(-1, &BASE, 0, &SIZE, RESERVE|COMMIT, PAGE_RW)
    mov r10, -1
    lea rdx, [rbx+0x260]
    xor r8d, r8d
    lea r9, [rbx+0x268]
    mov qword ptr [rsp+0x28], 0x3000
    mov qword ptr [rsp+0x30], 4
    mov eax, 0x40
    syscall
    test eax, eax
    jnz fail
    mov r12, [rbx+0x260]
    mov rax, 0x1122334455667788
    mov [r12], rax

    // NtProtectVirtualMemory(-1, &TB, &TS, PAGE_READONLY, &OLD)
    mov r15d, 9
    mov [rbx+0x270], r12
    mov qword ptr [rbx+0x278], 0x1000
    mov r10, -1
    lea rdx, [rbx+0x270]
    lea r8, [rbx+0x278]
    mov r9d, 2
    lea rax, [rbx+0x280]
    mov [rsp+0x28], rax
    mov eax, 0x42
    syscall
    test eax, eax
    jnz fail
    cmp dword ptr [rbx+0x280], 4
    jne fail

    // NtQueryVirtualMemory(-1, base, MemoryBasicInformation, &MBI, 48, &ret)
    mov r15d, 10
    mov r10, -1
    mov rdx, r12
    xor r8d, r8d
    lea r9, [rbx+0x290]
    mov qword ptr [rsp+0x28], 48
    lea rax, [rbx+0x2F8]
    mov [rsp+0x30], rax
    mov eax, 0x43
    syscall
    test eax, eax
    jnz fail
    cmp dword ptr [rbx+0x290+32], 0x1000
    jne fail
    cmp dword ptr [rbx+0x290+36], 2
    jne fail

    // NtFreeVirtualMemory(-1, &BASE, &SIZE2=0, MEM_RELEASE)
    mov r15d, 11
    mov [rbx+0x260], r12
    mov qword ptr [rbx+0x2E8], 0
    mov r10, -1
    lea rdx, [rbx+0x260]
    lea r8, [rbx+0x2E8]
    mov r9d, 0x8000
    mov eax, 0x41
    syscall
    test eax, eax
    jnz fail

    // ----- 4. section mapped twice ---------------------------------------
    mov r15d, 12
    mov qword ptr [rbx+0x2C0], 0x2000
    // NtCreateSection(&OUT_H, SECTION_ALL_ACCESS, NULL, &SMAX, PAGE_RW,
    //                 SEC_COMMIT, NULL)
    lea r10, [rbx+0x200]
    mov edx, 0xF001F
    xor r8d, r8d
    lea r9, [rbx+0x2C0]
    mov qword ptr [rsp+0x28], 4
    mov qword ptr [rsp+0x30], 0x08000000
    mov qword ptr [rsp+0x38], 0
    mov eax, 0x44
    syscall
    test eax, eax
    jnz fail

    // NtMapViewOfSection view 1.
    mov r15d, 13
    mov qword ptr [rbx+0x2C8], 0
    mov qword ptr [rbx+0x2D8], 0
    mov r10, [rbx+0x200]
    mov rdx, -1
    lea r8, [rbx+0x2C8]
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    lea rax, [rbx+0x2D8]
    mov [rsp+0x38], rax
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 4
    mov eax, 0x46
    syscall
    test eax, eax
    jnz fail

    // NtMapViewOfSection view 2.
    mov r15d, 14
    mov qword ptr [rbx+0x2D0], 0
    mov qword ptr [rbx+0x2E0], 0
    mov r10, [rbx+0x200]
    mov rdx, -1
    lea r8, [rbx+0x2D0]
    xor r9d, r9d
    mov qword ptr [rsp+0x28], 0
    mov qword ptr [rsp+0x30], 0
    lea rax, [rbx+0x2E0]
    mov [rsp+0x38], rax
    mov qword ptr [rsp+0x40], 2
    mov qword ptr [rsp+0x48], 0
    mov qword ptr [rsp+0x50], 4
    mov eax, 0x46
    syscall
    test eax, eax
    jnz fail

    // Write through view 1, read through view 2.
    mov r15d, 15
    mov r13, [rbx+0x2C8]
    mov r14, [rbx+0x2D0]
    mov rax, 0xCAFEF00DD00DFEED
    mov [r13+0x40], rax
    cmp [r14+0x40], rax
    jne fail

    // Unmap both views.
    mov r15d, 16
    mov r10, -1
    mov rdx, [rbx+0x2C8]
    mov eax, 0x47
    syscall
    test eax, eax
    jnz fail
    mov r10, -1
    mov rdx, [rbx+0x2D0]
    mov eax, 0x47
    syscall
    test eax, eax
    jnz fail
    // Close the section handle.
    mov r10, [rbx+0x200]
    mov eax, 0
    syscall

    // ----- success --------------------------------------------------------
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

build_oa:
    mov dword ptr [rbx+0x220], 48
    mov qword ptr [rbx+0x228], 0
    lea rax, [rbx+0x250]
    mov [rbx+0x230], rax
    mov dword ptr [rbx+0x238], 0x40
    mov qword ptr [rbx+0x240], 0
    mov qword ptr [rbx+0x248], 0
    ret

fail:
    // PolyTrace(0xBADBAD, step, status); then terminate with FAIL_CODE.
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
evt_name:
    .2byte 0x5c, 'B', 'a', 's', 'e', 'N', 'a', 'm', 'e', 'd'
    .2byte 'O', 'b', 'j', 'e', 'c', 't', 's', 0x5c, 'P', 'o'
    .2byte 'l', 'y', 'E', 'v', 't'
evt_name_end:
dir_name:
    .2byte 0x5c
dir_name_end:
    .global poly_nt_test_code_end
poly_nt_test_code_end:
"#,
    pass = const PASS_CODE as u32,
    fail = const FAIL_CODE as u32,
);

unsafe extern "C" {
    static poly_nt_test_code: u8;
    static poly_nt_test_code_end: u8;
}

/// The assembled entry code (position-independent).
fn code() -> &'static [u8] {
    let start = &raw const poly_nt_test_code;
    let end = &raw const poly_nt_test_code_end;
    // SAFETY: the two symbols bound the code in a read-only section of the
    // kernel image.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

/// The PE executable file: one `.text` section holding the code (entry at
/// RVA 0x1000), relocatable, a 64 KiB stack reserve.
pub fn image() -> Vec<u8> {
    let mut data = code().to_vec();
    let virtual_size = data.len() as u32;
    // Pad so the section covers its bytes.
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
