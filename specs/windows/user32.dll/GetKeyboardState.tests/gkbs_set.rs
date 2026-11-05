//! SetKeyboardState changes the calling thread's key state only: after
//! setting VK_SHIFT and VK_LSHIFT to 0x80 and flipping VK_CAPITAL's toggle
//! bit, GetKeyState and GetKeyboardState report them; GetAsyncKeyState
//! (physical) does not; another thread's GetKeyState does not; a posted
//! WM_KEYDOWN of 'A' (not real input) is translated by TranslateMessage
//! according to the set state (WM_CHAR 'A' with Shift, 'a' with Shift and
//! Caps Lock, 'A' with Caps Lock only); the original array is set back at
//! the end and reads back equal.
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
/// windef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct POINT {
    x: i32,
    y: i32,
}

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
#[derive(Clone, Copy)]
struct MSG {
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    time: u32,
    pt: POINT,
    private: u32,
}

const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const WS_POPUP: u32 = 0x8000_0000;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const VK_SHIFT: usize = 0x10;
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

/// Dispatch every waiting message.
fn pump() -> u32 {
    let mut msg: MSG = zeroed();
    let mut n = 0;
    // SAFETY: msg is writable; it is the message just removed.
    while unsafe { PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) } != 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        n += 1;
    }
    n
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

const VK_CAPITAL: i32 = 0x14;
const VK_LSHIFT: i32 = 0xA0;
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
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    fn GetKeyState(vk: i32) -> i16;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetKeyboardState(state: *mut u8) -> BOOL;
    fn SetKeyboardState(state: *const u8) -> BOOL;
}

static OTHER: Global<i16> = Global::new(0);

unsafe extern "system" fn worker(_p: *mut c_void) -> u32 {
    // SAFETY: plain query.
    *OTHER.get() = unsafe { GetKeyState(VK_SHIFT as i32) };
    0
}

unsafe extern "system" fn proc_k(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CHAR {
        logln!("    WM_CHAR ch={:#06x}", w);
    }
    // SAFETY: the default handling.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

fn post_a(t: &mut Test, h: HWND, label: &str) {
    // SAFETY: our window; scan code 0x1e, repeat 1.
    unsafe { PostMessageW(h, WM_KEYDOWN, 0x41, 0x001E_0001) };
    pump();
    say!(t, "{label}: posted WM_KEYDOWN 'A' translated:");
    flush_log(t, "");
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cls = wz!("GkbsSet");
    t.check("registered", register(cls, Some(proc_k), 0, 0, core::ptr::null_mut()) != 0, 0);
    let h = create(0, cls, wz!(""), WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    let mut saved = [0u8; 256];
    // SAFETY: 256 writable bytes.
    unsafe { GetKeyboardState(saved.as_mut_ptr()) };
    let caps0 = saved[VK_CAPITAL as usize] & 1;
    post_a(t, h, "as found (caps toggle printed separately)");
    say!(t, "caps toggle as found: {caps0}");
    let mut ks = saved;
    for i in 0..256 {
        ks[i] &= 0x01;
    }
    ks[VK_CAPITAL as usize] &= !1;
    // SAFETY: 256 readable bytes.
    let r = unsafe { SetKeyboardState(ks.as_ptr()) };
    say!(t, "SetKeyboardState (nothing down, caps off) returned {r}");
    post_a(t, h, "nothing down, caps off");
    ks[VK_SHIFT as usize] = 0x80;
    ks[VK_LSHIFT as usize] = 0x80;
    // SAFETY: 256 readable bytes.
    let r = unsafe { SetKeyboardState(ks.as_ptr()) };
    say!(t, "SetKeyboardState (Shift down) returned {r}");
    // SAFETY: plain queries.
    let (k, a, l) =
        unsafe { (GetKeyState(VK_SHIFT as i32), GetAsyncKeyState(VK_SHIFT as i32), GetKeyState(VK_LSHIFT)) };
    say!(
        t,
        "GetKeyState(VK_SHIFT)={:#06x} GetKeyState(VK_LSHIFT)={:#06x} async down={}",
        k as u16,
        l as u16,
        a as u16 >> 15
    );
    // SAFETY: the worker only writes OTHER while we wait.
    unsafe {
        let th = CreateThread(core::ptr::null_mut(), 0, worker, core::ptr::null_mut(), 0, core::ptr::null_mut());
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
    }
    say!(t, "another thread's GetKeyState(VK_SHIFT) down={}", *OTHER.get() as u16 >> 15);
    post_a(t, h, "Shift down");
    ks[VK_CAPITAL as usize] |= 1;
    // SAFETY: 256 readable bytes.
    unsafe { SetKeyboardState(ks.as_ptr()) };
    // SAFETY: plain query.
    let c = unsafe { GetKeyState(VK_CAPITAL) };
    say!(t, "caps toggle set: GetKeyState(VK_CAPITAL)={:#06x}", c as u16);
    post_a(t, h, "Shift down, caps on");
    ks[VK_SHIFT as usize] = 0;
    ks[VK_LSHIFT as usize] = 0;
    // SAFETY: 256 readable bytes.
    unsafe { SetKeyboardState(ks.as_ptr()) };
    post_a(t, h, "caps on only");
    let mut back = [0u8; 256];
    // SAFETY: 256 readable/writable bytes.
    unsafe {
        SetKeyboardState(saved.as_ptr());
        GetKeyboardState(back.as_mut_ptr());
    }
    t.check("restored", back == saved, 0);
    // SAFETY: our window.
    unsafe { DestroyWindow(h) };
}
