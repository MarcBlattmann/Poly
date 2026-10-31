# 0096: Cryptography: our `bcrypt.dll` and `crypt32.dll`, algorithms from the standards in host-tested crates, system stores as files, an empty `ROOT` store, a Poly-local `CryptProtectData`

**Status:** Accepted. Delivers the Phase 5 roadmap item "Cryptography:
`bcrypt`, `crypt32` certificate stores". Builds on
[ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md) (Win32 modules
built from Rust), [ADR 0079](0079-our-kernelbase-and-kernel32.md),
[ADR 0083](0083-advapi32-registry-tokens-and-security-for-one-user.md)
(`advapi32`, single user), [ADR 0087](0087-7-zip-command-line-on-poly.md)
(`SystemFunction036`, how a DLL is added) and
[ADR 0073](0073-windows-file-system-layout.md) (`%ProgramData%`,
`%APPDATA%`).

## Context

Almost every networked Windows program hashes, draws random numbers,
verifies a certificate chain or keeps a secret through CNG (`bcrypt.dll`)
and CryptoAPI (`crypt32.dll`). Winsock/WinHTTP come next on the roadmap
and will need TLS, so the algorithms and the certificate code have to be
reusable outside these two DLLs.

Sources (clean room, CLEAN_ROOM.md; the implementer is `impl-crypto`):
Microsoft Learn's reference pages of every function implemented and of
the structures they take (`BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO`,
`BCRYPT_KEY_DATA_BLOB_HEADER`, `BCRYPT_RSAKEY_BLOB`, `BCRYPT_ECCKEY_BLOB`,
the padding infos, `BCryptBufferDesc`, `CERT_CONTEXT`, `CERT_INFO`,
`CERT_CHAIN_CONTEXT` and its parts, `CERT_CHAIN_POLICY_PARA`/`STATUS`,
`HTTPSPolicyCallbackData`, `DATA_BLOB`, the `CryptDecodeObjectEx` output
structures), "CNG Algorithm Identifiers", "Cryptography Primitive Property
Identifiers", "CNG Algorithm Pseudo-handles", "System Store Locations";
the constants and layouts of the public SDK headers `bcrypt.h`,
`wincrypt.h`, `dpapi.h`, `wininet.h`, `ntstatus.h`, `winerror.h`; and for
the algorithms the public standards only: FIPS 180-4 (SHA-1, SHA-2), RFC
1321 (MD5), RFC 2104 / FIPS 198-1 (HMAC), RFC 8018 (PBKDF2), FIPS 197
(AES), SP 800-38A (ECB, CBC, CFB), SP 800-38D (GCM), RFC 8017 (RSA:
PKCS#1 v1.5, PSS, OAEP), FIPS 186-4 and SEC 1 (P-256, P-384, ECDSA), RFC
6979 (deterministic nonces), SP 800-56A (ECDH, the concatenation KDF), RFC
5246/2246 (the TLS PRFs), ITU-T X.690 (DER), RFC 5280, 3279, 4055, 5480,
5758 (X.509), RFC 6125 (host names), RFC 4648 (Base64), RFC 7468 (PEM). No
code of any cryptographic library, Wine or ReactOS was read; nothing was
run on Windows.

## Decision

### Three host-tested crates

- **`crates/poly-crypto`** — the algorithms, `no_std` + `alloc`, no
  dependencies: MD5, SHA-1, SHA-256/384/512, HMAC, PBKDF2, AES with
  ECB/CBC/CFB-8/CFB-128/GCM (and streaming GCM), PKCS#7 padding, big
  integers with Montgomery multiplication, RSA (raw, PKCS#1 v1.5
  signatures and encryption, PSS, OAEP, CRT private operations), P-256 and
  P-384 (ECDSA with RFC 6979 nonces plus extra randomness as in its
  section 3.6, ECDH). Tested with the standards' known-answer vectors:
  FIPS 180-4's examples, RFC 1321's suite, RFC 4231 and 2202, RFC 6070,
  FIPS 197 appendix C, SP 800-38A F.1-F.3, the GCM specification's test
  cases, RFC 6979 A.2.5/A.2.6; RSA fixtures made with the OpenSSL CLI and
  every vector cross-checked with OpenSSL or Python's `hashlib`. **Not
  hardened against side channels** (timing, cache); MAC and tag
  comparisons are constant-time. No RSA key generation (no primality
  test yet), no SHA-224/SHA-3, no CTR/CCM/XTS, no DRBG (randomness is the
  kernel's, below).
- **`crates/poly-x509`** — DER reading and writing, OIDs, X.509
  certificates and their common extensions (basic constraints, key usage,
  extended key usage, alternative names, key identifiers), distinguished
  names and their string forms, signature verification (RSA PKCS#1 v1.5,
  RSASSA-PSS, ECDSA), path building with per-certificate status in
  CryptoAPI's `CERT_TRUST_*` bits (validity, signature, basic constraints
  and path length, requested usages, untrusted root, partial chain,
  cycles), RFC 6125 host matching, Base64/hex/PEM. Fixtures:
  OpenSSL-made certificates in `src/testdata/` (RSA root, P-256
  intermediate with `pathlen:0`, P-384 leaf with SANs, an expired
  RSA-PSS-signed leaf, a self-signed cert, a CA that breaks the path
  length).
- **`crates/poly-wincrypt`** — every Windows-facing decision: algorithm
  identifiers, properties and pseudo-handles, hash objects, AES keys and
  their blobs, RSA/ECC blobs, padding, the KDFs of `BCryptDeriveKey`
  (`HASH`, `HMAC`, `TLS_PRF`, `SP800_56A_CONCAT`, `TRUNCATE`), `NTSTATUS`
  results; `CERT_INFO` and the `CryptDecodeObjectEx` structures laid out
  for the address they will live at (`flat`), `CertGetNameString` /
  `CertNameToStr`, `CryptBinaryToString` / `CryptStringToBinary`, store
  files, find types, add dispositions, chain policies, the data
  protection format. The DLLs only move bytes.

The Win32 DLLs may use these crates: they are this repo's own `no_std`
code, as `poly-kernelbase` and `poly-security` are for `kernelbase` and
`advapi32`; the rule "no third-party crates in the kernel" is kept (none
were added anywhere).

### `bcrypt.dll`

Algorithm providers `MD5`, `SHA1`, `SHA256`, `SHA384`, `SHA512` (and
their HMAC form with `BCRYPT_ALG_HANDLE_HMAC_FLAG`), `AES`, `RSA`,
`RSA_SIGN`, `ECDSA_P256`, `ECDSA_P384`, `ECDSA`/`ECDH` (curve through
`ECCCurveName`), `ECDH_P256`, `ECDH_P384`, `RNG`, opened by name (any
case; the Microsoft Primitive Provider or none) or used through the
hash, HMAC and RNG pseudo-handles. Exports: `BCryptOpenAlgorithmProvider`,
`CloseAlgorithmProvider`, `GetProperty`, `SetProperty`,
`GetFipsAlgorithmMode` (FALSE), `CreateHash`, `HashData`, `FinishHash`,
`DuplicateHash`, `DestroyHash`, `Hash`, `GenRandom`,
`GenerateSymmetricKey`, `Encrypt`, `Decrypt`, `ImportKey`, `ExportKey`,
`DuplicateKey`, `DestroyKey`, `DeriveKeyPBKDF2`, `ImportKeyPair`,
`GenerateKeyPair` + `FinalizeKeyPair` (ECC), `SignHash`,
`VerifySignature`, `SecretAgreement`, `DeriveKey`, `DestroySecret`.

- Handles are boxed objects on the process heap, checked against a list
  of live ones (a stale handle is `STATUS_INVALID_HANDLE`, never
  followed). Callers' `pbHashObject`/`pbKeyObject` buffers are never
  written; `ObjectLength` is 16 so every caller's buffer fits.
- **Randomness**: `BCryptGenRandom` (an RNG provider or the
  system-preferred RNG) reads the kernel's `/dev/urandom` through
  `advapi32`'s `SystemFunction036`, as `RtlGenRandom` does (ADR 0087).
- AES keys take the provider's chaining mode; IVs are updated in place
  (or kept in the key when none is passed); `BCRYPT_BLOCK_PADDING` is
  PKCS#7. GCM takes `BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO` (12-byte
  nonce, 12..16-byte tag) including chained calls
  (`BCRYPT_AUTH_MODE_CHAIN_CALLS_FLAG`, state kept in the key,
  `IN_PROGRESS` set in the caller's structure). Key blobs:
  `KeyDataBlob` and an opaque Poly blob.
- RSA and ECC blobs per `bcrypt.h` (`RSA1/2/3`, `ECS1-4`, `ECK1-4`, the
  generic magics); an RSA private blob with only the primes gets `d`
  and the CRT values computed. Signatures: PKCS#1 v1.5 (with or without
  `pszAlgId`), PSS; ECDSA `r || s`. RSA encryption: none, PKCS#1, OAEP.
  RSA key generation answers `STATUS_NOT_SUPPORTED`.

### `crypt32.dll`

Exports: `CertOpenStore` (memory and system providers, by number or
name), `CertOpenSystemStoreW/A`, `CertDuplicateStore`, `CertCloseStore`,
`CertEnumCertificatesInStore`, `CertFindCertificateInStore` (any, SHA-1,
MD5, SHA-256, subject/issuer name, subject/issuer string W/A, public key,
existing, issuer of, subject cert, key identifier, enhanced key usage),
`CertAddEncodedCertificateToStore`, `CertAddCertificateContextToStore`
(all seven dispositions), `CertDeleteCertificateFromStore`,
`CertCreateCertificateContext`, `CertDuplicateCertificateContext`,
`CertFreeCertificateContext`, `CertGetCertificateContextProperty` (SHA-1,
MD5, SHA-256 hashes, key identifier, friendly name),
`CertSetCertificateContextProperty` (friendly name), `CertGetNameStringW/A`
(all eight types), `CertNameToStrW`, `CertGetPublicKeyLength`,
`CertVerifyTimeValidity`, `CryptDecodeObjectEx`/`CryptDecodeObject`
(`X509_CERT`, `X509_CERT_TO_BE_SIGNED`, `X509_NAME`, `X509_UNICODE_NAME`,
`X509_PUBLIC_KEY_INFO`, `X509_EXTENSIONS`, `X509_BASIC_CONSTRAINTS2`,
`X509_KEY_USAGE`, `X509_BITS`, `X509_OCTET_STRING`, `X509_INTEGER`,
`X509_MULTI_BYTE_INTEGER`, `X509_SEQUENCE_OF_ANY`, `X509_ALTERNATE_NAME`,
`X509_ENHANCED_KEY_USAGE`, `X509_AUTHORITY_KEY_ID2`, `X509_ECC_SIGNATURE`,
`RSA_CSP_PUBLICKEYBLOB` and the matching `szOID_*` strings; integers
little-endian, `X509_CERT`'s signature byte-reversed as Learn says),
`CryptBinaryToStringW/A`, `CryptStringToBinaryW/A` (Base64 with and
without the three headers, hex, hex with addresses and ASCII, raw hex,
the `*_ANY` forms), `CertGetCertificateChain`,
`CertDuplicateCertificateChain`, `CertFreeCertificateChain`,
`CertVerifyCertificateChainPolicy` (base, SSL, basic constraints),
`CryptProtectData`, `CryptUnprotectData`.

- Contexts are real `CERT_CONTEXT`s followed by Poly's bookkeeping; the
  `CERT_INFO` and everything it points to is one block. Contexts are
  reference counted; a store holds one reference to each of its
  contexts, and contexts a caller holds outlive the store.
- **System stores are files, not the registry.** A store `NAME` of the
  current user is `%APPDATA%\Poly\SystemCertificates\NAME\`, of the local
  machine `%ProgramData%\Poly\SystemCertificates\NAME\`, one certificate
  per file (`<SHA-1>.cer`, DER; PEM files dropped there are read too).
  The user's stores also show the machine's (read-only), as Learn
  describes for `ROOT`. Windows keeps these in a registry format of its
  own that Learn does not document; nothing outside `crypt32` reads it, so
  Poly does not imitate it.
- **Chains** are built from the end certificate's store,
  `hAdditionalStore`, the `CA` stores and the `ROOT` stores (the trusted
  ones) of the engine's location (`HCCE_LOCAL_MACHINE` or the user), with
  `poly-x509`'s path building; one simple chain, no lower-quality chains.
  **Revocation is not checked** (no CRL/OCSP fetching yet): asking for it
  marks the elements `REVOCATION_STATUS_UNKNOWN | IS_OFFLINE_REVOCATION`,
  which the base policy reports as `CRYPT_E_NO_REVOCATION_CHECK` unless
  ignored. The SSL policy matches `pwszServerName` against the dNSName
  SANs (else the CNs) with RFC 6125's wildcard rule.

### The `ROOT` store ships empty

Poly ships **no** CA certificates. The Mozilla bundle (MPL-2.0) would
work as data, but it needs a refresh process, a distrust policy and an
entry in THIRD_PARTY.md, and nothing on Poly can make a TLS connection
yet. To trust a root, put its certificate (DER or PEM) into
`C:\ProgramData\Poly\SystemCertificates\ROOT\` (every user) or
`%APPDATA%\Poly\SystemCertificates\ROOT\`, or add it with
`CertAddEncodedCertificateToStore` to an opened `ROOT` store (no prompt
is shown, unlike Windows). Shipping a bundle is revisited with the
Winsock/WinHTTP item.

### `CryptProtectData`: honest and weaker than Windows

Windows protects with a key derived from the user's logon secret. Poly
has one user and no logon secret, so the key comes from a 32-byte
**master key file** made on first use: `%APPDATA%\Poly\Protect\masterkey`,
or `%ProgramData%\Poly\Protect\masterkey` with
`CRYPTPROTECT_LOCAL_MACHINE`. The blob (`PDP1`, flags, salt, nonce,
description, ciphertext, tag) is AES-256-GCM under
HMAC-SHA-256(master key, label || salt || optional entropy), the header
and description authenticated. It keeps a blob from opening on another
machine or profile, or without the right entropy; it does **not** protect
against anyone who can read the profile. Blobs are not compatible with
Windows' DPAPI.

### Wiring and tests

New Win32 workspace members `cryptrt` (shared heap allocator, spin lock,
handle registry, files, random bytes), `bcrypt`, `crypt32` and the test
program `cryptest`; the outputs, `kernel/build.rs`, `nt::BCRYPT`/`CRYPT32`/
`CRYPTEST`, the System32 install list and `KNOWN_DLLS` extended
additively; `cargo xtask win32` checks the exports and that `cryptest.exe`
imports only `bcrypt`, `crypt32` and `kernel32`.

The ring-3 test `nt_crypto_program_hashes_encrypts_and_verifies_a_chain`
runs `cryptest.exe` (144 checks): hashing and HMAC with known answers
(objects, duplicates, pseudo-handles, wrong sizes, stale handles), the
RNG, AES-GCM (the GCM specification's test case 3, a wrong tag) and CBC
with padding, PBKDF2 (RFC 6070), ECDSA sign/export/import/verify, ECDH
agreement, PEM decoding through `CryptStringToBinaryA`, Base64 round
trip, `CERT_INFO`, name strings, `CryptDecodeObjectEx` with allocation and
a truncated input, memory stores, duplicates refused, finds, a partial
chain (`CERT_E_CHAINING`), the root added to the user's `ROOT` store and
found after reopening, a full chain with the base policy and the SSL
policy (right name, `CERT_E_CN_NO_MATCH`), and `CryptProtectData` with
description and entropy.

## To be confirmed by the spec agent against real Windows

1. Algorithm names case-insensitive; `ObjectLength` values (Poly: 16);
   `BCryptHashData` after a non-reusable finish (Poly:
   `STATUS_INVALID_HANDLE`); `BCryptCreateHash` on a non-hash provider.
2. AES-CFB's default `MessageBlockLength` (Poly: 1, CFB-8); bad PKCS#7
   padding on decrypt (Poly: `STATUS_DATA_ERROR`); GCM nonce lengths
   other than 12 and the tag length range (Poly: 12..16); ECB with an IV.
3. Blob errors (Poly: `STATUS_INVALID_PARAMETER` for a malformed or
   mismatched blob, `STATUS_NOT_SUPPORTED` for an unknown blob type);
   exporting a private blob of a public key; RSA decryption padding
   failures; ECDSA signatures of the wrong length (Poly:
   `STATUS_INVALID_SIGNATURE`); `BCRYPT_KDF_HMAC` without `KDF_HMAC_KEY`
   (Poly: keyed with the secret).
4. `CertNameToStr`'s default RDN order (Poly: encoded order) and quoting;
   `CertGetNameString`'s simple display fallback; the exact
   `CryptBinaryToString` hex layouts and whether `HEXRAW` ends with a line
   break; `CryptStringToBinary`'s error for bad text (Poly:
   `ERROR_INVALID_DATA`).
5. `CryptDecodeObjectEx`'s error for an unknown structure type (Poly:
   `ERROR_FILE_NOT_FOUND`) and an unknown provider in `CertOpenStore`.
6. `hCertStore` of a `CertCreateCertificateContext` context (Poly: NULL);
   which certificates "match" for the add dispositions (Poly: the same
   encoding).
7. The base policy's order of checks and `lChainIndex`/`lElementIndex`
   when there is no error (Poly: -1); the chain's `dwInfoStatus` (Poly:
   `HAS_PREFERRED_ISSUER` for a trusted chain); `USAGE_MATCH_TYPE_OR`
   (Poly: treated as AND).
8. `CryptUnprotectData`'s error for a damaged blob or wrong entropy
   (Poly: `ERROR_INVALID_DATA`).

## Consequences

- Host tests: `poly-crypto` (33), `poly-x509` (27), `poly-wincrypt`
  (24); the ring-3 test in every self-test boot.
- Coverage: the 28 `bcrypt.dll` and 32 `crypt32.dll` entries above are
  claimed by `impl-crypto`; `ncrypt.dll` (key storage providers) is not
  started.
- Left: RSA key generation, CCM/CTR/XTS, `BCryptEnumAlgorithms` and the
  context/configuration functions, collection stores, CRL and CTL
  functions, revocation checking, PKCS#7/CMS messages
  (`CryptMsg*`, `CryptQueryObject`), PFX import, encoding
  (`CryptEncodeObjectEx`), Authenticode policies, a shipped CA bundle,
  `ncrypt.dll`.
