//! The names Chrome's `chrome.dll` delay-loads from `gdi32.dll` that Poly does not implement yet (ADR
//! 0155, step 9): each is an export that logs `stub: gdi32!name` and fails. A delay load of a name
//! the DLL does not export ends the browser, whatever the call would have been used for; a
//! failing call it copes with (the fault-injection table of the gap map). An implementation
//! replaces a name here by defining it elsewhere in the crate and deleting it from the list.

poly_win32_rt::fail_exports!(
    "gdi32",
    win32,
    [
        BeginPath,
        CancelDC,
        ChoosePixelFormat,
        CloseEnhMetaFile,
        CloseFigure,
        CreateDCW,
        CreateEnhMetaFileW,
        DeleteEnhMetaFile,
        DescribePixelFormat,
        EndDoc,
        EndPage,
        EndPath,
        EnumEnhMetaFile,
        EnumFontFamiliesExA,
        ExtCreatePen,
        ExtEscape,
        FillPath,
        GdiAlphaBlend,
        GdiComment,
        GetDIBits,
        GetEnhMetaFileBits,
        GetEnhMetaFileHeader,
        GetICMProfileW,
        GetOutlineTextMetricsW,
        GetPixelFormat,
        GetWorldTransform,
        ModifyWorldTransform,
        PlayEnhMetaFile,
        PlayEnhMetaFileRecord,
        PolyBezierTo,
        SelectClipPath,
        SetArcDirection,
        SetBrushOrgEx,
        SetDIBits,
        SetDIBitsToDevice,
        SetEnhMetaFileBits,
        SetGraphicsMode,
        SetMiterLimit,
        SetPixelFormat,
        SetPolyFillMode,
        SetROP2,
        SetStretchBltMode,
        SetWorldTransform,
        StartDocW,
        StartPage,
        StretchBlt,
        StretchDIBits,
        StrokeAndFillPath,
        StrokePath,
        SwapBuffers,
        WidenPath
    ]
);

// Learn: `GDI_ERROR` and `SP_ERROR`, both -1, are the failures.
poly_win32_rt::fail_exports!("gdi32", win32_minus_one, [GetFontData, SetAbortProc]);
