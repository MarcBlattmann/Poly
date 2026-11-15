//! The helper routines the compiler calls in 32-bit Windows code (ADR 0186). LLVM's
//! `i686-pc-windows-msvc` code generation, like MSVC's, does not inline every 64-bit operation
//! or stack probe: it calls these by name, with the conventions MSVC's 32-bit C runtime defines
//! for them (Learn: "/Gs (Control Stack Checking Calls)" for the stack probe; the 64-bit
//! arithmetic helpers take their two 64-bit operands on the stack, low half first, return in
//! EDX:EAX and pop their 16 bytes of arguments; the shifts take the value in EDX:EAX and the
//! count in CL). Hand-written from those contracts, not taken from any binary or source.
//!
//! On every other target the crate is empty.

#![no_std]

#[cfg(target_arch = "x86")]
mod x86 {
    use core::arch::{asm, global_asm};

    // `_chkstk` / `_alloca_probe`: EAX = the bytes the caller is about to allocate. The 32-bit
    // contract differs from x64's: the routine itself moves ESP down by EAX (after touching every
    // page on the way, top down, so a guard page grows the stack one page at a time) and returns
    // with every other register as it was.
    //
    // The 64-bit helpers: `_alldiv`, `_aulldiv`, `_allrem`, `_aullrem` re-push their operands
    // and call the Rust routines below (cdecl), then pop their own arguments (`ret 16`);
    // `_allmul` multiplies in place; `_allshl`, `_allshr`, `_aullshr` shift EDX:EAX by CL.
    global_asm!(
        r#"
    .text
    .globl __chkstk
    .globl __alloca_probe
__chkstk:
__alloca_probe:
    push ecx
    lea  ecx, [esp + 8]
2:
    cmp  eax, 0x1000
    jb   3f
    sub  ecx, 0x1000
    test dword ptr [ecx], eax
    sub  eax, 0x1000
    jmp  2b
3:
    sub  ecx, eax
    test dword ptr [ecx], eax
    mov  eax, esp
    mov  esp, ecx
    mov  ecx, dword ptr [eax]
    mov  eax, dword ptr [eax + 4]
    push eax
    ret

    .globl __aulldiv
__aulldiv:
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    call {udiv}
    add  esp, 16
    ret  16

    .globl __aullrem
__aullrem:
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    call {urem}
    add  esp, 16
    ret  16

    .globl __alldiv
__alldiv:
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    call {sdiv}
    add  esp, 16
    ret  16

    .globl __allrem
__allrem:
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    push dword ptr [esp + 16]
    call {srem}
    add  esp, 16
    ret  16

    .globl __allmul
__allmul:
    mov  eax, dword ptr [esp + 8]
    mov  ecx, dword ptr [esp + 16]
    or   ecx, eax
    mov  ecx, dword ptr [esp + 12]
    jnz  4f
    mov  eax, dword ptr [esp + 4]
    mul  ecx
    ret  16
4:
    push ebx
    mul  ecx
    mov  ebx, eax
    mov  eax, dword ptr [esp + 8]
    mul  dword ptr [esp + 20]
    add  ebx, eax
    mov  eax, dword ptr [esp + 8]
    mul  ecx
    add  edx, ebx
    pop  ebx
    ret  16

    .globl __allshl
__allshl:
    cmp  cl, 64
    jae  6f
    cmp  cl, 32
    jae  5f
    shld edx, eax, cl
    shl  eax, cl
    ret
5:
    mov  edx, eax
    xor  eax, eax
    and  cl, 31
    shl  edx, cl
    ret
6:
    xor  eax, eax
    xor  edx, edx
    ret

    .globl __aullshr
__aullshr:
    cmp  cl, 64
    jae  6b
    cmp  cl, 32
    jae  7f
    shrd eax, edx, cl
    shr  edx, cl
    ret
7:
    mov  eax, edx
    xor  edx, edx
    and  cl, 31
    shr  eax, cl
    ret

    .globl __allshr
__allshr:
    cmp  cl, 64
    jae  9f
    cmp  cl, 32
    jae  8f
    shrd eax, edx, cl
    sar  edx, cl
    ret
8:
    mov  eax, edx
    sar  edx, 31
    and  cl, 31
    sar  eax, cl
    ret
9:
    sar  edx, 31
    mov  eax, edx
    ret
"#,
        udiv = sym poly_udiv64,
        urem = sym poly_urem64,
        sdiv = sym poly_sdiv64,
        srem = sym poly_srem64,
    );

    /// `n / d` for a 32-bit divisor, with two `div`s (the high half first); a zero divisor
    /// raises the CPU's divide error, as the compiler's own division does.
    fn div_by_u32(n: u64, d: u32) -> (u64, u32) {
        let (hi, lo) = ((n >> 32) as u32, n as u32);
        let (qh, r): (u32, u32);
        let (ql, r2): (u32, u32);
        // SAFETY: `div` with EDX = 0 (then the first remainder, < d) cannot overflow; d = 0 is
        // the divide error the caller asked for.
        unsafe {
            asm!("div {d:e}", d = in(reg) d, inout("eax") hi => qh, inout("edx") 0u32 => r, options(nomem, nostack));
            asm!("div {d:e}", d = in(reg) d, inout("eax") lo => ql, inout("edx") r => r2, options(nomem, nostack));
        }
        ((u64::from(qh) << 32) | u64::from(ql), r2)
    }

    /// Unsigned `(n / d, n % d)`, without the helpers being defined (only shifts, compares,
    /// subtractions and 32-bit `div`).
    fn udivmod(n: u64, d: u64) -> (u64, u64) {
        if d >> 32 == 0 {
            let (q, r) = div_by_u32(n, d as u32);
            return (q, u64::from(r));
        }
        if n < d {
            return (0, n);
        }
        // d ≥ 2^32: the quotient fits in 32 bits; shift and subtract.
        let s = d.leading_zeros() - n.leading_zeros();
        let (mut q, mut r, mut dd) = (0u64, n, d << s);
        for _ in 0..=s {
            q <<= 1;
            if r >= dd {
                r -= dd;
                q |= 1;
            }
            dd >>= 1;
        }
        (q, r)
    }

    #[unsafe(no_mangle)]
    extern "C" fn poly_udiv64(n: u64, d: u64) -> u64 {
        udivmod(n, d).0
    }

    #[unsafe(no_mangle)]
    extern "C" fn poly_urem64(n: u64, d: u64) -> u64 {
        udivmod(n, d).1
    }

    /// Signed division truncates toward zero; the remainder has the dividend's sign.
    #[unsafe(no_mangle)]
    extern "C" fn poly_sdiv64(n: i64, d: i64) -> i64 {
        let q = udivmod(n.unsigned_abs(), d.unsigned_abs()).0;
        if (n < 0) != (d < 0) { (q as i64).wrapping_neg() } else { q as i64 }
    }

    #[unsafe(no_mangle)]
    extern "C" fn poly_srem64(n: i64, d: i64) -> i64 {
        let r = udivmod(n.unsigned_abs(), d.unsigned_abs()).1;
        if n < 0 { (r as i64).wrapping_neg() } else { r as i64 }
    }
}
