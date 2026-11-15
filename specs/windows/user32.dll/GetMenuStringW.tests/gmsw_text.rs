//! GetMenuStringW: returns the length copied (without the NUL), truncates to
//! the buffer (always NUL terminated), nMax 0 gives the full length,
//! separators, popups and owner-draw items, by command and by position,
//! absent items.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HMENU = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
const GWLP_USERDATA: i32 = -21;
/// A fixed-size text buffer for formatting lines.
struct Buf<const N: usize> {
    b: [u8; N],
    n: usize,
}

impl<const N: usize> Buf<N> {
    const fn new() -> Self {
        Buf { b: [0; N], n: 0 }
    }
    fn as_str(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            if self.n < N {
                self.b[self.n] = c;
                self.n += 1;
            }
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
}

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

/// How a window handle in a message is printed: "null", the tag letter of
/// a window of this process (GWLP_USERDATA), "ours" (this process, no tag),
/// "other" (another process: the desktop's windows, the shell) or
/// "invalid".
struct W(HWND);

impl core::fmt::Display for W {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.0.is_null() {
            return f.write_str("null");
        }
        let mut pid = 0u32;
        // SAFETY: plain queries; pid is writable.
        let (tid, me) = unsafe { (GetWindowThreadProcessId(self.0, &mut pid), GetCurrentProcessId()) };
        if tid == 0 {
            return f.write_str("invalid");
        }
        if pid != me {
            return f.write_str("other");
        }
        // SAFETY: a window of this process.
        let tag = unsafe { GetWindowLongPtrW(self.0, GWLP_USERDATA) };
        if (0x41..=0x5A).contains(&tag) || (0x61..=0x7A).contains(&tag) {
            f.write_char(tag as u8 as char)
        } else {
            f.write_str("ours")
        }
    }
}

const MF_BYCOMMAND: u32 = 0x0000;
const MF_BYPOSITION: u32 = 0x0400;
const MF_SEPARATOR: u32 = 0x0800;
const MF_STRING: u32 = 0x0000;
const MF_OWNERDRAW: u32 = 0x0100;
const MF_POPUP: u32 = 0x0010;
const MIIM_STATE: u32 = 0x0001;
const MIIM_ID: u32 = 0x0002;
const MIIM_SUBMENU: u32 = 0x0004;
const MIIM_CHECKMARKS: u32 = 0x0008;
const MIIM_DATA: u32 = 0x0020;
const MIIM_STRING: u32 = 0x0040;
const MIIM_FTYPE: u32 = 0x0100;
/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MENUITEMINFOW {
    cb_size: u32,
    f_mask: u32,
    f_type: u32,
    f_state: u32,
    w_id: u32,
    h_sub_menu: HMENU,
    hbmp_checked: HBITMAP,
    hbmp_unchecked: HBITMAP,
    dw_item_data: usize,
    dw_type_data: *mut u16,
    cch: u32,
    hbmp_item: HBITMAP,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn CreatePopupMenu() -> HMENU;
    fn DestroyMenu(menu: HMENU) -> BOOL;
    fn AppendMenuW(menu: HMENU, flags: u32, id: usize, item: *const u16) -> BOOL;
    fn GetMenuItemInfoW(menu: HMENU, item: u32, by_pos: BOOL, info: *mut MENUITEMINFOW) -> BOOL;
    fn GetMenuItemCount(menu: HMENU) -> i32;
    fn GetMenuStringW(menu: HMENU, id: u32, buf: *mut u16, max: i32, flags: u32) -> i32;
}

// ---- menu helpers of these tests ----

/// The text of an item as GetMenuStringW returns it: (return value, text).
fn mtext(menu: HMENU, item: u32, flags: u32) -> (i32, Buf<300>) {
    let mut buf = [0u16; 200];
    // SAFETY: buf has 200 units.
    let n = unsafe { GetMenuStringW(menu, item, buf.as_mut_ptr(), 200, flags) };
    let mut b = Buf::<300>::new();
    ascii_of(&buf, &mut b);
    (n, b)
}

fn mk_item(mask: u32) -> MENUITEMINFOW {
    let mut m: MENUITEMINFOW = zeroed();
    m.cb_size = core::mem::size_of::<MENUITEMINFOW>() as u32;
    m.f_mask = mask;
    m
}

/// One line per item: position, id (or sub), state flags, type flags, text.
fn mdump(t: &mut Test, label: &str, menu: HMENU) {
    // SAFETY: plain query.
    let n = unsafe { GetMenuItemCount(menu) };
    say!(t, "{label}: count {n}");
    for i in 0..n.max(0) {
        let mut buf = [0u16; 100];
        let mut m = mk_item(MIIM_ID | MIIM_STATE | MIIM_FTYPE | MIIM_SUBMENU | MIIM_STRING | MIIM_DATA | MIIM_CHECKMARKS);
        m.dw_type_data = buf.as_mut_ptr();
        m.cch = 100;
        // SAFETY: m describes buf.
        let ok = unsafe { GetMenuItemInfoW(menu, i as u32, 1, &mut m) };
        let mut b = Buf::<300>::new();
        ascii_of(&buf, &mut b);
        let id = if !m.h_sub_menu.is_null() { 0xFFFF_FFFF } else { m.w_id };
        say!(
            t,
            "  [{i}] ok={ok} id={id:#x} sub={} state={:#x} type={:#x} data={:#x} text='{}' cch={}",
            if m.h_sub_menu.is_null() { 0 } else { 1 },
            m.f_state,
            m.f_type,
            m.dw_item_data,
            b.as_str(),
            m.cch
        );
    }
}

/// "ok" or "err N" after a call that returned `r` (error read at once).
fn res(t: &mut Test, label: &str, r: i64) {
    let e = last_error();
    say!(t, "{label}: {r} err={e}");
}

/// Call `$e` with the last error set to a marker; print "label: result err=N"
/// (err -1: the call did not touch the last error). Gives the result.
macro_rules! call {
    ($t:expr, $label:expr, $e:expr) => {{
        set_error(0xDEAD);
        let r = ($e) as i64;
        let e = last_error();
        if r == 0 || r == -1 || stringify!($e).contains("is_null") {
            say!($t, "{}: {} err={}", $label, r, if e == 0xDEAD { -1 } else { e as i64 });
        } else {
            say!($t, "{}: {}", $label, r);
        }
        r
    }};
}

/// MAKEINTRESOURCE
fn mir(id: usize) -> *const u16 {
    id as *const u16
}

/// A menu handle made up by the test (never a real one).
const BOGUS_MENU: usize = 0x0bad_0bad;

fn show(t: &mut Test, label: &str, menu: HMENU, item: u32, flags: u32, max: i32) {
    let mut buf = [0xEEEEu16; 64];
    set_error(0xDEAD);
    // SAFETY: buf has 64 units; max is at most 64.
    let n = unsafe { GetMenuStringW(menu, item, buf.as_mut_ptr(), max, flags) };
    let e = last_error();
    let mut b = Buf::<300>::new();
    ascii_of(&buf, &mut b);
    // how many units were written (up to the first untouched 0xEEEE)
    let written = buf.iter().take_while(|&&c| c != 0xEEEE).count();
    let es = if n == 0 { if e == 0xDEAD { -1 } else { e as i64 } } else { -2 };
    say!(t, "{label}: {n} err={} text='{}' written={}", es, if written == 0 { "" } else { b.as_str() }, written);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    let p = unsafe { CreatePopupMenu() };
    let s = unsafe { CreatePopupMenu() };
    unsafe { AppendMenuW(p, MF_STRING, 1, wz!("Hello world").as_ptr()) };
    unsafe { AppendMenuW(p, MF_SEPARATOR, 2, core::ptr::null()) };
    unsafe { AppendMenuW(p, MF_POPUP, s as usize, wz!("Popup &Text").as_ptr()) };
    unsafe { AppendMenuW(p, MF_OWNERDRAW, 3, 0x77usize as *const u16) };
    unsafe { AppendMenuW(p, MF_STRING, 4, wz!("").as_ptr()) };
    unsafe { AppendMenuW(p, MF_STRING, 5, wz!("A\tB").as_ptr()) };
    show(t, "full buffer", p, 1, MF_BYCOMMAND, 64);
    show(t, "exact size 12", p, 1, MF_BYCOMMAND, 12);
    show(t, "size 11 (truncates)", p, 1, MF_BYCOMMAND, 11);
    show(t, "size 6", p, 1, MF_BYCOMMAND, 6);
    show(t, "size 1", p, 1, MF_BYCOMMAND, 1);
    show(t, "size 0", p, 1, MF_BYCOMMAND, 0);
    show(t, "size -1", p, 1, MF_BYCOMMAND, -1);
    show(t, "NULL buffer size 0", p, 1, MF_BYCOMMAND, 0);
    set_error(0xDEAD);
    // SAFETY: a NULL buffer with a size of 0.
    let n = unsafe { GetMenuStringW(p, 1, core::ptr::null_mut(), 0, MF_BYCOMMAND) };
    say!(t, "NULL buffer, nMax 0: {n}");
    show(t, "by position 0", p, 0, MF_BYPOSITION, 64);
    show(t, "separator", p, 1, MF_BYPOSITION, 64);
    show(t, "popup", p, 2, MF_BYPOSITION, 64);
    show(t, "command 2 is the separator's id", p, 2, MF_BYCOMMAND, 64);
    show(t, "owner draw", p, 3, MF_BYPOSITION, 64);
    show(t, "empty text", p, 4, MF_BYPOSITION, 64);
    show(t, "tab text", p, 5, MF_BYPOSITION, 64);
    show(t, "absent command", p, 999, MF_BYCOMMAND, 64);
    show(t, "absent position", p, 99, MF_BYPOSITION, 64);
    show(t, "bad menu", BOGUS_MENU as HMENU, 1, MF_BYCOMMAND, 64);
    unsafe { DestroyMenu(p) };
}
