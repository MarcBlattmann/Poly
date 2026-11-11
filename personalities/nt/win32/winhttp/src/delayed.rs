//! The names Chrome's `chrome.dll` delay-loads from `winhttp.dll` that Poly does not implement yet (ADR
//! 0155, step 9): each is an export that logs `stub: winhttp!name` and fails. A delay load of a name
//! the DLL does not export ends the browser, whatever the call would have been used for; a
//! failing call it copes with (the fault-injection table of the gap map). An implementation
//! replaces a name here by defining it elsewhere in the crate and deleting it from the list.

poly_win32_rt::fail_exports!("winhttp", win32, [WinHttpGetProxyForUrl, WinHttpFreeProxyResult]);

// These return a Win32 error code: `ERROR_CALL_NOT_IMPLEMENTED`.
poly_win32_rt::fail_exports!(
    "winhttp",
    value(120),
    [WinHttpCreateProxyResolver, WinHttpGetProxyForUrlEx, WinHttpGetProxyResult]
);

// `WinHttpSetStatusCallback`: `WINHTTP_INVALID_STATUS_CALLBACK` (-1) "if the callback function could
// not be installed".
poly_win32_rt::fail_exports!("winhttp", value(u64::MAX), [WinHttpSetStatusCallback]);
