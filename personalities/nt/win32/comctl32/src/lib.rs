//! Our `comctl32.dll` (ADR 0155, step 9): `InitCommonControlsEx`, which Chrome's browser
//! delay-loads and calls once when it starts (a failure is fatal for it); `DllGetVersion`
//! and the image lists (`ImageList_*`, [`imagelist`]). The common control window classes
//! themselves are not implemented yet: a program that creates one gets no window. Learn,
//! "InitCommonControlsEx function (commctrl.h)", "Image Lists"; not observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

mod imagelist;

use poly_win32_gui as _;
use poly_win32_rt as _;

/// `InitCommonControlsEx(picce)`: TRUE for an `INITCOMMONCONTROLSEX` of the right size.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn InitCommonControlsEx(init: *const u32) -> i32 {
    if init.is_null() {
        return 0;
    }
    // SAFETY: the caller's INITCOMMONCONTROLSEX; `dwSize` is its first member.
    let size = unsafe { init.read_unaligned() };
    if size != poly_kernelbase::session::INITCOMMONCONTROLSEX_SIZE {
        // A structure of another size has no `dwICC` to trust (it may be shorter than 8 bytes).
        return 0;
    }
    // SAFETY: `dwSize` says the structure is 8 bytes: `dwICC` follows (`commctrl.h`).
    let icc = unsafe { init.add(1).read_unaligned() };
    i32::from(poly_kernelbase::session::init_common_controls(size, icc))
}

/// `InitCommonControls()`: "Registers and initializes certain common control window classes"
/// (Learn); it returns nothing. Also export number 17 (`build.rs`): programs built with the
/// SDK's import library import it by that ordinal (7-Zip File Manager does).
#[unsafe(no_mangle)]
pub extern "system" fn InitCommonControls() {}

/// `DllGetVersion(pdvi)` (`shlwapi.h`): the version of this DLL. Programs without a
/// manifest get comctl32 5.82 (the oracle's tests run that way), so that is what is
/// reported; the structure is the 20-byte `DLLVERSIONINFO` or the 32-byte
/// `DLLVERSIONINFO2` (which adds `dwFlags` and the `ullVersion` of the same numbers).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetVersion(info: *mut u32) -> i32 {
    const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
    if info.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's DLLVERSIONINFO; `cbSize` is its first member.
    let size = unsafe { info.read_unaligned() };
    if size != 20 && size != 32 {
        return E_INVALIDARG;
    }
    const MAJOR: u32 = 5;
    const MINOR: u32 = 82;
    const BUILD: u32 = 26100;
    // SAFETY: `cbSize` says the structure has these members.
    unsafe {
        info.add(1).write_unaligned(MAJOR);
        info.add(2).write_unaligned(MINOR);
        info.add(3).write_unaligned(BUILD);
        // DLLVER_PLATFORM_NT
        info.add(4).write_unaligned(2);
        if size == 32 {
            info.add(5).write_unaligned(0);
            let ull = u64::from(MAJOR) << 48 | u64::from(MINOR) << 32 | u64::from(BUILD) << 16;
            info.add(6).write_unaligned(ull as u32);
            info.add(7).write_unaligned((ull >> 32) as u32);
        }
    }
    0
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn comctl32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

#[cfg(target_arch = "x86_64")]
mod apps;
