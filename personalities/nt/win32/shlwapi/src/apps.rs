//! The functions the Windows applications of the survey (`windows-app-survey`) import from `shlwapi.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: shlwapi!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "shlwapi",
    hresult,
    [SHAutoComplete, SHCreateStreamOnFileA, SHCreateStreamOnFileEx, SHCreateStreamOnFileW, SHStrDupW,]
);

poly_win32_rt::fail_exports!(
    "shlwapi",
    value(120),
    [SHDeleteEmptyKeyW, SHGetValueA, SHGetValueW, SHRegGetUSValueW, SHSetValueW,]
);

poly_win32_rt::fail_exports!(
    "shlwapi",
    win32,
    [
        ColorAdjustLuma,
        ColorHLSToRGB,
        ColorRGBToHLS,
        StrCSpnA,
        StrCSpnW,
        StrChrA,
        StrChrW,
        StrCmpIW,
        StrCmpLogicalW,
        StrCpyNW,
        StrFormatByteSizeW,
        StrIsIntlEqualA,
        StrIsIntlEqualW,
        StrRChrA,
        StrRChrW,
        StrRStrIW,
        StrSpnA,
        StrSpnW,
        StrStrIW,
        StrStrW,
        StrTrimA,
        StrTrimW,
        UrlEscapeW,
        UrlIsW,
        UrlUnescapeW,
        ord_176,
        ord_2,
        ord_213,
    ]
);

// The real versions are written by the implementer track `impl-win-apps` (branch `impl-win-apps`, spec-win-apps-1): when it
// merges, delete these names (and the group when it is empty).
poly_win32_rt::fail_exports!(
    "shlwapi",
    win32,
    [
        PathAddBackslashA,
        PathCommonPrefixW,
        PathCompactPathExW,
        PathCompactPathW,
        PathCreateFromUrlW,
        PathFileExistsA,
        PathFileExistsW,
        PathFindExtensionA,
        PathFindFileNameA,
        PathGetCharTypeW,
        PathIsContentTypeW,
        PathIsDirectoryW,
        PathIsRelativeA,
        PathMatchSpecA,
        PathQuoteSpacesW,
        PathRemoveArgsW,
        PathRemoveFileSpecA,
        PathRenameExtensionW,
        PathUnExpandEnvStringsW,
        PathUnquoteSpacesW,
    ]
);
