//! The functions the Windows applications of the survey (`windows-app-survey`) import from `comctl32.dll` that Poly does
//! not implement yet (ADR 0155, step 10): each is an export that logs `stub: comctl32!name` and fails as ADR 0155 fixes for a
//! function the system does not provide, so that the programs bind it (and `GetProcAddress` finds it). An
//! implementation replaces a name here by defining it elsewhere in the crate and deleting it from the list.
//! Generated from the programs' import tables (names only) and our export table.

poly_win32_rt::fail_exports!("comctl32", invalid_handle, [PropertySheetW,]);

poly_win32_rt::fail_exports!(
    "comctl32",
    win32,
    [
        CreatePropertySheetPageW,
        FlatSB_GetScrollInfo,
        FlatSB_GetScrollPos,
        FlatSB_SetScrollInfo,
        FlatSB_SetScrollPos,
        FlatSB_SetScrollProp,
        ImageList_BeginDrag,
        ImageList_DragEnter,
        ImageList_DragLeave,
        ImageList_DragMove,
        ImageList_DragShowNolock,
        ImageList_EndDrag,
        ImageList_GetDragImage,
        ImageList_Read,
        ImageList_Write,
        InitializeFlatSB,
        _TrackMouseEvent,
        ord_17,
        ord_345,
        ord_381,
    ]
);
