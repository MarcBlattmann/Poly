//! The names Chrome's `chrome.dll` delay-loads from `user32.dll` that Poly does not implement yet (ADR
//! 0155, step 9): each is an export that logs `stub: user32!name` and fails. A delay load of a name
//! the DLL does not export ends the browser, whatever the call would have been used for; a
//! failing call it copes with (the fault-injection table of the gap map). An implementation
//! replaces a name here by defining it elsewhere in the crate and deleting it from the list.

poly_win32_rt::fail_exports!(
    "user32",
    win32,
    [
        BringWindowToTop,
        CharToOemBuffW,
        ClipCursor,
        CloseDesktop,
        CloseTouchInputHandle,
        CloseWindowStation,
        CreateDesktopW,
        CreatePopupMenu,
        CreateWindowExA,
        CreateWindowStationW,
        DdeClientTransaction,
        DdeConnect,
        DdeCreateStringHandleW,
        DdeDisconnect,
        DdeFreeStringHandle,
        DdeUninitialize,
        DestroyMenu,
        DrawEdge,
        DrawFocusRect,
        DrawFrameControl,
        FlashWindowEx,
        FrameRect,
        GetGuiResources,
        GetMenuInfo,
        GetMenuItemInfoW,
        GetMonitorInfoA,
        GetPointerDevice,
        GetPointerPenInfo,
        GetPointerType,
        GetProcessWindowStation,
        GetUserObjectInformationW,
        InsertMenuItemW,
        InvertRect,
        LoadStringW,
        OemToCharA,
        OemToCharBuffA,
        OpenInputDesktop,
        PrintWindow,
        RegisterHotKey,
        RegisterRawInputDevices,
        SetMenuDefaultItem,
        SetMenuInfo,
        SetMenuItemInfoW,
        SetProcessWindowStation,
        SetThreadDesktop,
        TrackPopupMenu,
        UnregisterHotKey,
        MessageBoxW,
        UnregisterClassA
    ]
);

// Learn: these return -1 (`(UINT)-1`) when they fail.
poly_win32_rt::fail_exports!(
    "user32",
    win32_minus_one,
    [EnableMenuItem, GetMenuItemCount, GetRawInputData, GetRawInputDeviceInfoW, GetRawInputDeviceList]
);

// `DdeInitializeW` returns a `DMLERR_*` value: `DMLERR_SYS_ERROR` (`ddeml.h`).
poly_win32_rt::fail_exports!("user32", value(0x400F), [DdeInitializeW]);

// `DdeGetLastError`: `DMLERR_DLL_NOT_INITIALIZED` (`ddeml.h`), no DDE instance exists.
poly_win32_rt::fail_exports!("user32", value(0x4003), [DdeGetLastError]);

// `DefRawInputProc`: Learn: "If successful, the function returns S_OK. Otherwise it returns an error
// value."
poly_win32_rt::fail_exports!("user32", value(poly_win32_rt::E_NOTIMPL), [DefRawInputProc]);
