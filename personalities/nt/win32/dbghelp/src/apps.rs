//! The functions the Windows applications of the survey (`windows-app-survey`) import from `dbghelp.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: dbghelp!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "dbghelp",
    win32,
    [
        EnumerateLoadedModules64,
        EnumerateLoadedModulesW64,
        FindExecutableImageExW,
        ImageDirectoryEntryToData,
        ImagehlpApiVersionEx,
        SymFromAddrW,
        SymGetModuleInfoW64,
        SymGetSymFromName,
        SymLoadModule64,
        SymLoadModuleExW,
        SymRefreshModuleList,
        SymRegisterCallback64,
        SymUnloadModule64,
        UnDecorateSymbolName,
    ]
);
