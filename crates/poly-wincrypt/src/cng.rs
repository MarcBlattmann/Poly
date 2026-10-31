//! CNG (`bcrypt.dll`) algorithm providers, their properties, hash objects
//! and symmetric keys: everything of `bcrypt` that is not pointer handling.
//!
//! Sources: Microsoft Learn's reference pages of each `BCrypt*` function,
//! "CNG Algorithm Identifiers", "Cryptography Primitive Property
//! Identifiers", "CNG Algorithm Pseudo-handles", "BCRYPT_KEY_DATA_BLOB_HEADER",
//! "BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO"; the identifiers and constants
//! of `bcrypt.h`; `ntstatus.h`. Where Learn leaves a result open the code
//! says what Poly picks ("to be confirmed by the spec agent", ADR 0096).

use crate::status::*;
use alloc::string::String;
use alloc::vec::Vec;
use poly_crypto::HashAlg;
use poly_crypto::aes::Aes;
use poly_crypto::ec::Curve;
use poly_crypto::gcm::{Gcm, GcmStream};
use poly_crypto::modes;

/// `str` as a NUL-terminated UTF-16LE byte string (a CNG string property).
pub fn wide_bytes(s: &str) -> Vec<u8> {
    s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect()
}

/// A DWORD property value.
pub fn dword(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

/// What an algorithm provider handle does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlgKind {
    Hash(HashAlg),
    /// A hash provider opened with `BCRYPT_ALG_HANDLE_HMAC_FLAG`.
    Hmac(HashAlg),
    Aes,
    /// `RSA` (`sign_only` for `RSA_SIGN`).
    Rsa {
        sign_only: bool,
    },
    /// `ECDSA_P256`/`ECDSA_P384`, or `ECDSA` whose curve is set by the
    /// `ECCCurveName` property.
    Ecdsa(Option<Curve>),
    /// `ECDH_P256`/`ECDH_P384`, or `ECDH` with `ECCCurveName`.
    Ecdh(Option<Curve>),
    Rng,
}

// `bcrypt.h` flags.
pub const BCRYPT_ALG_HANDLE_HMAC_FLAG: u32 = 0x8;
pub const BCRYPT_PROV_DISPATCH: u32 = 0x1;
pub const BCRYPT_HASH_REUSABLE_FLAG: u32 = 0x20;
pub const BCRYPT_RNG_USE_ENTROPY_IN_BUFFER: u32 = 0x1;
pub const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x2;
pub const BCRYPT_BLOCK_PADDING: u32 = 0x1;
pub const BCRYPT_PAD_NONE: u32 = 0x1;
pub const BCRYPT_PAD_PKCS1: u32 = 0x2;
pub const BCRYPT_PAD_OAEP: u32 = 0x4;
pub const BCRYPT_PAD_PSS: u32 = 0x8;
pub const BCRYPT_AUTH_MODE_CHAIN_CALLS_FLAG: u32 = 0x1;
pub const BCRYPT_AUTH_MODE_IN_PROGRESS_FLAG: u32 = 0x2;
pub const BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION: u32 = 1;
/// `sizeof(BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO)` on x64.
pub const AUTH_INFO_SIZE: u32 = 88;
pub const BCRYPT_NO_KEY_VALIDATION: u32 = 0x8;
/// `MS_PRIMITIVE_PROVIDER`.
pub const MS_PRIMITIVE_PROVIDER: &str = "Microsoft Primitive Provider";

/// The provider an algorithm identifier names (case-insensitively; to be
/// confirmed) with the open flags: `STATUS_NOT_FOUND` for an algorithm
/// Poly does not have (Learn), `STATUS_INVALID_PARAMETER` for the HMAC
/// flag on a non-hash algorithm or unknown flags.
pub fn open_algorithm(id: &str, flags: u32) -> Result<AlgKind, NtStatus> {
    if flags & !(BCRYPT_ALG_HANDLE_HMAC_FLAG | BCRYPT_PROV_DISPATCH | BCRYPT_HASH_REUSABLE_FLAG) != 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let up = id.to_ascii_uppercase();
    let kind = match up.as_str() {
        "MD5" => AlgKind::Hash(HashAlg::Md5),
        "SHA1" => AlgKind::Hash(HashAlg::Sha1),
        "SHA256" => AlgKind::Hash(HashAlg::Sha256),
        "SHA384" => AlgKind::Hash(HashAlg::Sha384),
        "SHA512" => AlgKind::Hash(HashAlg::Sha512),
        "AES" => AlgKind::Aes,
        "RSA" => AlgKind::Rsa { sign_only: false },
        "RSA_SIGN" => AlgKind::Rsa { sign_only: true },
        "ECDSA_P256" => AlgKind::Ecdsa(Some(Curve::P256)),
        "ECDSA_P384" => AlgKind::Ecdsa(Some(Curve::P384)),
        "ECDSA" => AlgKind::Ecdsa(None),
        "ECDH_P256" => AlgKind::Ecdh(Some(Curve::P256)),
        "ECDH_P384" => AlgKind::Ecdh(Some(Curve::P384)),
        "ECDH" => AlgKind::Ecdh(None),
        "RNG" => AlgKind::Rng,
        _ => return Err(STATUS_NOT_FOUND),
    };
    if flags & BCRYPT_ALG_HANDLE_HMAC_FLAG != 0 {
        return match kind {
            AlgKind::Hash(h) => Ok(AlgKind::Hmac(h)),
            _ => Err(STATUS_NOT_FOUND),
        };
    }
    Ok(kind)
}

/// The algorithm of a pseudo-handle (`bcrypt.h`, Windows 10: "CNG
/// Algorithm Pseudo-handles"): the hash, HMAC and RNG ones.
pub fn pseudo_handle(h: usize) -> Option<AlgKind> {
    Some(match h {
        0x21 => AlgKind::Hash(HashAlg::Md5),
        0x31 => AlgKind::Hash(HashAlg::Sha1),
        0x41 => AlgKind::Hash(HashAlg::Sha256),
        0x51 => AlgKind::Hash(HashAlg::Sha384),
        0x61 => AlgKind::Hash(HashAlg::Sha512),
        0x81 => AlgKind::Rng,
        0x91 => AlgKind::Hmac(HashAlg::Md5),
        0xa1 => AlgKind::Hmac(HashAlg::Sha1),
        0xb1 => AlgKind::Hmac(HashAlg::Sha256),
        0xc1 => AlgKind::Hmac(HashAlg::Sha384),
        0xd1 => AlgKind::Hmac(HashAlg::Sha512),
        _ => return None,
    })
}

/// A hash algorithm by its CNG identifier (padding infos, KDF parameters).
pub fn hash_by_name(id: &str) -> Option<HashAlg> {
    match open_algorithm(id, 0) {
        Ok(AlgKind::Hash(h)) => Some(h),
        _ => None,
    }
}

/// The CNG identifier of a hash.
pub fn hash_name(h: HashAlg) -> &'static str {
    match h {
        HashAlg::Md5 => "MD5",
        HashAlg::Sha1 => "SHA1",
        HashAlg::Sha256 => "SHA256",
        HashAlg::Sha384 => "SHA384",
        HashAlg::Sha512 => "SHA512",
    }
}

/// The `AlgorithmName` property.
pub fn algorithm_name(k: AlgKind) -> &'static str {
    match k {
        AlgKind::Hash(h) | AlgKind::Hmac(h) => hash_name(h),
        AlgKind::Aes => "AES",
        AlgKind::Rsa { sign_only: false } => "RSA",
        AlgKind::Rsa { sign_only: true } => "RSA_SIGN",
        AlgKind::Ecdsa(Some(Curve::P256)) => "ECDSA_P256",
        AlgKind::Ecdsa(Some(Curve::P384)) => "ECDSA_P384",
        AlgKind::Ecdsa(None) => "ECDSA",
        AlgKind::Ecdh(Some(Curve::P256)) => "ECDH_P256",
        AlgKind::Ecdh(Some(Curve::P384)) => "ECDH_P384",
        AlgKind::Ecdh(None) => "ECDH",
        AlgKind::Rng => "RNG",
    }
}

/// A curve by its `ECCCurveName` (`BCRYPT_ECC_CURVE_*`).
pub fn curve_by_name(name: &str) -> Option<Curve> {
    match name {
        "nistP256" | "secP256r1" => Some(Curve::P256),
        "nistP384" | "secP384r1" => Some(Curve::P384),
        _ => None,
    }
}

/// A curve's `ECCCurveName`.
pub fn curve_name(c: Curve) -> &'static str {
    match c {
        Curve::P256 => "nistP256",
        Curve::P384 => "nistP384",
    }
}

/// A block cipher chaining mode (`BCRYPT_CHAIN_MODE_*`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainMode {
    Ecb,
    Cbc,
    Cfb,
    Gcm,
}

impl ChainMode {
    /// The property string.
    pub fn name(self) -> &'static str {
        match self {
            ChainMode::Ecb => "ChainingModeECB",
            ChainMode::Cbc => "ChainingModeCBC",
            ChainMode::Cfb => "ChainingModeCFB",
            ChainMode::Gcm => "ChainingModeGCM",
        }
    }

    /// The mode a property string names (CCM and others: not supported).
    pub fn from_name(s: &str) -> Option<ChainMode> {
        [ChainMode::Ecb, ChainMode::Cbc, ChainMode::Cfb, ChainMode::Gcm].into_iter().find(|m| m.name() == s)
    }
}

// Property names (`bcrypt.h`).
pub const BCRYPT_OBJECT_LENGTH: &str = "ObjectLength";
pub const BCRYPT_ALGORITHM_NAME: &str = "AlgorithmName";
pub const BCRYPT_PROVIDER_HANDLE: &str = "ProviderHandle";
pub const BCRYPT_CHAINING_MODE: &str = "ChainingMode";
pub const BCRYPT_BLOCK_LENGTH: &str = "BlockLength";
pub const BCRYPT_KEY_LENGTH: &str = "KeyLength";
pub const BCRYPT_KEY_OBJECT_LENGTH: &str = "KeyObjectLength";
pub const BCRYPT_KEY_STRENGTH: &str = "KeyStrength";
pub const BCRYPT_KEY_LENGTHS: &str = "KeyLengths";
pub const BCRYPT_BLOCK_SIZE_LIST: &str = "BlockSizeList";
pub const BCRYPT_HASH_LENGTH: &str = "HashDigestLength";
pub const BCRYPT_HASH_BLOCK_LENGTH: &str = "HashBlockLength";
pub const BCRYPT_HASH_OID_LIST: &str = "HashOIDList";
pub const BCRYPT_PADDING_SCHEMES: &str = "PaddingSchemes";
pub const BCRYPT_INITIALIZATION_VECTOR: &str = "IV";
pub const BCRYPT_MESSAGE_BLOCK_LENGTH: &str = "MessageBlockLength";
pub const BCRYPT_AUTH_TAG_LENGTH: &str = "AuthTagLength";
pub const BCRYPT_ECC_CURVE_NAME: &str = "ECCCurveName";
pub const BCRYPT_SIGNATURE_LENGTH: &str = "SignatureLength";
pub const BCRYPT_IS_KEYED_HASH: &str = "IsKeyedHash";
pub const BCRYPT_IS_REUSABLE_HASH: &str = "IsReusableHash";
pub const BCRYPT_MULTI_OBJECT_LENGTH: &str = "MultiObjectLength";

/// `BCRYPT_SUPPORTED_PAD_*` (`bcrypt.h`).
pub const SUPPORTED_PAD_PKCS1_ENC: u32 = 0x2;
pub const SUPPORTED_PAD_PKCS1_SIG: u32 = 0x4;
pub const SUPPORTED_PAD_OAEP: u32 = 0x8;
pub const SUPPORTED_PAD_PSS: u32 = 0x10;

/// The object length Poly reports for hash and key objects. Poly keeps
/// its state on its own heap and never touches a caller's object buffer
/// (Learn allows NULL for it since Windows 7), so a small number lets
/// every caller's buffer fit.
pub const OBJECT_LENGTH: u32 = 16;

/// `BCRYPT_KEY_LENGTHS_STRUCT` (also `BCRYPT_AUTH_TAG_LENGTHS_STRUCT`).
fn lengths(min: u32, max: u32, inc: u32) -> Vec<u8> {
    [min, max, inc].iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// GCM's tag lengths in bytes: 12 to 16 (to be confirmed).
pub const GCM_TAG_MIN: usize = 12;
pub const GCM_TAG_MAX: usize = 16;

/// A property of an algorithm provider (`mode`: its chaining mode, for
/// AES). `STATUS_NOT_SUPPORTED` for a property the provider has not.
pub fn algorithm_property(k: AlgKind, mode: ChainMode, prop: &str) -> Result<Vec<u8>, NtStatus> {
    let hash = match k {
        AlgKind::Hash(h) | AlgKind::Hmac(h) => Some(h),
        _ => None,
    };
    Ok(match prop {
        BCRYPT_ALGORITHM_NAME => wide_bytes(algorithm_name(k)),
        BCRYPT_OBJECT_LENGTH if hash.is_some() || k == AlgKind::Aes => dword(OBJECT_LENGTH),
        BCRYPT_HASH_LENGTH => dword(hash.ok_or(STATUS_NOT_SUPPORTED)?.output_len() as u32),
        BCRYPT_HASH_BLOCK_LENGTH => dword(hash.ok_or(STATUS_NOT_SUPPORTED)?.block_len() as u32),
        BCRYPT_IS_KEYED_HASH if hash.is_some() => dword(u32::from(matches!(k, AlgKind::Hmac(_)))),
        BCRYPT_IS_REUSABLE_HASH if hash.is_some() => dword(1),
        BCRYPT_BLOCK_LENGTH if k == AlgKind::Aes => dword(16),
        BCRYPT_CHAINING_MODE if k == AlgKind::Aes => wide_bytes(mode.name()),
        BCRYPT_BLOCK_SIZE_LIST if k == AlgKind::Aes => dword(16),
        BCRYPT_KEY_LENGTHS => match k {
            AlgKind::Aes => lengths(128, 256, 64),
            AlgKind::Rsa { .. } => lengths(512, 16384, 64),
            AlgKind::Ecdsa(Some(c)) | AlgKind::Ecdh(Some(c)) => {
                let b = c.field_len() as u32 * 8;
                lengths(b, b, 0)
            }
            AlgKind::Ecdsa(None) | AlgKind::Ecdh(None) => lengths(256, 384, 128),
            _ => return Err(STATUS_NOT_SUPPORTED),
        },
        BCRYPT_AUTH_TAG_LENGTH if k == AlgKind::Aes && mode == ChainMode::Gcm => {
            lengths(GCM_TAG_MIN as u32, GCM_TAG_MAX as u32, 1)
        }
        BCRYPT_PADDING_SCHEMES if matches!(k, AlgKind::Rsa { .. }) => {
            dword(SUPPORTED_PAD_PKCS1_ENC | SUPPORTED_PAD_PKCS1_SIG | SUPPORTED_PAD_OAEP | SUPPORTED_PAD_PSS)
        }
        BCRYPT_ECC_CURVE_NAME => match k {
            AlgKind::Ecdsa(Some(c)) | AlgKind::Ecdh(Some(c)) => wide_bytes(curve_name(c)),
            _ => return Err(STATUS_NOT_SUPPORTED),
        },
        _ => return Err(STATUS_NOT_SUPPORTED),
    })
}

/// A string property value (UTF-16LE, NUL-terminated within `cb`).
pub fn decode_wide(bytes: &[u8]) -> Option<String> {
    let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    let end = units.iter().position(|&u| u == 0)?;
    String::from_utf16(&units[..end]).ok()
}

// ----- Hash objects -------------------------------------------------------------------

/// An HMAC holds two hash states; boxing one would only add an
/// allocation per hash object.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
enum HashState {
    Plain(poly_crypto::Hasher),
    Mac(poly_crypto::Hmac),
}

/// A hash object (`BCRYPT_HASH_HANDLE`).
#[derive(Clone, Debug)]
pub struct HashObject {
    alg: HashAlg,
    /// The HMAC key (to restart a reusable object).
    key: Option<Vec<u8>>,
    state: HashState,
    reusable: bool,
    finished: bool,
}

impl HashObject {
    /// A hash object of provider `k` (a hash or HMAC provider; other
    /// providers: `STATUS_INVALID_HANDLE`, as their handle is not a hash
    /// provider's, to be confirmed). Non-HMAC providers ignore `secret`.
    pub fn new(k: AlgKind, secret: &[u8], flags: u32) -> Result<HashObject, NtStatus> {
        if flags & !BCRYPT_HASH_REUSABLE_FLAG != 0 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let (alg, key) = match k {
            AlgKind::Hash(h) => (h, None),
            AlgKind::Hmac(h) => (h, Some(secret.to_vec())),
            _ => return Err(STATUS_INVALID_HANDLE),
        };
        let mut o = HashObject {
            alg,
            key,
            state: HashState::Plain(poly_crypto::Hasher::new(alg)),
            reusable: false,
            finished: false,
        };
        o.reusable = flags & BCRYPT_HASH_REUSABLE_FLAG != 0;
        o.restart();
        Ok(o)
    }

    fn restart(&mut self) {
        self.state = match &self.key {
            Some(k) => HashState::Mac(poly_crypto::Hmac::new(self.alg, k)),
            None => HashState::Plain(poly_crypto::Hasher::new(self.alg)),
        };
        self.finished = false;
    }

    pub fn alg(&self) -> HashAlg {
        self.alg
    }

    pub fn is_hmac(&self) -> bool {
        self.key.is_some()
    }

    /// `BCryptHashData`. A finished non-reusable object refuses more data
    /// (`STATUS_INVALID_HANDLE`, to be confirmed).
    pub fn update(&mut self, data: &[u8]) -> Result<(), NtStatus> {
        if self.finished {
            return Err(STATUS_INVALID_HANDLE);
        }
        match &mut self.state {
            HashState::Plain(h) => h.update(data),
            HashState::Mac(m) => m.update(data),
        }
        Ok(())
    }

    /// `BCryptFinishHash`: `out_len` must be the digest length (Learn:
    /// "The size of this buffer must match the hash length");
    /// `STATUS_INVALID_PARAMETER` otherwise.
    pub fn finish(&mut self, out_len: usize) -> Result<Vec<u8>, NtStatus> {
        if self.finished {
            return Err(STATUS_INVALID_HANDLE);
        }
        if out_len != self.alg.output_len() {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let state = core::mem::replace(&mut self.state, HashState::Plain(poly_crypto::Hasher::new(self.alg)));
        let digest = match state {
            HashState::Plain(h) => h.finish(),
            HashState::Mac(m) => m.finish(),
        };
        if self.reusable {
            self.restart();
        } else {
            self.finished = true;
        }
        Ok(digest)
    }

    /// The properties of a hash object.
    pub fn property(&self, prop: &str) -> Result<Vec<u8>, NtStatus> {
        let k = if self.is_hmac() { AlgKind::Hmac(self.alg) } else { AlgKind::Hash(self.alg) };
        match prop {
            BCRYPT_OBJECT_LENGTH => Err(STATUS_NOT_SUPPORTED),
            _ => algorithm_property(k, ChainMode::Cbc, prop),
        }
    }
}

/// `BCryptHash`: one-shot hash or HMAC.
pub fn hash_once(k: AlgKind, secret: &[u8], input: &[u8], out_len: usize) -> Result<Vec<u8>, NtStatus> {
    let mut o = HashObject::new(k, secret, 0)?;
    o.update(input)?;
    o.finish(out_len)
}

// ----- Symmetric keys -----------------------------------------------------------------

/// `BCRYPT_KEY_DATA_BLOB_MAGIC` ('KDBM') and its version.
pub const KEY_DATA_BLOB_MAGIC: u32 = 0x4d42_444b;
pub const KEY_DATA_BLOB_VERSION1: u32 = 1;
/// Blob type names (`bcrypt.h`).
pub const BCRYPT_KEY_DATA_BLOB: &str = "KeyDataBlob";
pub const BCRYPT_OPAQUE_KEY_BLOB: &str = "OpaqueKeyBlob";
/// Poly's opaque blob: 'PLYK', version, chaining mode, key length, key.
pub const POLY_OPAQUE_MAGIC: u32 = 0x4b59_4c50;

/// Authenticated cipher parameters of one `BCryptEncrypt`/`Decrypt` call,
/// read from its `BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO`.
#[derive(Debug)]
pub struct AuthParams<'a> {
    pub nonce: &'a [u8],
    pub aad: &'a [u8],
    /// Encryption writes the tag here; decryption checks it.
    pub tag: &'a mut [u8],
    /// `BCRYPT_AUTH_MODE_CHAIN_CALLS_FLAG`.
    pub chain: bool,
}

/// An AES key (`BCRYPT_KEY_HANDLE` of the AES provider).
#[derive(Clone, Debug)]
pub struct SymKey {
    secret: Vec<u8>,
    aes: Aes,
    pub mode: ChainMode,
    /// CFB's feedback size in bytes (`MessageBlockLength`): 1 by default
    /// (CFB-8; to be confirmed) or 16.
    pub segment: usize,
    /// The IV used when a call passes none (zeros at first, then chained).
    iv: [u8; 16],
    stream: Option<GcmStream>,
}

impl SymKey {
    /// `BCryptGenerateSymmetricKey`: 16, 24 or 32 bytes of key
    /// (`STATUS_INVALID_PARAMETER` otherwise).
    pub fn new(secret: &[u8], mode: ChainMode) -> Result<SymKey, NtStatus> {
        let aes = Aes::new(secret).ok_or(STATUS_INVALID_PARAMETER)?;
        Ok(SymKey { secret: secret.to_vec(), aes, mode, segment: 1, iv: [0; 16], stream: None })
    }

    /// `BCryptImportKey` of a `KeyDataBlob` or Poly's opaque blob.
    pub fn import(blob_type: &str, blob: &[u8], mode: ChainMode) -> Result<SymKey, NtStatus> {
        let u32_at = |o: usize| blob.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        match blob_type {
            BCRYPT_KEY_DATA_BLOB => {
                let (magic, version, len) = (u32_at(0), u32_at(4), u32_at(8));
                if magic != Some(KEY_DATA_BLOB_MAGIC) || version != Some(KEY_DATA_BLOB_VERSION1) {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let len = len.ok_or(STATUS_INVALID_PARAMETER)? as usize;
                let key = blob.get(12..12 + len).ok_or(STATUS_INVALID_PARAMETER)?;
                SymKey::new(key, mode)
            }
            BCRYPT_OPAQUE_KEY_BLOB => {
                if u32_at(0) != Some(POLY_OPAQUE_MAGIC) || u32_at(4) != Some(1) {
                    return Err(STATUS_INVALID_PARAMETER);
                }
                let mode = match u32_at(8) {
                    Some(0) => ChainMode::Ecb,
                    Some(1) => ChainMode::Cbc,
                    Some(2) => ChainMode::Cfb,
                    Some(3) => ChainMode::Gcm,
                    _ => return Err(STATUS_INVALID_PARAMETER),
                };
                let len = u32_at(12).ok_or(STATUS_INVALID_PARAMETER)? as usize;
                SymKey::new(blob.get(16..16 + len).ok_or(STATUS_INVALID_PARAMETER)?, mode)
            }
            _ => Err(STATUS_NOT_SUPPORTED),
        }
    }

    /// `BCryptExportKey`.
    pub fn export(&self, blob_type: &str) -> Result<Vec<u8>, NtStatus> {
        let mut v = Vec::new();
        match blob_type {
            BCRYPT_KEY_DATA_BLOB => {
                for w in [KEY_DATA_BLOB_MAGIC, KEY_DATA_BLOB_VERSION1, self.secret.len() as u32] {
                    v.extend_from_slice(&w.to_le_bytes());
                }
            }
            BCRYPT_OPAQUE_KEY_BLOB => {
                let mode = match self.mode {
                    ChainMode::Ecb => 0u32,
                    ChainMode::Cbc => 1,
                    ChainMode::Cfb => 2,
                    ChainMode::Gcm => 3,
                };
                for w in [POLY_OPAQUE_MAGIC, 1, mode, self.secret.len() as u32] {
                    v.extend_from_slice(&w.to_le_bytes());
                }
            }
            _ => return Err(STATUS_NOT_SUPPORTED),
        }
        v.extend_from_slice(&self.secret);
        Ok(v)
    }

    /// A copy with fresh state (`BCryptDuplicateKey`).
    pub fn duplicate(&self) -> SymKey {
        let mut k = self.clone();
        k.stream = None;
        k
    }

    /// The key properties.
    pub fn property(&self, prop: &str) -> Result<Vec<u8>, NtStatus> {
        match prop {
            BCRYPT_KEY_LENGTH | BCRYPT_KEY_STRENGTH => Ok(dword(self.secret.len() as u32 * 8)),
            BCRYPT_INITIALIZATION_VECTOR => Ok(self.iv.to_vec()),
            BCRYPT_MESSAGE_BLOCK_LENGTH if self.mode == ChainMode::Cfb => Ok(dword(self.segment as u32)),
            BCRYPT_OBJECT_LENGTH | BCRYPT_KEY_LENGTHS => Err(STATUS_NOT_SUPPORTED),
            _ => algorithm_property(AlgKind::Aes, self.mode, prop),
        }
    }

    /// `BCryptSetProperty` on the key: `ChainingMode` and
    /// `MessageBlockLength` (1 or 16).
    pub fn set_property(&mut self, prop: &str, value: &[u8]) -> Result<(), NtStatus> {
        match prop {
            BCRYPT_CHAINING_MODE => {
                let m = decode_wide(value).and_then(|s| ChainMode::from_name(&s)).ok_or(STATUS_NOT_SUPPORTED)?;
                self.mode = m;
                self.stream = None;
                Ok(())
            }
            BCRYPT_MESSAGE_BLOCK_LENGTH => {
                let v = value.get(..4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                match v {
                    Some(1) => self.segment = 1,
                    Some(16) => self.segment = 16,
                    _ => return Err(STATUS_INVALID_PARAMETER),
                }
                Ok(())
            }
            _ => Err(STATUS_NOT_SUPPORTED),
        }
    }

    /// The output size of a block-mode call (`pbOutput` NULL):
    /// `STATUS_INVALID_BUFFER_SIZE` for input that is not whole blocks
    /// (or segments) without padding (Learn).
    pub fn output_len(&self, encrypt: bool, input_len: usize, padding: bool) -> Result<usize, NtStatus> {
        let unit = match self.mode {
            ChainMode::Cfb => self.segment,
            ChainMode::Gcm => return Ok(input_len),
            _ => 16,
        };
        if padding && self.mode != ChainMode::Gcm {
            return Ok(if encrypt { (input_len / 16 + 1) * 16 } else { input_len });
        }
        if !input_len.is_multiple_of(unit) {
            return Err(STATUS_INVALID_BUFFER_SIZE);
        }
        Ok(input_len)
    }

    fn take_iv(&self, iv: &Option<&mut [u8]>) -> Result<[u8; 16], NtStatus> {
        match iv {
            Some(v) => <[u8; 16]>::try_from(&v[..]).map_err(|_| STATUS_INVALID_PARAMETER),
            None => Ok(self.iv),
        }
    }

    /// `BCryptEncrypt` in ECB, CBC or CFB mode. A given IV must be 16 bytes
    /// and is updated for the next call (Learn); without one the key's
    /// own IV is used and kept. ECB ignores the IV.
    pub fn encrypt(&mut self, input: &[u8], mut iv: Option<&mut [u8]>, padding: bool) -> Result<Vec<u8>, NtStatus> {
        if self.mode == ChainMode::Gcm {
            return Err(STATUS_INVALID_PARAMETER);
        }
        self.output_len(true, input.len(), padding)?;
        let mut data = input.to_vec();
        if padding {
            modes::pkcs7_pad(&mut data, 16);
        }
        let mut chain = self.take_iv(&iv)?;
        let ok = match self.mode {
            ChainMode::Ecb => modes::ecb_encrypt(&self.aes, &mut data),
            ChainMode::Cbc => modes::cbc_encrypt(&self.aes, &mut chain, &mut data),
            ChainMode::Cfb => modes::cfb_encrypt(&self.aes, &mut chain, self.segment, &mut data),
            ChainMode::Gcm => false,
        };
        if !ok {
            return Err(STATUS_INVALID_BUFFER_SIZE);
        }
        self.store_iv(&mut iv, chain);
        Ok(data)
    }

    fn store_iv(&mut self, iv: &mut Option<&mut [u8]>, chain: [u8; 16]) {
        if self.mode == ChainMode::Ecb {
            return;
        }
        match iv {
            Some(v) => v.copy_from_slice(&chain),
            None => self.iv = chain,
        }
    }

    /// `BCryptDecrypt` in ECB, CBC or CFB mode; bad padding is
    /// `STATUS_DATA_ERROR` (to be confirmed).
    pub fn decrypt(&mut self, input: &[u8], mut iv: Option<&mut [u8]>, padding: bool) -> Result<Vec<u8>, NtStatus> {
        if self.mode == ChainMode::Gcm {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let unit = if self.mode == ChainMode::Cfb && !padding { self.segment } else { 16 };
        if !input.len().is_multiple_of(unit) || (padding && input.is_empty()) {
            return Err(STATUS_INVALID_BUFFER_SIZE);
        }
        let mut data = input.to_vec();
        let mut chain = self.take_iv(&iv)?;
        let ok = match self.mode {
            ChainMode::Ecb => modes::ecb_decrypt(&self.aes, &mut data),
            ChainMode::Cbc => modes::cbc_decrypt(&self.aes, &mut chain, &mut data),
            ChainMode::Cfb => modes::cfb_decrypt(&self.aes, &mut chain, self.segment, &mut data),
            ChainMode::Gcm => false,
        };
        if !ok {
            return Err(STATUS_INVALID_BUFFER_SIZE);
        }
        if padding {
            let n = modes::pkcs7_unpad(&data, 16).ok_or(STATUS_DATA_ERROR)?;
            data.truncate(n);
        }
        self.store_iv(&mut iv, chain);
        Ok(data)
    }

    /// `BCryptEncrypt`/`BCryptDecrypt` in GCM mode. The nonce must be 12
    /// bytes and the tag 12..16 (`STATUS_INVALID_PARAMETER` otherwise; to
    /// be confirmed). Chained calls (`chain`) keep the state in the key
    /// until a call without the flag finishes it; every chained call but
    /// the last must be whole blocks (Learn). A wrong tag is
    /// `STATUS_AUTH_TAG_MISMATCH` and no plaintext is returned from a
    /// single call.
    pub fn gcm(&mut self, encrypt: bool, input: &[u8], p: AuthParams<'_>) -> Result<Vec<u8>, NtStatus> {
        if self.mode != ChainMode::Gcm {
            return Err(STATUS_INVALID_PARAMETER);
        }
        if !(GCM_TAG_MIN..=GCM_TAG_MAX).contains(&p.tag.len()) {
            return Err(STATUS_INVALID_PARAMETER);
        }
        if self.stream.is_none() && p.nonce.len() != 12 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let mut data = input.to_vec();
        if p.chain || self.stream.is_some() {
            if p.chain && !input.len().is_multiple_of(16) {
                return Err(STATUS_INVALID_BUFFER_SIZE);
            }
            let aes = self.aes.clone();
            let s = self.stream.get_or_insert_with(|| {
                let mut s = Gcm::new(aes).stream(p.nonce);
                s.aad(p.aad);
                s
            });
            if encrypt {
                s.encrypt(&mut data);
            } else {
                s.decrypt(&mut data);
            }
            if p.chain {
                return Ok(data);
            }
            let mut tag = [0u8; 16];
            s.tag(&mut tag[..p.tag.len()]);
            self.stream = None;
            if encrypt {
                p.tag.copy_from_slice(&tag[..p.tag.len()]);
            } else if !poly_crypto::ct_eq(&tag[..p.tag.len()], p.tag) {
                return Err(STATUS_AUTH_TAG_MISMATCH);
            }
            return Ok(data);
        }
        let g = Gcm::new(self.aes.clone());
        if encrypt {
            g.seal(p.nonce, p.aad, &mut data, p.tag);
        } else if !g.open(p.nonce, p.aad, &mut data, p.tag) {
            return Err(STATUS_AUTH_TAG_MISMATCH);
        }
        Ok(data)
    }
}

/// `BCryptDeriveKeyPBKDF2`: the PRF must be an HMAC provider
/// (`STATUS_INVALID_HANDLE` otherwise, to be confirmed); zero iterations
/// are `STATUS_INVALID_PARAMETER`.
pub fn pbkdf2(prf: AlgKind, password: &[u8], salt: &[u8], iterations: u64, out: &mut [u8]) -> Result<(), NtStatus> {
    let AlgKind::Hmac(h) = prf else { return Err(STATUS_INVALID_HANDLE) };
    if iterations == 0 {
        return Err(STATUS_INVALID_PARAMETER);
    }
    poly_crypto::pbkdf2::pbkdf2_hmac(h, password, salt, iterations, out);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        poly_x509::text::hex_decode(s.as_bytes()).unwrap()
    }

    #[test]
    fn algorithms_and_properties() {
        assert_eq!(open_algorithm("SHA256", 0), Ok(AlgKind::Hash(HashAlg::Sha256)));
        assert_eq!(open_algorithm("sha256", BCRYPT_ALG_HANDLE_HMAC_FLAG), Ok(AlgKind::Hmac(HashAlg::Sha256)));
        assert_eq!(open_algorithm("AES", BCRYPT_ALG_HANDLE_HMAC_FLAG), Err(STATUS_NOT_FOUND));
        assert_eq!(open_algorithm("SHA3-256", 0), Err(STATUS_NOT_FOUND));
        assert_eq!(open_algorithm("SHA1", 0x4000), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(pseudo_handle(0x41), Some(AlgKind::Hash(HashAlg::Sha256)));
        assert_eq!(pseudo_handle(0x81), Some(AlgKind::Rng));
        assert_eq!(pseudo_handle(0x40), None);
        let k = AlgKind::Hash(HashAlg::Sha384);
        assert_eq!(algorithm_property(k, ChainMode::Cbc, BCRYPT_HASH_LENGTH), Ok(dword(48)));
        assert_eq!(algorithm_property(k, ChainMode::Cbc, BCRYPT_HASH_BLOCK_LENGTH), Ok(dword(128)));
        assert_eq!(algorithm_property(k, ChainMode::Cbc, BCRYPT_ALGORITHM_NAME), Ok(wide_bytes("SHA384")));
        assert_eq!(algorithm_property(k, ChainMode::Cbc, BCRYPT_BLOCK_LENGTH), Err(STATUS_NOT_SUPPORTED));
        assert_eq!(
            algorithm_property(AlgKind::Aes, ChainMode::Gcm, BCRYPT_CHAINING_MODE),
            Ok(wide_bytes("ChainingModeGCM"))
        );
        assert_eq!(algorithm_property(AlgKind::Aes, ChainMode::Cbc, BCRYPT_AUTH_TAG_LENGTH), Err(STATUS_NOT_SUPPORTED));
        assert_eq!(algorithm_property(AlgKind::Aes, ChainMode::Gcm, BCRYPT_AUTH_TAG_LENGTH), Ok(lengths(12, 16, 1)));
        assert_eq!(algorithm_property(AlgKind::Aes, ChainMode::Cbc, BCRYPT_KEY_LENGTHS), Ok(lengths(128, 256, 64)));
        assert_eq!(decode_wide(&wide_bytes("ChainingModeCBC")).as_deref(), Some("ChainingModeCBC"));
        assert_eq!(hash_by_name("SHA512"), Some(HashAlg::Sha512));
        assert_eq!(hash_by_name("AES"), None);
    }

    #[test]
    fn hash_objects() {
        // FIPS 180-4 "abc".
        let abc = hex("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let mut h = HashObject::new(AlgKind::Hash(HashAlg::Sha256), b"", 0).unwrap();
        h.update(b"a").unwrap();
        let mut dup = h.clone();
        h.update(b"bc").unwrap();
        assert_eq!(h.finish(31), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(h.finish(32).unwrap(), abc);
        assert_eq!(h.update(b"x"), Err(STATUS_INVALID_HANDLE));
        dup.update(b"bc").unwrap();
        assert_eq!(dup.finish(32).unwrap(), abc);
        // Reusable: starts over after finishing.
        let mut r = HashObject::new(AlgKind::Hash(HashAlg::Sha256), b"", BCRYPT_HASH_REUSABLE_FLAG).unwrap();
        for _ in 0..2 {
            r.update(b"abc").unwrap();
            assert_eq!(r.finish(32).unwrap(), abc);
        }
        // RFC 4231 test case 2 (HMAC-SHA-256, key "Jefe").
        let mac = hash_once(AlgKind::Hmac(HashAlg::Sha256), b"Jefe", b"what do ya want for nothing?", 32).unwrap();
        assert_eq!(mac, hex("5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"));
        assert!(HashObject::new(AlgKind::Aes, b"", 0).is_err());
        assert_eq!(h.property(BCRYPT_HASH_LENGTH), Ok(dword(32)));
    }

    #[test]
    fn cbc_with_padding_and_chained_ivs() {
        // SP 800-38A F.2.1 (CBC-AES128.Encrypt), first two blocks.
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let pt = hex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
        let ct = hex("7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2");
        let mut k = SymKey::new(&key, ChainMode::Cbc).unwrap();
        let mut iv = hex("000102030405060708090a0b0c0d0e0f");
        let first = k.encrypt(&pt[..16], Some(&mut iv), false).unwrap();
        let second = k.encrypt(&pt[16..], Some(&mut iv), false).unwrap();
        assert_eq!([first, second].concat(), ct);
        assert_eq!(k.output_len(true, 15, false), Err(STATUS_INVALID_BUFFER_SIZE));
        assert_eq!(k.output_len(true, 16, true), Ok(32));
        let mut iv = [7u8; 16];
        let enc = k.encrypt(b"hello", Some(&mut iv[..]), true).unwrap();
        assert_eq!(enc.len(), 16);
        let mut iv = [7u8; 16];
        assert_eq!(k.decrypt(&enc, Some(&mut iv[..]), true).unwrap(), b"hello");
        let mut iv = [7u8; 16];
        iv[15] ^= 0x80;
        assert!(k.decrypt(&enc, Some(&mut iv[..]), true).is_err_and(|e| e == STATUS_DATA_ERROR));
        assert_eq!(k.encrypt(&pt, Some(&mut [0u8; 8][..]), false), Err(STATUS_INVALID_PARAMETER));
        assert_eq!(SymKey::new(&[0; 15], ChainMode::Cbc).err(), Some(STATUS_INVALID_PARAMETER));
    }

    #[test]
    fn ecb_cfb_and_blobs() {
        // FIPS 197 C.1.
        let key = hex("000102030405060708090a0b0c0d0e0f");
        let mut k = SymKey::new(&key, ChainMode::Ecb).unwrap();
        let out = k.encrypt(&hex("00112233445566778899aabbccddeeff"), None, false).unwrap();
        assert_eq!(out, hex("69c4e0d86a7b0430d8cdb78070b4c55a"));
        let blob = k.export(BCRYPT_KEY_DATA_BLOB).unwrap();
        assert_eq!(&blob[..12], &[0x4b, 0x44, 0x42, 0x4d, 1, 0, 0, 0, 16, 0, 0, 0]);
        let k2 = SymKey::import(BCRYPT_KEY_DATA_BLOB, &blob, ChainMode::Ecb).unwrap();
        assert_eq!(k2.secret, key);
        let opaque = SymKey::new(&key, ChainMode::Cfb).unwrap().export(BCRYPT_OPAQUE_KEY_BLOB).unwrap();
        assert_eq!(SymKey::import(BCRYPT_OPAQUE_KEY_BLOB, &opaque, ChainMode::Cbc).unwrap().mode, ChainMode::Cfb);
        assert!(SymKey::import(BCRYPT_KEY_DATA_BLOB, &blob[..20], ChainMode::Ecb).is_err());
        // SP 800-38A F.3.7 (CFB8-AES128.Encrypt), first bytes.
        let key = hex("2b7e151628aed2a6abf7158809cf4f3c");
        let mut k = SymKey::new(&key, ChainMode::Cfb).unwrap();
        let mut iv = hex("000102030405060708090a0b0c0d0e0f");
        let out = k.encrypt(&hex("6bc1bee22e409f96e93d7e117393172a"), Some(&mut iv), false).unwrap();
        assert_eq!(out, hex("3b79424c9c0dd436bace9e0ed4586a4f"));
        assert_eq!(k.output_len(true, 3, false), Ok(3));
        k.set_property(BCRYPT_MESSAGE_BLOCK_LENGTH, &dword(16)).unwrap();
        assert_eq!(k.output_len(true, 3, false), Err(STATUS_INVALID_BUFFER_SIZE));
        k.set_property(BCRYPT_CHAINING_MODE, &wide_bytes("ChainingModeECB")).unwrap();
        assert_eq!(k.mode, ChainMode::Ecb);
        assert_eq!(k.set_property(BCRYPT_CHAINING_MODE, &wide_bytes("ChainingModeXTS")), Err(STATUS_NOT_SUPPORTED));
    }

    #[test]
    fn gcm_single_and_chained() {
        // GCM specification (McGrew-Viega) test case 3.
        let key = hex("feffe9928665731c6d6a8f9467308308");
        let iv = hex("cafebabefacedbaddecaf888");
        let pt =
            hex("d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525\
             b16aedf5aa0de657ba637b391aafd255");
        let ct =
            hex("42831ec2217774244b7221b784d0d49ce3aa212f2c02a4e035c17e2329aca12e21d514b25466931c7d8f6a5aac84aa05\
             1ba30b396a0aac973d58e091473f5985");
        let want_tag = hex("4d5c2af327cd64a62cf35abd2ba6fab4");
        let mut k = SymKey::new(&key, ChainMode::Gcm).unwrap();
        let mut tag = [0u8; 16];
        let out = k.gcm(true, &pt, AuthParams { nonce: &iv, aad: &[], tag: &mut tag, chain: false }).unwrap();
        assert_eq!((out.clone(), tag.to_vec()), (ct.clone(), want_tag.clone()));
        let back = k.gcm(false, &out, AuthParams { nonce: &iv, aad: &[], tag: &mut tag, chain: false }).unwrap();
        assert_eq!(back, pt);
        tag[0] ^= 1;
        let bad = k.gcm(false, &out, AuthParams { nonce: &iv, aad: &[], tag: &mut tag, chain: false });
        assert_eq!(bad, Err(STATUS_AUTH_TAG_MISMATCH));
        // Chained: two blocks, then the rest.
        let mut tag = [0u8; 16];
        let a = k.gcm(true, &pt[..32], AuthParams { nonce: &iv, aad: &[], tag: &mut tag, chain: true }).unwrap();
        let b = k.gcm(true, &pt[32..], AuthParams { nonce: &iv, aad: &[], tag: &mut tag, chain: false }).unwrap();
        assert_eq!(([a, b].concat(), tag.to_vec()), (ct, want_tag));
        let mut short = [0u8; 11];
        let r = k.gcm(true, &pt, AuthParams { nonce: &iv, aad: &[], tag: &mut short, chain: false });
        assert_eq!(r, Err(STATUS_INVALID_PARAMETER));
        let r = k.gcm(true, &pt[..5], AuthParams { nonce: &iv, aad: &[], tag: &mut tag, chain: true });
        assert_eq!(r, Err(STATUS_INVALID_BUFFER_SIZE));
    }

    #[test]
    fn pbkdf2_needs_hmac() {
        // RFC 6070 test 2.
        let mut out = [0u8; 20];
        pbkdf2(AlgKind::Hmac(HashAlg::Sha1), b"password", b"salt", 2, &mut out).unwrap();
        assert_eq!(out.to_vec(), hex("ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"));
        assert_eq!(pbkdf2(AlgKind::Hash(HashAlg::Sha1), b"p", b"s", 1, &mut out), Err(STATUS_INVALID_HANDLE));
        assert_eq!(pbkdf2(AlgKind::Hmac(HashAlg::Sha1), b"p", b"s", 0, &mut out), Err(STATUS_INVALID_PARAMETER));
    }
}
