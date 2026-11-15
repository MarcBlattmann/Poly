//! Small, fully documented functions the Windows applications of the survey import (ADR 0155
//! addendum, step 10): `MulDiv`, `EncodePointer` / `DecodePointer`, the process error mode and
//! `InterlockedIncrement` / `InterlockedDecrement`, plus `IsEqualGUID`. Doc-derived from Learn, not
//! observed on Windows.
//!
//! - `MulDiv` (`winbase.h`): "Multiplies two 32-bit values and then divides the 64-bit result by a
//!   third 32-bit value. The return value is rounded up or down to the nearest integer. If either an
//!   overflow occurred or `nDenominator` was 0, the return value is -1." A tie rounds away from zero
//!   (Learn does not say; the usual reading of "nearest", to be confirmed).
//! - `EncodePointer` / `DecodePointer` (`utilapiset.h`): opaque to the caller; the encoding here is
//!   a rotation and an exclusive-or with a per-process cookie, so a null pointer does not encode to
//!   null and `DecodePointer(EncodePointer(p)) == p`.
//! - `SetErrorMode(uMode)` / `GetErrorMode` (`errhandlingapi.h`): the four `SEM_*` flags Learn lists;
//!   `SetErrorMode` returns the previous state.
//! - `InterlockedIncrement` / `InterlockedDecrement` (`winnt.h`): "The resulting incremented /
//!   decremented value".
//! - `IsEqualGUID(rguid1, rguid2)` (`guiddef.h`): whether the two 128-bit values are equal.

use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// `MulDiv(number, numerator, denominator)`.
pub fn mul_div(number: i32, numerator: i32, denominator: i32) -> i32 {
    if denominator == 0 {
        return -1;
    }
    let product = i64::from(number) * i64::from(numerator); // fits: 2^62 at most
    let d = i64::from(denominator);
    // Round to nearest, ties away from zero: add half the divisor's magnitude on the side of the
    // product's sign, then divide (truncating toward zero).
    let half = d.abs() / 2;
    let adjusted = if product < 0 { product - half } else { product + half };
    i32::try_from(adjusted / d).unwrap_or(-1)
}

/// The pointer encoding: rotate right by the low six bits of the cookie, then exclusive-or with it.
pub fn encode_pointer(ptr: u64, cookie: u64) -> u64 {
    ptr.rotate_right((cookie & 63) as u32) ^ cookie
}

/// The inverse of [`encode_pointer`].
pub fn decode_pointer(encoded: u64, cookie: u64) -> u64 {
    (encoded ^ cookie).rotate_left((cookie & 63) as u32)
}

/// `SEM_FAILCRITICALERRORS`, `SEM_NOGPFAULTERRORBOX`, `SEM_NOALIGNMENTFAULTEXCEPT`,
/// `SEM_NOOPENFILEERRORBOX` (`errhandlingapi.h`).
pub const SEM_FLAGS: u32 = 0x0001 | 0x0002 | 0x0004 | 0x8000;

/// `SetErrorMode`: store the flags Learn lists, return the previous state.
pub fn set_error_mode(state: &AtomicU32, mode: u32) -> u32 {
    state.swap(mode & SEM_FLAGS, Ordering::SeqCst)
}

/// `InterlockedIncrement` / `InterlockedDecrement`: the new value (wrapping).
pub fn interlocked_add(value: &AtomicI32, delta: i32) -> i32 {
    value.fetch_add(delta, Ordering::SeqCst).wrapping_add(delta)
}

/// `IsEqualGUID`.
pub fn guid_eq(a: &[u8; 16], b: &[u8; 16]) -> bool {
    a == b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_div_rounds_to_the_nearest() {
        assert_eq!(mul_div(10, 3, 4), 8); // 7.5 rounds away from zero
        assert_eq!(mul_div(10, 3, 5), 6);
        assert_eq!(mul_div(-10, 3, 4), -8);
        assert_eq!(mul_div(10, -3, 4), -8);
        assert_eq!(mul_div(-10, -3, 4), 8);
        assert_eq!(mul_div(10, 3, -4), -8);
        assert_eq!(mul_div(7, 1, 3), 2); // 2.33 down
        assert_eq!(mul_div(8, 1, 3), 3); // 2.67 up
        assert_eq!(mul_div(0, 5, 7), 0);
        assert_eq!(mul_div(1, 1, 2), 1);
        assert_eq!(mul_div(-1, 1, 2), -1);
    }

    #[test]
    fn mul_div_fails_with_minus_one() {
        assert_eq!(mul_div(5, 5, 0), -1);
        assert_eq!(mul_div(i32::MAX, i32::MAX, 1), -1);
        assert_eq!(mul_div(i32::MAX, 2, 1), -1);
        assert_eq!(mul_div(i32::MAX, 1, 1), i32::MAX);
        // A 64-bit intermediate: the product does not overflow.
        assert_eq!(mul_div(i32::MAX, 2, 2), i32::MAX);
        assert_eq!(mul_div(i32::MIN, 1, 1), i32::MIN);
        assert_eq!(mul_div(i32::MIN, -1, 1), -1); // 2^31 does not fit
    }

    #[test]
    fn encoded_pointers_round_trip_and_hide_null() {
        let cookie = 0x9E37_79B9_7F4A_7C15u64;
        for p in [0u64, 1, 0x7FF0_0000_1000, 0xFFFF_FFFF_FFFF_FFFF, 0xDEAD_BEEF] {
            let e = encode_pointer(p, cookie);
            assert_eq!(decode_pointer(e, cookie), p);
        }
        assert_ne!(encode_pointer(0, cookie), 0);
        assert_ne!(encode_pointer(0x1000, cookie), 0x1000);
        // Another cookie decodes it to something else.
        assert_ne!(decode_pointer(encode_pointer(0x1000, cookie), cookie ^ 1), 0x1000);
    }

    #[test]
    fn the_error_mode_keeps_the_documented_flags_and_returns_the_old_state() {
        let state = AtomicU32::new(0);
        assert_eq!(set_error_mode(&state, 0x8003), 0);
        assert_eq!(state.load(Ordering::SeqCst), 0x8003);
        assert_eq!(set_error_mode(&state, 0x0004), 0x8003);
        assert_eq!(set_error_mode(&state, 0xFFFF_FFF0), 0x0004);
        assert_eq!(state.load(Ordering::SeqCst), 0x8000);
    }

    #[test]
    fn interlocked_functions_return_the_new_value() {
        let v = AtomicI32::new(41);
        assert_eq!(interlocked_add(&v, 1), 42);
        assert_eq!(interlocked_add(&v, -1), 41);
        let max = AtomicI32::new(i32::MAX);
        assert_eq!(interlocked_add(&max, 1), i32::MIN);
    }

    #[test]
    fn guids_compare_by_value() {
        let a = [1u8; 16];
        let mut b = a;
        assert!(guid_eq(&a, &b));
        b[15] = 2;
        assert!(!guid_eq(&a, &b));
    }
}
