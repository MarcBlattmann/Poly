//! The functions the Windows applications of the survey (`windows-app-survey`) import from `secur32.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: secur32!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!("secur32", status, [LsaEnumerateLogonSessions, LsaGetLogonSessionData,]);

poly_win32_rt::fail_exports!("secur32", value(0), [InitSecurityInterfaceA, InitSecurityInterfaceW,]);

poly_win32_rt::fail_exports!(
    "secur32",
    value(0x80090302),
    [
        AcceptSecurityContext,
        AcquireCredentialsHandleA,
        ApplyControlToken,
        CompleteAuthToken,
        DecryptMessage,
        EncryptMessage,
        InitializeSecurityContextA,
        QueryContextAttributesA,
        QuerySecurityContextToken,
        SetContextAttributesA,
        SetCredentialsAttributesW,
    ]
);
