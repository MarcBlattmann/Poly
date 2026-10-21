//! `__chkstk`: the MSVC x64 stack-probe routine the compiler calls before a
//! function with a frame larger than one page, so the guard page is touched
//! in order and the stack grows a page at a time (Learn: "/Gs (Control Stack
//! Checking Calls)"; the x64 contract is RAX = the frame size, RAX and every
//! register but R10/R11 preserved, RSP unchanged — the caller does
//! `sub rsp, rax` itself). Poly's kernel grows a thread's stack on each
//! guard-page fault (ADR 0063), so touching the pages top-down from the
//! caller's RSP down to RSP - RAX is enough. Hand-written; this is a public
//! ABI, not taken from any binary.

use core::arch::global_asm;

global_asm!(
    r#"
    .text
    .globl __chkstk
    .globl _chkstk
__chkstk:
_chkstk:
    push rcx
    push rax
    mov  rcx, rax              // bytes left to probe
    lea  rax, [rsp + 24]       // the caller's RSP (ret + two pushes)
2:
    cmp  rcx, 0x1000
    jb   3f
    sub  rax, 0x1000
    mov  byte ptr [rax], 0
    sub  rcx, 0x1000
    jmp  2b
3:
    pop  rax
    pop  rcx
    ret
"#
);
