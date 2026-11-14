//! Our `oleacc.dll` (ADR 0155, step 9): the entry points of Microsoft Active Accessibility that
//! Chrome's browser delay-loads. Poly has no accessibility support for Windows programs yet (no
//! system-provided accessible objects, no clients), so each function fails the way its Learn
//! page says a failure looks: an error `HRESULT` with the out-pointer cleared. The DLL has to
//! exist: a delay load that finds no DLL ends the browser.
//!
//! Learn (`oleacc.h`): "CreateStdAccessibleObject", "LresultFromObject",
//! "AccessibleObjectFromWindow", "AccessibleChildren", "WindowFromAccessibleObject". Not
//! observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_rt as _;

/// `E_NOTIMPL`, `E_INVALIDARG` (`winerror.h`).
const E_NOTIMPL: u32 = 0x8000_4001;
const E_INVALIDARG: u32 = 0x8007_0057;

/// Clear an out-pointer and report that there is nothing to give.
///
/// # Safety
/// `out` is null or points to a pointer the caller owns.
unsafe fn none(out: *mut *mut u8) -> u32 {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(core::ptr::null_mut()) };
    E_NOTIMPL
}

/// `CreateStdAccessibleObject(hwnd, idObject, riid, ppvObject)`: the system's accessible object
/// for a window element. Poly has none.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateStdAccessibleObject(
    _hwnd: usize,
    _object: i32,
    _riid: *const u8,
    out: *mut *mut u8,
) -> u32 {
    // SAFETY: the caller's out-pointer.
    unsafe { none(out) }
}

/// `AccessibleObjectFromWindow(hwnd, dwId, riid, ppvObject)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AccessibleObjectFromWindow(
    _hwnd: usize,
    _id: u32,
    _riid: *const u8,
    out: *mut *mut u8,
) -> u32 {
    // SAFETY: the caller's out-pointer.
    unsafe { none(out) }
}

/// `LresultFromObject(riid, wParam, punk)`: the value a window procedure returns for
/// `WM_GETOBJECT`. Learn: "If not successful, returns one of the following standard COM error
/// codes" (as the `LRESULT`); no client can take a reference on Poly.
#[unsafe(no_mangle)]
pub extern "system" fn LresultFromObject(_riid: *const u8, _wparam: usize, _object: *mut u8) -> isize {
    E_NOTIMPL as i32 as isize
}

/// `AccessibleChildren(paccContainer, iChildStart, cChildren, rgvarChildren, pcObtained)`: no
/// child is obtained.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AccessibleChildren(
    _container: *mut u8,
    _start: i32,
    _count: i32,
    _children: *mut u8,
    obtained: *mut i32,
) -> u32 {
    if obtained.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { obtained.write_unaligned(0) };
    E_NOTIMPL
}

/// `WindowFromAccessibleObject(pacc, phwnd)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WindowFromAccessibleObject(_object: *mut u8, hwnd: *mut usize) -> u32 {
    if hwnd.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { hwnd.write_unaligned(0) };
    E_NOTIMPL
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn oleacc_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
