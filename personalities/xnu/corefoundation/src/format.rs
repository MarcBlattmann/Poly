//! `CFStringCreateWithFormat` and `CFStringAppendFormat`: the variadic
//! entry points save the argument registers in a System V register save
//! area, build a `va_list` over it (as Poly's libSystem does for `printf`)
//! and call their `...AndArguments` form; the engine is
//! `poly_cf::format`.

use crate::base::{self, Ref};
use crate::string;
use alloc::vec::Vec;
use poly_libsystem::printf::{VaList, VaListTag};

struct Objects;

impl poly_cf::format::Host for Objects {
    fn describe(&mut self, obj: u64, out: &mut Vec<u16>) {
        out.extend_from_slice(&base::formatting(obj as Ref));
    }
}

/// Format with a caller's `va_list`.
///
/// # Safety
/// `args` is a live `va_list` with the arguments `fmt` names.
unsafe fn render(fmt: Ref, args: *mut VaListTag) -> Vec<u16> {
    let f = string::units(fmt).to_vec();
    let mut va = VaList(args);
    // SAFETY: the caller's contract.
    unsafe { poly_cf::format::format(&f, &mut va, &mut Objects) }
}

/// # Safety
/// `args` is a `va_list` with the arguments `fmt` names.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringCreateWithFormatAndArguments(
    alloc: Ref,
    _options: Ref,
    fmt: Ref,
    args: *mut VaListTag,
) -> Ref {
    // SAFETY: the caller's contract.
    let u = unsafe { render(fmt, args) };
    string::from_units(alloc, u, false)
}

/// # Safety
/// As for `CFStringCreateWithFormatAndArguments`; `s` is mutable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn CFStringAppendFormatAndArguments(s: Ref, _options: Ref, fmt: Ref, args: *mut VaListTag) {
    // SAFETY: the caller's contract.
    let u = unsafe { render(fmt, args) };
    let tmp = string::from_units(core::ptr::null(), u, false);
    string::CFStringAppend(s, tmp);
    base::release(tmp);
}

// The variadic functions: register save area at [rsp], the va_list after
// it at [rsp + 176], passed as the argument after the fixed ones.
macro_rules! variadic {
    ($($name:literal => $v:literal, $fixed:literal, $reg:literal;)*) => {
        core::arch::global_asm!(
            $(
                concat!(".globl _", $name),
                ".p2align 4",
                concat!("_", $name, ":"),
                "    push rbp",
                "    mov rbp, rsp",
                "    sub rsp, 208",
                "    mov [rsp], rdi",
                "    mov [rsp + 8], rsi",
                "    mov [rsp + 16], rdx",
                "    mov [rsp + 24], rcx",
                "    mov [rsp + 32], r8",
                "    mov [rsp + 40], r9",
                "    test al, al",
                "    je 2f",
                "    movaps [rsp + 48], xmm0",
                "    movaps [rsp + 64], xmm1",
                "    movaps [rsp + 80], xmm2",
                "    movaps [rsp + 96], xmm3",
                "    movaps [rsp + 112], xmm4",
                "    movaps [rsp + 128], xmm5",
                "    movaps [rsp + 144], xmm6",
                "    movaps [rsp + 160], xmm7",
                "2:",
                "    lea r10, [rsp + 176]",
                concat!("    mov dword ptr [r10], 8 * ", $fixed),
                "    mov dword ptr [r10 + 4], 48",
                "    lea r11, [rbp + 16]",
                "    mov [r10 + 8], r11",
                "    mov [r10 + 16], rsp",
                concat!("    mov ", $reg, ", r10"),
                concat!("    call _", $v),
                "    leave",
                "    ret",
            )*
        );
    };
}

variadic! {
    "CFStringCreateWithFormat" => "CFStringCreateWithFormatAndArguments", 3, "rcx";
    "CFStringAppendFormat" => "CFStringAppendFormatAndArguments", 3, "rcx";
}
