//! Unsigned big integers for RSA and elliptic curves.
//!
//! [`BigUint`] holds little-endian 64-bit limbs without high zero limbs.
//! Division is Knuth's Algorithm D (The Art of Computer Programming, vol. 2,
//! 4.3.1); modular exponentiation with an odd modulus uses Montgomery
//! multiplication ("Modular multiplication without trial division",
//! Montgomery 1985) in its coarsely integrated operand scanning form, with a
//! fixed 4-bit window. [`Mont`] exposes the Montgomery arithmetic on limb
//! slices for the curve code. Nothing here runs in constant time.

use alloc::vec;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// An unsigned integer of any size.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct BigUint {
    /// Little-endian limbs; the last one is nonzero (empty for zero).
    limbs: Vec<u64>,
}

impl PartialOrd for BigUint {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigUint {
    fn cmp(&self, other: &Self) -> Ordering {
        cmp_limbs(&self.limbs, &other.limbs)
    }
}

/// Compare two normalized limb vectors.
fn cmp_limbs(a: &[u64], b: &[u64]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

/// Compare two equally long limb slices.
fn cmp_same(a: &[u64], b: &[u64]) -> Ordering {
    a.iter().rev().cmp(b.iter().rev())
}

/// `a -= b` over equal lengths; the final borrow.
fn sub_in_place(a: &mut [u64], b: &[u64]) -> bool {
    let mut borrow = false;
    for (x, &y) in a.iter_mut().zip(b) {
        let (s1, b1) = x.overflowing_sub(y);
        let (s2, b2) = s1.overflowing_sub(u64::from(borrow));
        *x = s2;
        borrow = b1 | b2;
    }
    borrow
}

/// `a += b` over equal lengths; the final carry.
fn add_in_place(a: &mut [u64], b: &[u64]) -> bool {
    let mut carry = false;
    for (x, &y) in a.iter_mut().zip(b) {
        let (s1, c1) = x.overflowing_add(y);
        let (s2, c2) = s1.overflowing_add(u64::from(carry));
        *x = s2;
        carry = c1 | c2;
    }
    carry
}

impl BigUint {
    fn from_limbs(mut limbs: Vec<u64>) -> Self {
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        BigUint { limbs }
    }

    /// The limbs, least significant first.
    pub fn limbs(&self) -> &[u64] {
        &self.limbs
    }

    /// Zero.
    pub fn zero() -> Self {
        BigUint { limbs: Vec::new() }
    }

    /// The value `v`.
    pub fn from_u64(v: u64) -> Self {
        Self::from_limbs(vec![v])
    }

    /// The big-endian unsigned value of `b` (leading zeros allowed).
    pub fn from_be_bytes(b: &[u8]) -> Self {
        let mut limbs = Vec::with_capacity(b.len().div_ceil(8));
        for chunk in b.rchunks(8) {
            let mut w = [0u8; 8];
            w[8 - chunk.len()..].copy_from_slice(chunk);
            limbs.push(u64::from_be_bytes(w));
        }
        Self::from_limbs(limbs)
    }

    /// The minimal big-endian bytes (empty for zero).
    pub fn to_be_bytes(&self) -> Vec<u8> {
        let len = self.bits().div_ceil(8);
        self.to_be_bytes_padded(len).unwrap_or_default()
    }

    /// Big-endian bytes left-padded with zeros to `len`, or None if the
    /// value does not fit.
    pub fn to_be_bytes_padded(&self, len: usize) -> Option<Vec<u8>> {
        if self.bits().div_ceil(8) > len {
            return None;
        }
        let mut out = vec![0u8; len];
        for (i, limb) in self.limbs.iter().enumerate() {
            for (j, byte) in limb.to_le_bytes().iter().enumerate() {
                let pos = i * 8 + j;
                if pos < len {
                    out[len - 1 - pos] = *byte;
                }
            }
        }
        Some(out)
    }

    /// The number of significant bits (0 for zero).
    pub fn bits(&self) -> usize {
        match self.limbs.last() {
            None => 0,
            Some(&top) => self.limbs.len() * 64 - top.leading_zeros() as usize,
        }
    }

    /// Whether this is zero.
    pub fn is_zero(&self) -> bool {
        self.limbs.is_empty()
    }

    /// Whether this is odd.
    pub fn is_odd(&self) -> bool {
        self.limbs.first().is_some_and(|l| l & 1 == 1)
    }

    /// Bit `i` (0 = least significant).
    pub fn bit(&self, i: usize) -> bool {
        self.limbs.get(i / 64).is_some_and(|l| (l >> (i % 64)) & 1 == 1)
    }

    /// `self + o`.
    pub fn add(&self, o: &Self) -> Self {
        let (long, short) = if self.limbs.len() >= o.limbs.len() { (self, o) } else { (o, self) };
        let mut r = long.limbs.clone();
        r.push(0);
        let mut carry = add_in_place(&mut r[..short.limbs.len()], &short.limbs);
        for x in r[short.limbs.len()..].iter_mut() {
            if !carry {
                break;
            }
            let (s, c) = x.overflowing_add(1);
            *x = s;
            carry = c;
        }
        Self::from_limbs(r)
    }

    /// `self - o`, or None if `o > self`.
    pub fn sub(&self, o: &Self) -> Option<Self> {
        if *self < *o {
            return None;
        }
        let mut r = self.limbs.clone();
        let mut b = o.limbs.clone();
        b.resize(r.len(), 0);
        sub_in_place(&mut r, &b);
        Some(Self::from_limbs(r))
    }

    /// `self · o` (schoolbook).
    pub fn mul(&self, o: &Self) -> Self {
        if self.is_zero() || o.is_zero() {
            return Self::zero();
        }
        let mut r = vec![0u64; self.limbs.len() + o.limbs.len()];
        for (i, &a) in self.limbs.iter().enumerate() {
            let mut carry = 0u128;
            for (j, &b) in o.limbs.iter().enumerate() {
                let t = u128::from(a) * u128::from(b) + u128::from(r[i + j]) + carry;
                r[i + j] = t as u64;
                carry = t >> 64;
            }
            r[i + o.limbs.len()] = carry as u64;
        }
        Self::from_limbs(r)
    }

    /// `self << s` bits.
    pub fn shl(&self, s: usize) -> Self {
        if self.is_zero() {
            return Self::zero();
        }
        let (words, bits) = (s / 64, s % 64);
        let mut r = vec![0u64; words];
        r.extend_from_slice(&self.limbs);
        r.push(0);
        if bits > 0 {
            for i in (words + 1..r.len()).rev() {
                r[i] = (r[i] << bits) | (r[i - 1] >> (64 - bits));
            }
            r[words] <<= bits;
        }
        Self::from_limbs(r)
    }

    /// `self >> s` bits.
    pub fn shr(&self, s: usize) -> Self {
        let (words, bits) = (s / 64, s % 64);
        if words >= self.limbs.len() {
            return Self::zero();
        }
        let mut r: Vec<u64> = self.limbs[words..].to_vec();
        if bits > 0 {
            for i in 0..r.len() {
                let hi = r.get(i + 1).copied().unwrap_or(0);
                r[i] = (r[i] >> bits) | (hi << (64 - bits));
            }
        }
        Self::from_limbs(r)
    }

    /// Quotient and remainder, or None for division by zero.
    pub fn div_rem(&self, d: &Self) -> Option<(Self, Self)> {
        if d.is_zero() {
            return None;
        }
        if *self < *d {
            return Some((Self::zero(), self.clone()));
        }
        if d.limbs.len() == 1 {
            let (q, r) = self.div_rem_small(d.limbs[0]);
            return Some((q, Self::from_u64(r)));
        }
        Some(self.div_rem_knuth(d))
    }

    /// Division by one limb.
    fn div_rem_small(&self, d: u64) -> (Self, u64) {
        let mut q = vec![0u64; self.limbs.len()];
        let mut r = 0u128;
        for i in (0..self.limbs.len()).rev() {
            let cur = (r << 64) | u128::from(self.limbs[i]);
            q[i] = (cur / u128::from(d)) as u64;
            r = cur % u128::from(d);
        }
        (Self::from_limbs(q), r as u64)
    }

    /// Knuth's Algorithm D (TAOCP 4.3.1) for a divisor of two or more limbs
    /// and `self >= d`.
    fn div_rem_knuth(&self, d: &Self) -> (Self, Self) {
        let n = d.limbs.len();
        let m = self.limbs.len() - n;
        // D1: normalize so the divisor's top bit is set.
        let s = d.limbs[n - 1].leading_zeros() as usize;
        let v = d.shl(s).limbs;
        let mut u = self.shl(s).limbs;
        u.resize(self.limbs.len() + 1, 0);
        let mut q = vec![0u64; m + 1];
        let b = 1u128 << 64;
        for j in (0..=m).rev() {
            // D3: estimate q̂ from the top two limbs, correct it with the third.
            let num = (u128::from(u[j + n]) << 64) | u128::from(u[j + n - 1]);
            let mut qhat = num / u128::from(v[n - 1]);
            let mut rhat = num % u128::from(v[n - 1]);
            while qhat >= b || qhat * u128::from(v[n - 2]) > ((rhat << 64) | u128::from(u[j + n - 2])) {
                qhat -= 1;
                rhat += u128::from(v[n - 1]);
                if rhat >= b {
                    break;
                }
            }
            // D4: multiply and subtract.
            let mut borrow = 0u64;
            let mut carry = 0u128;
            for i in 0..n {
                let p = qhat * u128::from(v[i]) + carry;
                carry = p >> 64;
                let (s1, b1) = u[i + j].overflowing_sub(p as u64);
                let (s2, b2) = s1.overflowing_sub(borrow);
                u[i + j] = s2;
                borrow = u64::from(b1) + u64::from(b2);
            }
            let (s1, b1) = u[j + n].overflowing_sub(carry as u64);
            let (s2, b2) = s1.overflowing_sub(borrow);
            u[j + n] = s2;
            // D5/D6: q̂ was one too large; add the divisor back.
            if b1 || b2 {
                qhat -= 1;
                let mut c = 0u128;
                for i in 0..n {
                    let t = u128::from(u[i + j]) + u128::from(v[i]) + c;
                    u[i + j] = t as u64;
                    c = t >> 64;
                }
                u[j + n] = u[j + n].wrapping_add(c as u64);
            }
            q[j] = qhat as u64;
        }
        // D8: unnormalize the remainder.
        u.truncate(n);
        (Self::from_limbs(q), Self::from_limbs(u).shr(s))
    }

    /// `self mod m`, or None for `m == 0`.
    pub fn rem(&self, m: &Self) -> Option<Self> {
        self.div_rem(m).map(|(_, r)| r)
    }

    /// `self^e mod m`, or None for `m == 0`.
    pub fn mod_pow(&self, e: &Self, m: &Self) -> Option<Self> {
        if m.is_zero() {
            return None;
        }
        if m.limbs == [1] {
            return Some(Self::zero());
        }
        // Mont::new refuses an even modulus.
        if let Some(ctx) = Mont::new(m) {
            let base = ctx.to_mont(self);
            let r = ctx.pow(&base, e);
            return Some(ctx.from_mont(&r));
        }
        // An even (or huge) modulus: square and multiply with division.
        let base = self.rem(m)?;
        let mut acc = Self::from_u64(1);
        for i in (0..e.bits()).rev() {
            acc = acc.mul(&acc).rem(m)?;
            if e.bit(i) {
                acc = acc.mul(&base).rem(m)?;
            }
        }
        Some(acc)
    }

    /// The inverse of `self` modulo `m` (extended Euclid), or None when
    /// `m <= 1` or `gcd(self, m) != 1`.
    pub fn mod_inverse(&self, m: &Self) -> Option<Self> {
        if *m <= Self::from_u64(1) {
            return None;
        }
        // Invariant: r_i ≡ t_i · self (mod m), t as (magnitude, negative).
        let (mut r0, mut r1) = (m.clone(), self.rem(m)?);
        let (mut t0, mut t1) = ((Self::zero(), false), (Self::from_u64(1), false));
        while !r1.is_zero() {
            let (q, r) = r0.div_rem(&r1)?;
            r0 = core::mem::replace(&mut r1, r);
            let qt = (q.mul(&t1.0), t1.1);
            let next = signed_sub(&t0, &qt);
            t0 = core::mem::replace(&mut t1, next);
        }
        if r0.limbs != [1] {
            return None;
        }
        let mag = t0.0.rem(m)?;
        if t0.1 && !mag.is_zero() { m.sub(&mag) } else { Some(mag) }
    }

    /// The greatest common divisor (Euclid).
    pub fn gcd(&self, o: &Self) -> Self {
        let (mut a, mut b) = (self.clone(), o.clone());
        while !b.is_zero() {
            let r = a.rem(&b).unwrap_or_default();
            a = core::mem::replace(&mut b, r);
        }
        a
    }
}

/// `a - b` for signed values (magnitude, negative).
fn signed_sub(a: &(BigUint, bool), b: &(BigUint, bool)) -> (BigUint, bool) {
    match (a.1, b.1) {
        // a - (-b) = a + b; -a - b = -(a + b).
        (false, true) => (a.0.add(&b.0), false),
        (true, false) => (a.0.add(&b.0), true),
        // Same signs: the difference of magnitudes.
        (neg, _) => {
            if a.0 >= b.0 {
                (a.0.sub(&b.0).unwrap_or_default(), neg)
            } else {
                (b.0.sub(&a.0).unwrap_or_default(), !neg)
            }
        }
    }
}

/// The largest modulus [`Mont`] takes, in limbs (16384 bits).
pub const MAX_LIMBS: usize = 256;

/// Montgomery arithmetic modulo an odd `m` of `n` limbs, `R = 2^(64n)`.
/// Values are limb slices of exactly `n` limbs, less than `m`.
#[derive(Clone, Debug)]
pub struct Mont {
    m: Vec<u64>,
    /// `-m^-1 mod 2^64`.
    minv: u64,
    /// `R^2 mod m`.
    r2: Vec<u64>,
    /// `R mod m`: one in Montgomery form.
    one: Vec<u64>,
}

impl Mont {
    /// The context for an odd modulus of at most [`MAX_LIMBS`] limbs.
    pub fn new(m: &BigUint) -> Option<Mont> {
        let n = m.limbs.len();
        if !m.is_odd() || n > MAX_LIMBS {
            return None;
        }
        // Newton's iteration doubles the correct low bits each step.
        let m0 = m.limbs[0];
        let mut inv = 1u64;
        for _ in 0..6 {
            inv = inv.wrapping_mul(2u64.wrapping_sub(m0.wrapping_mul(inv)));
        }
        let r = BigUint::from_u64(1).shl(64 * n);
        let one = r.rem(m)?;
        let r2 = one.mul(&one).rem(m)?;
        let pad = |x: BigUint| {
            let mut l = x.limbs;
            l.resize(n, 0);
            l
        };
        Some(Mont { m: m.limbs.clone(), minv: inv.wrapping_neg(), r2: pad(r2), one: pad(one) })
    }

    /// The number of limbs.
    pub fn limbs(&self) -> usize {
        self.m.len()
    }

    /// The modulus' limbs.
    pub fn modulus(&self) -> &[u64] {
        &self.m
    }

    /// One in Montgomery form.
    pub fn one(&self) -> &[u64] {
        &self.one
    }

    /// `out = a · b · R^-1 mod m` (CIOS). `a`, `b` and `out` have `n` limbs.
    pub fn mul_into(&self, a: &[u64], b: &[u64], out: &mut [u64]) {
        // A small scratch area for curve-sized moduli, so the common case
        // does not clear the large one.
        if self.m.len() <= 8 {
            let mut t = [0u64; 10];
            self.cios(a, b, out, &mut t);
        } else {
            let mut t = [0u64; MAX_LIMBS + 2];
            self.cios(a, b, out, &mut t);
        }
    }

    /// CIOS with `t` (zeroed, at least `n + 2` limbs) as the accumulator.
    fn cios(&self, a: &[u64], b: &[u64], out: &mut [u64], t: &mut [u64]) {
        let n = self.m.len();
        for &bi in &b[..n] {
            let mut c = 0u128;
            for j in 0..n {
                let s = u128::from(t[j]) + u128::from(a[j]) * u128::from(bi) + c;
                t[j] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[n]) + c;
            t[n] = s as u64;
            t[n + 1] = (s >> 64) as u64;
            let q = t[0].wrapping_mul(self.minv);
            let s = u128::from(t[0]) + u128::from(q) * u128::from(self.m[0]);
            let mut c = s >> 64;
            for j in 1..n {
                let s = u128::from(t[j]) + u128::from(q) * u128::from(self.m[j]) + c;
                t[j - 1] = s as u64;
                c = s >> 64;
            }
            let s = u128::from(t[n]) + c;
            t[n - 1] = s as u64;
            t[n] = t[n + 1] + (s >> 64) as u64;
        }
        if t[n] != 0 || cmp_same(&t[..n], &self.m) != Ordering::Less {
            sub_in_place(&mut t[..n], &self.m);
        }
        out[..n].copy_from_slice(&t[..n]);
    }

    /// `a · b · R^-1 mod m` as a new vector.
    pub fn mul(&self, a: &[u64], b: &[u64]) -> Vec<u64> {
        let mut out = vec![0u64; self.m.len()];
        self.mul_into(a, b, &mut out);
        out
    }

    /// `out = a + b mod m`.
    pub fn add_into(&self, a: &[u64], b: &[u64], out: &mut [u64]) {
        let n = self.m.len();
        out[..n].copy_from_slice(&a[..n]);
        let carry = add_in_place(&mut out[..n], &b[..n]);
        if carry || cmp_same(&out[..n], &self.m) != Ordering::Less {
            sub_in_place(&mut out[..n], &self.m);
        }
    }

    /// `out = a - b mod m`.
    pub fn sub_into(&self, a: &[u64], b: &[u64], out: &mut [u64]) {
        let n = self.m.len();
        out[..n].copy_from_slice(&a[..n]);
        if sub_in_place(&mut out[..n], &b[..n]) {
            add_in_place(&mut out[..n], &self.m);
        }
    }

    /// `x` (any size) in Montgomery form: `x · R mod m`.
    pub fn to_mont(&self, x: &BigUint) -> Vec<u64> {
        let n = self.m.len();
        let mut l = x.rem(&BigUint { limbs: self.m.clone() }).unwrap_or_default().limbs;
        l.resize(n, 0);
        self.mul(&l, &self.r2)
    }

    /// Back from Montgomery form: `a · R^-1 mod m`.
    pub fn from_mont(&self, a: &[u64]) -> BigUint {
        let mut one = vec![0u64; self.m.len()];
        one[0] = 1;
        BigUint::from_limbs(self.mul(a, &one))
    }

    /// `base^e` in Montgomery form, with a fixed 4-bit window.
    pub fn pow(&self, base: &[u64], e: &BigUint) -> Vec<u64> {
        let mut table: Vec<Vec<u64>> = Vec::with_capacity(16);
        table.push(self.one.clone());
        for i in 1..16 {
            let next = self.mul(&table[i - 1], base);
            table.push(next);
        }
        let mut acc = self.one.clone();
        let mut tmp = acc.clone();
        let windows = e.bits().div_ceil(4);
        for w in (0..windows).rev() {
            for _ in 0..4 {
                self.mul_into(&acc, &acc, &mut tmp);
                core::mem::swap(&mut acc, &mut tmp);
            }
            let mut idx = 0usize;
            for b in 0..4 {
                if e.bit(4 * w + b) {
                    idx |= 1 << b;
                }
            }
            if idx != 0 {
                self.mul_into(&acc, &table[idx], &mut tmp);
                core::mem::swap(&mut acc, &mut tmp);
            }
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    fn n(s: &str) -> BigUint {
        BigUint::from_be_bytes(&hex(s))
    }

    #[test]
    fn bytes_round_trip() {
        assert!(BigUint::from_be_bytes(&[0, 0]).is_zero());
        assert_eq!(BigUint::zero().to_be_bytes(), Vec::<u8>::new());
        let x = n("0102030405060708090a0b0c0d0e0f1011");
        assert_eq!(x.to_be_bytes(), hex("0102030405060708090a0b0c0d0e0f1011"));
        assert_eq!(x.bits(), 17 * 8 - 7);
        assert_eq!(x.to_be_bytes_padded(20).unwrap(), hex("0000000102030405060708090a0b0c0d0e0f1011"));
        assert_eq!(x.to_be_bytes_padded(16), None);
        assert!(x.bit(0) && !x.bit(1) && x.bit(4));
    }

    #[test]
    fn arithmetic() {
        let a = n("ffffffffffffffffffffffffffffffff");
        let one = BigUint::from_u64(1);
        assert_eq!(a.add(&one), n("0100000000000000000000000000000000"));
        assert_eq!(a.add(&one).sub(&one).unwrap(), a);
        assert_eq!(one.sub(&a), None);
        assert_eq!(a.mul(&a), n("fffffffffffffffffffffffffffffffe00000000000000000000000000000001"));
        assert_eq!(a.shl(68).shr(68), a);
        assert_eq!(a.shr(200), BigUint::zero());
    }

    /// Division checked against Python's integers, including the rare
    /// add-back step of Algorithm D (a divisor whose top limbs make q̂ one
    /// too large).
    #[test]
    fn division() {
        let u = n("1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");
        let v = n("fedcba0987654321fedcba0987654321ff");
        let (q, r) = u.div_rem(&v).unwrap();
        assert_eq!(q.mul(&v).add(&r), u);
        assert!(r < v);
        assert_eq!(q, n("1249249c805663c40aa756c82221daeffb371452d0c5cf"));
        // Add-back case: u = 0x7fff800000000000_0000000000000000_0000000000000000,
        // v = 0x8000000000000000_0000000000000001.
        let u = n("7fff80000000000000000000000000000000000000000000");
        let v = n("80000000000000000000000000000001");
        let (q, r) = u.div_rem(&v).unwrap();
        assert_eq!(q.mul(&v).add(&r), u);
        assert!(r < v);
        // Many random-ish pairs: q·v + r = u and r < v.
        let mut x = 0x9e3779b97f4a7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..300 {
            let ul = (next() % 9 + 1) as usize;
            let vl = (next() % 5 + 1) as usize;
            let u = BigUint::from_limbs((0..ul).map(|_| next()).collect());
            let mut vv: Vec<u64> = (0..vl).map(|_| next()).collect();
            if next() % 3 == 0 {
                *vv.last_mut().unwrap() = u64::MAX;
            }
            let v = BigUint::from_limbs(vv);
            if v.is_zero() {
                continue;
            }
            let (q, r) = u.div_rem(&v).unwrap();
            assert_eq!(q.mul(&v).add(&r), u);
            assert!(r < v);
        }
        assert!(u.div_rem(&BigUint::zero()).is_none());
    }

    /// Exponentiation and inverses checked against Python's `pow`.
    #[test]
    fn mod_pow_and_inverse() {
        let m = n("f123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdf1");
        let b = n("abcdef0123456789abcdef0123456789abcdef");
        let e = n("010001");
        let want = n("4614567a68e1f0e497ed97a1817fdc901ac831278048aefd112442ff12438150");
        assert_eq!(b.mod_pow(&e, &m).unwrap(), want);
        // The same through the even-modulus path: m * 2 then reduce.
        let m2 = m.shl(1);
        assert_eq!(b.mod_pow(&e, &m2).unwrap().rem(&m).unwrap(), want);
        assert_eq!(b.mod_pow(&BigUint::zero(), &m).unwrap(), BigUint::from_u64(1));
        assert_eq!(b.mod_pow(&e, &BigUint::from_u64(1)).unwrap(), BigUint::zero());
        let inv = b.mod_inverse(&m).unwrap();
        assert_eq!(inv.mul(&b).rem(&m).unwrap(), BigUint::from_u64(1));
        // An even modulus.
        let inv = BigUint::from_u64(65537).mod_inverse(&n("0100000000000000000000")).unwrap();
        assert_eq!(inv.mul(&BigUint::from_u64(65537)).rem(&n("0100000000000000000000")).unwrap(), BigUint::from_u64(1));
        assert_eq!(BigUint::from_u64(6).mod_inverse(&BigUint::from_u64(9)), None);
        assert_eq!(BigUint::from_u64(12).gcd(&BigUint::from_u64(18)), BigUint::from_u64(6));
    }
}
