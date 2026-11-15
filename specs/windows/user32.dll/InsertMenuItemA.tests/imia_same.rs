//! InsertMenuItemA and InsertMenuItemW produce the same item for the same
//! text (compared field by field through the wide getter), including
//! MIIM_STRING, MIIM_TYPE, states, check marks, the bitmap form and the
//! default and radio flags.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type HBRUSH = *mut c_void;
type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type LRESULT = isize;
type BOOL = i32;
type WNDPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT>;
/// winuser.h
#[repr(C)]
struct WNDCLASSEXW {
    cb_size: u32,
    style: u32,
    wnd_proc: WNDPROC,
    cls_extra: i32,
    wnd_extra: i32,
    instance: HINSTANCE,
    icon: *mut c_void,
    cursor: *mut c_void,
    background: HBRUSH,
    menu_name: *const u16,
    class_name: *const u16,
    icon_sm: *mut c_void,
}

const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_USER: u32 = 0x0400;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const WS_CAPTION: u32 = 0x00C0_0000;
const WS_SYSMENU: u32 = 0x0008_0000;
const WS_THICKFRAME: u32 = 0x0004_0000;
const WS_MINIMIZEBOX: u32 = 0x0002_0000;
const WS_MAXIMIZEBOX: u32 = 0x0001_0000;
const WS_OVERLAPPEDWINDOW: u32 =
    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_MINIMIZEBOX | WS_MAXIMIZEBOX;

const GWLP_USERDATA: i32 = -21;
/// A global the window procedures and the test share (one thread at a
/// time unless a test says otherwise).
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}
impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }
    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        // SAFETY: see the Sync impl: no two live references at once.
        unsafe { &mut *self.0.get() }
    }
}

/// The message log: lines appended by window procedures, printed by the test.
struct Log {
    buf: [u8; 32768],
    len: usize,
    on: bool,
}

static LOG: Global<Log> = Global::new(Log { buf: [0; 32768], len: 0, on: true });
impl core::fmt::Write for Log {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &b in s.as_bytes() {
            if self.len < self.buf.len() {
                self.buf[self.len] = b;
                self.len += 1;
            }
        }
        Ok(())
    }
}

/// Append one line to the log (when logging is on).
fn logf(args: core::fmt::Arguments) {
    let log = LOG.get();
    if log.on {
        let _ = log.write_fmt(args);
        let _ = log.write_str("\n");
    }
}

macro_rules! logln {
    ($($a:tt)*) => { logf(format_args!($($a)*)) };
}

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

/// This executable's instance handle.
fn instance() -> HINSTANCE {
    // SAFETY: NULL names the executable.
    unsafe { GetModuleHandleW(core::ptr::null()) }
}

/// Register a class with `proc_` (and no menu, icon or cursor).
fn register(name: &[u16], proc_: WNDPROC, style: u32, wnd_extra: i32, brush: HBRUSH) -> u16 {
    let wc = WNDCLASSEXW {
        cb_size: core::mem::size_of::<WNDCLASSEXW>() as u32,
        style,
        wnd_proc: proc_,
        cls_extra: 0,
        wnd_extra,
        instance: instance(),
        icon: core::ptr::null_mut(),
        cursor: core::ptr::null_mut(),
        background: brush,
        menu_name: core::ptr::null(),
        class_name: name.as_ptr(),
        icon_sm: core::ptr::null_mut(),
    };
    // SAFETY: a filled WNDCLASSEXW; the name is NUL-terminated and static.
    unsafe { RegisterClassExW(&wc) }
}

/// CreateWindowExW with this executable's instance.
fn create(
    ex: u32,
    class: &[u16],
    name: &[u16],
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    parent: HWND,
    menu: usize,
) -> HWND {
    // SAFETY: NUL-terminated names; the parent is a window or NULL.
    unsafe {
        CreateWindowExW(
            ex,
            class.as_ptr(),
            if name.is_empty() { core::ptr::null() } else { name.as_ptr() },
            style,
            x,
            y,
            w,
            h,
            parent,
            menu as HMENU,
            instance(),
            core::ptr::null_mut(),
        )
    }
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

const MF_BYPOSITION: u32 = 0x0400;
const MIIM_STATE: u32 = 0x0001;
const MIIM_ID: u32 = 0x0002;
const MIIM_SUBMENU: u32 = 0x0004;
const MIIM_CHECKMARKS: u32 = 0x0008;
const MIIM_TYPE: u32 = 0x0010;
const MIIM_DATA: u32 = 0x0020;
const MIIM_STRING: u32 = 0x0040;
const MIIM_BITMAP: u32 = 0x0080;
const MIIM_FTYPE: u32 = 0x0100;
const MFT_BITMAP: u32 = 0x0004;
const MFT_OWNERDRAW: u32 = 0x0100;
const MFT_RADIOCHECK: u32 = 0x0200;
const MFS_GRAYED: u32 = 0x0003;
const MFS_CHECKED: u32 = 0x0008;
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

/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct MENUITEMINFOA {
    cb_size: u32,
    f_mask: u32,
    f_type: u32,
    f_state: u32,
    w_id: u32,
    h_sub_menu: HMENU,
    hbmp_checked: HBITMAP,
    hbmp_unchecked: HBITMAP,
    dw_item_data: usize,
    dw_type_data: *mut u8,
    cch: u32,
    hbmp_item: HBITMAP,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterClassExW(class: *const WNDCLASSEXW) -> u16;
    fn CreateWindowExW(
        ex_style: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HWND,
        menu: HMENU,
        instance: HINSTANCE,
        param: *mut c_void,
    ) -> HWND;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetWindowTextA(hwnd: HWND, buf: *mut u8, max: i32) -> i32;
    fn CreatePopupMenu() -> HMENU;
    fn DestroyMenu(menu: HMENU) -> BOOL;
    fn InsertMenuItemW(menu: HMENU, item: u32, by_pos: BOOL, info: *const MENUITEMINFOW) -> BOOL;
    fn GetMenuItemInfoW(menu: HMENU, item: u32, by_pos: BOOL, info: *mut MENUITEMINFOW) -> BOOL;
    fn GetMenuItemCount(menu: HMENU) -> i32;
    fn GetMenuItemID(menu: HMENU, pos: i32) -> u32;
    fn GetMenuState(menu: HMENU, id: u32, flags: u32) -> u32;
    fn GetMenuStringW(menu: HMENU, id: u32, buf: *mut u16, max: i32, flags: u32) -> i32;
    fn GetMenuStringA(menu: HMENU, item: u32, buf: *mut u8, max: i32, flags: u32) -> i32;
    fn InsertMenuItemA(menu: HMENU, item: u32, by_pos: BOOL, info: *const MENUITEMINFOA) -> BOOL;
    fn RegisterClassExA(class: *const c_void) -> u16;
    fn CreateWindowExA(ex: u32, class: *const u8, name: *const u8, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: HMENU, instance: HINSTANCE, param: *mut c_void) -> HWND;
    fn DefWindowProcA(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn CreateBitmap(w: i32, h: i32, planes: u32, bpp: u32, bits: *const c_void) -> HBITMAP;
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

// ---- ANSI helpers of these tests ----

// The code page of the oracle's ANSI strings (the tests record what depends on it).
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetACP() -> u32;
    fn GetOEMCP() -> u32;
}

/// Bytes up to the first NUL: printable ASCII as is, any other byte as <hh>.
fn abytes(buf: &[u8]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    for &c in buf {
        if c == 0 {
            break;
        }
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as char);
        } else {
            let _ = write!(b, "<{c:02x}>");
        }
    }
    b
}

/// UTF-16 up to the first NUL: printable ASCII as is, any other unit as <hhhh>.
fn wshow(buf: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    for &c in buf {
        if c == 0 {
            break;
        }
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// Length up to the first NUL.
fn alen(b: &[u8]) -> usize {
    b.iter().position(|&c| c == 0).unwrap_or(b.len())
}

/// Text of a window through GetWindowTextW, shown with `wshow`.
fn wtext_of(h: HWND) -> Buf<300> {
    let mut v = [0u16; 200];
    // SAFETY: v has 200 units.
    unsafe { GetWindowTextW(h, v.as_mut_ptr(), 200) };
    wshow(&v)
}

/// Text of a window through GetWindowTextA, shown with `abytes`.
fn atext_of(h: HWND) -> Buf<300> {
    let mut v = [0u8; 300];
    // SAFETY: v has 300 bytes.
    unsafe { GetWindowTextA(h, v.as_mut_ptr(), 300) };
    abytes(&v)
}

// A few strings; the bytes are those of the Windows ANSI code page 1252, the
// UTF-16 units what they stand for there.
/// "caf" + E9, space, 80 (the euro sign in 1252), space, 81 (not assigned in 1252), space, FF
const A_HIGH: &[u8] = b"caf\xE9 \x80 \x81 \xFF\0";
/// "caf" + U+00E9, U+4E2D (a Chinese character no single-byte code page has), U+0416 (Cyrillic), U+20AC (euro)
const W_MIXED: [u16; 8] = [0x63, 0x61, 0x66, 0xE9, 0x4E2D, 0x0416, 0x20AC, 0];

// A pair of window classes that keep the text they are given the way an edit control
// would, one registered with RegisterClassExA (an ANSI window procedure), one with
// RegisterClassExW. They log what their procedure receives.
static TXT_A: Global<[u8; 80]> = Global::new([0; 80]);
static TXT_W: Global<[u16; 80]> = Global::new([0; 80]);

unsafe extern "system" fn a_child(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: l is a NUL-terminated string (WM_SETTEXT) or a buffer of w bytes (WM_GETTEXT).
    unsafe {
        if m >= WM_USER && m < 0xC000 {
            logln!("   [A window] message {:#x} wParam {:#x} lParam {:#x}", m, w, l);
            return 0x1234;
        }
        match m {
            WM_SETTEXT => {
                let p = l as *const u8;
                let mut n = 0;
                while n < 79 && *p.add(n) != 0 {
                    (*TXT_A.get())[n] = *p.add(n);
                    n += 1;
                }
                (*TXT_A.get())[n] = 0;
                logln!("   [A window] WM_SETTEXT got bytes '{}'", abytes(&*TXT_A.get()).as_str());
                1
            }
            WM_GETTEXT => {
                let n = alen(&*TXT_A.get()).min((w as usize).saturating_sub(1));
                let p = l as *mut u8;
                for i in 0..n {
                    *p.add(i) = (*TXT_A.get())[i];
                }
                if w > 0 {
                    *p.add(n) = 0;
                }
                logln!("   [A window] WM_GETTEXT wParam {w} gave {n} bytes");
                n as LRESULT
            }
            WM_GETTEXTLENGTH => alen(&*TXT_A.get()) as LRESULT,
            _ => DefWindowProcA(h, m, w, l),
        }
    }
}

unsafe extern "system" fn w_child(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: l is a NUL-terminated string (WM_SETTEXT) or a buffer of w units (WM_GETTEXT).
    unsafe {
        if m >= WM_USER && m < 0xC000 {
            logln!("   [W window] message {:#x} wParam {:#x} lParam {:#x}", m, w, l);
            return 0x1234;
        }
        match m {
            WM_SETTEXT => {
                let p = l as *const u16;
                let mut n = 0;
                while n < 79 && *p.add(n) != 0 {
                    (*TXT_W.get())[n] = *p.add(n);
                    n += 1;
                }
                (*TXT_W.get())[n] = 0;
                logln!("   [W window] WM_SETTEXT got units '{}'", wshow(&*TXT_W.get()).as_str());
                1
            }
            WM_GETTEXT => {
                let mut n = 0;
                while (*TXT_W.get())[n] != 0 {
                    n += 1;
                }
                let n = n.min((w as usize).saturating_sub(1));
                let p = l as *mut u16;
                for i in 0..n {
                    *p.add(i) = (*TXT_W.get())[i];
                }
                if w > 0 {
                    *p.add(n) = 0;
                }
                logln!("   [W window] WM_GETTEXT wParam {w} gave {n} units");
                n as LRESULT
            }
            WM_GETTEXTLENGTH => {
                let mut n = 0;
                while (*TXT_W.get())[n] != 0 {
                    n += 1;
                }
                n as LRESULT
            }
            _ => DefWindowProcW(h, m, w, l),
        }
    }
}

/// A parent window (a Unicode one) with an ANSI child with id 11 and a Unicode child with id 12.
fn ansi_pair() -> (HWND, HWND, HWND) {
    let mut wc: WNDCLASSEXW = zeroed();
    wc.cb_size = core::mem::size_of::<WNDCLASSEXW>() as u32;
    wc.instance = instance();
    wc.wnd_proc = Some(a_child);
    wc.class_name = b"AnsiKid\0".as_ptr() as *const u16;
    // SAFETY: WNDCLASSEXA and WNDCLASSEXW have the same layout; the name is a byte string here.
    unsafe { RegisterClassExA(&wc as *const WNDCLASSEXW as *const c_void) };
    register(wz!("WideKid"), Some(w_child), 0, 0, core::ptr::null_mut());
    register(wz!("KidParent"), Some(caret_like_wp), 0, 0, core::ptr::null_mut());
    let p = create(0, wz!("KidParent"), wz!("parent"), WS_OVERLAPPEDWINDOW, 0, 0, 200, 100, core::ptr::null_mut(), 0);
    // SAFETY: class names are NUL-terminated byte strings and UTF-16 strings; the parent is ours.
    unsafe {
        let a = CreateWindowExA(0, b"AnsiKid\0".as_ptr(), b"\0".as_ptr(), WS_CHILD, 0, 0, 50, 20, p, 11 as HMENU, instance(), core::ptr::null_mut());
        let w = create(0, wz!("WideKid"), wz!(""), WS_CHILD, 60, 0, 50, 20, p, 12);
        (p, a, w)
    }
}

unsafe extern "system" fn caret_like_wp(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: default processing.
    unsafe { DefWindowProcW(h, m, w, l) }
}

// ---- ANSI menu helpers of these tests ----

/// Every item of the menu: id, state flags, the text through the wide and through the ANSI function.
fn mshow(t: &mut Test, label: &str, menu: HMENU) {
    // SAFETY: plain queries with buffers of 100 units.
    unsafe {
        let n = GetMenuItemCount(menu);
        say!(t, "{label}: count {n}");
        for i in 0..n.max(0) {
            let mut w = [0u16; 100];
            let rw = GetMenuStringW(menu, i as u32, w.as_mut_ptr(), 100, MF_BYPOSITION);
            let mut a = [0u8; 100];
            let ra = GetMenuStringA(menu, i as u32, a.as_mut_ptr(), 100, MF_BYPOSITION);
            let st = GetMenuState(menu, i as u32, MF_BYPOSITION);
            let id = GetMenuItemID(menu, i);
            say!(t, "  [{i}] id={id:#x} state={st:#x} W({rw}) '{}' A({ra}) '{}'", wshow(&w).as_str(), abytes(&a).as_str());
        }
    }
}

fn mk_a(mask: u32) -> MENUITEMINFOA {
    let mut m: MENUITEMINFOA = zeroed();
    m.cb_size = core::mem::size_of::<MENUITEMINFOA>() as u32;
    m.f_mask = mask;
    m
}

fn mk_item(mask: u32) -> MENUITEMINFOW {
    let mut m: MENUITEMINFOW = zeroed();
    m.cb_size = core::mem::size_of::<MENUITEMINFOW>() as u32;
    m.f_mask = mask;
    m
}

fn dump_w(t: &mut Test, label: &str, m: HMENU, pos: u32) {
    // SAFETY: plain query with a 100 unit buffer.
    unsafe {
        let mut b = [0u16; 100];
        let mut g = mk_item(MIIM_ID | MIIM_STATE | MIIM_FTYPE | MIIM_STRING | MIIM_DATA | MIIM_SUBMENU | MIIM_CHECKMARKS | MIIM_BITMAP);
        g.dw_type_data = b.as_mut_ptr();
        g.cch = 100;
        let r = GetMenuItemInfoW(m, pos, 1, &mut g);
        say!(t, "{label}: ok {r} id {:#x} state {:#x} type {:#x} data {:#x} cch {} sub {} checkmarks {}/{} text '{}'", g.w_id, g.f_state, g.f_type, g.dw_item_data, g.cch, (!g.h_sub_menu.is_null()) as i32, (!g.hbmp_checked.is_null()) as i32, (!g.hbmp_unchecked.is_null()) as i32, wshow(&b).as_str());
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    unsafe {
        let ma = CreatePopupMenu();
        let mw = CreatePopupMenu();
        let bmp = CreateBitmap(4, 4, 1, 1, core::ptr::null());
        // 1 plain
        let mut a = mk_a(MIIM_ID | MIIM_STRING | MIIM_STATE | MIIM_FTYPE);
        a.w_id = 1;
        a.f_state = MFS_GRAYED | MFS_CHECKED;
        a.dw_type_data = b"plain\0".as_ptr() as *mut u8;
        InsertMenuItemA(ma, 0, 1, &a);
        let mut w = mk_item(MIIM_ID | MIIM_STRING | MIIM_STATE | MIIM_FTYPE);
        w.w_id = 1;
        w.f_state = MFS_GRAYED | MFS_CHECKED;
        w.dw_type_data = wz!("plain").as_ptr() as *mut u16;
        InsertMenuItemW(mw, 0, 1, &w);
        // 2 radio, default
        let mut a = mk_a(MIIM_ID | MIIM_STRING | MIIM_STATE | MIIM_FTYPE);
        a.w_id = 2;
        a.f_type = MFT_RADIOCHECK;
        a.f_state = 0x1000;
        a.dw_type_data = b"radio\0".as_ptr() as *mut u8;
        InsertMenuItemA(ma, 1, 1, &a);
        let mut w = mk_item(MIIM_ID | MIIM_STRING | MIIM_STATE | MIIM_FTYPE);
        w.w_id = 2;
        w.f_type = MFT_RADIOCHECK;
        w.f_state = 0x1000;
        w.dw_type_data = wz!("radio").as_ptr() as *mut u16;
        InsertMenuItemW(mw, 1, 1, &w);
        // 3 check marks and data
        let mut a = mk_a(MIIM_ID | MIIM_STRING | MIIM_CHECKMARKS | MIIM_DATA);
        a.w_id = 3;
        a.hbmp_checked = bmp;
        a.hbmp_unchecked = bmp;
        a.dw_item_data = 77;
        a.dw_type_data = b"marks\0".as_ptr() as *mut u8;
        InsertMenuItemA(ma, 2, 1, &a);
        let mut w = mk_item(MIIM_ID | MIIM_STRING | MIIM_CHECKMARKS | MIIM_DATA);
        w.w_id = 3;
        w.hbmp_checked = bmp;
        w.hbmp_unchecked = bmp;
        w.dw_item_data = 77;
        w.dw_type_data = wz!("marks").as_ptr() as *mut u16;
        InsertMenuItemW(mw, 2, 1, &w);
        // 4 a bitmap item (MIIM_BITMAP)
        let mut a = mk_a(MIIM_ID | MIIM_BITMAP);
        a.w_id = 4;
        a.hbmp_item = bmp;
        InsertMenuItemA(ma, 3, 1, &a);
        let mut w = mk_item(MIIM_ID | MIIM_BITMAP);
        w.w_id = 4;
        w.hbmp_item = bmp;
        InsertMenuItemW(mw, 3, 1, &w);
        // 5 MIIM_TYPE bitmap (old form: dwTypeData is the bitmap)
        let mut a = mk_a(MIIM_ID | MIIM_TYPE);
        a.w_id = 5;
        a.f_type = MFT_BITMAP;
        a.dw_type_data = bmp as *mut u8;
        let ra = InsertMenuItemA(ma, 4, 1, &a);
        let mut w = mk_item(MIIM_ID | MIIM_TYPE);
        w.w_id = 5;
        w.f_type = MFT_BITMAP;
        w.dw_type_data = bmp as *mut u16;
        let rw = InsertMenuItemW(mw, 4, 1, &w);
        say!(t, "MFT_BITMAP through MIIM_TYPE: A {ra} W {rw}");
        // 6 owner-draw
        let mut a = mk_a(MIIM_ID | MIIM_TYPE);
        a.w_id = 6;
        a.f_type = MFT_OWNERDRAW;
        a.dw_type_data = 0x5150 as *mut u8;
        InsertMenuItemA(ma, 5, 1, &a);
        let mut w = mk_item(MIIM_ID | MIIM_TYPE);
        w.w_id = 6;
        w.f_type = MFT_OWNERDRAW;
        w.dw_type_data = 0x5150 as *mut u16;
        InsertMenuItemW(mw, 5, 1, &w);
        for i in 0..6 {
            dump_w(t, "ANSI ", ma, i);
            dump_w(t, "wide ", mw, i);
        }
        DestroyMenu(ma);
        DestroyMenu(mw);
        DeleteObject(bmp);
    }
}
