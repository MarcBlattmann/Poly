//! CreateWindowExW parameters as the window sees them: lpParam arrives as
//! CREATESTRUCT.lpCreateParams in WM_NCCREATE and WM_CREATE; hMenu is a
//! child's id (GWLP_ID) and is in CREATESTRUCT.hMenu; hInstance is in
//! CREATESTRUCT and GWLP_HINSTANCE; the parent is CREATESTRUCT.hwndParent;
//! lpszName is the caller's pointer, lpszClass is not (a copy); the window text is set
//! by DefWindowProc's WM_NCCREATE (and stays empty when the procedure
//! answers WM_NCCREATE itself); a NULL name gives an empty text.
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

/// winuser.h
#[repr(C)]
struct CREATESTRUCTW {
    create_params: *mut c_void,
    instance: HINSTANCE,
    menu: HMENU,
    parent: HWND,
    cy: i32,
    cx: i32,
    y: i32,
    x: i32,
    style: i32,
    name: *const u16,
    class: *const u16,
    ex_style: u32,
}

const WM_CREATE: u32 = 0x0001;
const WM_NCCREATE: u32 = 0x0081;
const WS_POPUP: u32 = 0x8000_0000;
const WS_CHILD: u32 = 0x4000_0000;
const GWLP_HINSTANCE: i32 = -6;
const GWLP_HWNDPARENT: i32 = -8;
const GWLP_ID: i32 = -12;
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

/// Print every logged line (prefixed) and empty the log.
fn flush_log(t: &mut Test, prefix: &str) {
    let log = LOG.get();
    let mut start = 0;
    for i in 0..log.len {
        if log.buf[i] == b'\n' {
            let line = core::str::from_utf8(&log.buf[start..i]).unwrap_or("?");
            let mut b = Buf::<300>::new();
            let _ = write!(b, "{prefix}{line}");
            t.line(b.as_str());
            start = i + 1;
        }
    }
    log.len = 0;
}

fn log_clear() {
    LOG.get().len = 0;
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

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
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
    fn DestroyWindow(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}

static SEEN: Global<[usize; 6]> = Global::new([0; 6]);
static SKIP_DEFAULT: Global<bool> = Global::new(false);

unsafe extern "system" fn proc_p(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE || msg == WM_CREATE {
        // SAFETY: lParam is a CREATESTRUCTW.
        let cs = unsafe { &*(l as *const CREATESTRUCTW) };
        if msg == WM_NCCREATE {
            *SEEN.get() = [
                cs.create_params as usize,
                cs.menu as usize,
                cs.instance as usize,
                cs.parent as usize,
                cs.name as usize,
                cs.class as usize,
            ];
            if *SKIP_DEFAULT.get() {
                return 1;
            }
        } else {
            logln!("WM_CREATE params same as WM_NCCREATE {}", (cs.create_params as usize == SEEN.get()[0]) as i32);
        }
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn text_of(h: HWND) -> Buf<300> {
    let mut buf = [0u16; 64];
    // SAFETY: buf has 64 units.
    let n = unsafe { GetWindowTextW(h, buf.as_mut_ptr(), 64) };
    let mut b = Buf::<300>::new();
    let _ = write!(b, "{n} \"");
    ascii_of(&buf, &mut b);
    let _ = b.write_str("\"");
    b
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cls = wz!("CwParams");
    t.check("registered", register(cls, Some(proc_p), 0, 0, core::ptr::null_mut()) != 0, 0);
    let name = wz!("Hello");
    let param = 0x1234_5678_9abc_usize;
    // SAFETY: NUL-terminated names; a made-up lpParam the procedure only compares.
    let p = unsafe {
        CreateWindowExW(
            0,
            cls.as_ptr(),
            name.as_ptr(),
            WS_POPUP,
            0,
            0,
            100,
            100,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            instance(),
            param as *mut c_void,
        )
    };
    t.check("created", !p.is_null(), 0);
    let s = *SEEN.get();
    t.check("lpcreateparams", s[0] == param, 0);
    t.check("hmenu_null", s[1] == 0, 0);
    t.check("hinstance", s[2] == instance() as usize, 0);
    t.check("parent_null", s[3] == 0, 0);
    t.check("name_pointer_is_the_callers", s[4] == name.as_ptr() as usize, 0);
    say!(t, "class pointer is the caller's: {}", (s[5] == cls.as_ptr() as usize) as i32);
    flush_log(t, "");
    let txt = text_of(p);
    say!(t, "text via default WM_NCCREATE: {}", txt.as_str());

    let c = create(0, cls, wz!("Kid"), WS_CHILD, 0, 0, 10, 10, p, 4321);
    let s = *SEEN.get();
    t.check("child_hmenu_is_id", s[1] == 4321, s[1] as i64);
    t.check("child_parent", s[3] == p as usize, 0);
    // SAFETY: plain queries.
    unsafe {
        t.expect("child_gwlp_id", GetWindowLongPtrW(c, GWLP_ID) as i64, 4321);
        t.check("child_gwlp_hinstance", GetWindowLongPtrW(c, GWLP_HINSTANCE) == instance() as isize, 0);
        t.check("child_gwlp_hwndparent", GetWindowLongPtrW(c, GWLP_HWNDPARENT) == p as isize, 0);
        t.expect("popup_gwlp_id", GetWindowLongPtrW(p, GWLP_ID) as i64, 0);
    }
    log_clear();

    *SKIP_DEFAULT.get() = true;
    let q = create(0, cls, wz!("NotSet"), WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    *SKIP_DEFAULT.get() = false;
    t.check("created_without_default_nccreate", !q.is_null(), 0);
    let txt = text_of(q);
    say!(t, "text when WM_NCCREATE is answered by the procedure: {}", txt.as_str());
    let n = create(0, cls, &[], WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    let txt = text_of(n);
    say!(t, "text with a NULL name: {} name pointer null {}", txt.as_str(), (SEEN.get()[4] == 0) as i32);
    log_clear();
    // SAFETY: our windows.
    unsafe {
        DestroyWindow(n);
        DestroyWindow(q);
        DestroyWindow(p);
    }
}
