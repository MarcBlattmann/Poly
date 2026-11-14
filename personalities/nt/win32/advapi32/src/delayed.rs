//! The names Chrome's `chrome.dll` delay-loads from `advapi32.dll` that Poly does not implement yet (ADR
//! 0155, step 9): each is an export that logs `stub: advapi32!name` and fails. A delay load of a name
//! the DLL does not export ends the browser, whatever the call would have been used for; a
//! failing call it copes with (the fault-injection table of the gap map). An implementation
//! replaces a name here by defining it elsewhere in the crate and deleting it from the list.

poly_win32_rt::fail_exports!(
    "advapi32",
    win32,
    [
        AddConditionalAce,
        AllocateLocallyUniqueId,
        CryptGenRandom,
        CryptGetProvParam,
        CryptSetHashParam,
        CryptSignHashW,
        ImpersonateAnonymousToken,
        LogonUserW,
        QueryServiceStatus,
        CloseServiceHandle,
        CreateProcessWithTokenW,
        CryptAcquireContextW,
        CryptCreateHash,
        CryptDestroyHash,
        CryptGetHashParam,
        CryptReleaseContext,
        DeregisterEventSource,
        OpenSCManagerW,
        OpenServiceW,
        RegisterEventSourceA,
        ReportEventA
    ]
);

// `RegDisablePredefinedCache` returns a Win32 error code: `ERROR_CALL_NOT_IMPLEMENTED`.
poly_win32_rt::fail_exports!("advapi32", value(120), [RegDisablePredefinedCache]);

// The LSA policy functions return an `NTSTATUS`.
poly_win32_rt::fail_exports!("advapi32", status, [LsaAddAccountRights, LsaClose, LsaOpenPolicy]);

// `LsaNtStatusToWinError` returns a Win32 error code for the status it is given.
poly_win32_rt::fail_exports!("advapi32", value(120), [LsaNtStatusToWinError]);
