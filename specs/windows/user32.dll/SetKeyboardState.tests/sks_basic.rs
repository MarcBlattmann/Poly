//! SetKeyboardState (once in a Chrome run) writes the 256-byte key state
//! table of the calling thread (the one GetKeyboardState and GetKeyState
//! read; documented as not affecting other threads). The test first takes a
//! copy with GetKeyboardState, clears the modifier and toggle entries of the
//! copy (Shift, Control, Alt, left/right variants, CapsLock) so that the
//! result does not depend on the machine's state, and puts the copy back
//! with SetKeyboardState. Then: the call's result and last error; a key that
//! is never pressed (VK_F13) set down (0x80): GetKeyState sees it,
//! GetAsyncKeyState and the physical state do not, GetKeyboardState reads
//! it back; a toggle bit (VK_SCROLL) set; Shift set down and the character a
//! posted WM_KEYDOWN for VK_P is translated into by TranslateMessage (the
//! window's WM_CHAR): 'p' with the cleared state, 'P' with Shift, 'P' with
//! CapsLock toggled, U+0010 with Ctrl, and 'p' again; the original table
//! is put back at the end. A NULL pointer is in sks_null.
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
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WS_OVERLAPPED: u32 = 0x0000_0000;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const QS_KEY: u32 = 0x0001;
const QS_MOUSEMOVE: u32 = 0x0002;
const QS_MOUSEBUTTON: u32 = 0x0004;
const QS_POSTMESSAGE: u32 = 0x0008;
const QS_TIMER: u32 = 0x0010;
const QS_PAINT: u32 = 0x0020;
const QS_SENDMESSAGE: u32 = 0x0040;
const QS_HOTKEY: u32 = 0x0080;
const QS_RAWINPUT: u32 = 0x0400;
const QS_INPUT: u32 = QS_MOUSEMOVE | QS_MOUSEBUTTON | QS_KEY | QS_RAWINPUT;
const QS_ALLEVENTS: u32 = QS_INPUT | QS_POSTMESSAGE | QS_TIMER | QS_PAINT | QS_HOTKEY;
const QS_ALLINPUT: u32 = QS_ALLEVENTS | QS_SENDMESSAGE;
const VK_SHIFT: usize = 0x10;
const VK_CONTROL: usize = 0x11;
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

const MAPVK_VK_TO_VSC: u32 = 0;
const VK_SCROLL: i32 = 0x91;
/// Dispatch everything waiting, then keep waiting up to `ms` for anything
/// further (input arrives asynchronously), dispatching as it comes.
fn settle(ms: u32) -> u32 {
    let mut n = pump();
    // SAFETY: plain timing call.
    let start = unsafe { GetTickCount() };
    loop {
        // SAFETY: plain timing call.
        let elapsed = unsafe { GetTickCount() }.wrapping_sub(start);
        if elapsed >= ms {
            break;
        }
        // SAFETY: no handles; wakes on any input or message.
        unsafe { MsgWaitForMultipleObjects(0, core::ptr::null(), 0, ms - elapsed, QS_ALLINPUT) };
        n += pump();
    }
    n
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

const VK_P: usize = 0x50;
/// A character as the tests print it: printable ASCII in quotes, control
/// characters as U+XXXX, anything else (layout dependent) as "non-ASCII".
struct Ch(u32);

impl core::fmt::Display for Ch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            0x20..=0x7E => write!(f, "'{}'", self.0 as u8 as char),
            0..=0x7F => write!(f, "U+{:04X}", self.0),
            _ => f.write_str("non-ASCII"),
        }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn GetTickCount() -> u32;
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
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetKeyState(vk: i32) -> i16;
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn GetKeyboardState(state: *mut u8) -> BOOL;
    fn SetKeyboardState(state: *const u8) -> BOOL;
}

static CHARS: Global<[u16; 4]> = Global::new([0; 4]);
static NCHARS: Global<usize> = Global::new(0);

unsafe extern "system" fn proc_chars(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CHAR {
        let n = *NCHARS.get();
        if n < 4 {
            CHARS.get()[n] = w as u16;
        }
        *NCHARS.get() = n + 1;
        return 0;
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// Post key down and up for a virtual key to a window, let the loop
/// translate and dispatch them, and give back the characters the window
/// received (the first one and how many).
fn typed(h: HWND, vk: usize) -> (u16, usize) {
    *NCHARS.get() = 0;
    *CHARS.get() = [0; 4];
    // SAFETY: our window; plain query.
    unsafe {
        let scan = MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) as isize;
        PostMessageW(h, WM_KEYDOWN, vk, 1 | (scan << 16));
        PostMessageW(h, WM_KEYUP, vk, 1 | (scan << 16) | (3 << 30));
    }
    settle(50);
    (CHARS.get()[0], *NCHARS.get())
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    t.check("registered", register(wz!("KeyState"), Some(proc_chars), 0, 0, core::ptr::null_mut()) != 0, 0);
    let h = create(0, wz!("KeyState"), &[], WS_OVERLAPPED, 0, 0, 10, 10, core::ptr::null_mut(), 0);
    let mut orig = [0u8; 256];
    // SAFETY: orig has 256 bytes throughout.
    unsafe {
        let ok = GetKeyboardState(orig.as_mut_ptr());
        say!(t, "GetKeyboardState = {ok}");
        let mut base = orig;
        for k in [0x10usize, 0x11, 0x12, 0x14, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x7C, 0x91] {
            base[k] = 0;
        }

        set_error(0xBEEF);
        let r = SetKeyboardState(base.as_ptr());
        say!(t, "SetKeyboardState(cleared copy) = {r} (last error {:#x})", last_error());
        let mut now = [0u8; 256];
        GetKeyboardState(now.as_mut_ptr());
        say!(t, "the table reads back with the cleared entries: {}", [0x10usize, 0x11, 0x12, 0x14, 0x7C].iter().all(|&k| now[k] == 0) as i32);

        say!(t, "== VK_F13 set down");
        say!(t, "  GetKeyState down before: {}", (GetKeyState(0x7C) < 0) as i32);
        let mut s = base;
        s[0x7C] = 0x80;
        let r = SetKeyboardState(s.as_ptr());
        say!(t, "  SetKeyboardState = {r}");
        say!(t, "  GetKeyState down: {}", (GetKeyState(0x7C) < 0) as i32);
        say!(t, "  GetAsyncKeyState down: {}", (GetAsyncKeyState(0x7C) < 0) as i32);
        GetKeyboardState(now.as_mut_ptr());
        say!(t, "  GetKeyboardState entry: {:#04x}", now[0x7C]);

        say!(t, "== VK_SCROLL toggle bit set");
        let mut s = base;
        s[0x91] = 1;
        SetKeyboardState(s.as_ptr());
        say!(t, "  GetKeyState low bit: {}, down: {}", (GetKeyState(0x91) & 1) as i32, (GetKeyState(0x91) < 0) as i32);

        say!(t, "== the character a key is translated into");
        SetKeyboardState(base.as_ptr());
        let (c, n) = typed(h, VK_P);
        say!(t, "  cleared table: {} (characters: {n})", Ch(c as u32));
        let mut s = base;
        s[VK_SHIFT] = 0x80;
        SetKeyboardState(s.as_ptr());
        say!(t, "  GetKeyState(VK_SHIFT) down: {}", (GetKeyState(VK_SHIFT as i32) < 0) as i32);
        let (c, n) = typed(h, VK_P);
        say!(t, "  Shift down: {} (characters: {n})", Ch(c as u32));
        let mut s = base;
        s[0x14] = 1;
        SetKeyboardState(s.as_ptr());
        let (c, n) = typed(h, VK_P);
        say!(t, "  CapsLock toggled: {} (characters: {n})", Ch(c as u32));
        let mut s = base;
        s[VK_CONTROL] = 0x80;
        SetKeyboardState(s.as_ptr());
        let (c, n) = typed(h, VK_P);
        say!(t, "  Ctrl down: {} (characters: {n})", Ch(c as u32));
        SetKeyboardState(base.as_ptr());
        let (c, n) = typed(h, VK_P);
        say!(t, "  cleared again: {} (characters: {n})", Ch(c as u32));

        let r = SetKeyboardState(orig.as_ptr());
        say!(t, "the original table put back: {r}");
        DestroyWindow(h);
    }
}
