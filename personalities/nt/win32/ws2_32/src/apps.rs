//! The functions the Windows applications of the survey (`windows-app-survey`) import from `ws2_32.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: ws2_32!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!("ws2_32", error(10045, 0), [gethostbyaddr,]);

poly_win32_rt::fail_exports!(
    "ws2_32",
    error(10045, 0xFFFFFFFF),
    [
        WSAAccept,
        WSAAsyncSelect,
        WSAEnumProtocolsA,
        WSAEnumProtocolsW,
        WSASetServiceW,
        WSCEnumProtocols,
        WSCGetProviderInfo,
        WSCGetProviderPath,
        ord_101,
        ord_103,
        ord_108,
        ord_51,
    ]
);

poly_win32_rt::fail_exports!("ws2_32", value(0), [FreeAddrInfoExW,]);

poly_win32_rt::fail_exports!("ws2_32", value(10045), [GetAddrInfoExCancel, GetAddrInfoExW,]);
