//! Our `comdlg32.dll`: the common dialogs of `commdlg.h` (Learn, "Common Dialog Box
//! Library"). Each dialog is an in-memory template (`poly_comdlg::template`) run by user32's
//! `DialogBoxIndirectParamW`, drawn by the dialog manager and the controls in Poly's
//! desktop look; the rules (which structures are valid, filters, names, colours, fonts) are
//! `poly_comdlg`'s, host-tested. The error of the last call is kept per thread
//! (`CommDlgExtendedError`).
//!
//! Observed on the oracle (`specs/windows/comdlg32.dll`): a cancelled dialog sets the error to
//! 0; every failing check names its `CDERR_*` code; the hook of an Explorer-style file dialog
//! gets a child dialog that is made inside the dialog (`file::run`).
//!
//! The Common Item Dialog's Save object (`CLSID_FileSaveDialog`, ADR 0223) is [`itemdlg`]: its
//! `Show` runs the same file dialog.

#![no_std]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

mod ansi;
mod colordlg;
mod file;
mod fontdlg;
mod itemdlg;
mod sys;
mod util;

use alloc::vec::Vec;
use poly_win32_gui::SpinLock;
use poly_win32_rt as _;

/// The last common-dialog error of each thread that called one.
static ERRORS: SpinLock<Vec<(u32, u32)>> = SpinLock::new(Vec::new());

/// Record `code` (0: success or Cancel) as the calling thread's extended error.
pub(crate) fn set_ext_error(code: u32) {
    // SAFETY: no preconditions.
    let tid = unsafe { sys::GetCurrentThreadId() };
    ERRORS.with(|v| match v.iter_mut().find(|(t, _)| *t == tid) {
        Some(e) => e.1 = code,
        None => v.push((tid, code)),
    });
}

/// `CommDlgExtendedError()`: the code of the thread's last common dialog call, 0 before any.
#[unsafe(no_mangle)]
pub extern "system" fn CommDlgExtendedError() -> u32 {
    // SAFETY: no preconditions.
    let tid = unsafe { sys::GetCurrentThreadId() };
    ERRORS.with(|v| v.iter().find(|(t, _)| *t == tid).map_or(0, |e| e.1))
}

/// `GetOpenFileNameW(lpofn)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetOpenFileNameW(ofn: *mut u8) -> i32 {
    file::run(ofn, false)
}

/// `GetSaveFileNameW(lpofn)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetSaveFileNameW(ofn: *mut u8) -> i32 {
    file::run(ofn, true)
}

/// `ChooseColorW(lpcc)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ChooseColorW(cc: *mut u8) -> i32 {
    colordlg::run(cc)
}

/// `ChooseFontW(lpcf)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ChooseFontW(cf: *mut u8) -> i32 {
    fontdlg::run(cf)
}

/// `GetFileTitleW(file, title, size)`: the name part of a path; 0 when it fits, else the
/// size needed (with the NUL), -1 for an invalid name (empty, or ending in a separator or colon).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetFileTitleW(file: *const u16, title: *mut u16, size: u16) -> i16 {
    let path = util::read_wstr(file, 32768);
    let last = path.last().copied();
    if path.is_empty() || matches!(last, Some(0x5C) | Some(0x2F) | Some(0x3A)) || poly_comdlg::path::invalid_name(&path)
    {
        return -1;
    }
    let name = poly_comdlg::path::file_title(&path);
    if name.len() + 1 > usize::from(size) {
        return (name.len() + 1) as i16;
    }
    util::write_wstr(title, usize::from(size), name);
    0
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn comdlg32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

// The ANSI entries (`ansi.rs`) are exported from there.
