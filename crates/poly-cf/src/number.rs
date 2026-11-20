//! CFNumber's values: the types of `<CoreFoundation/CFNumber.h>`, reading a
//! value as another type (`CFNumberGetValue`), comparing and hashing by
//! value, as the spec sheets `CFNumberCreate`, `CFNumberGetValue`, `CFEqual`
//! and `CFHash` describe them.

/// `CFNumberType` values.
pub const SINT8: i64 = 1;
pub const SINT16: i64 = 2;
pub const SINT32: i64 = 3;
pub const SINT64: i64 = 4;
pub const FLOAT32: i64 = 5;
pub const FLOAT64: i64 = 6;
pub const CHAR: i64 = 7;
pub const SHORT: i64 = 8;
pub const INT: i64 = 9;
pub const LONG: i64 = 10;
pub const LONG_LONG: i64 = 11;
pub const FLOAT: i64 = 12;
pub const DOUBLE: i64 = 13;
pub const CFINDEX: i64 = 14;
pub const NSINTEGER: i64 = 15;
pub const CGFLOAT: i64 = 16;

/// 2^63: the first double past the 64-bit integers.
const TWO_63: f64 = 9_223_372_036_854_775_808u64 as f64;

/// The multiplier of the integer hash (spec sheet `CFHash`).
pub const HASH_FACTOR: u64 = 2_654_435_761;

/// The fixed-size type a C type is stored as (`CFNumberGetType`); `None` for
/// an unknown type.
pub fn canonical(t: i64) -> Option<i64> {
    Some(match t {
        SINT8 | CHAR => SINT8,
        SINT16 | SHORT => SINT16,
        SINT32 | INT => SINT32,
        SINT64 | LONG | LONG_LONG | CFINDEX | NSINTEGER => SINT64,
        FLOAT32 | FLOAT => FLOAT32,
        FLOAT64 | DOUBLE | CGFLOAT => FLOAT64,
        _ => return None,
    })
}

/// The byte size of a value of type `t`.
pub fn byte_size(t: i64) -> usize {
    match canonical(t) {
        Some(SINT8) => 1,
        Some(SINT16) => 2,
        Some(SINT32) | Some(FLOAT32) => 4,
        _ => 8,
    }
}

pub fn is_float_type(t: i64) -> bool {
    matches!(canonical(t), Some(FLOAT32) | Some(FLOAT64))
}

/// The name `CFCopyDescription` gives a stored type.
pub fn type_name(t: i64) -> &'static str {
    match canonical(t) {
        Some(SINT8) => "kCFNumberSInt8Type",
        Some(SINT16) => "kCFNumberSInt16Type",
        Some(SINT32) => "kCFNumberSInt32Type",
        Some(FLOAT32) => "kCFNumberFloat32Type",
        Some(FLOAT64) => "kCFNumberFloat64Type",
        _ => "kCFNumberSInt64Type",
    }
}

/// A number's value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Num {
    Int(i64),
    Float(f64),
}

/// Read a C value of type `t` from its bytes (little endian, the hosts'
/// order). `None` for an unknown type.
pub fn from_bytes(t: i64, b: &[u8]) -> Option<Num> {
    let c = canonical(t)?;
    let at = |n: usize| -> [u8; 8] {
        let mut x = [0u8; 8];
        x[..n].copy_from_slice(&b[..n]);
        x
    };
    Some(match c {
        SINT8 => Num::Int(i64::from(b[0] as i8)),
        SINT16 => Num::Int(i64::from(i16::from_le_bytes([b[0], b[1]]))),
        SINT32 => Num::Int(i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))),
        SINT64 => Num::Int(i64::from_le_bytes(at(8))),
        FLOAT32 => Num::Float(f64::from(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))),
        _ => Num::Float(f64::from_le_bytes(at(8))),
    })
}

/// Whether `d` has no fractional part (and is finite).
pub fn is_whole(d: f64) -> bool {
    d.is_finite() && (d.abs() >= 4_503_599_627_370_496.0 || d == (d as i64) as f64)
}

/// The x86-64 truncating conversion (`cvttsd2si`): out of range and NaN
/// give the smallest integer of the width.
fn trunc_x86(d: f64, bits: u32) -> i64 {
    if bits == 64 {
        if d.is_nan() || !(-TWO_63..TWO_63).contains(&d) { i64::MIN } else { d as i64 }
    } else if d.is_nan() || d >= 2_147_483_648.0 || d <= -2_147_483_649.0 {
        i64::from(i32::MIN)
    } else {
        i64::from(d as i32)
    }
}

/// `CFNumberGetValue`: the value as type `t` in its bytes, and whether the
/// conversion was exact. Integers narrow by wrapping (exact); a fraction
/// read as an integer is truncated (not exact); NaN as an integer is not
/// exact; a double that a float cannot hold is not exact.
pub fn convert(n: Num, t: i64) -> Option<([u8; 8], bool)> {
    let c = canonical(t)?;
    let size = byte_size(c);
    let mut out = [0u8; 8];
    let exact = match (n, c) {
        (Num::Int(v), FLOAT32) => {
            let f = v as f32;
            out[..4].copy_from_slice(&f.to_le_bytes());
            f as f64 == v as f64 && (f as f64) as i64 == v
        }
        (Num::Int(v), FLOAT64) => {
            let d = v as f64;
            out.copy_from_slice(&d.to_le_bytes());
            d as i64 == v && d < TWO_63
        }
        (Num::Int(v), _) => {
            out[..size].copy_from_slice(&v.to_le_bytes()[..size]);
            true
        }
        (Num::Float(d), FLOAT32) => {
            let f = d as f32;
            out[..4].copy_from_slice(&f.to_le_bytes());
            d.is_nan() || f64::from(f) == d
        }
        (Num::Float(d), FLOAT64) => {
            out.copy_from_slice(&d.to_le_bytes());
            true
        }
        (Num::Float(d), _) => {
            let v = trunc_x86(d, if size == 8 { 64 } else { 32 });
            out[..size].copy_from_slice(&v.to_le_bytes()[..size]);
            !d.is_nan() && (is_whole(d) || d.is_infinite())
        }
    };
    Some((out, exact))
}

/// Compare an integer with a double exactly.
fn cmp_int_float(i: i64, d: f64) -> i32 {
    if d.is_nan() {
        return 1; // NaN is below every number
    }
    if d >= TWO_63 {
        return -1;
    }
    if d < -TWO_63 {
        return 1;
    }
    let t = d as i64; // truncated toward zero, in range
    if i != t {
        return if i < t { -1 } else { 1 };
    }
    let frac = d - t as f64;
    if frac > 0.0 {
        -1
    } else if frac < 0.0 {
        1
    } else {
        0
    }
}

/// `CFNumberCompare`: by value across types; NaN equals NaN and is below
/// every other number.
pub fn compare(a: Num, b: Num) -> i32 {
    match (a, b) {
        (Num::Int(x), Num::Int(y)) => (x > y) as i32 - (x < y) as i32,
        (Num::Int(x), Num::Float(y)) => cmp_int_float(x, y),
        (Num::Float(x), Num::Int(y)) => -cmp_int_float(y, x),
        (Num::Float(x), Num::Float(y)) => match (x.is_nan(), y.is_nan()) {
            (true, true) => 0,
            (true, false) => -1,
            (false, true) => 1,
            _ => (x > y) as i32 - (x < y) as i32,
        },
    }
}

/// `CFHash` of a number: an integer value hashes to its absolute value
/// times [`HASH_FACTOR`]; a whole double like the integer.
pub fn hash(n: Num) -> u64 {
    match n {
        Num::Int(v) => v.unsigned_abs().wrapping_mul(HASH_FACTOR),
        Num::Float(d) if is_whole(d) && d.abs() < 9.2e18 => (d as i64).unsigned_abs().wrapping_mul(HASH_FACTOR),
        Num::Float(d) if d.is_nan() => 0x7ff8_0000_0000_0000u64.wrapping_mul(HASH_FACTOR),
        Num::Float(d) => {
            let bits = d.to_bits();
            (bits ^ (bits >> 29)).wrapping_mul(HASH_FACTOR)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_int(n: Num, t: i64) -> (i64, bool) {
        let (b, exact) = convert(n, t).unwrap();
        let v = match byte_size(t) {
            1 => i64::from(b[0] as i8),
            2 => i64::from(i16::from_le_bytes([b[0], b[1]])),
            4 => i64::from(i32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            _ => i64::from_le_bytes(b),
        };
        (v, exact)
    }

    #[test]
    fn types() {
        assert_eq!(canonical(LONG), Some(SINT64));
        assert_eq!(canonical(CGFLOAT), Some(FLOAT64));
        assert_eq!(byte_size(FLOAT), 4);
        assert!(is_float_type(DOUBLE));
        assert_eq!(canonical(99), None);
    }

    #[test]
    fn narrowing() {
        assert_eq!(get_int(Num::Int(300), CHAR), (44, true));
        assert_eq!(get_int(Num::Int(-129), SINT8), (127, true));
        assert_eq!(get_int(Num::Int(70000), SHORT), (4464, true));
        assert_eq!(get_int(Num::Float(2.75), INT), (2, false));
        assert_eq!(get_int(Num::Float(-2.75), INT), (-2, false));
        assert_eq!(get_int(Num::Float(1e30), INT), (i64::from(i32::MIN), true));
        assert_eq!(get_int(Num::Float(1e30), LONG_LONG), (i64::MIN, true));
        assert!(!get_int(Num::Float(f64::NAN), INT).1);
        assert!(!convert(Num::Float(0.1), FLOAT).unwrap().1);
        assert!(convert(Num::Float(0.5), FLOAT).unwrap().1);
        assert!(convert(Num::Int(1 << 40), DOUBLE).unwrap().1);
        assert_eq!(get_int(Num::Float(3.0), INT), (3, true));
    }

    #[test]
    fn order_and_hash() {
        assert_eq!(compare(Num::Int(1), Num::Float(1.5)), -1);
        assert_eq!(compare(Num::Float(2.5), Num::Int(2)), 1);
        assert_eq!(compare(Num::Int(7), Num::Float(7.0)), 0);
        assert_eq!(compare(Num::Float(f64::NAN), Num::Float(f64::NAN)), 0);
        assert_eq!(compare(Num::Float(f64::NAN), Num::Int(1)), -1);
        assert_eq!(compare(Num::Int(1), Num::Float(f64::NAN)), 1);
        assert_eq!(compare(Num::Float(f64::NEG_INFINITY), Num::Int(-1000)), -1);
        assert_eq!(compare(Num::Int(i64::MAX), Num::Float(9.3e18)), -1);
        assert_eq!(hash(Num::Int(0)), 0);
        assert_eq!(hash(Num::Int(1)), 2654435761);
        assert_eq!(hash(Num::Int(-5)), 13272178805);
        assert_eq!(hash(Num::Int(12345)), 32769009469545);
        assert_eq!(hash(Num::Float(1.0)), hash(Num::Int(1)));
        assert_eq!(hash(Num::Int(i64::MIN)), (1u64 << 63).wrapping_mul(HASH_FACTOR));
    }

    #[test]
    fn bytes() {
        assert_eq!(from_bytes(INT, &(-12i32).to_le_bytes()), Some(Num::Int(-12)));
        assert_eq!(from_bytes(FLOAT, &1.5f32.to_le_bytes()), Some(Num::Float(1.5)));
        assert_eq!(from_bytes(SHORT, &(-3i16).to_le_bytes()), Some(Num::Int(-3)));
    }
}
