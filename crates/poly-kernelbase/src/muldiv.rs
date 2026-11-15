//! `MulDiv` (`winbase.h`; Learn: "MulDiv function"): `nNumber * nNumerator / nDenominator` through
//! a 64-bit intermediate product, rounded to the nearest integer; -1 when the denominator is 0 or
//! the result does not fit in 32 bits.

/// The result of `MulDiv(number, numerator, denominator)`.
///
/// Learn: "If the result is a positive half integer (ends in .5), it is rounded up. If the
/// result is a negative half integer, it is rounded down": halves round away from zero.
pub fn mul_div(number: i32, numerator: i32, denominator: i32) -> i32 {
    if denominator == 0 {
        return -1;
    }
    let product = i64::from(number) * i64::from(numerator);
    let d = i64::from(denominator).unsigned_abs();
    let magnitude = (product.unsigned_abs() + d / 2) / d;
    if (product < 0) != (denominator < 0) && product != 0 {
        if magnitude > 1 << 31 { -1 } else { (magnitude as i64).wrapping_neg() as i32 }
    } else if magnitude > i32::MAX as u64 {
        -1
    } else {
        magnitude as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_nearest_and_halves_away_from_zero() {
        assert_eq!(mul_div(1, 1, 2), 1); // 0.5 -> 1
        assert_eq!(mul_div(-1, 1, 2), -1); // -0.5 -> -1
        assert_eq!(mul_div(3, 1, 2), 2); // 1.5 -> 2
        assert_eq!(mul_div(-3, 1, 2), -2); // -1.5 -> -2
        assert_eq!(mul_div(10, 10, 3), 33); // 33.33
        assert_eq!(mul_div(10, 20, 3), 67); // 66.67
        assert_eq!(mul_div(-10, 20, 3), -67);
        assert_eq!(mul_div(10, 20, -3), -67);
        assert_eq!(mul_div(-10, -20, -3), -67);
        assert_eq!(mul_div(0, 5, -3), 0);
        // A typical DPI scaling.
        assert_eq!(mul_div(8, 96, 72), 11);
    }

    #[test]
    fn a_zero_denominator_or_an_overflow_is_minus_one() {
        assert_eq!(mul_div(5, 5, 0), -1);
        assert_eq!(mul_div(i32::MAX, 2, 1), -1);
        assert_eq!(mul_div(i32::MIN, 1, 1), i32::MIN);
        assert_eq!(mul_div(i32::MIN, -1, 1), -1);
        // The 64-bit intermediate does not overflow.
        assert_eq!(mul_div(i32::MAX, i32::MAX, i32::MAX), i32::MAX);
    }
}
