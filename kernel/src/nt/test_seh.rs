//! Self-assembled ring-3 PEs that drive kernel-to-user exception dispatch
//! (ADR 0068). No Microsoft toolchain or DLLs: position-independent code
//! that makes the NT system calls directly.
//!
//! Two programs:
//!
//! - [`handled_image`]: imports `RtlAddVectoredExceptionHandler` from our
//!   `ntdll` and registers a vectored handler, then triggers an access
//!   violation (a write to a null pointer) and an integer divide-by-zero.
//!   The kernel delivers each to `ntdll`'s `KiUserExceptionDispatcher` (ADR
//!   0068, 0071), which calls the handler with the `EXCEPTION_POINTERS`;
//!   the handler inspects the `EXCEPTION_RECORD`, fixes the `CONTEXT` up (a
//!   sentinel in `Rax`, `Rip` at a recovery point) and returns
//!   `EXCEPTION_CONTINUE_EXECUTION`, so `ntdll` resumes it with
//!   `NtContinue`. The main code checks the record fields the handler
//!   reported and ends with [`PASS_CODE`].
//! - [`unhandled_image`]: registers no handler and writes to a null
//!   pointer; `ntdll` finds no handler and raises it second chance, and the
//!   process must end with `STATUS_ACCESS_VIOLATION` as its exit status
//!   (ADR 0068).
//!
//! The handler and the main thread share a fixed, page-aligned scratch
//! region ([`SCRATCH`]) that the main thread commits first (the image's own
//! sections are not writable):
//!
//! ```text
//!   SCRATCH + 0x00   ExceptionCode the handler last saw
//!   SCRATCH + 0x08   ExceptionInformation[0] (access kind)
//!   SCRATCH + 0x10   ExceptionInformation[1] (address)
//!   SCRATCH + 0x18   recovery RIP the handler sets CONTEXT.Rip to
//!   SCRATCH + 0x20   count of handler calls
//! ```
//!
//! Every `global_asm!` label is prefixed `ntseh_` (the CI check for
//! clashes).

use alloc::vec;
use alloc::vec::Vec;
use core::arch::global_asm;
use poly_ldr::builder::{Import, PeBuilder, Section, Sym, TEXT};

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
    sub rsp, 0x1008              // 16-byte aligned for the call below
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
    // Zero the handler-call counter.
    mov qword ptr [r13+0x20], 0

    // RtlAddVectoredExceptionHandler(TRUE, &handler), through the import
    // address table slot the loader bound (its address is in the link
    // slot below, relocated with the image).
    mov ecx, 1
    lea rdx, [rip+ntseh_handler]
    mov rax, [rip+ntseh_link_veh]
    call [rax]
    test rax, rax
    jz ntseh_handled_fail

    // ----- access violation ----------------------------------------------
    lea rax, [rip+ntseh_after_av]
    mov [r13+0x18], rax          // recovery RIP
    xor eax, eax
    mov byte ptr [rax], 1        // write to null: #PF
ntseh_after_av:
    // The handler put a sentinel in Rax.
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
    // The handler ran exactly twice.
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

    // The vectored handler: LONG handler(EXCEPTION_POINTERS *p), p =
    // (&record, &context) (winnt.h). Only volatile registers are used.
    .balign 16
ntseh_handler:
    mov rdx, [rcx+0x08]          // ContextRecord
    mov rcx, [rcx+0x00]          // ExceptionRecord
    mov r8, {scratch}
    inc qword ptr [r8+0x20]      // one more call
    // Record ExceptionCode and the two information words.
    mov eax, [rcx+0x00]
    mov [r8+0x00], eax
    mov rax, [rcx+0x20]          // ExceptionInformation[0]
    mov [r8+0x08], rax
    mov rax, [rcx+0x28]          // ExceptionInformation[1]
    mov [r8+0x10], rax
    // Fix the context up: Rax = sentinel, Rip = the recovery point.
    mov qword ptr [rdx+0x78], 0x1234   // CONTEXT.Rax
    mov rax, [r8+0x18]
    mov [rdx+0xF8], rax                 // CONTEXT.Rip
    mov eax, -1                  // EXCEPTION_CONTINUE_EXECUTION
    ret

    // The link slot: the address of RtlAddVectoredExceptionHandler's import
    // address table slot, written when the image is built.
    .balign 8
    .global ntseh_link_veh
ntseh_link_veh:
    .quad 0
    .global poly_nt_seh_handled_end
poly_nt_seh_handled_end:

    // ===== the "unhandled" program =======================================
    .balign 16
    .global poly_nt_seh_unhandled
poly_nt_seh_unhandled:
    // No handler: a write to null ends the process with the exception code
    // as its exit status.
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
    static ntseh_link_veh: u8;
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

/// The executable: `code` as `.text` at RVA 0x1000 (entry at its start),
/// importing `imports` from `ntdll.dll`; `link` is the offset in `code` of
/// an 8-byte slot that gets the address of the first import's address
/// table slot (relocated).
fn pe(code: &[u8], imports: &[&str], link: Option<usize>) -> Vec<u8> {
    let mut b = PeBuilder {
        image_base: IMAGE_BASE,
        dll: false,
        entry: 0x1000,
        sections: vec![Section::new(".text", code.to_vec(), TEXT)],
        stack_reserve: 0x1_0000,
        stack_commit: 0x4000,
        ..PeBuilder::default()
    };
    if !imports.is_empty() {
        let symbols = imports.iter().map(|n| Sym::name(n)).collect();
        b.imports = vec![Import { dll: "ntdll.dll".into(), symbols, ..Import::default() }];
    }
    if let Some(at) = link {
        let slot = IMAGE_BASE + u64::from(b.layout().iat[0][0]);
        b.sections[0].data[at..at + 8].copy_from_slice(&slot.to_le_bytes());
        b.relocations.push(0x1000 + at as u32);
    }
    b.write()
}

/// The PE that handles its exceptions and continues.
pub fn handled_image() -> Vec<u8> {
    // SAFETY: the symbols bound the handled program's code.
    let code = unsafe { slice_between(&poly_nt_seh_handled, &poly_nt_seh_handled_end) };
    let link = (&raw const ntseh_link_veh) as usize - code.as_ptr() as usize;
    pe(code, &["RtlAddVectoredExceptionHandler"], Some(link))
}

/// The PE that leaves an exception unhandled.
pub fn unhandled_image() -> Vec<u8> {
    // SAFETY: the symbols bound the unhandled program's code.
    let code = unsafe { slice_between(&poly_nt_seh_unhandled, &poly_nt_seh_unhandled_end) };
    pe(code, &[], None)
}
