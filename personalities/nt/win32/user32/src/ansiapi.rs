//! The `A` forms of the window and message functions (PuTTY and the programs of the same
//! age call these): `RegisterClassA/ExA`, `CreateWindowExA`, `GetClassNameA`, `SendMessageA`,
//! `PostMessageA`, `DefWindowProcA`, `CallWindowProcA`, the window text functions and the
//! message pump. The conversions are `crate::ansi`'s: strings go through the ANSI code page of
//! the process (1252) and a message with text in it is converted for the window procedure it
//! reaches.

use super::ansi::{self, Form};
use super::{H, SYS, SpinLock, call, info, set_last_error, u};
use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use poly_win32k::proto::{ClassSpec, Reply, Request};
use poly_win32k::user::{CMsg, Env as _};

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;

/// Threads that are about to make a window of an ANSI creation function (`CreateWindowExA`, the
/// controls of an `A` dialog); taken by the next `CreateWindowExW` on the thread.
static ANSI_CREATE: SpinLock<BTreeSet<u32>> = SpinLock::new(BTreeSet::new());

/// Whether the calling thread's next window is to be an ANSI window (and clears the mark).
pub fn ansi_next_window() -> bool {
    let tid = SYS.tid();
    ANSI_CREATE.with(|s| s.remove(&tid))
}

/// Runs `make` with the thread's next window marked as an ANSI window.
pub fn with_ansi_window<R>(make: impl FnOnce() -> R) -> R {
    let tid = SYS.tid();
    ANSI_CREATE.with(|s| s.insert(tid));
    let r = make();
    ANSI_CREATE.with(|s| s.remove(&tid));
    r
}

/// A name or an atom of an `A` function as the `W` function takes it (an atom is below 0x10000).
///
/// # Safety
/// `p` is an atom or a NUL-terminated string.
unsafe fn wide_name(p: *const u8) -> Option<Vec<u16>> {
    if (p as usize) < 0x1_0000 {
        return None;
    }
    // SAFETY: as the caller's.
    let mut w = unsafe { ansi::wide_of_a(p) };
    w.push(0);
    Some(w)
}

/// Threads whose next dialog is an `A` dialog (taken by `create_dialog_window`).
static ANSI_DIALOG: SpinLock<BTreeSet<u32>> = SpinLock::new(BTreeSet::new());

pub fn take_ansi_dialog() -> bool {
    let tid = SYS.tid();
    ANSI_DIALOG.with(|s| s.remove(&tid))
}

/// Runs `make` with the thread's next dialog marked as an `A` dialog.
pub fn with_ansi_dialog<R>(make: impl FnOnce() -> R) -> R {
    let tid = SYS.tid();
    ANSI_DIALOG.with(|s| s.insert(tid));
    let r = make();
    ANSI_DIALOG.with(|s| s.remove(&tid));
    r
}

// ----- classes

/// `RegisterClassExA` and `RegisterClassA` share this: the structure's names are ANSI.
fn register_a(c: &super::cstructs::WndClassEx, with_sm: bool) -> u16 {
    let name = c.class_name as *const u8;
    if (name as usize) < 0x1_0000 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a NUL-terminated name.
    let name = unsafe { ansi::wide_of_a(name) };
    // The class's menu name stays: a copy lives as long as the process.
    let mut menu_name = c.menu_name as u64;
    if c.menu_name > 0xFFFF {
        // SAFETY: a NUL-terminated name.
        let mut w = unsafe { ansi::wide_of_a(c.menu_name as *const u8) };
        w.push(0);
        menu_name = alloc::boxed::Box::leak(w.into_boxed_slice()).as_ptr() as u64;
    }
    super::register_class(ClassSpec {
        name,
        style: c.style,
        wndproc: c.wndproc as u64,
        cls_extra: c.cls_extra,
        wnd_extra: c.wnd_extra,
        instance: c.instance as u64,
        icon: c.icon as u64,
        cursor: c.cursor as u64,
        brush: c.brush as u64,
        menu_name,
        icon_sm: if with_sm { c.icon_sm as u64 } else { 0 },
        unicode: false,
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterClassExA(wc: *const u8) -> u16 {
    if wc.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a WNDCLASSEXA (the layout of the W form).
    unsafe {
        if (wc as *const u32).read_unaligned() as usize != core::mem::size_of::<super::cstructs::WndClassEx>() {
            set_last_error(ERROR_INVALID_PARAMETER);
            return 0;
        }
        register_a(&(wc as *const super::cstructs::WndClassEx).read_unaligned(), true)
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterClassA(wc: *const u8) -> u16 {
    if wc.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a WNDCLASSA (the W layout of 72 bytes on x64, 40 in 32 bits).
    let c = unsafe { (wc as *const super::cstructs::WndClass).read_unaligned() };
    let ex = super::cstructs::WndClassEx {
        size: 0,
        style: c.style,
        wndproc: c.wndproc,
        cls_extra: c.cls_extra,
        wnd_extra: c.wnd_extra,
        instance: c.instance,
        icon: c.icon,
        cursor: c.cursor,
        brush: c.brush,
        menu_name: c.menu_name,
        class_name: c.class_name,
        icon_sm: 0,
    };
    register_a(&ex, false)
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnregisterClassA(name: *const u8, instance: H) -> i32 {
    if name.is_null() {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    // SAFETY: a name or an atom.
    match unsafe { wide_name(name) } {
        // SAFETY: a NUL-terminated wide name.
        Some(w) => unsafe { super::UnregisterClassW(w.as_ptr(), instance) },
        // SAFETY: an atom.
        None => unsafe { super::UnregisterClassW(name as *const u16, instance) },
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetClassNameA(hwnd: H, buf: *mut u8, max: i32) -> i32 {
    match call(Request::ClassName { hwnd: hwnd as u64 }) {
        Reply::Text { text } => {
            if max <= 0 || buf.is_null() {
                return 0;
            }
            // SAFETY: the caller's buffer of `max` bytes.
            unsafe { ansi::put_a(buf, max as usize, &ansi::ansi_of(&text)) as i32 }
        }
        _ => {
            set_last_error(ERROR_INVALID_WINDOW_HANDLE);
            0
        }
    }
}

// ----- windows

#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "system" fn CreateWindowExA(
    ex_style: u32,
    class: *const u8,
    name: *const u8,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: H,
    menu: H,
    instance: H,
    param: H,
) -> H {
    // SAFETY: an atom or a NUL-terminated name.
    let class_w = unsafe { wide_name(class) };
    let class_p = class_w.as_ref().map_or(class as *const u16, |v| v.as_ptr());
    let name_w: Vec<u16> = if name.is_null() {
        Vec::new()
    } else {
        // SAFETY: a NUL-terminated name.
        let mut v = unsafe { ansi::wide_of_a(name) };
        v.push(0);
        v
    };
    let name_p = if name.is_null() { core::ptr::null() } else { name_w.as_ptr() };
    // SAFETY: NUL-terminated wide names (or the atom) and the caller's other arguments.
    with_ansi_window(|| unsafe {
        super::CreateWindowExW(ex_style, class_p, name_p, style, x, y, w, h, parent, menu, instance, param)
    })
}

// ----- messages

/// Sends `msg` of form `from` to window `hwnd`.
pub fn send_form(hwnd: H, msg: u32, w: usize, l: isize, from: Form) -> isize {
    if from == Form::Wide || !ansi::is_text_message(msg) {
        return u::send(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize;
    }
    let Some(i) = info(hwnd) else {
        return u::send(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize;
    };
    if msg == ansi::WM_GETTEXTLENGTH && i.unicode && (i.wndproc > poly_win32k::dlg::sysproc::MAX || i.wndproc == poly_win32k::dlg::sysproc::DIALOG) {
        // The length of a UTF-16 text in bytes: the window procedure of a Unicode window gets the
        // length and then the text, as far as it needs (DefDlgProcW.yaml: WM_GETTEXT with the length
        // and the NUL).
        let n = u::send(&SYS, hwnd as u64, msg, 0, 0) as usize;
        if n == 0 {
            return 0;
        }
        let mut buf = alloc::vec![0u16; n + 1];
        let got = u::send(&SYS, hwnd as u64, ansi::WM_GETTEXT, (n + 1) as u64, buf.as_mut_ptr() as u64) as usize;
        return ansi::ansi_of(&buf[..got.min(n)]).len() as isize;
    }
    if i.tid == SYS.tid() {
        // The same thread: the window procedure is called from here and converts if it must.
        ansi::with_form(from, || u::send(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize)
    } else {
        // Another thread's window: its thread knows nothing of our form; make it UTF-16 here.
        let c = ansi::enter(hwnd, from, Form::Wide, msg, w, l);
        let r = u::send(&SYS, hwnd as u64, msg, c.w as u64, c.l as u64) as isize;
        c.finish(r)
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn SendMessageA(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    send_form(hwnd, msg, w, l, Form::Ansi)
}

#[unsafe(no_mangle)]
pub extern "system" fn PostMessageA(hwnd: H, msg: u32, w: usize, l: isize) -> i32 {
    // Posted text (a pointer that has to outlive the post) is not converted; characters are.
    let w = if matches!(msg, ansi::WM_CHAR | ansi::WM_DEADCHAR | ansi::WM_SYSCHAR | ansi::WM_SYSDEADCHAR) {
        ansi::enter(hwnd, Form::Ansi, Form::Wide, msg, w, l).w
    } else {
        w
    };
    super::PostMessageW(hwnd, msg, w, l)
}

/// `DefWindowProcA`: the default handling of a message that carries ANSI text.
#[unsafe(no_mangle)]
pub extern "system" fn DefWindowProcA(hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    def_form(hwnd, msg, w, l, Form::Ansi)
}

/// The default window procedure for a message of form `from`.
pub fn def_form(hwnd: H, msg: u32, w: usize, l: isize, from: Form) -> isize {
    if from == Form::Wide || !ansi::is_text_message(msg) {
        return u::def_window_proc(&SYS, hwnd as u64, msg, w as u64, l as u64) as isize;
    }
    let c = ansi::enter(hwnd, from, Form::Wide, msg, w, l);
    let r = u::def_window_proc(&SYS, hwnd as u64, msg, c.w as u64, c.l as u64) as isize;
    c.finish(r)
}

/// `CallWindowProcA`: calls `proc_` with an ANSI message.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CallWindowProcA(proc_: H, hwnd: H, msg: u32, w: usize, l: isize) -> isize {
    ansi::with_form(Form::Ansi, || {
        // SAFETY: as `CallWindowProcW`.
        unsafe { super::CallWindowProcW(proc_, hwnd, msg, w, l) }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetWindowTextA(hwnd: H, buf: *mut u8, max: i32) -> i32 {
    if info(hwnd).is_none() {
        if max > 0 && !buf.is_null() {
            // SAFETY: the caller's buffer.
            unsafe { *buf = 0 };
        }
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    if max <= 0 || buf.is_null() {
        return 0;
    }
    send_form(hwnd, ansi::WM_GETTEXT, max as usize, buf as isize, Form::Ansi) as i32
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetWindowTextA(hwnd: H, text: *const u8) -> i32 {
    // Observed (`SetWindowTextW`): a made-up handle returns 1 without an error.
    if info(hwnd).is_some() {
        send_form(hwnd, ansi::WM_SETTEXT, 0, text as isize, Form::Ansi);
    }
    1
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowTextLengthA(hwnd: H) -> i32 {
    if info(hwnd).is_none() {
        set_last_error(ERROR_INVALID_WINDOW_HANDLE);
        return 0;
    }
    send_form(hwnd, ansi::WM_GETTEXTLENGTH, 0, 0, Form::Ansi) as i32
}

// ----- the message pump

/// A message as the `A` functions give it: a character is a byte.
fn narrowed(mut m: CMsg) -> CMsg {
    if matches!(m.message, ansi::WM_CHAR | ansi::WM_DEADCHAR | ansi::WM_SYSCHAR | ansi::WM_SYSDEADCHAR) {
        m.w = usize::from(ansi::ansi_of(&[m.w as u16])[0]);
    }
    m
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetMessageA(msg: *mut CMsg, hwnd: H, min: u32, max: u32) -> i32 {
    // SAFETY: as `GetMessageW`.
    let r = unsafe { super::GetMessageW(msg, hwnd, min, max) };
    if r != -1 && r != 0 && !msg.is_null() {
        // SAFETY: the caller's MSG.
        unsafe { *msg = narrowed(*msg) };
    }
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn PeekMessageA(msg: *mut CMsg, hwnd: H, min: u32, max: u32, remove: u32) -> i32 {
    // SAFETY: as `PeekMessageW`.
    let r = unsafe { super::PeekMessageW(msg, hwnd, min, max, remove) };
    if r != 0 && !msg.is_null() {
        // SAFETY: the caller's MSG.
        unsafe { *msg = narrowed(*msg) };
    }
    r
}

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DispatchMessageA(msg: *const CMsg) -> isize {
    if msg.is_null() {
        return 0;
    }
    // SAFETY: the caller's MSG.
    let mut m = unsafe { *msg };
    if matches!(m.message, ansi::WM_CHAR | ansi::WM_DEADCHAR | ansi::WM_SYSCHAR | ansi::WM_SYSDEADCHAR) {
        m.w = ansi::enter(m.hwnd, Form::Ansi, Form::Wide, m.message, m.w, m.l as isize).w;
    }
    // SAFETY: a MSG of ours.
    unsafe { super::DispatchMessageW(&m) }
}

/// What an ANSI procedure reads as through the wide `GetWindowLongPtr` (a thunk, here the pointer with a mark).
pub const THUNK_TAG: u64 = 1 << 62;

/// Whether `hwnd` is an ANSI dialog window of the system dialog class.
pub fn is_ansi_dialog(hwnd: H) -> bool {
    info(hwnd).is_some_and(|i| !i.unicode && i.wndproc == poly_win32k::dlg::sysproc::DIALOG)
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowLongPtrA(hwnd: H, index: i32) -> isize {
    match call(Request::GetLong { hwnd: hwnd as u64, index, size: 8 }) {
        Reply::Value { value } => value as isize,
        Reply::Error { code } => {
            set_last_error(code);
            0
        }
        _ => 0,
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn GetWindowLongA(hwnd: H, index: i32) -> i32 {
    super::GetWindowLongW(hwnd, index)
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowLongPtrA(hwnd: H, index: i32, value: isize) -> isize {
    super::SetWindowLongPtrW(hwnd, index, value)
}

#[unsafe(no_mangle)]
pub extern "system" fn SetWindowLongA(hwnd: H, index: i32, value: i32) -> i32 {
    super::SetWindowLongW(hwnd, index, value)
}
