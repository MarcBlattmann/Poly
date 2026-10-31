//! The Windows-facing decisions of Poly's `bcrypt.dll` and `crypt32.dll`
//! (ADR 0096), host-tested; the DLLs only move bytes between these and
//! their callers' pointers:
//!
//! - [`status`]: the `NTSTATUS`, Win32 and `HRESULT` codes they return.
//! - [`cng`]: CNG algorithm providers and properties, hash objects,
//!   symmetric keys (AES in ECB/CBC/CFB/GCM), PBKDF2.
//! - [`keys`]: CNG asymmetric keys (RSA, ECDSA, ECDH), their blobs,
//!   signatures, RSA encryption, secret agreement and key derivation.
//! - [`flat`]: CryptoAPI structures with pointers laid out in one block.
//! - [`decode`]: `CryptDecodeObjectEx`'s structures and `CERT_INFO`.
//! - [`certstr`]: name strings and `CryptBinaryToString`/`StringToBinary`.
//! - [`store`]: system store files, finds, add dispositions, chain policies.
//! - [`dpapi`]: `CryptProtectData`'s Poly-local blob format.
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's reference pages
//! of the CNG and CryptoAPI functions and structures named in each module,
//! and the constants and layouts of the public SDK headers (`bcrypt.h`,
//! `wincrypt.h`, `dpapi.h`, `ntstatus.h`, `winerror.h`). The algorithms are
//! `poly-crypto`'s, the certificate handling `poly-x509`'s. Where Learn
//! leaves a result open, the code says what Poly picks; those are listed
//! in ADR 0096 for the spec agent to confirm on Windows.

#![no_std]

extern crate alloc;

pub mod certstr;
pub mod cng;
pub mod decode;
pub mod dpapi;
pub mod flat;
pub mod keys;
pub mod status;
pub mod store;

/// The functions our `bcrypt.dll` exports (checked by `cargo xtask win32`).
pub const BCRYPT_EXPORTS: &[&str] = &[
    "BCryptOpenAlgorithmProvider",
    "BCryptCloseAlgorithmProvider",
    "BCryptGetProperty",
    "BCryptSetProperty",
    "BCryptGetFipsAlgorithmMode",
    "BCryptCreateHash",
    "BCryptHashData",
    "BCryptFinishHash",
    "BCryptDuplicateHash",
    "BCryptDestroyHash",
    "BCryptHash",
    "BCryptGenRandom",
    "BCryptGenerateSymmetricKey",
    "BCryptEncrypt",
    "BCryptDecrypt",
    "BCryptImportKey",
    "BCryptExportKey",
    "BCryptDuplicateKey",
    "BCryptDestroyKey",
    "BCryptDeriveKeyPBKDF2",
    "BCryptImportKeyPair",
    "BCryptGenerateKeyPair",
    "BCryptFinalizeKeyPair",
    "BCryptSignHash",
    "BCryptVerifySignature",
    "BCryptSecretAgreement",
    "BCryptDeriveKey",
    "BCryptDestroySecret",
];

/// The functions our `crypt32.dll` exports (checked by `cargo xtask win32`).
pub const CRYPT32_EXPORTS: &[&str] = &[
    "CertOpenStore",
    "CertOpenSystemStoreW",
    "CertOpenSystemStoreA",
    "CertDuplicateStore",
    "CertCloseStore",
    "CertEnumCertificatesInStore",
    "CertFindCertificateInStore",
    "CertAddEncodedCertificateToStore",
    "CertAddCertificateContextToStore",
    "CertDeleteCertificateFromStore",
    "CertCreateCertificateContext",
    "CertDuplicateCertificateContext",
    "CertFreeCertificateContext",
    "CertGetCertificateContextProperty",
    "CertSetCertificateContextProperty",
    "CertGetNameStringW",
    "CertGetNameStringA",
    "CertNameToStrW",
    "CertGetPublicKeyLength",
    "CertVerifyTimeValidity",
    "CryptDecodeObjectEx",
    "CryptDecodeObject",
    "CryptBinaryToStringW",
    "CryptBinaryToStringA",
    "CryptStringToBinaryW",
    "CryptStringToBinaryA",
    "CertGetCertificateChain",
    "CertDuplicateCertificateChain",
    "CertFreeCertificateChain",
    "CertVerifyCertificateChainPolicy",
    "CryptProtectData",
    "CryptUnprotectData",
];
