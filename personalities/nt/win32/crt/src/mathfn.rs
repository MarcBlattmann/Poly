//! The transcendental `math.h` functions (ADR 0092): the computation is
//! `poly_libm`'s; here each call's math error (`poly_libm::fault`) goes to
//! the program's `_matherr` (installed with `__setusermatherr`) and to
//! `errno`.
//!
//! Learn, `_matherr`: the handler gets a `struct _exception` (`math.h`:
//! `int type; char *name; double arg1, arg2, retval;`); if it returns
//! nonzero, `errno` stays as it was and the function returns `retval` (which
//! the handler may change); if it returns 0, or there is none, `errno` is
//! set. `_DOMAIN` sets `EDOM`; `_SING` (a pole, C11 7.12.1), `_OVERFLOW`
//! and `_UNDERFLOW` set `ERANGE` (Poly's reading of C11, to be confirmed).
//!
//! The functions both DLLs export are `#[no_mangle]` here; the ones only
//! `ucrtbase.dll` (the C99 additions) or only `msvcrt.dll` (`frexpf`)
//! export are plain functions their DLL wraps.

use crate::global;
use crate::misc::MATHERR;
use core::sync::atomic::Ordering;
use poly_crt::errno::{EDOM, ERANGE};
use poly_libm::{Fault, Function as F};

/// `math.h`'s `struct _exception`.
#[repr(C)]
#[derive(Debug)]
struct Exception {
    kind: i32,
    name: *const u8,
    arg1: f64,
    arg2: f64,
    retval: f64,
}

/// Report the math error of `f(x, y) = r`, if any: the program's handler,
/// then `errno`. Returns the value to return.
pub fn finish(f: F, x: f64, y: f64, r: f64, single: bool) -> f64 {
    let Some(fault) = poly_libm::fault(f, x, y, r, single) else { return r };
    let errno = if fault == Fault::Domain { EDOM } else { ERANGE };
    let handler = MATHERR.load(Ordering::Acquire);
    if handler == 0 {
        global::set_errno(errno);
        return r;
    }
    let mut e = Exception { kind: fault as i32, name: f.name(single).as_ptr(), arg1: x, arg2: y, retval: r };
    // SAFETY: the program installed this `int _matherr(struct _exception *)`.
    let handled = unsafe {
        let h: extern "C" fn(*mut Exception) -> i32 = core::mem::transmute(handler);
        h(&mut e)
    };
    if handled == 0 {
        global::set_errno(errno);
    }
    e.retval
}

/// A one-argument double function.
pub fn d1(f: F, op: fn(f64) -> f64, x: f64) -> f64 {
    finish(f, x, 0.0, op(x), false)
}

/// A two-argument double function.
pub fn d2(f: F, op: fn(f64, f64) -> f64, x: f64, y: f64) -> f64 {
    finish(f, x, y, op(x, y), false)
}

/// A one-argument float function: computed in double, rounded once more.
pub fn f1(f: F, op: fn(f64) -> f64, x: f32) -> f32 {
    let x = f64::from(x);
    finish(f, x, 0.0, f64::from(op(x) as f32), true) as f32
}

/// A two-argument float function.
pub fn f2(f: F, op: fn(f64, f64) -> f64, x: f32, y: f32) -> f32 {
    let (x, y) = (f64::from(x), f64::from(y));
    finish(f, x, y, f64::from(op(x, y) as f32), true) as f32
}

macro_rules! both {
    ($($name:ident($($a:ident),+): $t:ty = $how:ident($f:expr, $op:expr);)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name($($a: $t),+) -> $t {
            $how($f, $op, $($a),+)
        }
    )*};
}

both! {
    sin(x): f64 = d1(F::Sin, poly_libm::sin);
    cos(x): f64 = d1(F::Cos, poly_libm::cos);
    tan(x): f64 = d1(F::Tan, poly_libm::tan);
    asin(x): f64 = d1(F::Asin, poly_libm::asin);
    acos(x): f64 = d1(F::Acos, poly_libm::acos);
    atan(x): f64 = d1(F::Atan, poly_libm::atan);
    atan2(y, x): f64 = d2(F::Atan2, poly_libm::atan2);
    sinh(x): f64 = d1(F::Sinh, poly_libm::sinh);
    cosh(x): f64 = d1(F::Cosh, poly_libm::cosh);
    tanh(x): f64 = d1(F::Tanh, poly_libm::tanh);
    exp(x): f64 = d1(F::Exp, poly_libm::exp);
    log(x): f64 = d1(F::Log, poly_libm::log);
    log10(x): f64 = d1(F::Log10, poly_libm::log10);
    pow(x, y): f64 = d2(F::Pow, poly_libm::pow);
    _hypot(x, y): f64 = d2(F::Hypot, poly_libm::hypot);
    sinf(x): f32 = f1(F::Sin, poly_libm::sin);
    cosf(x): f32 = f1(F::Cos, poly_libm::cos);
    tanf(x): f32 = f1(F::Tan, poly_libm::tan);
    asinf(x): f32 = f1(F::Asin, poly_libm::asin);
    acosf(x): f32 = f1(F::Acos, poly_libm::acos);
    atanf(x): f32 = f1(F::Atan, poly_libm::atan);
    atan2f(y, x): f32 = f2(F::Atan2, poly_libm::atan2);
    sinhf(x): f32 = f1(F::Sinh, poly_libm::sinh);
    coshf(x): f32 = f1(F::Cosh, poly_libm::cosh);
    tanhf(x): f32 = f1(F::Tanh, poly_libm::tanh);
    expf(x): f32 = f1(F::Exp, poly_libm::exp);
    logf(x): f32 = f1(F::Log, poly_libm::log);
    log10f(x): f32 = f1(F::Log10, poly_libm::log10);
    powf(x, y): f32 = f2(F::Pow, poly_libm::pow);
    sqrtf(x): f32 = f1(F::Sqrt, poly_libm::sqrt);
    _hypotf(x, y): f32 = f2(F::Hypot, poly_libm::hypot);
}

/// `fmodf`: exact in double, so exact as a float.
#[unsafe(no_mangle)]
pub extern "C" fn fmodf(x: f32, y: f32) -> f32 {
    poly_crt::math::fmod(f64::from(x), f64::from(y)) as f32
}

/// `modff`.
///
/// # Safety
/// `int_part` NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn modff(x: f32, int_part: *mut f32) -> f32 {
    let (f, i) = poly_crt::math::modf(f64::from(x));
    if !int_part.is_null() {
        // SAFETY: writable.
        unsafe { *int_part = i as f32 };
    }
    f as f32
}

/// `frexpf` (`msvcrt.dll`): a float's subnormals are normal doubles, so the
/// double split is exact.
///
/// # Safety
/// `exp` NULL or writable.
pub unsafe fn frexpf(x: f32, exp: *mut i32) -> f32 {
    let (m, e) = poly_crt::math::frexp(f64::from(x));
    if !exp.is_null() {
        // SAFETY: writable.
        unsafe { *exp = e };
    }
    m as f32
}
