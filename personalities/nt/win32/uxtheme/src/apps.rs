//! The functions the Windows applications of the survey (`windows-app-survey`) import from `uxtheme.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: uxtheme!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!(
    "uxtheme",
    hresult,
    [
        BufferedPaintRenderAnimation,
        BufferedPaintSetAlpha,
        BufferedPaintStopAllAnimations,
        EndBufferedAnimation,
        EndPanningFeedback,
        SetWindowThemeAttribute,
        UpdatePanningFeedback,
    ]
);

poly_win32_rt::fail_exports!(
    "uxtheme",
    value(0),
    [BeginBufferedAnimation, BeginPanningFeedback, OpenThemeDataForDpi, ord_133, ord_135,]
);

poly_win32_rt::fail_exports!("uxtheme", value(0x80070006), [DrawThemeTextEx, GetThemeTransitionDuration,]);
