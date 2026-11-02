//! What Poly's GUI DLLs export, for `cargo xtask win32` to check (ADR 0128).
//!
//! - `win32u.dll` exports one stub per service of
//!   [`crate::services::WIN32K_SERVICES`], under the service's name.
//! - `user32.dll` and `gdi32.dll` export the functions below. Until the
//!   spec sheets of their slice arrive, they are **stubs**: each sets the
//!   last error to `ERROR_CALL_NOT_IMPLEMENTED` and returns the failure
//!   value its Learn page documents (`grep -rn not_implemented` finds
//!   them). A function moves out of these lists into an implemented list
//!   when its slice delivers it.
//!
//! The names are the first slice's (window classes and windows, the message
//! loop, timers, painting with GDI), from the coverage map's export lists.

/// `ERROR_CALL_NOT_IMPLEMENTED` (`winerror.h`): what every stub sets.
pub const ERROR_CALL_NOT_IMPLEMENTED: u32 = 120;

/// `user32.dll` functions that are stubs for now.
pub const USER32_STUBS: &[&str] = &[
    "RegisterClassExW",
    "RegisterClassW",
    "UnregisterClassW",
    "GetClassInfoExW",
    "CreateWindowExW",
    "DestroyWindow",
    "ShowWindow",
    "UpdateWindow",
    "SetWindowPos",
    "MoveWindow",
    "GetWindowRect",
    "GetClientRect",
    "GetWindowLongPtrW",
    "SetWindowLongPtrW",
    "SetWindowTextW",
    "GetWindowTextW",
    "DefWindowProcW",
    "GetMessageW",
    "PeekMessageW",
    "TranslateMessage",
    "DispatchMessageW",
    "PostMessageW",
    "PostThreadMessageW",
    "SendMessageW",
    "PostQuitMessage",
    "WaitMessage",
    "MsgWaitForMultipleObjectsEx",
    "SetTimer",
    "KillTimer",
    "BeginPaint",
    "EndPaint",
    "InvalidateRect",
    "ValidateRect",
    "GetUpdateRect",
    "GetDC",
    "ReleaseDC",
    "FillRect",
    "LoadCursorW",
    "LoadIconW",
];

/// `gdi32.dll` functions that are stubs for now.
pub const GDI32_STUBS: &[&str] = &[
    "GetStockObject",
    "CreateCompatibleDC",
    "DeleteDC",
    "SelectObject",
    "DeleteObject",
    "GetObjectW",
    "GetObjectType",
    "CreateSolidBrush",
    "CreatePen",
    "CreateCompatibleBitmap",
    "CreateDIBSection",
    "CreateRectRgn",
    "SelectClipRgn",
    "BitBlt",
    "PatBlt",
    "Rectangle",
    "MoveToEx",
    "LineTo",
    "SetPixel",
    "GetPixel",
    "SetBkColor",
    "SetTextColor",
    "SetBkMode",
    "CreateFontIndirectW",
    "TextOutW",
    "ExtTextOutW",
    "GetTextExtentPoint32W",
    "GetDeviceCaps",
    "GdiFlush",
];

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn names_are_unique_identifiers() {
        for list in [USER32_STUBS, GDI32_STUBS] {
            let set: HashSet<_> = list.iter().collect();
            assert_eq!(set.len(), list.len());
            assert!(list.iter().all(|n| n.chars().all(|c| c.is_ascii_alphanumeric())));
        }
        // Stubs never shadow Poly's own service names.
        assert!(USER32_STUBS.iter().chain(GDI32_STUBS).all(|n| !n.starts_with("Poly")));
    }
}
