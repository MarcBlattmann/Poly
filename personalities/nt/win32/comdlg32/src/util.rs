//! Small helpers the dialogs share: structure fields by offset, strings
//! from pointers, the hook call and running a modal dialog.

#![allow(dead_code)]

use crate::sys::{self, H, WndProc};
use alloc::vec::Vec;
use poly_comdlg::cderr;

/// Window messages and notification codes used here (`winuser.h`, `commdlg.h`).
pub const WM_INITDIALOG: u32 = 0x0110;
pub const WM_COMMAND: u32 = 0x0111;
pub const WM_NOTIFY: u32 = 0x004E;
pub const WM_SETFONT: u32 = 0x0030;
pub const WM_PAINT: u32 = 0x000F;
pub const WM_ERASEBKGND: u32 = 0x0014;
pub const WM_LBUTTONDOWN: u32 = 0x0201;
pub const WM_GETDLGCODE: u32 = 0x0087;
pub const IDOK: i32 = 1;
pub const IDCANCEL: i32 = 2;
pub const GWLP_USERDATA: i32 = -21;
pub const BM_SETCHECK: u32 = 0x00F1;
pub const BST_CHECKED: u32 = 1;
pub const EN_CHANGE: u32 = 0x0300;
pub const EM_SETSEL: u32 = 0x00B1;
pub const LB_ADDSTRING: u32 = 0x0180;
pub const LB_RESETCONTENT: u32 = 0x0184;
pub const LB_SETSEL: u32 = 0x0185;
pub const LB_SETCURSEL: u32 = 0x0186;
pub const LB_GETSEL: u32 = 0x0187;
pub const LB_GETCURSEL: u32 = 0x0188;
pub const LB_GETTEXT: u32 = 0x0189;
pub const LB_GETTEXTLEN: u32 = 0x018A;
pub const LB_GETCOUNT: u32 = 0x018B;
pub const LB_GETSELCOUNT: u32 = 0x0190;
pub const LB_GETSELITEMS: u32 = 0x0191;
pub const LB_SETITEMDATA: u32 = 0x019A;
pub const LB_GETITEMDATA: u32 = 0x0199;
pub const LB_ERR: isize = -1;
pub const LBN_SELCHANGE: u32 = 1;
pub const LBN_DBLCLK: u32 = 2;
pub const BN_CLICKED: u32 = 0;
pub const MB_YESNO: u32 = 4;
pub const MB_ICONWARNING: u32 = 0x30;
pub const MB_ICONQUESTION: u32 = 0x20;
pub const MB_ICONERROR: u32 = 0x10;
pub const IDYES: i32 = 6;
pub const RT_DIALOG: usize = 5;

/// `HOOKPROC` as the common dialogs use it: a dialog-procedure-like function.
pub type HookProc = unsafe extern "system" fn(H, u32, usize, isize) -> usize;

/// Read a `T` at byte `off` of a structure the caller passed.
pub fn rd<T: Copy>(base: *const u8, off: usize) -> T {
    // SAFETY: the caller's structure of the size its `lStructSize` says covers `off`.
    unsafe { core::ptr::read_unaligned(base.add(off) as *const T) }
}

/// Write a `T` at byte `off` of a structure the caller passed.
pub fn wr<T: Copy>(base: *mut u8, off: usize, v: T) {
    // SAFETY: as for `rd`.
    unsafe { core::ptr::write_unaligned(base.add(off) as *mut T, v) };
}

/// The NUL-terminated string at `p` (empty for NULL), at most `max` units.
pub fn read_wstr(p: *const u16, max: usize) -> Vec<u16> {
    let mut v = Vec::new();
    if p.is_null() {
        return v;
    }
    for i in 0..max {
        // SAFETY: a NUL-terminated string of the caller's.
        let c = unsafe { *p.add(i) };
        if c == 0 {
            break;
        }
        v.push(c);
    }
    v
}

/// The units of a double-NUL-terminated list at `p`, without the last NUL.
pub fn read_double_nul(p: *const u16) -> Vec<u16> {
    let mut v = Vec::new();
    if p.is_null() {
        return v;
    }
    let mut i = 0usize;
    loop {
        // SAFETY: a list the caller ended with two NULs.
        let c = unsafe { *p.add(i) };
        if c == 0 && i > 0 && v.last() == Some(&0) {
            break;
        }
        if c == 0 && i == 0 {
            break;
        }
        v.push(c);
        i += 1;
        if i > 1 << 16 {
            break;
        }
    }
    v
}

/// Copy `s` to `buf` of `cap` units with a NUL, cut short when it does not fit.
pub fn write_wstr(buf: *mut u16, cap: usize, s: &[u16]) {
    if buf.is_null() || cap == 0 {
        return;
    }
    let n = s.len().min(cap - 1);
    // SAFETY: the caller's buffer of `cap` units.
    unsafe {
        core::ptr::copy_nonoverlapping(s.as_ptr(), buf, n);
        *buf.add(n) = 0;
    }
}

pub fn dll_instance() -> H {
    let name = sys::wz("comdlg32.dll");
    // SAFETY: a NUL-terminated name.
    unsafe { sys::GetModuleHandleW(name.as_ptr()) }
}

/// Run a modal dialog from an in-memory template; the extended error is
/// `CDERR_DIALOGFAILURE` when it could not be made. The result is the value given to `EndDialog`.
pub fn run_modal(template: &[u8], owner: H, proc_: WndProc, param: isize) -> Result<isize, u32> {
    // SAFETY: a template made by `poly_comdlg::template`; the procedure and parameter are ours.
    let r = unsafe { sys::DialogBoxIndirectParamW(dll_instance(), template.as_ptr(), owner, proc_, param) };
    if r == -1 { Err(cderr::DIALOGFAILURE) } else { Ok(r) }
}

/// Run a hook if there is one: its answer, or 0.
pub fn call_hook(hook: usize, hwnd: H, msg: u32, w: usize, l: isize) -> usize {
    if hook == 0 {
        return 0;
    }
    // SAFETY: the app's hook procedure (a `HOOKPROC`).
    let f: HookProc = unsafe { core::mem::transmute(hook) };
    // SAFETY: the documented call.
    unsafe { f(hwnd, msg, w, l) }
}

/// A `LOWORD`/`HIWORD` pair.
pub fn loword(v: usize) -> u32 {
    (v & 0xFFFF) as u32
}

pub fn hiword(v: usize) -> u32 {
    ((v >> 16) & 0xFFFF) as u32
}

/// `FindResourceW` on `module` (0: the executable) for a dialog `name`.
pub fn has_dialog_resource(module: H, name: *const u16) -> bool {
    // SAFETY: a module handle and a name or resource number of the caller's.
    unsafe { sys::FindResourceW(module, name, RT_DIALOG as *const u16) != 0 }
}

/// Put `text` in the item `id` of `hdlg`.
pub fn set_text(hdlg: H, id: i32, text: &[u16]) {
    let mut v = text.to_vec();
    v.push(0);
    // SAFETY: a NUL-terminated string.
    unsafe { sys::SetDlgItemTextW(hdlg, id, v.as_ptr()) };
}

/// The text of the item `id` of `hdlg`.
pub fn get_text(hdlg: H, id: i32) -> Vec<u16> {
    let mut buf = alloc::vec![0u16; 2048];
    // SAFETY: a buffer of 2048 units.
    let n = unsafe { sys::GetDlgItemTextW(hdlg, id, buf.as_mut_ptr(), 2048) } as usize;
    buf.truncate(n.min(2047));
    buf
}

/// Add `text` to list box `id`; the item's index.
pub fn lb_add(hdlg: H, id: i32, text: &[u16], data: isize) -> isize {
    let mut v = text.to_vec();
    v.push(0);
    // SAFETY: a NUL-terminated string for LB_ADDSTRING.
    let i = unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_ADDSTRING, 0, v.as_ptr() as isize) };
    if i >= 0 {
        // SAFETY: plain messages.
        unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_SETITEMDATA, i as usize, data) };
    }
    i
}

pub fn lb_reset(hdlg: H, id: i32) {
    // SAFETY: a plain message.
    unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_RESETCONTENT, 0, 0) };
}

pub fn lb_sel(hdlg: H, id: i32, index: usize) {
    // SAFETY: a plain message.
    unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_SETCURSEL, index, 0) };
}

pub fn lb_cur(hdlg: H, id: i32) -> Option<usize> {
    // SAFETY: a plain message.
    let r = unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_GETCURSEL, 0, 0) };
    usize::try_from(r).ok()
}

pub fn lb_data(hdlg: H, id: i32, index: usize) -> isize {
    // SAFETY: a plain message.
    unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_GETITEMDATA, index, 0) }
}

pub fn lb_text(hdlg: H, id: i32, index: usize) -> Vec<u16> {
    // SAFETY: a plain message.
    let n = unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_GETTEXTLEN, index, 0) };
    if n <= 0 {
        return Vec::new();
    }
    let mut buf = alloc::vec![0u16; n as usize + 1];
    // SAFETY: LB_GETTEXT copies `n` units and a NUL into the buffer.
    unsafe { sys::SendDlgItemMessageW(hdlg, id, LB_GETTEXT, index, buf.as_mut_ptr() as isize) };
    buf.truncate(n as usize);
    buf
}
