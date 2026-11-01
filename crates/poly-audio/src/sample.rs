//! Sample formats, little-endian, and their conversion to and from the
//! internal domain ([`FULL_SCALE`](crate::FULL_SCALE): signed, 24
//! significant bits in an `i32`).
//!
//! Integer formats shift into place (unsigned ones are offset by half
//! their range first). IEEE 754 binary32 and binary64 samples, nominally
//! in -1.0..1.0, are decoded and built from their sign, exponent and
//! significand with integer arithmetic only; values at or beyond full
//! scale are clipped, NaN is silence. Encoding clips to the format's
//! range.

use crate::FULL_SCALE;

/// A sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sample {
    U8,
    S8,
    S16,
    U16,
    /// 24 significant bits in the low bytes of 32.
    S24,
    U24,
    /// 24 bits packed in 3 bytes.
    S24_3,
    U24_3,
    S32,
    U32,
    /// IEEE 754 binary32.
    Float,
    /// IEEE 754 binary64.
    Float64,
}

const MAX: i32 = FULL_SCALE - 1;
const MIN: i32 = -FULL_SCALE;

fn le(b: &[u8], n: usize) -> u64 {
    let mut v = 0u64;
    for (i, &x) in b.iter().take(n).enumerate() {
        v |= (x as u64) << (8 * i);
    }
    v
}

fn put_le(out: &mut [u8], n: usize, v: u64) {
    for (i, o) in out.iter_mut().take(n).enumerate() {
        *o = (v >> (8 * i)) as u8;
    }
}

/// Sign-extend the low `bits` of `v`.
fn sext(v: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((v << shift) as i64) >> shift
}

impl Sample {
    /// Bytes per sample.
    pub const fn bytes(self) -> usize {
        match self {
            Sample::U8 | Sample::S8 => 1,
            Sample::S16 | Sample::U16 => 2,
            Sample::S24_3 | Sample::U24_3 => 3,
            Sample::S24 | Sample::U24 | Sample::S32 | Sample::U32 | Sample::Float => 4,
            Sample::Float64 => 8,
        }
    }

    /// One sample of silence, `bytes()` long (the first bytes of the
    /// array).
    pub fn silence(self) -> [u8; 8] {
        let mut s = [0u8; 8];
        self.encode(0, &mut s);
        s
    }

    /// The sample at the start of `b` (at least [`bytes`](Self::bytes)
    /// long; missing bytes read as zero) in the internal domain.
    pub fn decode(self, b: &[u8]) -> i32 {
        let n = self.bytes();
        let v = le(b, n);
        match self {
            Sample::U8 => ((v as i32) - 0x80) << 16,
            Sample::S8 => (sext(v, 8) as i32) << 16,
            Sample::S16 => (sext(v, 16) as i32) << 8,
            Sample::U16 => ((v as i32) - 0x8000) << 8,
            Sample::S24 | Sample::S24_3 => sext(v & 0xff_ffff, 24) as i32,
            Sample::U24 | Sample::U24_3 => ((v & 0xff_ffff) as i32) - 0x80_0000,
            Sample::S32 => (sext(v, 32) >> 8) as i32,
            Sample::U32 => ((v as u32 ^ 0x8000_0000) as i32) >> 8,
            Sample::Float => float_to_int(v, 8, 23),
            Sample::Float64 => float_to_int(v, 11, 52),
        }
    }

    /// Store `v` (clipped to full scale) at the start of `out`.
    pub fn encode(self, v: i32, out: &mut [u8]) {
        let v = v.clamp(MIN, MAX);
        let n = self.bytes();
        let bits: u64 = match self {
            Sample::U8 => ((v >> 16) + 0x80) as u64,
            Sample::S8 => (v >> 16) as u8 as u64,
            Sample::S16 => (v >> 8) as u16 as u64,
            Sample::U16 => ((v >> 8) + 0x8000) as u64,
            Sample::S24 => v as u32 as u64 & 0xffff_ffff,
            Sample::S24_3 => v as u32 as u64 & 0xff_ffff,
            Sample::U24 | Sample::U24_3 => (v + 0x80_0000) as u64,
            Sample::S32 => ((v as i64) << 8) as u32 as u64,
            Sample::U32 => (((v as i64) << 8) as u32 ^ 0x8000_0000) as u64,
            Sample::Float => int_to_float(v, 8, 23),
            Sample::Float64 => int_to_float(v, 11, 52),
        };
        put_le(out, n, bits);
    }
}

/// An IEEE 754 value with `e` exponent bits and `m` significand bits, as
/// a sample: `value * FULL_SCALE`, clipped.
fn float_to_int(bits: u64, e: u32, m: u32) -> i32 {
    let negative = (bits >> (e + m)) & 1 != 0;
    let exp = ((bits >> m) & ((1 << e) - 1)) as i64;
    let frac = bits & ((1u64 << m) - 1);
    let max_exp = (1i64 << e) - 1;
    let bias = (1i64 << (e - 1)) - 1;
    if exp == max_exp && frac != 0 {
        return 0; // NaN
    }
    let magnitude: i64 = if exp == 0 {
        0 // zero and subnormals (far below one step)
    } else if exp == max_exp || exp >= bias {
        FULL_SCALE as i64 // infinity, or |value| >= 1
    } else {
        // value = 1.frac * 2^(exp - bias); times 2^23.
        let significand = (1u64 << m) | frac;
        // shift right by m - 23 + (bias - exp)
        let shift = (m as i64 - 23) + (bias - exp);
        if shift >= 64 {
            0
        } else if shift >= 0 {
            (significand >> shift) as i64
        } else {
            (significand << (-shift)) as i64
        }
    };
    if negative { (-magnitude).max(MIN as i64) as i32 } else { magnitude.min(MAX as i64) as i32 }
}

/// A sample as an IEEE 754 value with `e` exponent bits and `m`
/// significand bits: `v / FULL_SCALE`, exactly (`|v| < 2^24`).
fn int_to_float(v: i32, e: u32, m: u32) -> u64 {
    if v == 0 {
        return 0;
    }
    let sign = if v < 0 { 1u64 << (e + m) } else { 0 };
    let a = (v as i64).unsigned_abs();
    let top = 63 - a.leading_zeros() as i64; // 0..=23
    let bias = (1i64 << (e - 1)) - 1;
    // a = 1.f * 2^top; value = a / 2^23 = 1.f * 2^(top - 23)
    let exp = (bias + top - 23) as u64;
    let frac = if (m as i64) >= top {
        (a << (m as i64 - top)) & ((1u64 << m) - 1)
    } else {
        (a >> (top - m as i64)) & ((1u64 << m) - 1)
    };
    sign | (exp << m) | frac
}
