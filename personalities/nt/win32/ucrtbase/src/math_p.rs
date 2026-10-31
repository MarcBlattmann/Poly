//! What only the UCRT exports of ADR 0092's slice: the C99 math functions
//! (`exp2`, `expm1`, `log1p`, `log2`, `cbrt`, `hypot`, `remainder` and
//! their `f`/`l` forms; `long double` is `double`, Learn "Data type
//! ranges") and the positional-parameter `printf` entry points
//! (`__stdio_common_v*printf_p`, Learn "printf_p Positional Parameters").
//! `setjmp`/`longjmp` are the shared runtime's assembly, exported by
//! `build.rs`.

use poly_crt::printf::{POSITIONAL, VaList};
use poly_libm::Function as F;
use poly_win32_crt::mathfn::{d1, d2, f1, f2};
use poly_win32_crt::stdio::{self, BufferRule, File};

macro_rules! math {
    ($($name:ident($($a:ident),+): $t:ty = $how:ident($f:expr, $op:expr);)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name($($a: $t),+) -> $t {
            $how($f, $op, $($a),+)
        }
    )*};
}

math! {
    exp2(x): f64 = d1(F::Exp2, poly_libm::exp2);
    exp2l(x): f64 = d1(F::Exp2, poly_libm::exp2);
    exp2f(x): f32 = f1(F::Exp2, poly_libm::exp2);
    expm1(x): f64 = d1(F::Expm1, poly_libm::expm1);
    expm1l(x): f64 = d1(F::Expm1, poly_libm::expm1);
    expm1f(x): f32 = f1(F::Expm1, poly_libm::expm1);
    log1p(x): f64 = d1(F::Log1p, poly_libm::log1p);
    log1pl(x): f64 = d1(F::Log1p, poly_libm::log1p);
    log1pf(x): f32 = f1(F::Log1p, poly_libm::log1p);
    log2(x): f64 = d1(F::Log2, poly_libm::log2);
    log2l(x): f64 = d1(F::Log2, poly_libm::log2);
    log2f(x): f32 = f1(F::Log2, poly_libm::log2);
    cbrt(x): f64 = d1(F::Cbrt, poly_libm::cbrt);
    cbrtl(x): f64 = d1(F::Cbrt, poly_libm::cbrt);
    cbrtf(x): f32 = f1(F::Cbrt, poly_libm::cbrt);
    hypot(x, y): f64 = d2(F::Hypot, poly_libm::hypot);
    remainder(x, y): f64 = d2(F::Remainder, poly_libm::remainder);
    remainderl(x, y): f64 = d2(F::Remainder, poly_libm::remainder);
    remainderf(x, y): f32 = f2(F::Remainder, poly_libm::remainder);
}

/// The buffer rule of `__stdio_common_vsprintf_p`: `_sprintf_p` is
/// `sprintf_s` with positional parameters (Learn), and the counting form
/// (`_scprintf_p`: no buffer, no size) follows the option bits.
fn rule<C>(buf: *mut C, count: usize) -> BufferRule {
    if buf.is_null() && count == 0 { BufferRule::Options } else { BufferRule::Secure }
}

/// # Safety
/// The UCRT headers' contract: `format` and its `va_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfprintf_p(
    options: u64,
    f: *mut File,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vfprintf(f, format, &mut VaList(args), options | POSITIONAL) }
}

/// # Safety
/// As `__stdio_common_vfprintf_p`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vfwprintf_p(
    options: u64,
    f: *mut File,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vfprintf(f, format, &mut VaList(args), options | POSITIONAL) }
}

/// # Safety
/// `buf` holds `count` units (or is NULL with 0); as above otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vsprintf_p(
    options: u64,
    buf: *mut u8,
    count: usize,
    format: *const u8,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options | POSITIONAL, rule(buf, count)) }
}

/// # Safety
/// As `__stdio_common_vsprintf_p`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __stdio_common_vswprintf_p(
    options: u64,
    buf: *mut u16,
    count: usize,
    format: *const u16,
    _locale: *mut u8,
    args: *const u64,
) -> i32 {
    // SAFETY: the caller's arguments.
    unsafe { stdio::vsprintf(buf, count, format, &mut VaList(args), options | POSITIONAL, rule(buf, count)) }
}
