//! Cryptographic algorithms for Poly's `bcrypt.dll` and `crypt32.dll` (ADR
//! 0096), written from their public standards and reusable by anything
//! else that needs them (TLS later):
//!
//! - [`md5`] (RFC 1321), [`sha1`], [`sha256`], [`sha512`] (SHA-1, SHA-256,
//!   SHA-384, SHA-512; FIPS 180-4), one interface over them in [`hash`].
//! - [`hmac`] (RFC 2104 / FIPS 198-1) and [`pbkdf2`] (RFC 8018 5.2).
//! - [`aes`] (FIPS 197), its confidentiality modes in [`modes`] (ECB, CBC,
//!   CFB-8 and CFB-128 of SP 800-38A; PKCS#7 padding of RFC 5652 6.3) and
//!   [`gcm`] (SP 800-38D).
//! - [`bignum`]: unsigned big integers with Montgomery multiplication.
//! - [`rsa`] (RFC 8017: RSASSA-PKCS1-v1_5, RSASSA-PSS, RSAES-PKCS1-v1_5,
//!   RSAES-OAEP).
//! - [`ec`]: the NIST curves P-256 and P-384 (FIPS 186-4 D.1.2.3/D.1.2.4,
//!   SEC 1): ECDSA (FIPS 186-4 6) with deterministic nonces (RFC 6979) and
//!   ECDH (SP 800-56A 5.7.1.2).
//!
//! Clean room (CLEAN_ROOM.md): only the standards named above were used; no
//! code of any cryptographic library was read or copied. The tests use the
//! known-answer vectors the standards publish (each module says which);
//! RSA fixtures were made with the OpenSSL command-line tool and every
//! vector was checked with OpenSSL or Python's `hashlib`/`hmac` before it
//! was written down.
//!
//! None of this is hardened against side channels (timing, cache): table
//! lookups and big-integer operations depend on secret data. MAC and tag
//! comparisons are constant-time ([`ct_eq`]).

#![no_std]

extern crate alloc;

pub mod aes;
pub mod bignum;
mod block;
pub mod ec;
pub mod gcm;
pub mod hash;
pub mod hmac;
pub mod md5;
pub mod modes;
pub mod pbkdf2;
pub mod rsa;
#[cfg(test)]
mod rsa_vectors;
pub mod sha1;
pub mod sha256;
pub mod sha512;

pub use hash::{HashAlg, Hasher, digest};
pub use hmac::{Hmac, hmac};

/// Whether `a` and `b` are equal, in time that depends only on their
/// lengths; false when the lengths differ.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let diff = a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y));
    core::hint::black_box(diff) == 0
}

#[cfg(test)]
pub(crate) mod testutil {
    extern crate std;
    use alloc::vec::Vec;

    /// Bytes from hex digits; whitespace is ignored.
    pub fn hex(s: &str) -> Vec<u8> {
        let digits: Vec<u8> = s.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
        assert!(digits.len().is_multiple_of(2), "odd hex string");
        digits.chunks(2).map(|p| u8::from_str_radix(core::str::from_utf8(p).unwrap(), 16).unwrap()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_compares() {
        assert!(ct_eq(b"", b""));
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }
}
