//! The C99 `math.h` functions of the UCRT that only round, scale or compare, so the answer is exact
//! and the logic is `poly_crt::math` (host-tested): `ceilf`, `floorf`, `truncf`, `roundf`,
//! `copysign`, `fmax`, `fmin`, `rint`, `nearbyint`, `lrint`, `llrint`, `lround`, `llround`, `scalbn`,
//! `ilogb`, `nextafter`, `nexttoward`, `nan`, `imaxdiv` and the `float` / `long double` forms.
//! `long double` is `double` on Windows (Learn, "Data type ranges"), and `long` is 32 bits.
//! Doc-derived (Learn's `math.h` pages), not observed on Windows (ADR 0155 addendum, step 10).

use poly_crt::math;
use poly_win32_crt::misc;

macro_rules! unary {
    ($($name:ident($t:ty) -> $r:ty = $f:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: $t) -> $r {
            ($f)(x)
        }
    )*};
}

macro_rules! binary {
    ($($name:ident($t:ty, $u:ty) -> $r:ty = $f:expr;)*) => {$(
        #[unsafe(no_mangle)]
        pub extern "C" fn $name(x: $t, y: $u) -> $r {
            ($f)(x, y)
        }
    )*};
}

unary! {
    ceilf(f32) -> f32 = |x: f32| math::ceil(f64::from(x)) as f32;
    floorf(f32) -> f32 = |x: f32| math::floor(f64::from(x)) as f32;
    truncf(f32) -> f32 = |x: f32| math::trunc(f64::from(x)) as f32;
    roundf(f32) -> f32 = |x: f32| math::round(f64::from(x)) as f32;
    rint(f64) -> f64 = math::rint;
    rintf(f32) -> f32 = |x: f32| math::rint(f64::from(x)) as f32;
    nearbyint(f64) -> f64 = math::rint;
    nearbyintf(f32) -> f32 = |x: f32| math::rint(f64::from(x)) as f32;
    lrint(f64) -> i32 = |x: f64| math::to_i32(math::rint(x));
    lrintf(f32) -> i32 = |x: f32| math::to_i32(math::rint(f64::from(x)));
    llrint(f64) -> i64 = |x: f64| math::to_i64(math::rint(x));
    llrintf(f32) -> i64 = |x: f32| math::to_i64(math::rint(f64::from(x)));
    lround(f64) -> i32 = |x: f64| math::to_i32(math::round(x));
    lroundf(f32) -> i32 = |x: f32| math::to_i32(math::round(f64::from(x)));
    llround(f64) -> i64 = |x: f64| math::to_i64(math::round(x));
    llroundf(f32) -> i64 = |x: f32| math::to_i64(math::round(f64::from(x)));
    llroundl(f64) -> i64 = |x: f64| math::to_i64(math::round(x));
    ilogb(f64) -> i32 = math::ilogb;
    ilogbf(f32) -> i32 = |x: f32| math::ilogb(f64::from(x));
}

binary! {
    copysign(f64, f64) -> f64 = math::copysign;
    copysignf(f32, f32) -> f32 = |x: f32, y: f32| f32::from_bits((x.to_bits() & 0x7FFF_FFFF) | (y.to_bits() & 0x8000_0000));
    fmax(f64, f64) -> f64 = math::fmax;
    fmaxf(f32, f32) -> f32 = |x: f32, y: f32| math::fmax(f64::from(x), f64::from(y)) as f32;
    fmin(f64, f64) -> f64 = math::fmin;
    fminf(f32, f32) -> f32 = |x: f32, y: f32| math::fmin(f64::from(x), f64::from(y)) as f32;
    scalbn(f64, i32) -> f64 = math::ldexp;
    scalbnf(f32, i32) -> f32 = |x: f32, n: i32| math::ldexp(f64::from(x), n) as f32;
    nextafter(f64, f64) -> f64 = math::nextafter;
    nextafterf(f32, f32) -> f32 = math::nextafterf;
    nexttoward(f64, f64) -> f64 = math::nextafter;
}

/// `nan(tagp)`: a quiet NaN (the tag is not interpreted).
#[unsafe(no_mangle)]
pub extern "C" fn nan(_tag: *const u8) -> f64 {
    f64::from_bits(0x7FF8_0000_0000_0000)
}

/// `nanf(tagp)`.
#[unsafe(no_mangle)]
pub extern "C" fn nanf(_tag: *const u8) -> f32 {
    f32::from_bits(0x7FC0_0000)
}

/// `imaxdiv(numer, denom)`: `intmax_t` is `long long`.
#[unsafe(no_mangle)]
pub extern "C" fn imaxdiv(numer: i64, denom: i64) -> misc::LlDiv {
    misc::lldiv(numer, denom)
}
