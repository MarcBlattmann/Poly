//! The `A` forms of the resource loaders and of `IsDialogMessage`: `LoadStringA`, `LoadIconA`,
//! `LoadCursorA`, `LoadImageA`, `LoadBitmapA` (`specs/windows/user32.dll/*A.yaml`). A name given
//! as a string goes through the ANSI code page before the lookup; the results are those of the
//! `W` functions.

use super::H;
use super::ansi::{self, Form};
use alloc::vec::Vec;
use poly_win32k::user::CMsg;

/// A resource name of an `A` function: the number as it is, a string widened.
///
/// # Safety
/// `p` is a number below 0x10000 or a NUL-terminated string.
unsafe fn name_w(p: *const u8) -> (Vec<u16>, *const u16) {
    if (p as usize) < 0x1_0000 {
        return (Vec::new(), p as *const u16);
    }
    // SAFETY: as the caller's.
    let mut w = unsafe { ansi::wide_of_a(p) };
    w.push(0);
    let q = w.as_ptr();
    (w, q)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadIconA(instance: H, name: *const u8) -> usize {
    // SAFETY: as the caller's.
    let (_keep, p) = unsafe { name_w(name) };
    // SAFETY: the name just made.
    unsafe { super::icons::LoadIconW(instance, p) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadCursorA(instance: H, name: *const u8) -> usize {
    // SAFETY: as the caller's.
    let (_keep, p) = unsafe { name_w(name) };
    // SAFETY: the name just made.
    unsafe { super::LoadCursorW(instance, p) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadBitmapA(instance: H, name: *const u8) -> usize {
    // SAFETY: as the caller's.
    let (_keep, p) = unsafe { name_w(name) };
    // SAFETY: the name just made.
    unsafe { super::icons::LoadBitmapW(instance, p) }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadImageA(
    instance: H,
    name: *const u8,
    kind: u32,
    cx: i32,
    cy: i32,
    flags: u32,
) -> usize {
    // SAFETY: as the caller's.
    let (_keep, p) = unsafe { name_w(name) };
    // SAFETY: the name just made (a file name too with LR_LOADFROMFILE).
    unsafe { super::icons::LoadImageW(instance, p, kind, cx, cy, flags) }
}

/// `LoadStringA(hInstance, uID, lpBuffer, cchBufferMax)`: bytes, without the NUL. A size of 0
/// gives -1 and a negative size -2 (`LoadStringA.yaml` `sizes_zero_and_negative`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadStringA(instance: *mut u8, id: u32, buf: *mut u8, cch: i32) -> i32 {
    if cch == 0 {
        return -1;
    }
    if cch < 0 {
        return -2;
    }
    if buf.is_null() {
        super::set_last_error(87);
        return 0;
    }
    let mut at: *const u16 = core::ptr::null();
    // SAFETY: size 0 asks for the address of the string inside the resource.
    let n = unsafe { super::strings::LoadStringW(instance, id, &mut at as *mut *const u16 as *mut u16, 0) };
    if n <= 0 || at.is_null() {
        // SAFETY: the caller's buffer of at least one byte.
        unsafe { *buf = 0 };
        return 0;
    }
    // SAFETY: `n` units at `at`.
    let units = unsafe { core::slice::from_raw_parts(at, n as usize) };
    // SAFETY: the caller's buffer of `cch` bytes.
    unsafe { ansi::put_a(buf, cch as usize, &ansi::ansi_of(units)) as i32 }
}

/// `IsDialogMessageA`: the characters of the message are bytes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn IsDialogMessageA(dlg: H, msg: *mut CMsg) -> i32 {
    if msg.is_null() {
        return 0;
    }
    // SAFETY: the caller's MSG.
    let mut m = unsafe { *msg };
    if matches!(m.message, ansi::WM_CHAR | ansi::WM_DEADCHAR | ansi::WM_SYSCHAR | ansi::WM_SYSDEADCHAR) {
        m.w = ansi::enter(m.hwnd, Form::Ansi, Form::Wide, m.message, m.w, m.l as isize).w;
    }
    // SAFETY: a MSG of ours.
    unsafe { super::dlg::IsDialogMessageW(dlg, &mut m) }
}
