//! SetMenuItemBitmaps: sets the bitmaps shown for an item's checked and
//! unchecked state, by position and by command; both, one, none (NULL
//! clears); the bitmaps read back through GetMenuItemInfoW (MIIM_CHECKMARKS)
//! and as the item data of GetMenuItemInfoW; the bitmaps are not copied or
//! freed; flags MF_USECHECKBITMAPS; errors.
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
type HGDIOBJ = *mut c_void;
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
const MF_CHECKED: u32 = 0x0008;
const MF_STRING: u32 = 0x0000;
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
    fn GetMenuState(menu: HMENU, id: u32, flags: u32) -> u32;
    fn GetMenuStringW(menu: HMENU, id: u32, buf: *mut u16, max: i32, flags: u32) -> i32;
    fn SetMenuItemBitmaps(menu: HMENU, pos: u32, flags: u32, unchecked: HBITMAP, checked: HBITMAP) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectType(obj: HGDIOBJ) -> u32;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
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
        if stringify!($e).contains("is_null") && r == 0 {
            say!($t, "{}: {}", $label, r);
        } else if r == 0 || r == -1 || stringify!($e).contains("is_null") {
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

/// MAKEINTRESOURCE for the ANSI functions
fn mira(id: usize) -> *const u8 {
    id as *const u8
}

fn marks(t: &mut Test, label: &str, m: HMENU, pos: u32, un: HBITMAP, ch: HBITMAP) {
    let mut g = mk_item(MIIM_CHECKMARKS | MIIM_STATE);
    // SAFETY: g is a MENUITEMINFOW.
    let r = unsafe { GetMenuItemInfoW(m, pos, 1, &mut g) };
    let name = |h: HBITMAP| if h.is_null() { "none" } else if h == un { "the unchecked bitmap" } else if h == ch { "the checked bitmap" } else { "another" };
    say!(t, "{label}: ok {r} checked: {} unchecked: {} state {:#x}", name(g.hbmp_checked), name(g.hbmp_unchecked), g.f_state);
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    unsafe {
        let m = CreatePopupMenu();
        AppendMenuW(m, MF_STRING, 1, wz!("one").as_ptr());
        AppendMenuW(m, MF_STRING, 2, wz!("two").as_ptr());
        AppendMenuW(m, MF_STRING | MF_CHECKED, 3, wz!("three").as_ptr());
        let un = CreateBitmap(8, 8, 1, 1, core::ptr::null());
        let ch = CreateBitmap(8, 8, 1, 1, core::ptr::null());
        marks(t, "before", m, 0, un, ch);
        call!(t, "SetMenuItemBitmaps by position 0, both", SetMenuItemBitmaps(m, 0, MF_BYPOSITION, un, ch));
        marks(t, "item 0", m, 0, un, ch);
        call!(t, "SetMenuItemBitmaps by command 2, only the checked", SetMenuItemBitmaps(m, 2, MF_BYCOMMAND, core::ptr::null_mut(), ch));
        marks(t, "item 1", m, 1, un, ch);
        call!(t, "SetMenuItemBitmaps by command 3, only the unchecked", SetMenuItemBitmaps(m, 3, MF_BYCOMMAND, un, core::ptr::null_mut()));
        marks(t, "item 2", m, 2, un, ch);
        call!(t, "SetMenuItemBitmaps item 0 again with NULL, NULL (clears)", SetMenuItemBitmaps(m, 0, MF_BYPOSITION, core::ptr::null_mut(), core::ptr::null_mut()));
        marks(t, "item 0 after clearing", m, 0, un, ch);
        call!(t, "SetMenuItemBitmaps with the same bitmap for both", SetMenuItemBitmaps(m, 0, MF_BYPOSITION, un, un));
        marks(t, "item 0", m, 0, un, ch);
        // the flags on the item
        let st = GetMenuState(m, 0, MF_BYPOSITION);
        say!(t, "GetMenuState item 0 {st:#x}");
        // the bitmaps are not freed with the menu items
        say!(t, "bitmap type before DestroyMenu {}", GetObjectType(un));
        DestroyMenu(m);
        say!(t, "bitmap type after DestroyMenu {} (the menu does not free them)", GetObjectType(un));
        // errors
        let m2 = CreatePopupMenu();
        AppendMenuW(m2, MF_STRING, 1, wz!("x").as_ptr());
        call!(t, "position 5 (absent)", SetMenuItemBitmaps(m2, 5, MF_BYPOSITION, un, ch));
        call!(t, "command 99 (absent)", SetMenuItemBitmaps(m2, 99, MF_BYCOMMAND, un, ch));
        call!(t, "bad menu", SetMenuItemBitmaps(0x0bad_0bad as HMENU, 0, MF_BYPOSITION, un, ch));
        call!(t, "NULL menu", SetMenuItemBitmaps(core::ptr::null_mut(), 0, MF_BYPOSITION, un, ch));
        call!(t, "made-up bitmap handles", SetMenuItemBitmaps(m2, 0, MF_BYPOSITION, 0x0bad_0bad as HBITMAP, 0x0bad_0bee as HBITMAP));
        marks(t, "item with made-up bitmaps", m2, 0, un, ch);
        call!(t, "flags 0x8000 (invalid)", SetMenuItemBitmaps(m2, 0, 0x8000, un, ch));
        DestroyMenu(m2);
        DeleteObject(un);
        DeleteObject(ch);
    }
}
