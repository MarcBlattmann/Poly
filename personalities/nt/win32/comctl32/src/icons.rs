//! `LoadIconMetric` and `LoadIconWithScaleDown` (Learn, `commctrl.h`): an icon of a module (or a
//! system icon, `instance` NULL) loaded at the small or large system icon size, or at a given
//! size, through user32's `LoadImageW`. Exported by name and by the ordinals Windows' own comctl32.dll
//! exports them by (380 and 381, `build.rs`). Windows apps survey batch W1 (Notepad++ imports
//! `LoadIconWithScaleDown` by ordinal 381); no sheet yet, to be sharpened when one lands.

type H = usize;

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LoadImageW(instance: H, name: *const u16, kind: u32, cx: i32, cy: i32, flags: u32) -> H;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetLastError() -> u32;
}

/// `IMAGE_ICON`, `LR_SHARED` (`winuser.h`).
const IMAGE_ICON: u32 = 1;
const LR_SHARED: u32 = 0x8000;
/// `SM_CXICON`, `SM_CYICON`, `SM_CXSMICON`, `SM_CYSMICON` (`winuser.h`).
const SM_CXICON: i32 = 11;
const SM_CYICON: i32 = 12;
const SM_CXSMICON: i32 = 49;
const SM_CYSMICON: i32 = 50;
/// `LIM_SMALL`, `LIM_LARGE` (`commctrl.h`).
const LIM_SMALL: i32 = 0;
const LIM_LARGE: i32 = 1;
/// `S_OK`, `E_INVALIDARG`, `E_FAIL` (`winerror.h`).
const S_OK: i32 = 0;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_FAIL: i32 = 0x8000_4005_u32 as i32;

/// `HRESULT_FROM_WIN32` (`winerror.h`) of the thread's last error, `E_FAIL` when there is none.
fn last_error_hresult() -> i32 {
    // SAFETY: a kernel32 call without arguments.
    match unsafe { GetLastError() } {
        0 => E_FAIL,
        e => ((e & 0xFFFF) | 0x8007_0000) as i32,
    }
}

/// The icon `name` of `instance` at `cx` by `cy` into `*out`. A system icon (`instance` NULL) is
/// one of the shared ones.
unsafe fn load(instance: H, name: *const u16, cx: i32, cy: i32, out: *mut H) -> i32 {
    if out.is_null() || cx <= 0 || cy <= 0 {
        return E_INVALIDARG;
    }
    let flags = if instance == 0 { LR_SHARED } else { 0 };
    // SAFETY: the caller's resource name (a string or an integer resource id).
    let icon = unsafe { LoadImageW(instance, name, IMAGE_ICON, cx, cy, flags) };
    // SAFETY: the caller's HICON out-pointer, checked above.
    unsafe { *out = icon };
    if icon == 0 { last_error_hresult() } else { S_OK }
}

/// `LoadIconMetric`: the icon at `SM_CXSMICON` by `SM_CYSMICON` (`LIM_SMALL`) or `SM_CXICON` by
/// `SM_CYICON` (`LIM_LARGE`); any other `lims` or a NULL `phico` is `E_INVALIDARG`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadIconMetric(instance: H, name: *const u16, lims: i32, phico: *mut H) -> i32 {
    let (x, y) = match lims {
        LIM_SMALL => (SM_CXSMICON, SM_CYSMICON),
        LIM_LARGE => (SM_CXICON, SM_CYICON),
        _ => return E_INVALIDARG,
    };
    // SAFETY: user32 calls without pointers.
    let (cx, cy) = unsafe { (GetSystemMetrics(x), GetSystemMetrics(y)) };
    // SAFETY: the caller's arguments, passed on.
    unsafe { load(instance, name, cx, cy, phico) }
}

/// `LoadIconWithScaleDown`: the icon at `cx` by `cy`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadIconWithScaleDown(
    instance: H,
    name: *const u16,
    cx: i32,
    cy: i32,
    phico: *mut H,
) -> i32 {
    // SAFETY: the caller's arguments, passed on.
    unsafe { load(instance, name, cx, cy, phico) }
}
