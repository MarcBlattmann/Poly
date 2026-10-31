//! What every proxy DLL of the Windows API tracer shares (ADR 0090): the
//! thunk body `pt_common`, the entry point and the panic handler. The
//! generated `lib.rs` of each proxy includes this file as a module and
//! adds the table of its functions and one two-instruction thunk per
//! export (`lea r10, [rip + <Func>]; jmp pt_common`).
//!
//! # `pt_common`
//!
//! On entry `r10` points to the function's [`Func`](../polytrace) record
//! and everything else is as the caller left it for the real function
//! (x64 calling convention, Learn's "x64 calling convention" and "x64
//! stack usage"): `rcx`, `rdx`, `r8`, `r9` and `xmm0`-`xmm3` hold the first
//! four arguments, the return address is at `[rsp]`, the caller's home
//! area above it and the stack arguments from `[rsp + 0x28]` on.
//!
//! It makes a frame of its own (one push and a fixed allocation, described
//! by unwind codes, so an exception or `RtlVirtualUnwind` can pass through
//! it), saves the argument registers, copies the stack arguments (as many
//! slots as the record's `stack` field says, at most sixteen: arguments
//! five to twenty; never past the thread's stack base, `NT_TIB.StackBase`
//! at `gs:[8]`, so a call near the top of a fresh thread's stack does not
//! read beyond it) to the bottom of its frame where the real function
//! expects them, calls `pt_enter`, puts the argument registers
//! back and calls the real function through its import address table slot.
//! The result (`rax`, `xmm0`) is kept across `pt_leave` and returned.
//! Non-volatile registers are never touched (the Rust functions keep
//! them). With the `TAIL` flag it instead takes its frame down and jumps
//! to the real function, which then sees the caller's own frame.
//!
//! Frame (offsets from `rsp` after the prologue):
//!
//! | Offset | Size | What |
//! |---|---|---|
//! | 0x000 | 0x20 | home area of the functions called |
//! | 0x020 | 0x80 | stack arguments 5..20 (copied) |
//! | 0x0A0 | 0x20 | `rcx`, `rdx`, `r8`, `r9` |
//! | 0x0C0 | 0x40 | `xmm0`-`xmm3` |
//! | 0x100 | 8 | `pt_enter`'s cookie |
//! | 0x108 | 8 | `rax` after the call |
//! | 0x110 | 0x10 | `xmm0` after the call |
//! | 0x120 | 0x800 | the record being written |
//!
//! 0x920 bytes plus the pushed `rbx` and the return address: the caller's
//! fifth argument is at `rsp + 0x950`.

// Referenced from the assembly only (`__imp_pt_enter`, `__imp_pt_leave`).
#[allow(dead_code)]
#[link(name = "polytrace", kind = "raw-dylib")]
unsafe extern "C" {
    pub fn pt_enter();
    pub fn pt_leave();
}

core::arch::global_asm!(
    r#"
    .text
    .p2align 4
    .def pt_common
    .scl 3
    .type 32
    .endef
    .seh_proc pt_common
pt_common:
    push rbx
    .seh_pushreg rbx
    sub rsp, 0x920
    .seh_stackalloc 0x920
    .seh_endprologue
    mov rbx, r10
    mov [rsp + 0xA0], rcx
    mov [rsp + 0xA8], rdx
    mov [rsp + 0xB0], r8
    mov [rsp + 0xB8], r9
    movups [rsp + 0xC0], xmm0
    movups [rsp + 0xD0], xmm1
    movups [rsp + 0xE0], xmm2
    movups [rsp + 0xF0], xmm3
    mov ecx, dword ptr [rbx + 36]
    cmp ecx, 16
    jbe 5f
    mov ecx, 16
5:
    lea rdx, [rsp + 0x950]
    mov r11, qword ptr gs:[8]
    cmp r11, rdx
    ja 6f
    mov r11, -1
6:
    xor eax, eax
7:
    cmp eax, ecx
    jae 8f
    lea r8, [rdx + rax * 8 + 8]
    cmp r8, r11
    ja 8f
    mov r9, [rdx + rax * 8]
    mov [rsp + rax * 8 + 0x20], r9
    inc eax
    jmp 7b
8:
    mov rcx, rbx
    lea rdx, [rsp + 0xA0]
    lea r8, [rsp + 0x20]
    lea r9, [rsp + 0x120]
    call qword ptr [rip + __imp_pt_enter]
    mov [rsp + 0x100], rax
    mov rcx, [rsp + 0xA0]
    mov rdx, [rsp + 0xA8]
    mov r8, [rsp + 0xB0]
    mov r9, [rsp + 0xB8]
    movups xmm0, [rsp + 0xC0]
    movups xmm1, [rsp + 0xD0]
    movups xmm2, [rsp + 0xE0]
    movups xmm3, [rsp + 0xF0]
    test dword ptr [rbx + 32], 2
    jnz 2f
    mov rax, [rbx]
    call qword ptr [rax]
    mov [rsp + 0x108], rax
    movups [rsp + 0x110], xmm0
    mov rcx, rbx
    mov rdx, [rsp + 0x100]
    mov r8, rax
    lea r9, [rsp + 0x120]
    call qword ptr [rip + __imp_pt_leave]
    mov rax, [rsp + 0x108]
    movups xmm0, [rsp + 0x110]
    add rsp, 0x920
    pop rbx
    ret
2:
    mov r11, [rbx]
    mov r11, [r11]
    add rsp, 0x920
    pop rbx
    jmp r11
    .seh_endproc
"#
);

// The entry point `pt_proxy_main` (`DllMain`: nothing to set up, the real
// DLLs are imports bound by the loader and `polytrace.dll` keeps the
// state, so it returns TRUE), and `_fltused`, the MSVC toolchain's marker
// that floating point is used, which `core` references. Both in assembly
// so that they are not exports of the proxy, whose export table must be
// the real DLL's names only.
core::arch::global_asm!(
    r#"
    .text
    .globl pt_proxy_main
pt_proxy_main:
    mov eax, 1
    ret
    .data
    .globl _fltused
    .p2align 2
_fltused:
    .long 0
"#
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: `ud2` raises an illegal-instruction exception; nothing in a
    // proxy panics on purpose.
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}
