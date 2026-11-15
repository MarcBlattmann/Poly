//! More names the Windows applications of the survey import from `ncrypt.dll` (ADR 0155, step 10), fail-only like the
//! ones of `lib.rs` (same failure values); generated from the import tables of the programs and the coverage map.

poly_win32_rt::fail_exports!(
    "ncrypt",
    status,
    [
        BCryptCloseAlgorithmProvider,
        BCryptCreateHash,
        BCryptDestroyHash,
        BCryptDestroyKey,
        BCryptFinishHash,
        BCryptGenRandom,
        BCryptHashData,
        BCryptOpenAlgorithmProvider,
        BCryptVerifySignature,
    ]
);

poly_win32_rt::fail_exports!(
    "ncrypt",
    value(0x80090029),
    [NCryptDecrypt, NCryptDeriveKey, NCryptSecretAgreement, NCryptVerifySignature,]
);
