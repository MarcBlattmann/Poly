//! The functions the Windows applications of the survey (`windows-app-survey`) import from `iphlpapi.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: iphlpapi!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!("iphlpapi", value(1), [GetCurrentThreadCompartmentId,]);

poly_win32_rt::fail_exports!(
    "iphlpapi",
    value(50),
    [
        AllocateAndGetTcpExTable2FromStack,
        AllocateAndGetUdpExTable2FromStack,
        CancelMibChangeNotify2,
        ConvertInterfaceIndexToLuid,
        ConvertInterfaceLuidToGuid,
        ConvertInterfaceLuidToIndex,
        ConvertInterfaceLuidToNameA,
        ConvertInterfaceLuidToNameW,
        ConvertInterfaceNameToLuidW,
        GetBestRoute2,
        GetExtendedTcpTable,
        GetExtendedUdpTable,
        GetIfEntry2,
        GetIfTable2Ex,
        GetIpAddrTable,
        GetIpInterfaceTable,
        GetIpNetEntry2,
        GetNetworkParams,
        GetUnicastIpAddressTable,
        NotifyIpInterfaceChange,
        ResolveIpNetEntry2,
        SetCurrentThreadCompartmentId,
    ]
);
