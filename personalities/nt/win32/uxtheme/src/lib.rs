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

/// `BufferedPaintInit()`: "Initialize buffered painting for the current thread" (Learn, `uxtheme.h`):
/// S_OK. Poly paints unthemed and unbuffered (a buffered paint cannot be made, below), but
/// the VCL and others call this and un-init as part of their start-up and shut-down.
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintInit() -> u32 {
    0
}

/// `BufferedPaintUnInit()`: S_OK.
#[unsafe(no_mangle)]
pub extern "system" fn BufferedPaintUnInit() -> u32 {
    0
}

/// `BeginBufferedPaint(hdcTarget, prcTarget, dwFormat, pPaintParams, phdc)`: NULL, with `*phdc` NULL
/// too: Learn gives "NULL if ... failed" as the way to say no buffer could be made, and a caller
/// then paints straight to the target.
#[unsafe(no_mangle)]
pub extern "system" fn BeginBufferedPaint(
    _target: usize,
    _rect: *const i32,
    _format: u32,
    _params: *const u8,
    dc: *mut usize,
) -> usize {
    if !dc.is_null() {
        // SAFETY: the caller's out variable.
        unsafe { dc.write(0) };
    }
    0
}

/// `EndBufferedPaint(hBufferedPaint, fUpdateTarget)`: `E_HANDLE`, as no buffer was ever given out.
#[unsafe(no_mangle)]
pub extern "system" fn EndBufferedPaint(_buffer: usize, _update: i32) -> u32 {
    E_HANDLE
}

// ----- the rest of the theme API: no theme is open, so none of it has anything to say -------------

/// The functions that read or draw with a theme handle, as `uxtheme.h` declares them: each takes the
/// handle `OpenThemeData` never gave, so each answers `E_HANDLE`, with the argument count of its
/// prototype (`stdcall` pops it: a program that asks `GetProcAddress` for one calls it as it would
/// the real one). `bool_fn` ones return `FALSE`, `null_fn` ones NULL.
macro_rules! hresult_fn {
    ($($name:ident($($arg:ident),*);)*) => {
        $(
            #[doc = concat!("`", stringify!($name), "`: `E_HANDLE`, there is no theme.")]
            #[unsafe(no_mangle)]
            pub extern "system" fn $name($(_: $arg),*) -> u32 {
                E_HANDLE
            }
        )*
    };
}

macro_rules! false_fn {
    ($($name:ident($($arg:ident),*);)*) => {
        $(
            #[doc = concat!("`", stringify!($name), "`: FALSE.")]
            #[unsafe(no_mangle)]
            pub extern "system" fn $name($(_: $arg),*) -> i32 {
                0
            }
        )*
    };
}

type P = usize;
type I = i32;
type U = u32;

hresult_fn! {
    DrawThemeText(P, P, I, I, P, I, U, U, P);
    GetThemeBackgroundExtent(P, P, I, I, P, P);
    GetThemeTextExtent(P, P, I, I, P, I, U, P, P);
    GetThemeTextMetrics(P, P, I, I, P);
    GetThemeBackgroundRegion(P, P, I, I, P, P);
    HitTestThemeBackground(P, P, I, I, U, P, P, I, I, P);
    DrawThemeEdge(P, P, I, I, P, U, U, P);
    DrawThemeIcon(P, P, I, I, P, P, I);
    GetThemeColor(P, I, I, I, P);
    GetThemeMetric(P, P, I, I, I, P);
    GetThemeString(P, I, I, I, P, I);
    GetThemeBool(P, I, I, I, P);
    GetThemeInt(P, I, I, I, P);
    GetThemeEnumValue(P, I, I, I, P);
    GetThemePosition(P, I, I, I, P);
    GetThemeFont(P, P, I, I, I, P);
    GetThemeRect(P, I, I, I, P);
    GetThemeMargins(P, P, I, I, I, P, P);
    GetThemeIntList(P, I, I, I, P);
    GetThemePropertyOrigin(P, I, I, I, P);
    GetThemeFilename(P, I, I, I, P, I);
    GetThemeSysFont(P, I, P);
    GetThemeSysString(P, I, P, I);
    GetThemeSysInt(P, I, P);
    GetCurrentThemeName(P, I, P, I, P, I);
    GetThemeDocumentationProperty(P, P, P, I);
    DrawThemeParentBackground(P, P, P);
}

false_fn! {
    IsThemePartDefined(P, I, I);
    IsThemeBackgroundPartiallyTransparent(P, I, I);
    GetThemeSysBool(P, I);
    IsThemeDialogTextureEnabled(P);
}

/// `SetWindowTheme(hwnd, pszSubAppName, pszSubIdList)`: S_OK, as a call that changes nothing visible.
#[unsafe(no_mangle)]
pub extern "system" fn SetWindowTheme(_hwnd: P, _app: P, _ids: P) -> u32 {
    0
}

/// `EnableThemeDialogTexture(hwnd, dwFlags)` and `EnableTheming(fEnable)`: S_OK.
#[unsafe(no_mangle)]
pub extern "system" fn EnableThemeDialogTexture(_hwnd: P, _flags: U) -> u32 {
    0
}

#[unsafe(no_mangle)]
pub extern "system" fn EnableTheming(_enable: I) -> u32 {
    0
}

/// `GetWindowTheme(hwnd)`: NULL, no window has a theme.
#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTheme(_hwnd: P) -> P {
    0
}

/// `GetThemeAppProperties()`: no `STAP_*` flag, the application takes no part in themes.
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeAppProperties() -> U {
    0
}

/// `SetThemeAppProperties(dwFlags)`: nothing to set.
#[unsafe(no_mangle)]
pub extern "system" fn SetThemeAppProperties(_flags: U) {}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetSysColor(index: i32) -> u32;
    fn GetSysColorBrush(index: i32) -> usize;
    fn GetSystemMetrics(index: i32) -> i32;
}

/// `GetThemeSysColor(hTheme, iColorId)`: with no theme, the system colour (Learn: a NULL theme gives the
/// colour of `GetSysColor`).
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeSysColor(_theme: P, color: I) -> U {
    // SAFETY: no preconditions.
    unsafe { GetSysColor(color) }
}

/// `GetThemeSysColorBrush(hTheme, iColorId)`: the system colour's brush.
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeSysColorBrush(_theme: P, color: I) -> P {
    // SAFETY: no preconditions.
    unsafe { GetSysColorBrush(color) }
}

/// `GetThemeSysSize(hTheme, iSizeId)`: the system metric of that index (`TMT_*` sizes are `SM_*` indices).
#[unsafe(no_mangle)]
pub extern "system" fn GetThemeSysSize(_theme: P, size: I) -> I {
    // SAFETY: no preconditions.
    unsafe { GetSystemMetrics(size) }
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn uxtheme_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod apps;
