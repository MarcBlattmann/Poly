//! GetKeyboardLayout (16 calls in a Chrome run): the layout (HKL) of the
//! calling thread (thread id 0) and of a thread by id. Only relations are
//! printed, never the handle: it is not 0, equal on repeated calls, its low
//! word (the language identifier) is not 0, the own thread id gives the same
//! as 0, the low word equals the last four hexadecimal digits of
//! GetKeyboardLayoutNameW, the call leaves the last error alone; an unknown
//! thread id (0xFFFFFFF0 and 0xFFFFFFFF) gives what? (printed as zero /
//! equal to ours / other); a thread of the shell (another process); another
//! thread of this process that has no window and is still running; the same
//! thread id after the thread has ended.
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
type BOOL = i32;
type THREADPROC = unsafe extern "system" fn(*mut c_void) -> u32;
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

fn lo(v: isize) -> i32 {
    (v as u32 & 0xFFFF) as u16 as i16 as i32
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

type HKL = *mut c_void;
/// The value of up to 8 hexadecimal digits (0-9, A-F upper case only) of a
/// wide string, or None when a character is none of them. The string is
/// read up to its NUL or the end of the slice.
fn hex_of(buf: &[u16]) -> Option<u32> {
    let mut v = 0u32;
    let mut n = 0;
    for &c in buf {
        if c == 0 {
            break;
        }
        let d = match c {
            0x30..=0x39 => c - 0x30,
            0x41..=0x46 => c - 0x41 + 10,
            _ => return None,
        };
        v = (v << 4) | d as u32;
        n += 1;
    }
    if n == 0 || n > 8 { None } else { Some(v) }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentThreadId() -> u32;
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
    fn Sleep(ms: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetKeyboardLayout(thread: u32) -> HKL;
    fn GetKeyboardLayoutNameW(buf: *mut u16) -> BOOL;
    fn GetShellWindow() -> HWND;
}

unsafe extern "system" fn thread_sleep(_p: *mut c_void) -> u32 {
    // SAFETY: plain call.
    unsafe { Sleep(600) };
    0
}

fn rel(v: HKL, ours: HKL) -> &'static str {
    if v.is_null() {
        "zero"
    } else if v == ours {
        "equal to ours"
    } else {
        "other"
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls throughout; the thread is ours; buffers are sized.
    unsafe {
        set_error(0xBEEF);
        let me = GetKeyboardLayout(0);
        say!(t, "last error after GetKeyboardLayout(0): {:#x}", last_error());
        say!(t, "GetKeyboardLayout(0) is not zero: {}", (!me.is_null()) as i32);
        say!(t, "a second call is equal: {}", (GetKeyboardLayout(0) == me) as i32);
        let lo = me as usize & 0xFFFF;
        say!(t, "low word (language identifier) is not zero: {}", (lo != 0) as i32);
        say!(t, "GetKeyboardLayout(own thread id) equals the call with 0: {}", (GetKeyboardLayout(GetCurrentThreadId()) == me) as i32);

        let mut name = [0u16; 16];
        let ok = GetKeyboardLayoutNameW(name.as_mut_ptr());
        let nv = hex_of(&name);
        say!(t, "GetKeyboardLayoutNameW = {ok}, a hexadecimal name: {}", nv.is_some() as i32);
        say!(t, "low word of the layout equals the name's last four digits: {}", (nv.map(|v| v as usize & 0xFFFF) == Some(lo)) as i32);

        let mut count = 0;
        for _ in 0..16 {
            count += (GetKeyboardLayout(0) == me) as i32;
        }
        say!(t, "16 calls in a row all equal: {}", (count == 16) as i32);

        say!(t, "== thread ids that are not ours");
        set_error(0);
        let r = GetKeyboardLayout(0xFFFF_FFF0);
        say!(t, "  unknown thread id 0xFFFFFFF0: {} (last error {})", rel(r, me), last_error());
        set_error(0);
        let r = GetKeyboardLayout(0xFFFF_FFFF);
        say!(t, "  thread id 0xFFFFFFFF: {} (last error {})", rel(r, me), last_error());

        let sh = GetShellWindow();
        if !sh.is_null() {
            let mut pid = 0u32;
            let tid = GetWindowThreadProcessId(sh, &mut pid);
            let r = GetKeyboardLayout(tid);
            say!(t, "  a thread of the shell (another process): {}", rel(r, me));
        }

        let mut id = 0u32;
        let th = CreateThread(core::ptr::null_mut(), 0, thread_sleep, core::ptr::null_mut(), 0, &mut id);
        say!(t, "  a running thread of this process without a window: {}", rel(GetKeyboardLayout(id), me));
        WaitForSingleObject(th, 5000);
        CloseHandle(th);
        set_error(0);
        let r = GetKeyboardLayout(id);
        say!(t, "  the same thread id after it ended: {} (last error {})", rel(r, me), last_error());
    }
}
