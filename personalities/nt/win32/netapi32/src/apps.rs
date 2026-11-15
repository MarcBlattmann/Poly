//! The functions the Windows applications of the survey (`windows-app-survey`) import from `netapi32.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: netapi32!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "netapi32",
    value(50),
    [
        DavGetHTTPFromUNCPath,
        DsGetDcNameW,
        NetGetDCName,
        NetGroupEnum,
        NetGroupGetInfo,
        NetLocalGroupEnum,
        NetLocalGroupGetInfo,
        NetShareEnum,
        NetUserChangePassword,
        NetUserEnum,
        NetUserGetLocalGroups,
        NetUserModalsGet,
        NetUserModalsSet,
        NetUserSetInfo,
        NetWkstaGetInfo,
    ]
);
