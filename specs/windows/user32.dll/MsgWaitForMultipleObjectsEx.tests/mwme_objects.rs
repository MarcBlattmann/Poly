//! MsgWaitForMultipleObjectsEx and its objects, with no window and no message
//! in the queue: the return values (WAIT_OBJECT_0 + index, WAIT_OBJECT_0 +
//! count for input, WAIT_TIMEOUT, WAIT_ABANDONED_0 + index, WAIT_FAILED), the
//! lowest signaled index winning, MWMO_WAITALL, zero objects, the limit on
//! the count, bad handles and flags, timeouts of 0 and of a short time
//! (only that they pass: the elapsed time is at least the timeout and far
//! below a second), a wait ended by another thread's SetEvent, a mutex
//! abandoned by a thread that exited, and the wait on a semaphore-like event
//! that is auto-reset (consumed by the wait).
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
type HINSTANCE = *mut c_void;
type HMENU = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type BOOL = i32;
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

const MWMO_WAITALL: u32 = 1;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;
const WAIT_ABANDONED_0: u32 = 0x80;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CreateThread(
        attributes: *mut c_void,
        stack: usize,
        start: THREADPROC,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn CreateEventW(attributes: *mut c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(event: HANDLE) -> BOOL;
    fn ResetEvent(event: HANDLE) -> BOOL;
    fn WaitForSingleObject(handle: HANDLE, ms: u32) -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn Sleep(ms: u32);
    fn GetTickCount() -> u32;
    fn CreateMutexW(attributes: *mut c_void, owner: BOOL, name: *const u16) -> HANDLE;
    fn ReleaseMutex(mutex: HANDLE) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
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
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn MsgWaitForMultipleObjectsEx(n: u32, handles: *const HANDLE, ms: u32, mask: u32, flags: u32) -> u32;
}

poly_wtest::main!(test);

unsafe extern "system" fn setter(param: *mut c_void) -> u32 {
    // SAFETY: the event handle outlives the thread.
    unsafe {
        Sleep(120);
        SetEvent(param);
    }
    0
}

unsafe extern "system" fn grabber(param: *mut c_void) -> u32 {
    // SAFETY: the mutex handle outlives the thread; it ends owning it.
    unsafe {
        WaitForSingleObject(param, 5000);
    }
    0
}

fn test(t: &mut Test) {
    // SAFETY: handles we create; no window exists in this test.
    unsafe {
        let none: *const HANDLE = core::ptr::null();
        // no queue activity: the thread has no messages
        let mut msg: MSG = zeroed();
        while PeekMessageW(&mut msg, core::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {}
        set_error(0xDEAD);
        let r = MsgWaitForMultipleObjectsEx(0, none, 0, QS_ALLINPUT, 0);
        say!(t, "no objects, empty queue, timeout 0: {r:#x}, last error left alone {}", last_error() == 0xDEAD);
        let t0 = GetTickCount();
        let r = MsgWaitForMultipleObjectsEx(0, none, 150, QS_ALLINPUT, 0);
        let el = GetTickCount().wrapping_sub(t0);
        say!(t, "no objects, empty queue, timeout 150: {r:#x}, took at least 140 ms {}, less than a second {}", el >= 140, el < 1000);

        let e1 = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
        let e2 = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
        let e3 = CreateEventW(core::ptr::null_mut(), 0, 0, core::ptr::null());
        let hs = [e1, e2, e3];
        say!(t, "three unsignaled events, timeout 0: {:#x}", MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, 0));
        SetEvent(e2);
        say!(t, "e2 signaled: {:#x} (index 1)", MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, 0));
        SetEvent(e1);
        say!(t, "e1 and e2 signaled: {:#x} (the lowest index)", MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, 0));
        ResetEvent(e1);
        ResetEvent(e2);
        SetEvent(e3);
        say!(t, "e3 (auto-reset) signaled: {:#x}, then {:#x} (consumed by the wait)", MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, 0), MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, 0));
        // MWMO_WAITALL
        SetEvent(e1);
        say!(t, "WAITALL with only e1 signaled: {:#x}", MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, MWMO_WAITALL));
        SetEvent(e2);
        SetEvent(e3);
        say!(t, "WAITALL with all signaled: {:#x}", MsgWaitForMultipleObjectsEx(3, hs.as_ptr(), 0, QS_ALLINPUT, MWMO_WAITALL));
        ResetEvent(e1);
        ResetEvent(e2);
        // the mask 0
        SetEvent(e1);
        say!(t, "input mask 0, e1 signaled: {:#x}", MsgWaitForMultipleObjectsEx(1, hs.as_ptr(), 0, 0, 0));
        ResetEvent(e1);
        say!(t, "input mask 0, nothing signaled: {:#x}", MsgWaitForMultipleObjectsEx(1, hs.as_ptr(), 0, 0, 0));
        // a wait ended by another thread
        let ev = CreateEventW(core::ptr::null_mut(), 1, 0, core::ptr::null());
        let h = CreateThread(core::ptr::null_mut(), 0, setter, ev, 0, core::ptr::null_mut());
        let t0 = GetTickCount();
        let r = MsgWaitForMultipleObjectsEx(1, &ev, 5000, QS_ALLINPUT, 0);
        let el = GetTickCount().wrapping_sub(t0);
        say!(t, "SetEvent from another thread after 120 ms ends a 5 s wait: {r:#x}, took between 100 ms and 2 s {}", el >= 100 && el < 2000);
        WaitForSingleObject(h, 5000);
        CloseHandle(h);
        // errors
        set_error(0xDEAD);
        let r = MsgWaitForMultipleObjectsEx(1, &(0x1234usize as HANDLE), 0, QS_ALLINPUT, 0);
        say!(t, "an invalid handle: {r:#x}, last error {}", last_error());
        set_error(0xDEAD);
        let r = MsgWaitForMultipleObjectsEx(1, &(core::ptr::null_mut::<c_void>()), 0, QS_ALLINPUT, 0);
        say!(t, "a NULL handle: {r:#x}, last error {}", last_error());
        let many: [HANDLE; 64] = [e1; 64];
        for n in [62u32, 63, 64, 65] {
            set_error(0xDEAD);
            let r = MsgWaitForMultipleObjectsEx(n, many.as_ptr(), 0, QS_ALLINPUT, 0);
            say!(t, "{n} handles (e1 unsignaled): {r:#x}, last error {}", if r == WAIT_FAILED { last_error() } else { 0 });
        }
        set_error(0xDEAD);
        let r = MsgWaitForMultipleObjectsEx(1, hs.as_ptr(), 0, QS_ALLINPUT, 0x80);
        say!(t, "an unknown flag 0x80: {r:#x}, last error {}", if r == WAIT_FAILED { last_error() } else { 0 });
        set_error(0xDEAD);
        let r = MsgWaitForMultipleObjectsEx(1, hs.as_ptr(), 0, 0x8000, 0);
        say!(t, "an unknown mask bit 0x8000: {r:#x}, last error {}", if r == WAIT_FAILED { last_error() } else { 0 });
        // a mutex abandoned by a thread
        let mx = CreateMutexW(core::ptr::null_mut(), 0, core::ptr::null());
        let h = CreateThread(core::ptr::null_mut(), 0, grabber, mx, 0, core::ptr::null_mut());
        WaitForSingleObject(h, 5000);
        CloseHandle(h);
        let ms = [e1, mx];
        let r = MsgWaitForMultipleObjectsEx(2, ms.as_ptr(), 0, QS_ALLINPUT, 0);
        say!(t, "a mutex abandoned by an exited thread (index 1): {r:#x} (WAIT_ABANDONED_0 + 1 is {:#x})", WAIT_ABANDONED_0 + 1);
        ReleaseMutex(mx);
        for h in [e1, e2, e3, ev, mx] {
            CloseHandle(h);
        }
    }
}
