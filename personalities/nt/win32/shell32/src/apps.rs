//! The functions the Windows applications of the survey (`windows-app-survey`) import from `shell32.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: shell32!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "shell32",
    hresult,
    [
        GetCurrentProcessExplicitAppUserModelID,
        SHCreateItemFromIDList,
        SHCreateShellItem,
        SHCreateShellItemArrayFromShellItem,
        SHGetIDListFromObject,
        SHGetKnownFolderIDList,
        SHGetKnownFolderItem,
        SHGetPropertyStoreFromParsingName,
        Shell_NotifyIconGetRect,
        ord_102,
        ord_153,
        ord_16,
        ord_162,
        ord_165,
        ord_17,
        ord_18,
        ord_19,
        ord_195,
        ord_2,
        ord_25,
        ord_4,
        ord_51,
        ord_6,
        ord_644,
        ord_645,
        ord_701,
        ord_716,
        ord_727,
    ]
);

poly_win32_rt::fail_exports!(
    "shell32",
    value(0),
    [
        DragAcceptFiles,
        DragFinish,
        DragQueryPoint,
        ExtractAssociatedIconW,
        ExtractIconExW,
        ExtractIconW,
        SHAddToRecentDocs,
        SHBrowseForFolderA,
        SHBrowseForFolderW,
        SHFreeNameMappings,
        SHGetPathFromIDListA,
        SHGetSettings,
        SHGetSpecialFolderPathA,
    ]
);

poly_win32_rt::fail_exports!("shell32", value(120), [SHCreateDirectoryExW, SHFileOperationW,]);

poly_win32_rt::fail_exports!("shell32", value(31), [FindExecutableW,]);
