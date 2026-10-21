//! Self-assembled ring-3 PEs that drive kernel-to-user exception dispatch
//! (ADR 0068). No Microsoft toolchain or DLLs: position-independent code
//! that makes the NT system calls directly.
//!
//! Two programs:
//!
//! - [`handled_image`]: registers a user-mode exception dispatcher with
//!   `PolySetExceptionDispatcher`, triggers an access violation (a write to
//!   a null pointer) and then an integer divide-by-zero, and for each one
//!   the dispatcher inspects the `EXCEPTION_RECORD`, fixes the `CONTEXT` up
//!   (a sentinel in `Rax`, `Rip` at a recovery point) and resumes with
//!   `NtContinue`. The main code checks the record fields the dispatcher
//!   reported and ends with [`PASS_CODE`].
//! - [`unhandled_image`]: registers no dispatcher and writes to a null
//!   pointer; the process must end with `STATUS_ACCESS_VIOLATION` as its
//!   exit status (second chance / no handler, ADR 0068).
//!
//! The dispatcher and the main thread share a fixed, page-aligned scratch
//! region ([`SCRATCH`]) that the main thread commits first, since the
//! dispatcher is entered fresh and the image's own data section is not
//! writable:
//!
//! ```text
//!   SCRATCH + 0x00   ExceptionCode the dispatcher last saw
//!   SCRATCH + 0x08   ExceptionInformation[0] (access kind)
//!   SCRATCH + 0x10   ExceptionInformation[1] (address)
//!   SCRATCH + 0x18   recovery RIP the dispatcher sets CONTEXT.Rip to
//!   SCRATCH + 0x20   count of dispatcher calls
//! ```
//!
//! Every `global_asm!` label is prefixed `ntseh_` (the CI check for
//! clashes).

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_personality_nt::pe_writer::{self, PeImage, PeSection};

/// The exit status when every check passed.
pub const PASS_CODE: u8 = 0x5A;
/// The exit status on the first failed check.
pub const FAIL_CODE: u8 = 0xEE;
/// The image's preferred base.
pub const IMAGE_BASE: u64 = 0x1_4000_0000;
/// The shared scratch region (committed by the main thread).
pub const SCRATCH: u64 = 0x3000_0000;
/// `STATUS_ACCESS_VIOLATION`, the unhandled program's expected exit status.
pub const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;

global_asm!(
    r#"
    .section .rodata.poly_nt_seh, "a", @progbits
    .balign 16

    // ===== the "handled" program =========================================
    .global poly_nt_seh_handled
poly_nt_seh_handled:
    sub rsp, 0x1000
    mov rbx, rsp
    mov r13, {scratch}            // the shared scratch region

    // Commit the scratch page: NtAllocateVirtualMemory(-1, &base=SCRATCH, 0,
    // &size=0x1000, RESERVE|COMMIT, PAGE_READWRITE).
    mov [rbx+0x00], r13
    mov qword ptr [rbx+0x08], 0x1000
    mov r10, -1
    lea rdx, [rbx+0x00]
    xor r8d, r8d
    lea r9, [rbx+0x08]
    mov qword ptr [rsp+0x28], 0x3000
    mov qword ptr [rsp+0x30], 4
    mov eax, 0x40
    syscall
    test eax, eax
    jnz ntseh_handled_fail
    // Zero the dispatcher-call counter.
    mov qword ptr [r13+0x20], 0

    // PolySetExceptionDispatcher(&dispatcher).
    lea r10, [rip+ntseh_dispatcher]
    mov eax, 0x3010
    syscall
    test eax, eax
    jnz ntseh_handled_fail

    // ----- access violation ----------------------------------------------
    lea rax, [rip+ntseh_after_av]
    mov [r13+0x18], rax          // recovery RIP
    xor eax, eax
    mov byte ptr [rax], 1        // write to null: #PF
ntseh_after_av:
    // The dispatcher put a sentinel in Rax.
    cmp rax, 0x1234
    jne ntseh_handled_fail
    mov eax, [r13+0x00]          // the code it saw
    cmp eax, 0xC0000005          // STATUS_ACCESS_VIOLATION
    jne ntseh_handled_fail
    mov rax, [r13+0x08]          // ExceptionInformation[0]: a write
    cmp rax, 1
    jne ntseh_handled_fail
    mov rax, [r13+0x10]          // ExceptionInformation[1]: the address 0
    test rax, rax
    jnz ntseh_handled_fail

    // ----- integer divide by zero ----------------------------------------
    lea rax, [rip+ntseh_after_div]
    mov [r13+0x18], rax
    xor edx, edx
    xor ecx, ecx
    mov eax, 1
    div ecx                      // #DE
ntseh_after_div:
    mov eax, [r13+0x00]
    cmp eax, 0xC0000094          // STATUS_INTEGER_DIVIDE_BY_ZERO
    jne ntseh_handled_fail
    // The dispatcher ran exactly twice.
    mov rax, [r13+0x20]
    cmp rax, 2
    jne ntseh_handled_fail

    // ----- success --------------------------------------------------------
    mov r10, -1
    mov edx, {pass}
    mov eax, 0x80
    syscall
    ud2

ntseh_handled_fail:
    mov r8d, eax
    lea rdx, [rip+ntseh_after_av]
    mov r10, 0xBADBAD
    mov eax, 0x3000
    syscall
    mov r10, -1
    mov edx, {fail}
    mov eax, 0x80
    syscall
    ud2

    // The user-mode exception dispatcher. Entered with RCX = &record,
    // RDX = &context (and a fake return address on the stack).
    .balign 16
ntseh_dispatcher:
    mov r13, {scratch}
    inc qword ptr [r13+0x20]     // one more call
    // Record ExceptionCode and the two information words.
    mov eax, [rcx+0x00]
    mov [r13+0x00], eax
    mov rax, [rcx+0x20]          // ExceptionInformation[0]
    mov [r13+0x08], rax
    mov rax, [rcx+0x28]          // ExceptionInformation[1]
    mov [r13+0x10], rax
    // Fix the context up: Rax = sentinel, Rip = the recovery point.
    mov qword ptr [rdx+0x78], 0x1234   // CONTEXT.Rax
    mov rax, [r13+0x18]
    mov [rdx+0xF8], rax                 // CONTEXT.Rip
    // NtContinue(&context, FALSE).
    mov r10, rdx
    xor edx, edx
    mov eax, 0xA1
    syscall
    ud2
    .global poly_nt_seh_handled_end
poly_nt_seh_handled_end:

    // ===== the "unhandled" program =======================================
    .balign 16
    .global poly_nt_seh_unhandled
poly_nt_seh_unhandled:
    // No dispatcher: a write to null ends the process with the exception
    // code as its exit status.
    xor eax, eax
    mov byte ptr [rax], 1
    ud2
    .global poly_nt_seh_unhandled_end
poly_nt_seh_unhandled_end:
"#,
    pass = const PASS_CODE as u32,
    fail = const FAIL_CODE as u32,
    scratch = const SCRATCH,
);

unsafe extern "C" {
    static poly_nt_seh_handled: u8;
    static poly_nt_seh_handled_end: u8;
    static poly_nt_seh_unhandled: u8;
    static poly_nt_seh_unhandled_end: u8;
}

fn slice_between(start: &'static u8, end: &'static u8) -> &'static [u8] {
    let (start, end) = (start as *const u8, end as *const u8);
    // SAFETY: the two symbols bound one run of bytes in a read-only section
    // of the kernel image.
    unsafe { core::slice::from_raw_parts(start, end as usize - start as usize) }
}

fn pe(code: &[u8]) -> Vec<u8> {
    let mut data = code.to_vec();
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

/// The PE that handles its exceptions and continues.
pub fn handled_image() -> Vec<u8> {
    // SAFETY: the symbols bound the handled program's code.
    let code = unsafe { slice_between(&poly_nt_seh_handled, &poly_nt_seh_handled_end) };
    pe(code)
}

/// The PE that leaves an exception unhandled.
pub fn unhandled_image() -> Vec<u8> {
    // SAFETY: the symbols bound the unhandled program's code.
    let code = unsafe { slice_between(&poly_nt_seh_unhandled, &poly_nt_seh_unhandled_end) };
    pe(code)
}
