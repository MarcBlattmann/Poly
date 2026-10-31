//! CNG asymmetric keys: RSA and ECC key blobs (`BCRYPT_RSAKEY_BLOB`,
//! `BCRYPT_ECCKEY_BLOB`), signatures, RSA encryption, ECDH secret
//! agreement and `BCryptDeriveKey`'s key derivation functions.
//!
//! Sources: Learn's pages of `BCryptImportKeyPair`, `BCryptExportKey`,
//! `BCryptSignHash`, `BCryptVerifySignature`, `BCryptEncrypt`,
//! `BCryptSecretAgreement`, `BCryptDeriveKey`, `BCRYPT_RSAKEY_BLOB`,
//! `BCRYPT_ECCKEY_BLOB`, `BCRYPT_PKCS1_PADDING_INFO`,
//! `BCRYPT_PSS_PADDING_INFO`, `BCRYPT_OAEP_PADDING_INFO`, `BCryptBuffer`;
//! `bcrypt.h` for the magics, blob names and KDF parameter types; RFC 5246
//! 5 and RFC 2246 5 (the TLS PRFs); SP 800-56A 5.8.1 (the concatenation
//! KDF).

use crate::cng::AlgKind;
use crate::status::*;
use alloc::vec::Vec;
use poly_crypto::HashAlg;
use poly_crypto::bignum::BigUint;
use poly_crypto::ec::{self, Curve};
use poly_crypto::rsa::{self, RsaPrivateKey, RsaPublicKey};

// Blob types (`bcrypt.h`).
pub const BCRYPT_PUBLIC_KEY_BLOB: &str = "PUBLICBLOB";
pub const BCRYPT_PRIVATE_KEY_BLOB: &str = "PRIVATEBLOB";
pub const BCRYPT_RSAPUBLIC_BLOB: &str = "RSAPUBLICBLOB";
pub const BCRYPT_RSAPRIVATE_BLOB: &str = "RSAPRIVATEBLOB";
pub const BCRYPT_RSAFULLPRIVATE_BLOB: &str = "RSAFULLPRIVATEBLOB";
pub const BCRYPT_ECCPUBLIC_BLOB: &str = "ECCPUBLICBLOB";
pub const BCRYPT_ECCPRIVATE_BLOB: &str = "ECCPRIVATEBLOB";

// Magics (`bcrypt.h`).
pub const RSAPUBLIC_MAGIC: u32 = 0x3141_5352;
pub const RSAPRIVATE_MAGIC: u32 = 0x3241_5352;
pub const RSAFULLPRIVATE_MAGIC: u32 = 0x3341_5352;
pub const ECDH_PUBLIC_P256_MAGIC: u32 = 0x314B_4345;
pub const ECDH_PRIVATE_P256_MAGIC: u32 = 0x324B_4345;
pub const ECDH_PUBLIC_P384_MAGIC: u32 = 0x334B_4345;
pub const ECDH_PRIVATE_P384_MAGIC: u32 = 0x344B_4345;
pub const ECDSA_PUBLIC_P256_MAGIC: u32 = 0x3153_4345;
pub const ECDSA_PRIVATE_P256_MAGIC: u32 = 0x3253_4345;
pub const ECDSA_PUBLIC_P384_MAGIC: u32 = 0x3353_4345;
pub const ECDSA_PRIVATE_P384_MAGIC: u32 = 0x3453_4345;
pub const ECDH_PUBLIC_GENERIC_MAGIC: u32 = 0x504B_4345;
pub const ECDH_PRIVATE_GENERIC_MAGIC: u32 = 0x564B_4345;
pub const ECDSA_PUBLIC_GENERIC_MAGIC: u32 = 0x5044_4345;
pub const ECDSA_PRIVATE_GENERIC_MAGIC: u32 = 0x5644_4345;

/// The ECC magic for a curve, use and half.
pub fn ecc_magic(curve: Curve, ecdsa: bool, private: bool) -> u32 {
    match (curve, ecdsa, private) {
        (Curve::P256, false, false) => ECDH_PUBLIC_P256_MAGIC,
        (Curve::P256, false, true) => ECDH_PRIVATE_P256_MAGIC,
        (Curve::P384, false, false) => ECDH_PUBLIC_P384_MAGIC,
        (Curve::P384, false, true) => ECDH_PRIVATE_P384_MAGIC,
        (Curve::P256, true, false) => ECDSA_PUBLIC_P256_MAGIC,
        (Curve::P256, true, true) => ECDSA_PRIVATE_P256_MAGIC,
        (Curve::P384, true, false) => ECDSA_PUBLIC_P384_MAGIC,
        (Curve::P384, true, true) => ECDSA_PRIVATE_P384_MAGIC,
    }
}

/// An asymmetric key.
#[derive(Clone, Debug)]
pub enum AsymKey {
    Rsa { public: RsaPublicKey, private: Option<RsaPrivateKey>, sign_only: bool },
    Ec { curve: Curve, ecdsa: bool, x: Vec<u8>, y: Vec<u8>, d: Option<Vec<u8>> },
}

/// A padding scheme and its parameters, from the flags and padding info.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Padding {
    /// No padding (`BCRYPT_PAD_NONE`, or no flag for ECDSA).
    None,
    /// PKCS#1 v1.5; the hash for the `DigestInfo` (None: the bytes as is).
    Pkcs1(Option<HashAlg>),
    Pss {
        hash: HashAlg,
        salt_len: usize,
    },
    Oaep {
        hash: HashAlg,
        label: Vec<u8>,
    },
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|v| u32::from_le_bytes([v[0], v[1], v[2], v[3]]))
}

/// Read consecutive big-endian fields of the given sizes after `start`.
fn fields<'a>(blob: &'a [u8], start: usize, sizes: &[usize]) -> Option<Vec<&'a [u8]>> {
    let mut at = start;
    let mut out = Vec::new();
    for &n in sizes {
        out.push(blob.get(at..at.checked_add(n)?)?);
        at += n;
    }
    Some(out)
}

impl AsymKey {
    /// `BCryptImportKeyPair` for provider `k`. A blob that does not fit
    /// the provider or is malformed is `STATUS_INVALID_PARAMETER`; an
    /// unknown blob type `STATUS_NOT_SUPPORTED` (to be confirmed).
    pub fn import(k: AlgKind, blob_type: &str, blob: &[u8]) -> Result<AsymKey, NtStatus> {
        const BAD: NtStatus = STATUS_INVALID_PARAMETER;
        let magic = u32_at(blob, 0).ok_or(BAD)?;
        match k {
            AlgKind::Rsa { sign_only } => {
                let private = match (blob_type, magic) {
                    (BCRYPT_RSAPUBLIC_BLOB | BCRYPT_PUBLIC_KEY_BLOB, RSAPUBLIC_MAGIC) => false,
                    (
                        BCRYPT_RSAPRIVATE_BLOB | BCRYPT_RSAFULLPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB,
                        RSAPRIVATE_MAGIC,
                    ) => true,
                    (BCRYPT_RSAFULLPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB, RSAFULLPRIVATE_MAGIC) => true,
                    (
                        BCRYPT_RSAPUBLIC_BLOB
                        | BCRYPT_PUBLIC_KEY_BLOB
                        | BCRYPT_RSAPRIVATE_BLOB
                        | BCRYPT_RSAFULLPRIVATE_BLOB
                        | BCRYPT_PRIVATE_KEY_BLOB,
                        _,
                    ) => return Err(BAD),
                    _ => return Err(STATUS_NOT_SUPPORTED),
                };
                let h: Vec<usize> =
                    (1..6).map(|i| u32_at(blob, 4 * i).map(|v| v as usize)).collect::<Option<_>>().ok_or(BAD)?;
                let (bits, ce, cm, cp, cq) = (h[0], h[1], h[2], h[3], h[4]);
                let mut sizes = Vec::from([ce, cm]);
                if private {
                    sizes.extend([cp, cq]);
                }
                if magic == RSAFULLPRIVATE_MAGIC {
                    sizes.extend([cp, cq, cp, cm]);
                }
                let f = fields(blob, 24, &sizes).ok_or(BAD)?;
                let n = BigUint::from_be_bytes(f[1]);
                if n.bits() != bits {
                    return Err(BAD);
                }
                let e = BigUint::from_be_bytes(f[0]);
                let public = RsaPublicKey::new(n.clone(), e.clone()).ok_or(BAD)?;
                let private = if !private {
                    None
                } else if magic == RSAFULLPRIVATE_MAGIC {
                    let b = |i: usize| BigUint::from_be_bytes(f[i]);
                    Some(RsaPrivateKey::from_parts(n, e, b(7), b(2), b(3), b(4), b(5), b(6)).ok_or(BAD)?)
                } else {
                    let (p, q) = (BigUint::from_be_bytes(f[2]), BigUint::from_be_bytes(f[3]));
                    Some(RsaPrivateKey::from_primes(n, e, p, q).ok_or(BAD)?)
                };
                Ok(AsymKey::Rsa { public, private, sign_only })
            }
            AlgKind::Ecdsa(curve) | AlgKind::Ecdh(curve) => {
                let ecdsa = matches!(k, AlgKind::Ecdsa(_));
                let len = u32_at(blob, 4).ok_or(BAD)? as usize;
                let generic = if ecdsa {
                    [ECDSA_PUBLIC_GENERIC_MAGIC, ECDSA_PRIVATE_GENERIC_MAGIC]
                } else {
                    [ECDH_PUBLIC_GENERIC_MAGIC, ECDH_PRIVATE_GENERIC_MAGIC]
                };
                let mut found = None;
                for c in [Curve::P256, Curve::P384] {
                    for private in [false, true] {
                        if magic == ecc_magic(c, ecdsa, private) {
                            found = Some((c, private));
                        }
                    }
                }
                let (c, private) = match found {
                    Some(f) => f,
                    // A generic magic: the curve from the length.
                    None if magic == generic[0] || magic == generic[1] => {
                        let c = [Curve::P256, Curve::P384].into_iter().find(|c| c.field_len() == len).ok_or(BAD)?;
                        (c, magic == generic[1])
                    }
                    None => return Err(BAD),
                };
                if curve.is_some_and(|want| want != c) || len != c.field_len() {
                    return Err(BAD);
                }
                match (blob_type, private) {
                    (BCRYPT_ECCPUBLIC_BLOB | BCRYPT_PUBLIC_KEY_BLOB, false) => {}
                    (BCRYPT_ECCPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB, true) => {}
                    (
                        BCRYPT_ECCPUBLIC_BLOB
                        | BCRYPT_PUBLIC_KEY_BLOB
                        | BCRYPT_ECCPRIVATE_BLOB
                        | BCRYPT_PRIVATE_KEY_BLOB,
                        _,
                    ) => {
                        return Err(BAD);
                    }
                    _ => return Err(STATUS_NOT_SUPPORTED),
                }
                let sizes = [len; 3];
                let f = fields(blob, 8, &sizes[..if private { 3 } else { 2 }]).ok_or(BAD)?;
                if !ec::is_on_curve(c, f[0], f[1]) {
                    return Err(BAD);
                }
                let d = if private {
                    let (x, y) = ec::public_key(c, f[2]).ok_or(BAD)?;
                    if x != f[0] || y != f[1] {
                        return Err(BAD);
                    }
                    Some(f[2].to_vec())
                } else {
                    None
                };
                Ok(AsymKey::Ec { curve: c, ecdsa, x: f[0].to_vec(), y: f[1].to_vec(), d })
            }
            _ => Err(STATUS_NOT_SUPPORTED),
        }
    }

    /// `BCryptExportKey`: a private blob of a public key is
    /// `STATUS_INVALID_PARAMETER` (to be confirmed).
    pub fn export(&self, blob_type: &str) -> Result<Vec<u8>, NtStatus> {
        let mut v = Vec::new();
        match self {
            AsymKey::Rsa { public, private, .. } => {
                let e = public.e.to_be_bytes();
                let n = public.n.to_be_bytes();
                let (magic, parts): (u32, Vec<Vec<u8>>) = match (blob_type, private) {
                    (BCRYPT_RSAPUBLIC_BLOB | BCRYPT_PUBLIC_KEY_BLOB, _) => (RSAPUBLIC_MAGIC, Vec::new()),
                    (BCRYPT_RSAPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB, Some(p)) => {
                        (RSAPRIVATE_MAGIC, Vec::from([p.p.to_be_bytes(), p.q.to_be_bytes()]))
                    }
                    (BCRYPT_RSAFULLPRIVATE_BLOB, Some(p)) => {
                        let lp = p.p.to_be_bytes().len();
                        let lq = p.q.to_be_bytes().len();
                        let pad = |b: &BigUint, l: usize| b.to_be_bytes_padded(l).unwrap_or_default();
                        (
                            RSAFULLPRIVATE_MAGIC,
                            Vec::from([
                                p.p.to_be_bytes(),
                                p.q.to_be_bytes(),
                                pad(&p.dp, lp),
                                pad(&p.dq, lq),
                                pad(&p.qinv, lp),
                                pad(&p.d, n.len()),
                            ]),
                        )
                    }
                    (BCRYPT_RSAPRIVATE_BLOB | BCRYPT_RSAFULLPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB, None) => {
                        return Err(STATUS_INVALID_PARAMETER);
                    }
                    _ => return Err(STATUS_NOT_SUPPORTED),
                };
                let (cp, cq) = parts.first().zip(parts.get(1)).map_or((0, 0), |(p, q)| (p.len(), q.len()));
                for w in [magic, public.bits() as u32, e.len() as u32, n.len() as u32, cp as u32, cq as u32] {
                    v.extend_from_slice(&w.to_le_bytes());
                }
                v.extend_from_slice(&e);
                v.extend_from_slice(&n);
                for p in parts {
                    v.extend_from_slice(&p);
                }
            }
            AsymKey::Ec { curve, ecdsa, x, y, d } => {
                let private = match (blob_type, d) {
                    (BCRYPT_ECCPUBLIC_BLOB | BCRYPT_PUBLIC_KEY_BLOB, _) => false,
                    (BCRYPT_ECCPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB, Some(_)) => true,
                    (BCRYPT_ECCPRIVATE_BLOB | BCRYPT_PRIVATE_KEY_BLOB, None) => return Err(STATUS_INVALID_PARAMETER),
                    _ => return Err(STATUS_NOT_SUPPORTED),
                };
                v.extend_from_slice(&ecc_magic(*curve, *ecdsa, private).to_le_bytes());
                v.extend_from_slice(&(curve.field_len() as u32).to_le_bytes());
                v.extend_from_slice(x);
                v.extend_from_slice(y);
                if let (true, Some(d)) = (private, d) {
                    v.extend_from_slice(d);
                }
            }
        }
        Ok(v)
    }

    /// A new ECC key pair (`BCryptGenerateKeyPair` + `FinalizeKeyPair`):
    /// `random` fills buffers with random bytes. The length in bits must
    /// be the curve's (or pick the curve for the generic providers).
    /// RSA key generation is not implemented (`STATUS_NOT_SUPPORTED`).
    pub fn generate(k: AlgKind, bits: u32, random: &mut dyn FnMut(&mut [u8])) -> Result<AsymKey, NtStatus> {
        let (curve, ecdsa) = match k {
            AlgKind::Ecdsa(c) => (c, true),
            AlgKind::Ecdh(c) => (c, false),
            AlgKind::Rsa { .. } => return Err(STATUS_NOT_SUPPORTED),
            _ => return Err(STATUS_INVALID_PARAMETER),
        };
        let c = match curve {
            Some(c) if c.field_len() as u32 * 8 == bits || bits == 0 => c,
            Some(_) => return Err(STATUS_INVALID_PARAMETER),
            None => [Curve::P256, Curve::P384]
                .into_iter()
                .find(|c| c.field_len() as u32 * 8 == bits)
                .ok_or(STATUS_INVALID_PARAMETER)?,
        };
        let mut d = alloc::vec![0u8; c.field_len()];
        for _ in 0..64 {
            random(&mut d);
            if let Some((x, y)) = ec::public_key(c, &d) {
                return Ok(AsymKey::Ec { curve: c, ecdsa, x, y, d: Some(d) });
            }
        }
        Err(STATUS_INTERNAL_ERROR)
    }

    /// The key's size in bits (`KeyLength`).
    pub fn bits(&self) -> u32 {
        match self {
            AsymKey::Rsa { public, .. } => public.bits() as u32,
            AsymKey::Ec { curve, .. } => curve.field_len() as u32 * 8,
        }
    }

    /// The signature size (`BCryptSignHash` with no output buffer).
    pub fn signature_len(&self) -> usize {
        match self {
            AsymKey::Rsa { public, .. } => public.size(),
            AsymKey::Ec { curve, .. } => 2 * curve.field_len(),
        }
    }

    /// `BCryptSignHash`. RSA needs PKCS#1 or PSS padding
    /// (`STATUS_INVALID_PARAMETER` otherwise); ECDSA none. `random` gives
    /// PSS salts and ECDSA's extra nonce input (RFC 6979 3.6). A public key
    /// is `STATUS_INVALID_PARAMETER` (to be confirmed).
    pub fn sign(&self, padding: &Padding, hash: &[u8], random: &mut dyn FnMut(&mut [u8])) -> Result<Vec<u8>, NtStatus> {
        match self {
            AsymKey::Rsa { private: Some(p), .. } => match padding {
                Padding::Pkcs1(h) => {
                    if h.is_some_and(|h| h.output_len() != hash.len()) {
                        return Err(STATUS_INVALID_PARAMETER);
                    }
                    rsa::sign_pkcs1v15(p, *h, hash).ok_or(STATUS_INVALID_PARAMETER)
                }
                Padding::Pss { hash: h, salt_len } => {
                    let mut salt = alloc::vec![0u8; *salt_len];
                    random(&mut salt);
                    rsa::sign_pss(p, *h, hash, &salt).ok_or(STATUS_INVALID_PARAMETER)
                }
                _ => Err(STATUS_INVALID_PARAMETER),
            },
            AsymKey::Ec { curve, ecdsa: true, d: Some(d), .. } => {
                if *padding != Padding::None {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let mut extra = [0u8; 32];
                random(&mut extra);
                let (r, s) = ec::ecdsa_sign(*curve, d, hash, &extra).ok_or(STATUS_INVALID_PARAMETER)?;
                Ok([r, s].concat())
            }
            _ => Err(STATUS_INVALID_PARAMETER),
        }
    }

    /// `BCryptVerifySignature`: `STATUS_INVALID_SIGNATURE` for a signature
    /// that does not verify (including one of the wrong size).
    pub fn verify(&self, padding: &Padding, hash: &[u8], sig: &[u8]) -> Result<(), NtStatus> {
        let ok = match self {
            AsymKey::Rsa { public, .. } => match padding {
                Padding::Pkcs1(h) => rsa::verify_pkcs1v15(public, *h, hash, sig),
                Padding::Pss { hash: h, salt_len } => rsa::verify_pss(public, *h, hash, sig, *salt_len),
                _ => return Err(STATUS_INVALID_PARAMETER),
            },
            AsymKey::Ec { curve, ecdsa: true, x, y, .. } => {
                if *padding != Padding::None {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let l = curve.field_len();
                sig.len() == 2 * l && ec::ecdsa_verify(*curve, x, y, hash, &sig[..l], &sig[l..])
            }
            _ => return Err(STATUS_INVALID_PARAMETER),
        };
        if ok { Ok(()) } else { Err(STATUS_INVALID_SIGNATURE) }
    }

    /// `BCryptEncrypt` with an RSA key.
    pub fn encrypt(
        &self,
        padding: &Padding,
        input: &[u8],
        random: &mut dyn FnMut(&mut [u8]),
    ) -> Result<Vec<u8>, NtStatus> {
        let AsymKey::Rsa { public, sign_only: false, .. } = self else { return Err(STATUS_NOT_SUPPORTED) };
        match padding {
            Padding::None => {
                if input.len() != public.size() {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                rsa::public_op(public, input)
            }
            Padding::Pkcs1(None) => rsa::encrypt_pkcs1v15(public, input, random),
            Padding::Oaep { hash, label } => rsa::encrypt_oaep(public, *hash, label, input, random),
            _ => return Err(STATUS_INVALID_PARAMETER),
        }
        .ok_or(STATUS_INVALID_PARAMETER)
    }

    /// `BCryptDecrypt` with an RSA private key; a padding failure is
    /// `STATUS_INVALID_PARAMETER` (to be confirmed).
    pub fn decrypt(&self, padding: &Padding, input: &[u8]) -> Result<Vec<u8>, NtStatus> {
        let AsymKey::Rsa { private: Some(p), sign_only: false, .. } = self else { return Err(STATUS_NOT_SUPPORTED) };
        match padding {
            Padding::None => rsa::private_op(p, input),
            Padding::Pkcs1(None) => rsa::decrypt_pkcs1v15(p, input),
            Padding::Oaep { hash, label } => rsa::decrypt_oaep(p, *hash, label, input),
            _ => return Err(STATUS_INVALID_PARAMETER),
        }
        .ok_or(STATUS_INVALID_PARAMETER)
    }

    /// The size `BCryptEncrypt`/`Decrypt` reports for an RSA key.
    pub fn cipher_len(&self) -> usize {
        self.signature_len()
    }
}

/// `BCryptSecretAgreement`: both ECDH keys on one curve, the first with
/// its private half; the shared x-coordinate (big-endian).
pub fn agree(private: &AsymKey, public: &AsymKey) -> Result<Vec<u8>, NtStatus> {
    match (private, public) {
        (
            AsymKey::Ec { curve: c1, ecdsa: false, d: Some(d), .. },
            AsymKey::Ec { curve: c2, ecdsa: false, x, y, .. },
        ) if c1 == c2 => ec::ecdh(*c1, d, x, y).ok_or(STATUS_INVALID_PARAMETER),
        _ => Err(STATUS_INVALID_PARAMETER),
    }
}

// KDF names and `BCryptBuffer` types (`bcrypt.h`).
pub const BCRYPT_KDF_HASH: &str = "HASH";
pub const BCRYPT_KDF_HMAC: &str = "HMAC";
pub const BCRYPT_KDF_TLS_PRF: &str = "TLS_PRF";
pub const BCRYPT_KDF_SP80056A_CONCAT: &str = "SP800_56A_CONCAT";
pub const BCRYPT_KDF_RAW_SECRET: &str = "TRUNCATE";
pub const KDF_HASH_ALGORITHM: u32 = 0;
pub const KDF_SECRET_PREPEND: u32 = 1;
pub const KDF_SECRET_APPEND: u32 = 2;
pub const KDF_HMAC_KEY: u32 = 3;
pub const KDF_TLS_PRF_LABEL: u32 = 4;
pub const KDF_TLS_PRF_SEED: u32 = 5;
pub const KDF_TLS_PRF_PROTOCOL: u32 = 7;
pub const KDF_ALGORITHMID: u32 = 8;
pub const KDF_PARTYUINFO: u32 = 9;
pub const KDF_PARTYVINFO: u32 = 10;
pub const KDF_SUPPPUBINFO: u32 = 11;
pub const KDF_SUPPPRIVINFO: u32 = 12;
/// `TLS1_0_PROTOCOL_VERSION` .. `TLS1_2_PROTOCOL_VERSION` (`bcrypt.h`).
pub const TLS1_0: u32 = 0x0301;
pub const TLS1_1: u32 = 0x0302;
pub const TLS1_2: u32 = 0x0303;

/// The parameters of one `BCryptDeriveKey` call: (`BufferType`, bytes).
pub type KdfParams<'a> = [(u32, &'a [u8])];

fn param<'a>(p: &KdfParams<'a>, t: u32) -> Option<&'a [u8]> {
    p.iter().find(|(k, _)| *k == t).map(|(_, v)| *v)
}

fn concat_params(p: &KdfParams<'_>, t: u32) -> Vec<u8> {
    p.iter().filter(|(k, _)| *k == t).flat_map(|(_, v)| v.iter().copied()).collect()
}

fn hash_param(p: &KdfParams<'_>, default: HashAlg) -> Result<HashAlg, NtStatus> {
    match param(p, KDF_HASH_ALGORITHM) {
        None => Ok(default),
        Some(v) => crate::cng::decode_wide(v).and_then(|s| crate::cng::hash_by_name(&s)).ok_or(STATUS_NOT_SUPPORTED),
    }
}

/// TLS's `P_hash` (RFC 5246 5).
fn p_hash(h: HashAlg, secret: &[u8], seed: &[u8], out: &mut [u8]) {
    let mut a = poly_crypto::hmac(h, secret, seed);
    let mut done = 0;
    while done < out.len() {
        let mut m = poly_crypto::Hmac::new(h, secret);
        m.update(&a);
        m.update(seed);
        let block = m.finish();
        let n = block.len().min(out.len() - done);
        out[done..done + n].copy_from_slice(&block[..n]);
        done += n;
        a = poly_crypto::hmac(h, secret, &a);
    }
}

/// `BCryptDeriveKey` over the agreed secret `z` (big-endian). `want` is
/// the caller's buffer size (None: a size query). The natural sizes: the
/// hash length (HASH, HMAC), 48 (TLS_PRF), the secret's length (TRUNCATE,
/// little-endian as Learn says); SP800_56A_CONCAT makes exactly `want`
/// bytes. A longer natural result is truncated to `want`.
pub fn derive(z: &[u8], kdf: &str, p: &KdfParams<'_>, want: Option<usize>) -> Result<Vec<u8>, NtStatus> {
    let mut out = match kdf {
        BCRYPT_KDF_RAW_SECRET => z.iter().rev().copied().collect(),
        BCRYPT_KDF_HASH => {
            let h = hash_param(p, HashAlg::Sha1)?;
            let mut s = poly_crypto::Hasher::new(h);
            s.update(&concat_params(p, KDF_SECRET_PREPEND));
            s.update(z);
            s.update(&concat_params(p, KDF_SECRET_APPEND));
            s.finish()
        }
        BCRYPT_KDF_HMAC => {
            // Keyed with KDF_HMAC_KEY, or the secret without one (to be
            // confirmed), over prepend || Z || append.
            let h = hash_param(p, HashAlg::Sha1)?;
            let mut m = poly_crypto::Hmac::new(h, param(p, KDF_HMAC_KEY).unwrap_or(z));
            m.update(&concat_params(p, KDF_SECRET_PREPEND));
            m.update(z);
            m.update(&concat_params(p, KDF_SECRET_APPEND));
            m.finish()
        }
        BCRYPT_KDF_TLS_PRF => {
            let label = param(p, KDF_TLS_PRF_LABEL).ok_or(STATUS_INVALID_PARAMETER)?;
            let seed = param(p, KDF_TLS_PRF_SEED).ok_or(STATUS_INVALID_PARAMETER)?;
            if seed.len() != 64 {
                return Err(STATUS_INVALID_PARAMETER);
            }
            let proto = param(p, KDF_TLS_PRF_PROTOCOL).and_then(|v| u32_at(v, 0)).unwrap_or(TLS1_2);
            let ls = [label, seed].concat();
            let mut out = alloc::vec![0u8; 48];
            match proto {
                TLS1_2 => p_hash(hash_param(p, HashAlg::Sha256)?, z, &ls, &mut out),
                TLS1_0 | TLS1_1 => {
                    // RFC 2246 5: P_MD5 over the first half XOR P_SHA-1 over the second.
                    let half = z.len().div_ceil(2);
                    let mut b = alloc::vec![0u8; 48];
                    p_hash(HashAlg::Md5, &z[..half], &ls, &mut out);
                    p_hash(HashAlg::Sha1, &z[z.len() - half..], &ls, &mut b);
                    out.iter_mut().zip(&b).for_each(|(o, x)| *o ^= x);
                }
                _ => return Err(STATUS_NOT_SUPPORTED),
            }
            out
        }
        BCRYPT_KDF_SP80056A_CONCAT => {
            let h = hash_param(p, HashAlg::Sha256)?;
            let len = want.unwrap_or(h.output_len());
            let mut other = Vec::new();
            for t in [KDF_ALGORITHMID, KDF_PARTYUINFO, KDF_PARTYVINFO, KDF_SUPPPUBINFO, KDF_SUPPPRIVINFO] {
                other.extend_from_slice(&concat_params(p, t));
            }
            let mut out = Vec::new();
            let mut counter = 1u32;
            while out.len() < len {
                let mut s = poly_crypto::Hasher::new(h);
                s.update(&counter.to_be_bytes());
                s.update(z);
                s.update(&other);
                out.extend_from_slice(&s.finish());
                counter += 1;
            }
            out.truncate(len);
            out
        }
        _ => return Err(STATUS_NOT_SUPPORTED),
    };
    if let Some(w) = want {
        out.truncate(w);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        poly_x509::text::hex_decode(s.as_bytes()).unwrap()
    }

    fn counter_rng() -> impl FnMut(&mut [u8]) {
        let mut n = 1u8;
        move |b: &mut [u8]| {
            for x in b {
                *x = n;
                n = n.wrapping_mul(31).wrapping_add(7);
            }
        }
    }

    #[test]
    fn ecdsa_blobs_sign_and_verify() {
        let mut rng = counter_rng();
        let k = AsymKey::generate(AlgKind::Ecdsa(Some(Curve::P256)), 256, &mut rng).unwrap();
        let blob = k.export(BCRYPT_ECCPRIVATE_BLOB).unwrap();
        assert_eq!(u32_at(&blob, 0), Some(ECDSA_PRIVATE_P256_MAGIC));
        assert_eq!((u32_at(&blob, 4), blob.len()), (Some(32), 8 + 96));
        let pubblob = k.export(BCRYPT_ECCPUBLIC_BLOB).unwrap();
        let pk = AsymKey::import(AlgKind::Ecdsa(Some(Curve::P256)), BCRYPT_ECCPUBLIC_BLOB, &pubblob).unwrap();
        let hash = poly_crypto::digest(HashAlg::Sha256, b"message");
        let sig = k.sign(&Padding::None, &hash, &mut rng).unwrap();
        assert_eq!(sig.len(), 64);
        assert_eq!(pk.verify(&Padding::None, &hash, &sig), Ok(()));
        let mut bad = sig.clone();
        bad[5] ^= 1;
        assert_eq!(pk.verify(&Padding::None, &hash, &bad), Err(STATUS_INVALID_SIGNATURE));
        assert_eq!(pk.verify(&Padding::None, &hash, &sig[..63]), Err(STATUS_INVALID_SIGNATURE));
        assert_eq!(pk.sign(&Padding::None, &hash, &mut rng), Err(STATUS_INVALID_PARAMETER));
        // Wrong provider or blob type.
        assert!(AsymKey::import(AlgKind::Ecdh(Some(Curve::P256)), BCRYPT_ECCPUBLIC_BLOB, &pubblob).is_err());
        assert!(AsymKey::import(AlgKind::Ecdsa(Some(Curve::P384)), BCRYPT_ECCPUBLIC_BLOB, &pubblob).is_err());
        assert!(AsymKey::import(AlgKind::Ecdsa(Some(Curve::P256)), BCRYPT_ECCPRIVATE_BLOB, &pubblob).is_err());
        let generic = AsymKey::import(AlgKind::Ecdsa(None), BCRYPT_ECCPRIVATE_BLOB, &blob).unwrap();
        assert_eq!(generic.bits(), 256);
        let mut off = pubblob.clone();
        off[40] ^= 1;
        assert!(AsymKey::import(AlgKind::Ecdsa(Some(Curve::P256)), BCRYPT_ECCPUBLIC_BLOB, &off).is_err());
    }

    #[test]
    fn rfc6979_key_through_a_blob() {
        // RFC 6979 A.2.5: the P-256 key; its signature of SHA-256("sample")
        // verifies through the blob path.
        let d = hex("C9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721");
        let x = hex("60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6");
        let y = hex("7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299");
        let blob = [&ECDSA_PRIVATE_P256_MAGIC.to_le_bytes()[..], &32u32.to_le_bytes(), &x, &y, &d].concat();
        let k = AsymKey::import(AlgKind::Ecdsa(Some(Curve::P256)), BCRYPT_ECCPRIVATE_BLOB, &blob).unwrap();
        let r = hex("EFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716");
        let s = hex("F7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8");
        let hash = poly_crypto::digest(HashAlg::Sha256, b"sample");
        assert_eq!(k.verify(&Padding::None, &hash, &[r, s].concat()), Ok(()));
    }

    #[test]
    fn ecdh_agreement_and_kdfs() {
        let mut rng = counter_rng();
        let a = AsymKey::generate(AlgKind::Ecdh(Some(Curve::P384)), 384, &mut rng).unwrap();
        let b = AsymKey::generate(AlgKind::Ecdh(None), 384, &mut rng).unwrap();
        let bpub =
            AsymKey::import(AlgKind::Ecdh(None), BCRYPT_ECCPUBLIC_BLOB, &b.export(BCRYPT_ECCPUBLIC_BLOB).unwrap())
                .unwrap();
        let apub =
            AsymKey::import(AlgKind::Ecdh(None), BCRYPT_ECCPUBLIC_BLOB, &a.export(BCRYPT_ECCPUBLIC_BLOB).unwrap())
                .unwrap();
        let z1 = agree(&a, &bpub).unwrap();
        let z2 = agree(&b, &apub).unwrap();
        assert_eq!(z1, z2);
        assert_eq!(z1.len(), 48);
        assert_eq!(agree(&apub, &bpub), Err(STATUS_INVALID_PARAMETER));
        let raw = derive(&z1, BCRYPT_KDF_RAW_SECRET, &[], None).unwrap();
        assert_eq!(raw.iter().rev().copied().collect::<Vec<_>>(), z1);
        let sha256 = crate::cng::wide_bytes("SHA256");
        let params = [(KDF_HASH_ALGORITHM, &sha256[..]), (KDF_SECRET_PREPEND, &b"pre"[..])];
        let h = derive(&z1, BCRYPT_KDF_HASH, &params, None).unwrap();
        assert_eq!(h, poly_crypto::digest(HashAlg::Sha256, &[&b"pre"[..], &z1].concat()));
        assert_eq!(derive(&z1, BCRYPT_KDF_HASH, &params, Some(16)).unwrap(), h[..16]);
        assert_eq!(derive(&z1, BCRYPT_KDF_HASH, &[], None).unwrap().len(), 20);
        assert_eq!(derive(&z1, "PBKDF2", &[], None), Err(STATUS_NOT_SUPPORTED));
        let concat = derive(&z1, BCRYPT_KDF_SP80056A_CONCAT, &[(KDF_ALGORITHMID, &b"A"[..])], Some(40)).unwrap();
        assert_eq!(concat.len(), 40);
        let first = poly_crypto::digest(HashAlg::Sha256, &[&[0, 0, 0, 1][..], &z1, b"A"].concat());
        assert_eq!(concat[..32], first[..]);
    }

    /// An RSA-1024 key made with the OpenSSL command-line tool for this test.
    fn rsa_private_blob() -> Vec<u8> {
        let n = hex(
            "b5c55f58a192609de65ebf4ca0fcdd36c6ccf99b3e19f51231da3db29264222cb5b8811d076591c8e4e82aac206b91ed6dc98f04\
             8f227ffbd7b37a088c0d016c27106a8f5accf0a57a797a7268afc23d97758ab4adcacab78addb74cae6a385161707706690a4f23\
             2a186a14a7cb27626c8b4dc228c2e040f4d3eedb70ee6b29",
        );
        let p = hex(
            "dd36596753a24f23732ecc31d81302fe7788a6605408b2219a6b576effdd07ad0092bf054a073891aa2d91247ff6e34a964ddfa9\
             03e728605d04db646c5bb887",
        );
        let q = hex(
            "d25b2deddc1def276c887e793b657d8ebf190cc1a2f51ab5bd55bf089692e71121d783c290a231bf9d8ecadba08f889cb0522b86\
             42256e198d7c676503e59acf",
        );
        let mut v = Vec::new();
        for w in [RSAPRIVATE_MAGIC, 1024, 3, 128, 64, 64] {
            v.extend_from_slice(&w.to_le_bytes());
        }
        [v, Vec::from([1, 0, 1]), n, p, q].concat()
    }

    #[test]
    fn rsa_blobs_signatures_and_encryption() {
        let rsa = AlgKind::Rsa { sign_only: false };
        let mut rng = counter_rng();
        let k = AsymKey::import(rsa, BCRYPT_RSAPRIVATE_BLOB, &rsa_private_blob()).unwrap();
        assert_eq!(k.bits(), 1024);
        assert_eq!(k.export(BCRYPT_RSAPRIVATE_BLOB).unwrap(), rsa_private_blob());
        let full = k.export(BCRYPT_RSAFULLPRIVATE_BLOB).unwrap();
        assert_eq!(full.len(), 24 + 3 + 128 + 64 * 5 + 128);
        let k2 = AsymKey::import(rsa, BCRYPT_RSAFULLPRIVATE_BLOB, &full).unwrap();
        let public = k.export(BCRYPT_RSAPUBLIC_BLOB).unwrap();
        assert_eq!(u32_at(&public, 0), Some(RSAPUBLIC_MAGIC));
        let pk = AsymKey::import(rsa, BCRYPT_PUBLIC_KEY_BLOB, &public).unwrap();
        assert_eq!(pk.export(BCRYPT_RSAPRIVATE_BLOB), Err(STATUS_INVALID_PARAMETER));
        let hash = poly_crypto::digest(HashAlg::Sha256, b"abc");
        for pad in [
            Padding::Pkcs1(Some(HashAlg::Sha256)),
            Padding::Pkcs1(None),
            Padding::Pss { hash: HashAlg::Sha256, salt_len: 32 },
        ] {
            let sig = k2.sign(&pad, &hash, &mut rng).unwrap();
            assert_eq!(sig.len(), 128);
            assert_eq!(pk.verify(&pad, &hash, &sig), Ok(()));
            assert_eq!(pk.verify(&pad, &hash[1..], &sig), Err(STATUS_INVALID_SIGNATURE));
        }
        assert_eq!(k.sign(&Padding::None, &hash, &mut rng), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(k.sign(&Padding::Pkcs1(Some(HashAlg::Sha1)), &hash, &mut rng), Err(STATUS_INVALID_PARAMETER));
        for pad in [Padding::Pkcs1(None), Padding::Oaep { hash: HashAlg::Sha1, label: Vec::new() }] {
            let ct = pk.encrypt(&pad, b"secret", &mut rng).unwrap();
            assert_eq!(k.decrypt(&pad, &ct).unwrap(), b"secret");
        }
        assert_eq!(pk.decrypt(&Padding::Pkcs1(None), &[0; 128]), Err(STATUS_NOT_SUPPORTED));
        // A blob whose bit length disagrees with its modulus.
        let mut bad = public.clone();
        bad[5] = 8;
        assert_eq!(AsymKey::import(rsa, BCRYPT_RSAPUBLIC_BLOB, &bad).err(), Some(STATUS_INVALID_PARAMETER));
        assert!(AsymKey::import(rsa, "LEGACY_RSAPUBLIC_BLOB", &public).is_err());
    }

    #[test]
    fn tls12_prf() {
        // P_SHA256 shape check: the first block is HMAC(secret, A(1) || seed).
        let z = [0x0bu8; 48];
        let seed = [7u8; 64];
        let out = derive(
            &z,
            BCRYPT_KDF_TLS_PRF,
            &[(KDF_TLS_PRF_LABEL, &b"master secret"[..]), (KDF_TLS_PRF_SEED, &seed[..])],
            None,
        )
        .unwrap();
        let ls = [&b"master secret"[..], &seed].concat();
        let a1 = poly_crypto::hmac(HashAlg::Sha256, &z, &ls);
        let b1 = poly_crypto::hmac(HashAlg::Sha256, &z, &[&a1[..], &ls].concat());
        assert_eq!(out[..32], b1[..]);
        assert_eq!(out.len(), 48);
        assert!(derive(&z, BCRYPT_KDF_TLS_PRF, &[(KDF_TLS_PRF_LABEL, &b"x"[..])], None).is_err());
    }
}
