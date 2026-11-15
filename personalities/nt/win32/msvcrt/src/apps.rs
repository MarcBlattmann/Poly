//! The functions the Windows applications of the survey (`windows-app-survey`) import from `msvcrt.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: msvcrt!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "msvcrt",
    value(0),
    [
        __fpecode,
        __p___initenv,
        __p___winitenv,
        __p__iob,
        __pioinfo,
        __pxcptinfoptrs,
        _adjust_fdiv,
        _assert,
        _chmod,
        _except_handler3,
        _filbuf,
        _flsbuf,
        _fstati64,
        _getdcwd,
        _getdrive,
        _getmaxstdio,
        _j0,
        _j1,
        _jn,
        _lfind,
        _lock,
        _locking,
        _mkgmtime,
        _mktemp,
        _nextafter,
        _setjmp3,
        _setmaxstdio,
        _stati64,
        _swab,
        _tempnam,
        _unlock,
        _utime64,
        _wchmod,
        _wgetdcwd,
        _wstat,
        _wtempnam,
        _wutime64,
        _y0,
        _y1,
        _yn,
        gets,
        tmpnam,
    ]
);

#[unsafe(export_name = "??1type_info@@UAE@XZ")]
pub extern "system" fn weird_19() -> u64 {
    poly_win32_rt::unimplemented_export_error("stub: msvcrt!??1type_info@@UAE@XZ", 0, 0)
}
