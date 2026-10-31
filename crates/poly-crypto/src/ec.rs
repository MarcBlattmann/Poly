//! The NIST prime curves P-256 and P-384 (FIPS 186-4 D.1.2.3, D.1.2.4;
//! `y^2 = x^3 - 3x + b` over GF(p)), ECDSA (FIPS 186-4 6.4) with
//! deterministic nonces (RFC 6979 3.2, 3.6) and ECDH (SP 800-56A
//! 5.7.1.2).
//!
//! Field arithmetic is Montgomery multiplication ([`Mont`]) on fixed limb
//! arrays; points use Jacobian coordinates (`x = X/Z^2`, `y = Y/Z^3`) with
//! the textbook doubling for `a = -3` and general addition (Hankerson,
//! Menezes, Vanstone, "Guide to Elliptic Curve Cryptography", 3.2.2).
//! Scalar multiplication is plain double-and-add: not constant time.

use crate::bignum::{BigUint, Mont};
use crate::hash::HashAlg;
use crate::hmac::Hmac;
use alloc::vec::Vec;

/// A curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Curve {
    /// P-256 (secp256r1, prime256v1).
    P256,
    /// P-384 (secp384r1).
    P384,
}

/// Domain parameters as big-endian hex (FIPS 186-4 D.1.2): p, n, b, Gx, Gy.
struct Hex {
    p: &'static str,
    n: &'static str,
    b: &'static str,
    gx: &'static str,
    gy: &'static str,
}

const P256: Hex = Hex {
    p: "ffffffff00000001000000000000000000000000ffffffffffffffffffffffff",
    n: "ffffffff00000000ffffffffffffffffbce6faada7179e84f3b9cac2fc632551",
    b: "5ac635d8aa3a93e7b3ebbd55769886bc651d06b0cc53b0f63bce3c3e27d2604b",
    gx: "6b17d1f2e12c4247f8bce6e563a440f277037d812deb33a0f4a13945d898c296",
    gy: "4fe342e2fe1a7f9b8ee7eb4a7c0f9e162bce33576b315ececbb6406837bf51f5",
};

const P384: Hex = Hex {
    p: "fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffeffffffff0000000000000000ffffffff",
    n: "ffffffffffffffffffffffffffffffffffffffffffffffffc7634d81f4372ddf581a0db248b0a77aecec196accc52973",
    b: "b3312fa7e23ee7e4988e056be3f82d19181d9c6efe8141120314088f5013875ac656398d8a2ed19d2a85c8edd3ec2aef",
    gx: "aa87ca22be8b05378eb1c71ef320ad746e1d3b628ba79b9859f741e082542a385502f25dbf55296c3a545e3872760ab7",
    gy: "3617de4a96262c6f5d9e98bf9292dc29f8f41dbd289a147ce9da3113b5f0b8c00a60b1ce1d7e819d7a431d7c90ea0e5f",
};

fn from_hex(s: &str) -> BigUint {
    let digits: Vec<u8> = s
        .as_bytes()
        .chunks(2)
        .map(|p| {
            let v = |c: u8| match c {
                b'0'..=b'9' => c - b'0',
                _ => c - b'a' + 10,
            };
            (v(p[0]) << 4) | v(p[1])
        })
        .collect();
    BigUint::from_be_bytes(&digits)
}

impl Curve {
    /// The length of a coordinate (and of a scalar) in bytes.
    pub const fn field_len(self) -> usize {
        match self {
            Curve::P256 => 32,
            Curve::P384 => 48,
        }
    }

    fn hex(self) -> &'static Hex {
        match self {
            Curve::P256 => &P256,
            Curve::P384 => &P384,
        }
    }

    /// The group order `n`, big-endian, [`field_len`](Self::field_len) bytes.
    pub fn order_be(self) -> Vec<u8> {
        from_hex(self.hex().n).to_be_bytes_padded(self.field_len()).unwrap_or_default()
    }

    /// The hash RFC 6979's HMAC-DRBG uses for this curve here.
    fn drbg_hash(self) -> HashAlg {
        match self {
            Curve::P256 => HashAlg::Sha256,
            Curve::P384 => HashAlg::Sha384,
        }
    }
}

/// The most limbs a coordinate takes (P-384).
const L: usize = 6;
/// A field element in Montgomery form (the first `limbs` limbs used).
type Fe = [u64; L];

/// A point in Jacobian coordinates; `z == 0` is the point at infinity.
#[derive(Clone, Copy, Debug)]
struct Point {
    x: Fe,
    y: Fe,
    z: Fe,
}

/// A curve's arithmetic.
struct Group {
    curve: Curve,
    field: Mont,
    limbs: usize,
    p: BigUint,
    n: BigUint,
    b: Fe,
    g: Point,
}

impl Group {
    fn new(curve: Curve) -> Group {
        let h = curve.hex();
        let p = from_hex(h.p);
        let field = Mont::new(&p).expect("the curve primes are odd");
        let limbs = field.limbs();
        let mut g = Group { curve, field, limbs, p, n: from_hex(h.n), b: [0; L], g: Point::infinity() };
        g.b = g.fe(&from_hex(h.b));
        let one = g.one();
        g.g = Point { x: g.fe(&from_hex(h.gx)), y: g.fe(&from_hex(h.gy)), z: one };
        g
    }

    /// `x` (reduced) in Montgomery form.
    fn fe(&self, x: &BigUint) -> Fe {
        let mut out = [0u64; L];
        out[..self.limbs].copy_from_slice(&self.field.to_mont(x));
        out
    }

    /// A coordinate from bytes, or None unless it is below `p`.
    fn fe_from_bytes(&self, b: &[u8]) -> Option<Fe> {
        let x = BigUint::from_be_bytes(b);
        if x >= self.p {
            return None;
        }
        Some(self.fe(&x))
    }

    fn fe_to_bytes(&self, a: &Fe) -> Vec<u8> {
        self.field.from_mont(&a[..self.limbs]).to_be_bytes_padded(self.curve.field_len()).unwrap_or_default()
    }

    fn one(&self) -> Fe {
        let mut out = [0u64; L];
        out[..self.limbs].copy_from_slice(self.field.one());
        out
    }

    fn mul(&self, a: &Fe, b: &Fe) -> Fe {
        let mut out = [0u64; L];
        self.field.mul_into(a, b, &mut out);
        out
    }

    fn sqr(&self, a: &Fe) -> Fe {
        self.mul(a, a)
    }

    fn add(&self, a: &Fe, b: &Fe) -> Fe {
        let mut out = [0u64; L];
        self.field.add_into(a, b, &mut out);
        out
    }

    fn sub(&self, a: &Fe, b: &Fe) -> Fe {
        let mut out = [0u64; L];
        self.field.sub_into(a, b, &mut out);
        out
    }

    fn is_zero(a: &Fe) -> bool {
        a.iter().all(|&l| l == 0)
    }

    /// `a^-1 = a^(p-2)` (Fermat).
    fn inv(&self, a: &Fe) -> Fe {
        let e = self.p.sub(&BigUint::from_u64(2)).unwrap_or_default();
        let mut out = [0u64; L];
        out[..self.limbs].copy_from_slice(&self.field.pow(&a[..self.limbs], &e));
        out
    }

    /// Whether the affine point satisfies `y^2 = x^3 - 3x + b`.
    fn on_curve(&self, x: &Fe, y: &Fe) -> bool {
        let x3 = self.mul(&self.sqr(x), x);
        let three_x = self.add(&self.add(x, x), x);
        let rhs = self.add(&self.sub(&x3, &three_x), &self.b);
        self.sqr(y) == rhs
    }

    /// `2P` (doubling with `a = -3`).
    fn double(&self, p: &Point) -> Point {
        if Self::is_zero(&p.z) {
            return *p;
        }
        let delta = self.sqr(&p.z);
        let gamma = self.sqr(&p.y);
        let beta = self.mul(&p.x, &gamma);
        // alpha = 3 (X - delta)(X + delta) = 3X^2 - 3Z^4.
        let t = self.mul(&self.sub(&p.x, &delta), &self.add(&p.x, &delta));
        let alpha = self.add(&self.add(&t, &t), &t);
        let beta2 = self.add(&beta, &beta);
        let beta4 = self.add(&beta2, &beta2);
        let beta8 = self.add(&beta4, &beta4);
        let x3 = self.sub(&self.sqr(&alpha), &beta8);
        let yz = self.add(&p.y, &p.z);
        let z3 = self.sub(&self.sub(&self.sqr(&yz), &gamma), &delta);
        let g2 = self.sqr(&gamma);
        let g2_2 = self.add(&g2, &g2);
        let g2_4 = self.add(&g2_2, &g2_2);
        let g2_8 = self.add(&g2_4, &g2_4);
        let y3 = self.sub(&self.mul(&alpha, &self.sub(&beta4, &x3)), &g2_8);
        Point { x: x3, y: y3, z: z3 }
    }

    /// `P + Q`.
    fn add_points(&self, p: &Point, q: &Point) -> Point {
        if Self::is_zero(&p.z) {
            return *q;
        }
        if Self::is_zero(&q.z) {
            return *p;
        }
        let z1z1 = self.sqr(&p.z);
        let z2z2 = self.sqr(&q.z);
        let u1 = self.mul(&p.x, &z2z2);
        let u2 = self.mul(&q.x, &z1z1);
        let s1 = self.mul(&self.mul(&p.y, &q.z), &z2z2);
        let s2 = self.mul(&self.mul(&q.y, &p.z), &z1z1);
        let h = self.sub(&u2, &u1);
        let r = self.sub(&s2, &s1);
        if Self::is_zero(&h) {
            return if Self::is_zero(&r) { self.double(p) } else { Point::infinity() };
        }
        let hh = self.sqr(&h);
        let hhh = self.mul(&h, &hh);
        let v = self.mul(&u1, &hh);
        let x3 = self.sub(&self.sub(&self.sqr(&r), &hhh), &self.add(&v, &v));
        let y3 = self.sub(&self.mul(&r, &self.sub(&v, &x3)), &self.mul(&s1, &hhh));
        let z3 = self.mul(&self.mul(&p.z, &q.z), &h);
        Point { x: x3, y: y3, z: z3 }
    }

    /// `kP` (double-and-add from the top bit).
    fn scalar_mul(&self, k: &BigUint, p: &Point) -> Point {
        let mut acc = Point::infinity();
        for i in (0..k.bits()).rev() {
            acc = self.double(&acc);
            if k.bit(i) {
                acc = self.add_points(&acc, p);
            }
        }
        acc
    }

    /// `aG + bQ` (Shamir's trick: one pass over both scalars).
    fn double_scalar_mul(&self, a: &BigUint, b: &BigUint, q: &Point) -> Point {
        let gq = self.add_points(&self.g, q);
        let mut acc = Point::infinity();
        for i in (0..a.bits().max(b.bits())).rev() {
            acc = self.double(&acc);
            acc = match (a.bit(i), b.bit(i)) {
                (true, true) => self.add_points(&acc, &gq),
                (true, false) => self.add_points(&acc, &self.g),
                (false, true) => self.add_points(&acc, q),
                (false, false) => acc,
            };
        }
        acc
    }

    /// The affine coordinates, or None for infinity.
    fn to_affine(&self, p: &Point) -> Option<(Fe, Fe)> {
        if Self::is_zero(&p.z) {
            return None;
        }
        let zi = self.inv(&p.z);
        let zi2 = self.sqr(&zi);
        Some((self.mul(&p.x, &zi2), self.mul(&self.mul(&p.y, &zi2), &zi)))
    }

    /// A public point from bytes, checked to be on the curve.
    fn point(&self, x: &[u8], y: &[u8]) -> Option<Point> {
        let fl = self.curve.field_len();
        if x.len() > fl || y.len() > fl {
            return None;
        }
        let (x, y) = (self.fe_from_bytes(x)?, self.fe_from_bytes(y)?);
        self.on_curve(&x, &y).then(|| Point { x, y, z: self.one() })
    }

    /// A private scalar in `[1, n - 1]`.
    fn scalar(&self, d: &[u8]) -> Option<BigUint> {
        let d = BigUint::from_be_bytes(d);
        (!d.is_zero() && d < self.n).then_some(d)
    }

    /// `bits2int` (RFC 6979 2.3.2, the same as FIPS 186-4 6.4's use of the
    /// leftmost bits of the hash): the integer of the leftmost `qlen` bits.
    fn bits2int(&self, b: &[u8]) -> BigUint {
        let x = BigUint::from_be_bytes(b);
        let blen = 8 * b.len();
        let qlen = self.n.bits();
        if blen > qlen { x.shr(blen - qlen) } else { x }
    }

    /// `x mod n` for `x < 2n`.
    fn reduce_once(&self, x: BigUint) -> BigUint {
        if x >= self.n { x.sub(&self.n).unwrap_or_default() } else { x }
    }

    /// `int2octets` (RFC 6979 2.3.3).
    fn int2octets(&self, x: &BigUint) -> Vec<u8> {
        x.to_be_bytes_padded(self.n.bits().div_ceil(8)).unwrap_or_default()
    }

    fn inv_mod_n(&self, k: &BigUint) -> Option<BigUint> {
        k.mod_pow(&self.n.sub(&BigUint::from_u64(2))?, &self.n)
    }
}

impl Point {
    fn infinity() -> Point {
        Point { x: [0; L], y: [0; L], z: [0; L] }
    }
}

/// RFC 6979 3.2's HMAC-DRBG, step by step.
struct Rfc6979<'a> {
    group: &'a Group,
    alg: HashAlg,
    k: Vec<u8>,
    v: Vec<u8>,
}

impl<'a> Rfc6979<'a> {
    /// Steps a–g, with the additional data `extra` (3.6) after
    /// `bits2octets(h1)`.
    fn new(group: &'a Group, x: &BigUint, h1: &[u8], extra: &[u8]) -> Self {
        let alg = group.curve.drbg_hash();
        let hlen = alg.output_len();
        let mut s = Rfc6979 { group, alg, k: alloc::vec![0; hlen], v: alloc::vec![1; hlen] };
        let x = group.int2octets(x);
        let h = group.int2octets(&group.reduce_once(group.bits2int(h1)));
        for sep in [0u8, 1] {
            let mut m = Hmac::new(alg, &s.k);
            m.update(&s.v);
            m.update(&[sep]);
            m.update(&x);
            m.update(&h);
            m.update(extra);
            s.k = m.finish();
            s.v = crate::hmac(alg, &s.k, &s.v);
        }
        s
    }

    /// Step h: the next candidate `k` in `[1, q - 1]`.
    fn next(&mut self) -> BigUint {
        let qlen = self.group.n.bits();
        loop {
            let mut t = Vec::new();
            while t.len() * 8 < qlen {
                self.v = crate::hmac(self.alg, &self.k, &self.v);
                t.extend_from_slice(&self.v);
            }
            let k = self.group.bits2int(&t);
            // Whatever happens to this candidate, the state moves on.
            let mut m = Hmac::new(self.alg, &self.k);
            m.update(&self.v);
            m.update(&[0]);
            self.k = m.finish();
            self.v = crate::hmac(self.alg, &self.k, &self.v);
            if !k.is_zero() && k < self.group.n {
                return k;
            }
        }
    }
}

/// Whether `(x, y)` (big-endian, at most [`Curve::field_len`] bytes each) is
/// a point on the curve: coordinates below `p` satisfying the equation.
pub fn is_on_curve(c: Curve, x: &[u8], y: &[u8]) -> bool {
    Group::new(c).point(x, y).is_some()
}

/// The public point `dG` of a private scalar `1 <= d < n`.
pub fn public_key(c: Curve, d: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let g = Group::new(c);
    let d = g.scalar(d)?;
    let (x, y) = g.to_affine(&g.scalar_mul(&d, &g.g))?;
    Some((g.fe_to_bytes(&x), g.fe_to_bytes(&y)))
}

/// ECDSA signature verification (FIPS 186-4 6.4.2) of `hash` with public
/// point `(x, y)`.
pub fn ecdsa_verify(c: Curve, x: &[u8], y: &[u8], hash: &[u8], r: &[u8], s: &[u8]) -> bool {
    let g = Group::new(c);
    let Some(q) = g.point(x, y) else { return false };
    let (r, s) = (BigUint::from_be_bytes(r), BigUint::from_be_bytes(s));
    if r.is_zero() || s.is_zero() || r >= g.n || s >= g.n {
        return false;
    }
    let e = g.bits2int(hash);
    let Some(w) = g.inv_mod_n(&s) else { return false };
    let (Some(u1), Some(u2)) = (e.mul(&w).rem(&g.n), r.mul(&w).rem(&g.n)) else { return false };
    let Some((rx, _)) = g.to_affine(&g.double_scalar_mul(&u1, &u2, &q)) else { return false };
    let v = g.field.from_mont(&rx[..g.limbs]).rem(&g.n).unwrap_or_default();
    v == r
}

/// ECDSA signature generation (FIPS 186-4 6.4.1) with the per-message
/// secret from RFC 6979 (3.2, with `extra` as the additional data of 3.6).
/// Returns `(r, s)`, [`Curve::field_len`] bytes each.
pub fn ecdsa_sign(c: Curve, d: &[u8], hash: &[u8], extra: &[u8]) -> Option<(Vec<u8>, Vec<u8>)> {
    let g = Group::new(c);
    let d = g.scalar(d)?;
    let e = g.bits2int(hash);
    let mut drbg = Rfc6979::new(&g, &d, hash, extra);
    loop {
        let k = drbg.next();
        let (rx, _) = g.to_affine(&g.scalar_mul(&k, &g.g))?;
        let r = g.field.from_mont(&rx[..g.limbs]).rem(&g.n)?;
        if r.is_zero() {
            continue;
        }
        let s = g.inv_mod_n(&k)?.mul(&e.add(&r.mul(&d))).rem(&g.n)?;
        if s.is_zero() {
            continue;
        }
        let fl = c.field_len();
        return Some((r.to_be_bytes_padded(fl)?, s.to_be_bytes_padded(fl)?));
    }
}

/// ECDH (SP 800-56A 5.7.1.2): the x-coordinate of `d · Q`, [`Curve::field_len`]
/// bytes; None if `Q` is not on the curve, `d` is out of range or the
/// product is the point at infinity.
pub fn ecdh(c: Curve, d: &[u8], qx: &[u8], qy: &[u8]) -> Option<Vec<u8>> {
    let g = Group::new(c);
    let q = g.point(qx, qy)?;
    let d = g.scalar(d)?;
    let (x, _) = g.to_affine(&g.scalar_mul(&d, &q))?;
    Some(g.fe_to_bytes(&x))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::hex;

    /// RFC 6979 A.2.5: P-256, its key pair, and the signature of "sample"
    /// with SHA-256.
    #[test]
    fn rfc_6979_p256() {
        let x = hex("c9afa9d845ba75166b5c215767b1d6934e50c3db36e89b127b8a622b120f6721");
        let (ux, uy) = public_key(Curve::P256, &x).unwrap();
        assert_eq!(ux, hex("60fed4ba255a9d31c961eb74c6356d68c049b8923b61fa6ce669622e60f29fb6"));
        assert_eq!(uy, hex("7903fe1008b8bc99a41ae9e95628bc64f2f1b20c2d7e9f5177a3c294d4462299"));
        assert!(is_on_curve(Curve::P256, &ux, &uy));
        let h = crate::digest(HashAlg::Sha256, b"sample");
        let (r, s) = ecdsa_sign(Curve::P256, &x, &h, &[]).unwrap();
        assert_eq!(r, hex("efd48b2aacb6a8fd1140dd9cd45e81d69d2c877b56aaf991c34d0ea84eaf3716"));
        assert_eq!(s, hex("f7cb1c942d657c41d436c7a1b6e29f65f3e900dbb9aff4064dc4ab2f843acda8"));
        assert!(ecdsa_verify(Curve::P256, &ux, &uy, &h, &r, &s));
        let other = crate::digest(HashAlg::Sha256, b"test");
        assert!(!ecdsa_verify(Curve::P256, &ux, &uy, &other, &r, &s));
        assert!(!ecdsa_verify(Curve::P256, &ux, &uy, &h, &s, &r));
        // Extra data changes k but not validity.
        let (r2, s2) = ecdsa_sign(Curve::P256, &x, &h, b"random").unwrap();
        assert_ne!(r2, r);
        assert!(ecdsa_verify(Curve::P256, &ux, &uy, &h, &r2, &s2));
        // SHA-512 hash, truncated to the order's 256 bits (FIPS 186-4 6.4).
        let h512 = crate::digest(HashAlg::Sha512, b"sample");
        let (r, s) = ecdsa_sign(Curve::P256, &x, &h512, &[]).unwrap();
        assert!(ecdsa_verify(Curve::P256, &ux, &uy, &h512, &r, &s));
    }

    /// RFC 6979 A.2.6: P-384, its key pair, and the signature of "sample"
    /// with SHA-384.
    #[test]
    fn rfc_6979_p384() {
        let x = hex("6b9d3dad2e1b8c1c05b19875b6659f4de23c3b667bf297ba9aa47740787137d896d5724e4c70a825f872c9ea60d2edf5");
        let (ux, uy) = public_key(Curve::P384, &x).unwrap();
        assert_eq!(
            ux,
            hex("ec3a4e415b4e19a4568618029f427fa5da9a8bc4ae92e02e06aae5286b300c64def8f0ea9055866064a254515480bc13")
        );
        assert_eq!(
            uy,
            hex("8015d9b72d7d57244ea8ef9ac0c621896708a59367f9dfb9f54ca84b3f1c9db1288b231c3ae0d4fe7344fd2533264720")
        );
        let h = crate::digest(HashAlg::Sha384, b"sample");
        let (r, s) = ecdsa_sign(Curve::P384, &x, &h, &[]).unwrap();
        assert_eq!(
            r,
            hex("94edbb92a5ecb8aad4736e56c691916b3f88140666ce9fa73d64c4ea95ad133c81a648152e44acf96e36dd1e80fabe46")
        );
        assert_eq!(
            s,
            hex("99ef4aeb15f178cea1fe40db2603138f130e740a19624526203b6351d0a3a94fa329c145786e679e7b82c71a38628ac8")
        );
        assert!(ecdsa_verify(Curve::P384, &ux, &uy, &h, &r, &s));
        assert!(!ecdsa_verify(Curve::P256, &ux[..32], &uy[..32], &h, &r[..32], &s[..32]));
    }

    /// Both sides of an ECDH exchange get the same secret; bad inputs fail.
    #[test]
    fn ecdh_agrees() {
        for c in [Curve::P256, Curve::P384] {
            let fl = c.field_len();
            let a: Vec<u8> = (1..=fl as u8).collect();
            let b: Vec<u8> = (0..fl as u8).map(|i| 0x40 ^ i).collect();
            let (ax, ay) = public_key(c, &a).unwrap();
            let (bx, by) = public_key(c, &b).unwrap();
            let s1 = ecdh(c, &a, &bx, &by).unwrap();
            let s2 = ecdh(c, &b, &ax, &ay).unwrap();
            assert_eq!(s1, s2);
            assert_eq!(s1.len(), fl);
            let mut bad = by.clone();
            bad[fl - 1] ^= 1;
            assert!(ecdh(c, &a, &bx, &bad).is_none());
            assert!(!is_on_curve(c, &bx, &bad));
            assert!(public_key(c, &[0]).is_none());
            assert!(public_key(c, &c.order_be()).is_none());
        }
    }

    /// The group order times G is infinity; (n - 1)G is -G.
    #[test]
    fn group_order() {
        for c in [Curve::P256, Curve::P384] {
            let g = Group::new(c);
            assert!(g.to_affine(&g.scalar_mul(&g.n, &g.g)).is_none());
            let n1 = g.n.sub(&BigUint::from_u64(1)).unwrap();
            let (x, y) = public_key(c, &n1.to_be_bytes()).unwrap();
            let (gx, gy) = g.to_affine(&g.g).unwrap();
            assert_eq!(x, g.fe_to_bytes(&gx));
            let neg = g.sub(&[0; L], &gy);
            assert_eq!(y, g.fe_to_bytes(&neg));
        }
    }
}
