//! libsystem_m (ADR 0257 addendum): `<math.h>`. The transcendental
//! functions are poly-libm's (ADR 0092, correctly rounded nearly always);
//! the rest are exact operations on the bits. `long double` is `double`
//! on arm64.

use poly_libm as m;

macro_rules! unary {
    ($($name:ident = $f:expr;)*) => {
        $(
            #[unsafe(no_mangle)]
            extern "C" fn $name(x: f64) -> f64 {
                $f(x)
            }
        )*
    };
}

unary! {
    acos = m::acos;
    asin = m::asin;
    atan = m::atan;
    cos = m::cos;
    sin = m::sin;
    tan = m::tan;
    cosh = m::cosh;
    sinh = m::sinh;
    tanh = m::tanh;
    exp = m::exp;
    exp2 = m::exp2;
    expm1 = m::expm1;
    log = m::log;
    log2 = m::log2;
    log10 = m::log10;
    log1p = m::log1p;
    sqrt = m::sqrt;
    cbrt = m::cbrt;
    floor = floor_;
    ceil = ceil_;
    trunc = trunc_;
    round = round_;
    fabs = f64::abs;
}

#[unsafe(no_mangle)]
extern "C" fn atan2(y: f64, x: f64) -> f64 {
    m::atan2(y, x)
}

#[unsafe(no_mangle)]
extern "C" fn pow(x: f64, y: f64) -> f64 {
    m::pow(x, y)
}

#[unsafe(no_mangle)]
extern "C" fn hypot(x: f64, y: f64) -> f64 {
    m::hypot(x, y)
}

#[unsafe(no_mangle)]
extern "C" fn remainder(x: f64, y: f64) -> f64 {
    m::remainder(x, y)
}

#[unsafe(no_mangle)]
extern "C" fn copysign(x: f64, y: f64) -> f64 {
    x.copysign(y)
}

#[unsafe(no_mangle)]
extern "C" fn fmin(x: f64, y: f64) -> f64 {
    if x.is_nan() || (y < x) { y } else { x }
}

#[unsafe(no_mangle)]
extern "C" fn fmax(x: f64, y: f64) -> f64 {
    if x.is_nan() || (y > x) { y } else { x }
}

/// `x` without its fraction, rounded toward zero (exact on the bits).
fn trunc_(x: f64) -> f64 {
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32 - 1023;
    if e >= 52 {
        return x; // already an integer, an infinity or a NaN
    }
    if e < 0 {
        return f64::from_bits(bits & (1 << 63)); // ±0
    }
    f64::from_bits(bits & !((1u64 << (52 - e)) - 1))
}

fn floor_(x: f64) -> f64 {
    let t = trunc_(x);
    if t > x { t - 1.0 } else { t }
}

fn ceil_(x: f64) -> f64 {
    let t = trunc_(x);
    if t < x { t + 1.0 } else { t }
}

/// Halfway cases away from zero (C11 7.12.9.6).
fn round_(x: f64) -> f64 {
    let t = trunc_(x);
    if (x - t).abs() >= 0.5 { t + x.signum() } else { t }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn modf(x: f64, ip: *mut f64) -> f64 {
    let t = trunc_(x);
    // SAFETY: the C contract.
    unsafe { *ip = t };
    if x.is_infinite() { 0.0f64.copysign(x) } else { (x - t).copysign(x) }
}

/// `frexp`: x = m × 2^e with 0.5 ≤ |m| < 1.
#[unsafe(no_mangle)]
unsafe extern "C" fn frexp(x: f64, e: *mut i32) -> f64 {
    let (m, ex) = frexp_(x);
    // SAFETY: the C contract.
    unsafe { *e = ex };
    m
}

fn frexp_(x: f64) -> (f64, i32) {
    if x == 0.0 || x.is_nan() || x.is_infinite() {
        return (x, 0);
    }
    let mut bits = x.to_bits();
    let mut e = ((bits >> 52) & 0x7ff) as i32;
    let mut bias = 0;
    if e == 0 {
        // Subnormal: scale into the normal range first.
        bits = (x * f64::from_bits(0x4350_0000_0000_0000)).to_bits(); // × 2^54
        e = ((bits >> 52) & 0x7ff) as i32;
        bias = 54;
    }
    let m = f64::from_bits((bits & !(0x7ffu64 << 52)) | (1022u64 << 52));
    (m, e - 1022 - bias)
}

/// `ldexp`: x × 2^n, rounded once.
#[unsafe(no_mangle)]
extern "C" fn ldexp(x: f64, n: i32) -> f64 {
    scalbn(x, n)
}

#[unsafe(no_mangle)]
extern "C" fn scalbn(mut x: f64, mut n: i32) -> f64 {
    // Steps of at most 2^1023 / 2^-969 keep every step but the last exact.
    let big = f64::from_bits(0x7fe0_0000_0000_0000); // 2^1023
    let small = f64::from_bits(0x0360_0000_0000_0000); // 2^-969
    while n > 1023 {
        x *= big;
        n -= 1023;
        if x.is_infinite() {
            return x;
        }
    }
    while n < -969 {
        x *= small;
        n += 969;
        if x == 0.0 {
            return x;
        }
    }
    x * f64::from_bits(((n + 1023) as u64) << 52)
}

/// `fmod`: x − n × y with n = trunc(x / y), exact (C11 7.12.10.1).
#[unsafe(no_mangle)]
extern "C" fn fmod(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() || x.is_infinite() || y == 0.0 {
        return f64::NAN;
    }
    if y.is_infinite() || x == 0.0 {
        return x;
    }
    let (ax, ay) = (x.abs(), y.abs());
    if ax < ay {
        return x;
    }
    // Integer mantissas and exponents, then binary long division.
    let parts = |v: f64| {
        let (m, e) = frexp_(v);
        ((m * f64::from_bits(0x4340_0000_0000_0000)) as u64, e - 53) // m × 2^53
    };
    let (mut mx, ex) = parts(ax);
    let (my, ey) = parts(ay);
    let mut d = ex - ey;
    while d > 0 {
        if mx >= my {
            mx -= my;
        }
        mx <<= 1;
        d -= 1;
    }
    if mx >= my {
        mx -= my;
    }
    let r = scalbn(mx as f64, ey);
    r.copysign(x)
}

#[cfg(test)]
mod tests {}
