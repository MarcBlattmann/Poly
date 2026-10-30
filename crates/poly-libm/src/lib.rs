//! The transcendental functions of our C runtime's `math.h` (ADR 0092),
//! written from first principles: argument reduction, Taylor series with
//! error bounds, and double-double arithmetic ([`dd`]) so that each
//! function's result is rounded once from a value whose relative error is
//! below about 2^-75.
//!
//! | Module | Functions |
//! |---|---|
//! | [`trig`] | `sin`, `cos`, `tan` (Payne–Hanek reduction for every argument) |
//! | [`atan`] | `atan`, `atan2`, `asin`, `acos` |
//! | [`exp`] | `exp`, `exp2`, `expm1`, `sinh`, `cosh`, `tanh` |
//! | [`log`] | `log`, `log2`, `log10`, `log1p` |
//! | [`pow`] | `pow` |
//! | [`roots`] | `sqrt`, `cbrt`, `hypot` |
//! | [`rem`] | `remainder` |
//! | [`fault`] | which `_DOMAIN`/`_SING`/`_OVERFLOW`/`_UNDERFLOW` a call reports |
//!
//! **Accuracy.** The target is at most one ulp for every function and
//! every argument; the design gives within 0.5 + 2^-20 ulp (each module
//! states its bound), so nearly every result is the correctly rounded
//! one. The tests measure every function against 4,000 exact values
//! (`tests/reference.txt`, computed with Python's `decimal` module at 70+
//! digits by `tests/gen_reference.py`) and check identities over hundreds
//! of thousands of random arguments. The `float` forms compute in double
//! and round once more, which is within 0.5 + 2^-28 ulp of `float`.
//! `long double` is `double` on Windows (Learn, "Data type ranges"), so the
//! `l` forms are the double ones.
//!
//! Special values follow C11 annex F (IEEE 754): signed zeros are kept
//! (`sin(−0) = −0`), infinities and NaNs give the results F.10 lists.
//!
//! Clean room (CLEAN_ROOM.md): no libm or C runtime source was read; the
//! methods are the textbook ones each module cites.

#![no_std]

#[cfg(test)]
extern crate std;

pub mod atan;
pub mod consts;
pub mod dd;
pub mod exp;
pub mod fault;
pub mod log;
pub mod pow;
pub mod rem;
pub mod roots;
pub mod trig;

pub use atan::{acos, asin, atan, atan2};
pub use exp::{cosh, exp, exp2, expm1, sinh, tanh};
pub use fault::{Fault, Function, fault};
pub use log::{log, log1p, log2, log10};
pub use pow::pow;
pub use rem::remainder;
pub use roots::{cbrt, hypot, sqrt};
pub use trig::{cos, sin, tan};

/// The `float` forms: the double function, rounded once more.
pub mod single {
    macro_rules! one {
        ($($name:ident = $f:path;)*) => {$(
            pub fn $name(x: f32) -> f32 {
                $f(f64::from(x)) as f32
            }
        )*};
    }
    macro_rules! two {
        ($($name:ident = $f:path;)*) => {$(
            pub fn $name(x: f32, y: f32) -> f32 {
                $f(f64::from(x), f64::from(y)) as f32
            }
        )*};
    }
    one! {
        sinf = crate::sin;
        cosf = crate::cos;
        tanf = crate::tan;
        asinf = crate::asin;
        acosf = crate::acos;
        atanf = crate::atan;
        sinhf = crate::sinh;
        coshf = crate::cosh;
        tanhf = crate::tanh;
        expf = crate::exp;
        exp2f = crate::exp2;
        expm1f = crate::expm1;
        logf = crate::log;
        log2f = crate::log2;
        log10f = crate::log10;
        log1pf = crate::log1p;
        sqrtf = crate::sqrt;
        cbrtf = crate::cbrt;
    }
    two! {
        atan2f = crate::atan2;
        powf = crate::pow;
        hypotf = crate::hypot;
        remainderf = crate::remainder;
    }
}
