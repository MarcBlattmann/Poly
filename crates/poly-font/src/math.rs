//! The few float functions `core` lacks (`floor`, `ceil`, `sqrt`, ...
//! live in `std`): enough for coordinates of glyphs and text, which stay
//! far inside `i32`.

/// The largest integer not above `x` (for `|x| < 2^31`; beyond that the
/// value is clamped, which no glyph coordinate reaches).
pub fn floor(x: f32) -> f32 {
    if x.is_nan() {
        return 0.0;
    }
    let t = x as i32 as f32;
    if t > x { t - 1.0 } else { t }
}

/// The smallest integer not below `x`.
pub fn ceil(x: f32) -> f32 {
    -floor(-x)
}

/// `x` rounded to the nearest integer, halves away from zero.
pub fn round(x: f32) -> f32 {
    if x < 0.0 { -floor(-x + 0.5) } else { floor(x + 0.5) }
}

/// The square root of `x` (0 for `x <= 0`): a guess from the exponent and
/// three Newton steps, within a few ulps.
pub fn sqrt(x: f32) -> f32 {
    if x <= 0.0 || x.is_nan() {
        return 0.0;
    }
    if x.is_infinite() {
        return x;
    }
    let mut y = f32::from_bits((x.to_bits() >> 1) + 0x1fbd_1df5);
    for _ in 0..3 {
        y = 0.5 * (y + x / y);
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_ceil_round() {
        assert_eq!(floor(1.5), 1.0);
        assert_eq!(floor(-1.5), -2.0);
        assert_eq!(floor(-2.0), -2.0);
        assert_eq!(ceil(1.2), 2.0);
        assert_eq!(ceil(-1.2), -1.0);
        assert_eq!(ceil(3.0), 3.0);
        assert_eq!(round(2.5), 3.0);
        assert_eq!(round(-2.5), -3.0);
        assert_eq!(round(0.49), 0.0);
        assert_eq!(floor(f32::NAN), 0.0);
    }

    #[test]
    fn square_roots() {
        for &(x, want) in &[(4.0f32, 2.0f32), (2.0, core::f32::consts::SQRT_2), (1e-6, 1e-3), (12345.0, 111.10805)] {
            let got = sqrt(x);
            assert!((got - want).abs() <= want * 1e-5, "sqrt({x}) = {got}, want {want}");
        }
        assert_eq!(sqrt(-1.0), 0.0);
        assert_eq!(sqrt(0.0), 0.0);
    }
}
