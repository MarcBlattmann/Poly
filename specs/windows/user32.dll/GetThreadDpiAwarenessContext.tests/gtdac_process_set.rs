//! GetThreadDpiAwarenessContext follows the process awareness (set first with
//! SetProcessDpiAwarenessContext(SYSTEM_AWARE)) until the thread has a context of
//! its own; after SetThreadDpiAwarenessContext another thread of the process is
//! not affected; GetDpiAwarenessContextForProcess(current process) and the
//! window created under each state.
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
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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

const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
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

const DPI_UNAWARE: isize = -1;
const DPI_SYSTEM_AWARE: isize = -2;
const DPI_PER_MONITOR_AWARE: isize = -3;
const DPI_PER_MONITOR_AWARE_V2: isize = -4;
const DPI_UNAWARE_GDISCALED: isize = -5;
/// A tiny helper: the name of an awareness context handle as the tests print
/// it (the pseudo handles by name, anything else as "other").
fn dp_ctx(c: isize) -> &'static str {
    match c {
        0 => "NULL",
        -1 => "UNAWARE",
        -2 => "SYSTEM_AWARE",
        -3 => "PER_MONITOR_AWARE",
        -4 => "PER_MONITOR_AWARE_V2",
        -5 => "UNAWARE_GDISCALED",
        _ => "other",
    }
}

/// Which pseudo handle a context equals (AreDpiAwarenessContextsEqual), or
/// "none".
fn dp_which(c: isize) -> &'static str {
    // SAFETY: plain queries on handle values.
    unsafe {
        for p in [DPI_UNAWARE, DPI_SYSTEM_AWARE, DPI_PER_MONITOR_AWARE, DPI_PER_MONITOR_AWARE_V2, DPI_UNAWARE_GDISCALED]
        {
            if AreDpiAwarenessContextsEqual(c, p) != 0 {
                return dp_ctx(p);
            }
        }
    }
    "none"
}

/// A window procedure that does nothing but the default handling (nothing
/// is logged).
unsafe extern "system" fn dp_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Register `name` with `dp_proc` and create a hidden popup window.
fn dp_popup(name: &[u16]) -> HWND {
    register(name, Some(dp_proc), 0, 0, core::ptr::null_mut());
    create(WS_EX_NOACTIVATE, name, wz!(""), WS_POPUP, 10, 10, 100, 50, core::ptr::null_mut(), 0)
}

/// The awareness of a window, by pseudo handle name.
fn dp_wa(h: HWND) -> &'static str {
    // SAFETY: plain query.
    dp_which(unsafe { GetWindowDpiAwarenessContext(h) })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetCurrentProcessId() -> u32;
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
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
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SetProcessDpiAwarenessContext(ctx: isize) -> BOOL;
    fn GetThreadDpiAwarenessContext() -> isize;
    fn SetThreadDpiAwarenessContext(ctx: isize) -> isize;
    fn GetWindowDpiAwarenessContext(hwnd: HWND) -> isize;
    fn AreDpiAwarenessContextsEqual(a: isize, b: isize) -> BOOL;
    fn GetDpiAwarenessContextForProcess(process: HANDLE) -> isize;
}

poly_wtest::main!(test);

static G_N: Global<usize> = Global::new(0);

unsafe extern "system" fn g_thread(_p: *mut c_void) -> u32 {
    // SAFETY: plain calls.
    unsafe {
        logln!("other thread: {}", dp_which(GetThreadDpiAwarenessContext()));
    }
    0
}

fn test(t: &mut Test) {
    // SAFETY: plain calls; the thread is joined.
    unsafe {
        let r = SetProcessDpiAwarenessContext(DPI_SYSTEM_AWARE);
        say!(t, "SetProcessDpiAwarenessContext(SYSTEM_AWARE) -> {}", r);
        say!(
            t,
            "thread {} ; process {}",
            dp_which(GetThreadDpiAwarenessContext()),
            dp_which(GetDpiAwarenessContextForProcess(GetCurrentProcess()))
        );
        SetThreadDpiAwarenessContext(DPI_PER_MONITOR_AWARE_V2);
        say!(
            t,
            "after SetThread(PER_MONITOR_AWARE_V2): thread {} ; process {}",
            dp_which(GetThreadDpiAwarenessContext()),
            dp_which(GetDpiAwarenessContextForProcess(GetCurrentProcess()))
        );
        let th = CreateThread(core::ptr::null_mut(), 0, g_thread, core::ptr::null_mut(), 0, core::ptr::null_mut());
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
        flush_log(t, "  ");
        let w = dp_popup(wz!("GtdacProcess"));
        say!(t, "window made now: {}", dp_wa(w));
        SetThreadDpiAwarenessContext(DPI_UNAWARE);
        let w2 = dp_popup(wz!("GtdacProcess"));
        say!(t, "window made after UNAWARE: {} ; the first still {}", dp_wa(w2), dp_wa(w));
        DestroyWindow(w2);
        DestroyWindow(w);
    }
}
