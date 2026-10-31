//! Certificates for Poly's `crypt32.dll` (ADR 0096), as host-tested code
//! reusable by anything else that needs them (TLS later):
//!
//! - [`der`]: ASN.1 DER (ITU-T X.690) reading and writing.
//! - [`oid`]: object identifiers as dotted strings, and the ones X.509 uses.
//! - [`time`]: certificate times as `FILETIME`s.
//! - [`name`]: distinguished names, their string forms, general names.
//! - [`cert`]: X.509 certificates and their common extensions (RFC 5280).
//! - [`verify`]: certificate signatures (RSA PKCS#1 v1.5, RSASSA-PSS,
//!   ECDSA), with `poly-crypto`'s algorithms.
//! - [`chain`]: path building to a trusted root with per-certificate
//!   status, and host name matching (RFC 6125).
//! - [`text`]: Base64 (RFC 4648), hex and PEM (RFC 7468).
//!
//! Clean room (CLEAN_ROOM.md): written from ITU-T X.690, RFC 5280, RFC
//! 3279, RFC 4055, RFC 5480, RFC 5758, RFC 6125, RFC 4648, RFC 7468, and
//! for the CryptoAPI-facing details (status bits, name string forms)
//! `wincrypt.h` and Microsoft Learn's pages. The test certificates were
//! made with the OpenSSL command-line tool (`src/testdata/`); no code of any
//! certificate library was read or copied.

#![no_std]

extern crate alloc;

pub mod cert;
pub mod chain;
pub mod der;
pub mod name;
pub mod oid;
pub mod text;
pub mod time;
pub mod verify;

pub use cert::Certificate;
