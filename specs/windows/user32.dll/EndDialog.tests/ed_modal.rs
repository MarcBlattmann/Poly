//! EndDialog in a modal dialog: the dialog still exists after the call
//! until the procedure returns, messages are still handled, a second call
//! changes the result, EndDialog on windows that are not dialogs and on bad
//! handles.
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
type WPARAM = usize;
type LPARAM = isize;
type BOOL = i32;
const WM_DESTROY: u32 = 0x0002;
const WM_APP: u32 = 0x8000;
const WS_POPUP: u32 = 0x8000_0000;
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

type DLGPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> isize>;
const WM_INITDIALOG: u32 = 0x0110;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetClassNameW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
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
    fn IsWindowVisible(hwnd: HWND) -> BOOL;
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn IsWindow(hwnd: HWND) -> BOOL;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    fn GetSystemMetrics(index: i32) -> i32;
    fn DialogBoxParamW(instance: HINSTANCE, name: *const u16, parent: HWND, proc_: DLGPROC, param: isize) -> isize;
    fn EndDialog(hwnd: HWND, result: isize) -> BOOL;
    fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
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

// ---- dialog helpers of these tests ----

static CUR_T: Global<usize> = Global::new(0);
static CUR_BODY: Global<Option<fn(&mut Test, HWND)>> = Global::new(None);
static CUR_PARAM: Global<isize> = Global::new(0);

/// A dialog procedure that runs the body of the test inside WM_INITDIALOG and
/// ends the dialog with `result` (the body may call EndDialog itself first).
unsafe extern "system" fn run_proc(hwnd: HWND, msg: u32, _w: WPARAM, l: LPARAM) -> isize {
    if msg == WM_INITDIALOG {
        *CUR_PARAM.get() = l;
        if let Some(f) = *CUR_BODY.get() {
            // SAFETY: CUR_T holds the address of the running test's `Test`, alive for the whole dialog.
            let t = unsafe { &mut *(*CUR_T.get() as *mut Test) };
            f(t, hwnd);
        }
        // SAFETY: the dialog window.
        unsafe { EndDialog(hwnd, 99) };
        return 0;
    }
    0
}

/// Show the dialog resource `id` modally and run `body` inside it.
fn with_dialog(t: &mut Test, id: usize, body: fn(&mut Test, HWND)) -> isize {
    *CUR_T.get() = t as *mut Test as usize;
    *CUR_BODY.get() = Some(body);
    // SAFETY: a resource dialog of this executable; run_proc is a dialog procedure.
    unsafe { DialogBoxParamW(instance(), mir(id), core::ptr::null_mut(), Some(run_proc), 7) }
}

/// The class name of a window as ASCII.
fn class_of(h: HWND, out: &mut Buf<300>) {
    let mut b = [0u16; 60];
    // SAFETY: b has 60 units.
    unsafe { GetClassNameW(h, b.as_mut_ptr(), 60) };
    ascii_of(&b, out);
}

/// The text of a window as ASCII.
fn text_of(h: HWND, out: &mut Buf<300>) {
    let mut b = [0u16; 200];
    // SAFETY: b has 200 units.
    unsafe { GetWindowTextW(h, b.as_mut_ptr(), 200) };
    ascii_of(&b, out);
}

core::arch::global_asm!(
    ".section .rsrc$01,\"dr\"\n",
    ".balign 4\n",
    ".long 0, 0, 0\n",
    ".short 0, 2\n",
    ".long 4\n",
    ".long 2147483680\n",
    ".long 5\n",
    ".long 2147483704\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 200\n",
    ".long 2147483768\n",
    ".long 0, 0, 0\n",
    ".short 1, 5\n",
    ".long 2147484048\n",
    ".long 2147483792\n",
    ".long 100\n",
    ".long 2147483816\n",
    ".long 101\n",
    ".long 2147483840\n",
    ".long 102\n",
    ".long 2147483864\n",
    ".long 103\n",
    ".long 2147483888\n",
    ".long 104\n",
    ".long 2147483912\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 288\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 304\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 320\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 336\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 352\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 368\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 384\n",
    ".rva .Lrsrc_d0\n",
    ".long 32, 0, 0\n",
    ".rva .Lrsrc_d1\n",
    ".long 64, 0, 0\n",
    ".rva .Lrsrc_d2\n",
    ".long 480, 0, 0\n",
    ".rva .Lrsrc_d3\n",
    ".long 312, 0, 0\n",
    ".rva .Lrsrc_d4\n",
    ".long 24, 0, 0\n",
    ".rva .Lrsrc_d5\n",
    ".long 114, 0, 0\n",
    ".rva .Lrsrc_d6\n",
    ".long 88, 0, 0\n",
    ".short 8\n",
    ".short 78, 65, 77, 69, 68, 68, 76, 71\n",
    ".section .rsrc$02,\"dr\"\n",
    ".balign 4\n",
    ".Lrsrc_d0:\n",
    ".byte 0, 0, 0, 0, 144, 0, 38, 0, 70, 0, 105, 0, 108, 0, 101, 0, 0, 0, 128, 0, 45, 1, 79, 0\n",
    ".byte 112, 0, 101, 0, 110, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d1:\n",
    ".byte 128, 0, 0, 128, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 50, 0, 30, 0, 0, 0, 0, 0, 110, 0\n",
    ".byte 97, 0, 109, 0, 101, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0\n",
    ".byte 20, 0, 10, 0, 5, 0, 255, 255, 130, 0, 110, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d2:\n",
    ".byte 192, 0, 200, 128, 0, 0, 0, 0, 11, 0, 10, 0, 10, 0, 160, 0, 120, 0, 0, 0, 0, 0, 84, 0\n",
    ".byte 101, 0, 115, 0, 116, 0, 32, 0, 100, 0, 105, 0, 97, 0, 108, 0, 111, 0, 103, 0, 0, 0, 8, 0\n",
    ".byte 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 10, 0, 233, 3, 255, 255\n",
    ".byte 130, 0, 38, 0, 78, 0, 97, 0, 109, 0, 101, 0, 58, 0, 0, 0, 0, 0, 0, 0, 128, 0, 129, 80\n",
    ".byte 0, 0, 0, 0, 5, 0, 18, 0, 80, 0, 12, 0, 234, 3, 255, 255, 129, 0, 105, 0, 110, 0, 105, 0\n",
    ".byte 116, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 3, 0, 1, 80, 0, 0, 0, 0, 5, 0, 34, 0\n",
    ".byte 60, 0, 10, 0, 235, 3, 255, 255, 128, 0, 38, 0, 67, 0, 104, 0, 101, 0, 99, 0, 107, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 9, 0, 3, 80, 0, 0, 0, 0, 5, 0, 46, 0, 40, 0, 10, 0, 236, 3, 255, 255\n",
    ".byte 128, 0, 38, 0, 79, 0, 110, 0, 101, 0, 0, 0, 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0\n",
    ".byte 50, 0, 46, 0, 40, 0, 10, 0, 237, 3, 255, 255, 128, 0, 84, 0, 38, 0, 119, 0, 111, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 9, 0, 0, 80, 0, 0, 0, 0, 95, 0, 46, 0, 40, 0, 10, 0, 238, 3, 255, 255\n",
    ".byte 128, 0, 84, 0, 38, 0, 104, 0, 114, 0, 101, 0, 101, 0, 0, 0, 0, 0, 0, 0, 6, 0, 3, 80\n",
    ".byte 0, 0, 0, 0, 5, 0, 58, 0, 60, 0, 10, 0, 239, 3, 255, 255, 128, 0, 84, 0, 38, 0, 114, 0\n",
    ".byte 105, 0, 0, 0, 0, 0, 0, 0, 1, 0, 129, 80, 0, 0, 0, 0, 5, 0, 70, 0, 60, 0, 30, 0\n",
    ".byte 240, 3, 255, 255, 131, 0, 0, 0, 0, 0, 0, 0, 1, 0, 3, 80, 0, 0, 0, 0, 100, 0, 100, 0\n",
    ".byte 50, 0, 14, 0, 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80\n",
    ".byte 0, 0, 0, 0, 45, 0, 100, 0, 50, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0\n",
    ".byte 99, 0, 101, 0, 108, 0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 80, 0, 0, 0, 0, 90, 0, 5, 0\n",
    ".byte 65, 0, 40, 0, 241, 3, 255, 255, 128, 0, 71, 0, 114, 0, 111, 0, 117, 0, 112, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d3:\n",
    ".byte 1, 0, 255, 255, 99, 0, 0, 0, 0, 0, 0, 0, 192, 0, 192, 128, 5, 0, 10, 0, 10, 0, 100, 0\n",
    ".byte 60, 0, 0, 0, 0, 0, 69, 0, 120, 0, 32, 0, 100, 0, 105, 0, 97, 0, 108, 0, 111, 0, 103, 0\n",
    ".byte 0, 0, 8, 0, 188, 2, 0, 1, 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0\n",
    ".byte 32, 0, 68, 0, 108, 0, 103, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 80\n",
    ".byte 5, 0, 5, 0, 40, 0, 10, 0, 255, 255, 0, 0, 255, 255, 130, 0, 83, 0, 116, 0, 97, 0, 116, 0\n",
    ".byte 105, 0, 99, 0, 32, 0, 45, 0, 49, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 129, 80, 5, 0, 18, 0, 40, 0, 12, 0, 69, 35, 1, 0, 255, 255, 129, 0, 98, 0, 105, 0\n",
    ".byte 103, 0, 32, 0, 105, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80\n",
    ".byte 5, 0, 34, 0, 40, 0, 12, 0, 7, 0, 0, 0, 255, 255, 129, 0, 115, 0, 101, 0, 118, 0, 101, 0\n",
    ".byte 110, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 50, 0, 34, 0\n",
    ".byte 40, 0, 12, 0, 7, 0, 0, 0, 255, 255, 129, 0, 115, 0, 101, 0, 118, 0, 101, 0, 110, 0, 32, 0\n",
    ".byte 116, 0, 111, 0, 111, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80\n",
    ".byte 50, 0, 45, 0, 40, 0, 12, 0, 1, 0, 0, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d4:\n",
    ".byte 128, 0, 0, 128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 0, 30, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d5:\n",
    ".byte 128, 0, 0, 128, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 50, 0, 30, 0, 0, 0, 0, 0, 117, 0\n",
    ".byte 110, 0, 107, 0, 110, 0, 111, 0, 119, 0, 110, 0, 32, 0, 99, 0, 108, 0, 97, 0, 115, 0, 115, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 20, 0, 10, 0, 1, 0, 78, 0\n",
    ".byte 111, 0, 83, 0, 117, 0, 99, 0, 104, 0, 67, 0, 111, 0, 110, 0, 116, 0, 114, 0, 111, 0, 108, 0\n",
    ".byte 67, 0, 108, 0, 97, 0, 115, 0, 115, 0, 0, 0, 120, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d6:\n",
    ".byte 192, 0, 192, 128, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 80, 0, 40, 0, 255, 255, 200, 0, 0, 0\n",
    ".byte 119, 0, 105, 0, 116, 0, 104, 0, 32, 0, 109, 0, 101, 0, 110, 0, 117, 0, 0, 0, 10, 0, 65, 0\n",
    ".byte 114, 0, 105, 0, 97, 0, 108, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0, 5, 0, 5, 0\n",
    ".byte 40, 0, 12, 0, 234, 3, 255, 255, 129, 0, 101, 0, 0, 0, 0, 0\n",
);

unsafe extern "system" fn dp(hwnd: HWND, msg: u32, _w: WPARAM, _l: LPARAM) -> isize {
    // SAFETY: the test runs inside the dialog.
    let t = unsafe { &mut *(*CUR_T.get() as *mut Test) };
    if msg == WM_INITDIALOG {
        // SAFETY: calls on the dialog window.
        unsafe {
            call!(t, "EndDialog(5)", EndDialog(hwnd, 5));
            say!(t, "still a window {} visible {}", IsWindow(hwnd), IsWindowVisible(hwnd));
            call!(t, "EndDialog(6) again", EndDialog(hwnd, 6));
            PostMessageW(hwnd, WM_APP, 0, 0);
            call!(t, "EndDialog on a bad handle", EndDialog(0x0bad_0bad as HWND, 1));
            call!(t, "EndDialog on NULL", EndDialog(core::ptr::null_mut(), 1));
            let c = GetDlgItem(hwnd, 2);
            call!(t, "EndDialog on a control of the dialog", EndDialog(c, 77));
            let plain = create(0, wz!("Static"), wz!("s"), WS_POPUP, 0, 0, 10, 10, core::ptr::null_mut(), 0);
            call!(t, "EndDialog on a window that is not a dialog", EndDialog(plain, 78));
            DestroyWindow(plain);
        }
        return 1;
    }
    if msg == WM_APP {
        logln!("WM_APP arrived after EndDialog");
    }
    if msg == WM_DESTROY {
        logln!("WM_DESTROY");
    }
    0
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    *CUR_T.get() = t as *mut Test as usize;
    log_clear();
    let r = unsafe { DialogBoxParamW(instance(), mir(100), core::ptr::null_mut(), Some(dp), 0) };
    say!(t, "dialog result {r}");
    flush_log(t, "  ");
}
