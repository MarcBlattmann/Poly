//! The names Chrome's `chrome.dll` delay-loads from `ole32.dll` that Poly does not implement yet (ADR
//! 0155, step 9): each is an export that logs `stub: ole32!name` and fails. A delay load of a name
//! the DLL does not export ends the browser, whatever the call would have been used for; a
//! failing call it copes with (the fault-injection table of the gap map). An implementation
//! replaces a name here by defining it elsewhere in the crate and deleting it from the list.

poly_win32_rt::fail_exports!(
    "ole32",
    hresult,
    [
        CoAllowSetForegroundWindow,
        CoGetInterfaceAndReleaseStream,
        CoMarshalInterThreadInterfaceInStream,
        CreateILockBytesOnHGlobal,
        CreateStreamOnHGlobal,
        DoDragDrop,
        GetHGlobalFromILockBytes,
        GetHGlobalFromStream,
        PropVariantCopy,
        ReleaseStgMedium,
        RoGetAgileReference,
        StgCreateDocfileOnILockBytes,
        CoRegisterInitializeSpy,
        CoRevokeInitializeSpy
    ]
);

// `OleDuplicateData` returns a handle: NULL.
poly_win32_rt::fail_exports!("ole32", win32, [OleDuplicateData]);
