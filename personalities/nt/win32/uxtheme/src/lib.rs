//! Our `uxtheme.dll` (ADR 0155, step 9): visual styles as a system has them on which none is
//! active. Windows programs on Poly draw their controls unthemed (the classic look), so no theme
//! data can be opened: `OpenThemeData` returns NULL, which Learn gives as the answer when "visual
//! styles are disabled" and tells applications to take as "use unthemed drawing"; every function
//! that takes a theme handle then has no valid handle to be given and answers `E_HANDLE`.
//!
//! Chrome's `chrome.dll` delay-loads six of these functions (`DrawThemeBackgroundEx` by ordinal
//! 47); without the DLL the delay load fails and the browser ends itself.
//!
//! Learn (`uxtheme.h`): "OpenThemeData", "CloseThemeData", "DrawThemeBackground",
//! "DrawThemeBackgroundEx", "GetThemeBackgroundContentRect", "GetThemePartSize", "IsThemeActive",
//! "IsAppThemed". Not observed on Windows.

#![no_std]
// rustc exports every `#[no_mangle]` function by name; `build.rs` exports one again with its
// ordinal, which lld-link keeps (the first specification), noting that the name came twice.
#![allow(linker_messages)]

use poly_win32_rt as _;

/// `E_HANDLE` (`winerror.h`): "Handle that is not valid".
const E_HANDLE: u32 = 0x8007_0006;

/// `OpenThemeData(hwnd, pszClassList)`: no visual style is active: NULL.
#[unsafe(no_mangle)]
pub extern "system" fn OpenThemeData(_hwnd: usize, _class_list: *const u16) -> usize {
    0
}

/// `CloseThemeData(hTheme)`: no handle was ever given out.
#[unsafe(no_mangle)]
pub extern "system" fn CloseThemeData(_theme: usize) -> u32 {
    E_HANDLE
}

/// `DrawThemeBackground(hTheme, hdc, iPartId, iStateId, pRect, pClipRect)`.
#[unsafe(no_mangle)]
pub extern "system" fn DrawThemeBackground(
    _theme: usize,
    _dc: usize,
    _part: i32,
    _state: i32,
    _rect: *const i32,
    _clip: *const i32,
) -> u32 {
    E_HANDLE
}

/// `DrawThemeBackgroundEx(hTheme, hdc, iPartId, iStateId, pRect, pOptions)` (ordinal 47).
#[unsafe(no_mangle)]
pub extern "system" fn DrawThemeBackgroundEx(
    _theme: usize,
    _dc: usize,
    _part: i32,
    _state: i32,
    _rect: *const i32,
    _options: *const u8,
) -> u32 {
    E_HANDLE
}

/// `GetThemeBackgroundContentRect(hTheme, hdc, iPartId, iStateId, pBoundingRect, pContentRect)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeBackgroundContentRect(
    _theme: usize,
    _dc: usize,
    _part: i32,
    _state: i32,
    _bounding: *const i32,
    _content: *mut i32,
) -> u32 {
    E_HANDLE
}

/// `GetThemePartSize(hTheme, hdc, iPartId, iStateId, prc, eSize, psz)`.
#[unsafe(no_mangle)]
pub extern "system" fn GetThemePartSize(
    _theme: usize,
    _dc: usize,
    _part: i32,
    _state: i32,
    _rect: *const i32,
    _size_kind: i32,
    _size: *mut i32,
) -> u32 {
    E_HANDLE
}

/// `IsThemeActive()`: FALSE, "a visual style is not enabled".
#[unsafe(no_mangle)]
pub extern "system" fn IsThemeActive() -> i32 {
    0
}

/// `IsAppThemed()`: FALSE, the application is not drawn with visual styles.
#[unsafe(no_mangle)]
pub extern "system" fn IsAppThemed() -> i32 {
    0
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn uxtheme_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
