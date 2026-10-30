//! Which math errors a call reports: the `_exception` types of `math.h`
//! (`_DOMAIN` 1, `_SING` 2, `_OVERFLOW` 3, `_UNDERFLOW` 4) that the C
//! runtime hands to the program's `_matherr` and turns into `errno`.
//!
//! Sources: C11 7.12.1 (domain, pole and range errors) and annex F, and the
//! tables on Learn's function pages ("Input / SEH exception / `_matherr`
//! exception"): for `exp`, `log`/`log10`, `sin`/`cos`/`tan` and their
//! siblings, a quiet NaN argument is listed as `_DOMAIN`, ±infinity as
//! `_DOMAIN` for `exp` and the circular functions, ±0 as `_SING` for the
//! logarithms, `exp`'s `_OVERFLOW` from 7.097827e+002 and `_UNDERFLOW`
//! from −7.083964e+002 (where the result leaves the normal range). The
//! C99 additions (`exp2`, `expm1`, `log1p`, `log2`, `cbrt`, `hypot`,
//! `remainder`) have no such table and follow C: a NaN argument is no
//! error. Poly's reading, to be confirmed by the spec agent (ADR 0092).

/// A math error, as `math.h`'s `_exception.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Fault {
    Domain = 1,
    Sing = 2,
    Overflow = 3,
    Underflow = 4,
}

/// The functions whose errors are classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Sinh,
    Cosh,
    Tanh,
    Exp,
    Exp2,
    Expm1,
    Log,
    Log2,
    Log10,
    Log1p,
    Pow,
    Sqrt,
    Cbrt,
    Hypot,
    Remainder,
    Fmod,
}

impl Function {
    /// Whether Learn documents the classic table (NaN → `_DOMAIN`).
    fn classic(self) -> bool {
        !matches!(
            self,
            Function::Exp2
                | Function::Expm1
                | Function::Log2
                | Function::Log1p
                | Function::Cbrt
                | Function::Hypot
                | Function::Remainder
        )
    }

    /// The name `_exception.name` points to (NUL-terminated), with the `f`
    /// of the single-precision form.
    pub fn name(self, single: bool) -> &'static [u8] {
        let (d, f): (&[u8], &[u8]) = match self {
            Function::Sin => (b"sin\0", b"sinf\0"),
            Function::Cos => (b"cos\0", b"cosf\0"),
            Function::Tan => (b"tan\0", b"tanf\0"),
            Function::Asin => (b"asin\0", b"asinf\0"),
            Function::Acos => (b"acos\0", b"acosf\0"),
            Function::Atan => (b"atan\0", b"atanf\0"),
            Function::Atan2 => (b"atan2\0", b"atan2f\0"),
            Function::Sinh => (b"sinh\0", b"sinhf\0"),
            Function::Cosh => (b"cosh\0", b"coshf\0"),
            Function::Tanh => (b"tanh\0", b"tanhf\0"),
            Function::Exp => (b"exp\0", b"expf\0"),
            Function::Exp2 => (b"exp2\0", b"exp2f\0"),
            Function::Expm1 => (b"expm1\0", b"expm1f\0"),
            Function::Log => (b"log\0", b"logf\0"),
            Function::Log2 => (b"log2\0", b"log2f\0"),
            Function::Log10 => (b"log10\0", b"log10f\0"),
            Function::Log1p => (b"log1p\0", b"log1pf\0"),
            Function::Pow => (b"pow\0", b"powf\0"),
            Function::Sqrt => (b"sqrt\0", b"sqrtf\0"),
            Function::Cbrt => (b"cbrt\0", b"cbrtf\0"),
            Function::Hypot => (b"_hypot\0", b"_hypotf\0"),
            Function::Remainder => (b"remainder\0", b"remainderf\0"),
            Function::Fmod => (b"fmod\0", b"fmodf\0"),
        };
        if single { f } else { d }
    }
}

/// The error a call `f(x, y)` that returned `r` reports (`y` is ignored
/// for one-argument functions; `single` checks the range of `float`).
pub fn fault(f: Function, x: f64, y: f64, r: f64, single: bool) -> Option<Fault> {
    use Function as F;
    let two = matches!(f, F::Atan2 | F::Pow | F::Hypot | F::Remainder | F::Fmod);
    let y = if two { y } else { 0.0 };
    let min_normal = if single { f64::from(f32::MIN_POSITIVE) } else { f64::MIN_POSITIVE };
    if x.is_nan() || y.is_nan() {
        // A NaN argument: an error only for the classic functions, and only
        // when the result is a NaN too (`pow(NaN, 0)` is 1).
        return (f.classic() && r.is_nan()).then_some(Fault::Domain);
    }
    let finite = x.is_finite() && y.is_finite();
    match f {
        F::Sin | F::Cos | F::Tan => (!x.is_finite()).then_some(Fault::Domain),
        F::Exp => {
            if x.is_infinite() {
                Some(Fault::Domain)
            } else if r.is_infinite() {
                Some(Fault::Overflow)
            } else if r < min_normal {
                Some(Fault::Underflow)
            } else {
                None
            }
        }
        F::Exp2 | F::Pow => {
            if f == F::Pow && x < 0.0 && x.is_finite() && y.is_finite() && r.is_nan() {
                Some(Fault::Domain)
            } else if f == F::Pow && x == 0.0 && y < 0.0 {
                Some(Fault::Sing)
            } else if !finite {
                None
            } else if r.is_infinite() {
                Some(Fault::Overflow)
            } else if r.abs() < min_normal && !(f == F::Pow && x == 0.0) {
                Some(Fault::Underflow)
            } else {
                None
            }
        }
        F::Expm1 | F::Sinh | F::Cosh | F::Hypot => (finite && r.is_infinite()).then_some(Fault::Overflow),
        F::Log | F::Log2 | F::Log10 => {
            if x == 0.0 {
                Some(Fault::Sing)
            } else if x < 0.0 {
                Some(Fault::Domain)
            } else {
                None
            }
        }
        F::Log1p => {
            if x == -1.0 {
                Some(Fault::Sing)
            } else if x < -1.0 {
                Some(Fault::Domain)
            } else {
                None
            }
        }
        F::Asin | F::Acos => (x.abs() > 1.0).then_some(Fault::Domain),
        F::Sqrt => (x < 0.0).then_some(Fault::Domain),
        F::Remainder | F::Fmod => (x.is_infinite() || y == 0.0).then_some(Fault::Domain),
        F::Atan | F::Atan2 | F::Tanh | F::Cbrt => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Function as F;

    #[test]
    fn classification() {
        let (inf, nan) = (f64::INFINITY, f64::NAN);
        assert_eq!(fault(F::Log, 0.0, 0.0, -inf, false), Some(Fault::Sing));
        assert_eq!(fault(F::Log, -1.0, 0.0, nan, false), Some(Fault::Domain));
        assert_eq!(fault(F::Log, nan, 0.0, nan, false), Some(Fault::Domain));
        assert_eq!(fault(F::Log2, nan, 0.0, nan, false), None);
        assert_eq!(fault(F::Exp, 710.0, 0.0, inf, false), Some(Fault::Overflow));
        assert_eq!(fault(F::Exp, -709.0, 0.0, 1.2e-308, false), Some(Fault::Underflow));
        assert_eq!(fault(F::Exp, -708.0, 0.0, 3.3e-308, false), None);
        assert_eq!(fault(F::Exp, -inf, 0.0, 0.0, false), Some(Fault::Domain));
        assert_eq!(fault(F::Exp, 89.0, 0.0, inf, true), Some(Fault::Overflow));
        assert_eq!(fault(F::Sin, inf, 0.0, nan, false), Some(Fault::Domain));
        assert_eq!(fault(F::Sin, 1e300, 0.0, 0.5, false), None);
        assert_eq!(fault(F::Pow, -2.0, 0.5, nan, false), Some(Fault::Domain));
        assert_eq!(fault(F::Pow, 0.0, -1.0, inf, false), Some(Fault::Sing));
        assert_eq!(fault(F::Pow, 10.0, 400.0, inf, false), Some(Fault::Overflow));
        assert_eq!(fault(F::Pow, 10.0, -400.0, 0.0, false), Some(Fault::Underflow));
        assert_eq!(fault(F::Pow, 0.0, 2.0, 0.0, false), None);
        assert_eq!(fault(F::Pow, nan, 0.0, 1.0, false), None);
        assert_eq!(fault(F::Pow, inf, 2.0, inf, false), None);
        assert_eq!(fault(F::Asin, 2.0, 0.0, nan, false), Some(Fault::Domain));
        assert_eq!(fault(F::Sinh, 800.0, 0.0, inf, false), Some(Fault::Overflow));
        assert_eq!(fault(F::Sinh, inf, 0.0, inf, false), None);
        assert_eq!(fault(F::Log1p, -1.0, 0.0, -inf, false), Some(Fault::Sing));
        assert_eq!(fault(F::Remainder, 1.0, 0.0, nan, false), Some(Fault::Domain));
        assert_eq!(fault(F::Atan2, 0.0, 0.0, 0.0, false), None);
        assert_eq!(F::Hypot.name(true), b"_hypotf\0");
        assert_eq!(Fault::Underflow as i32, 4);
    }
}
