//! IsDialogMessageA: keyboard handling of dialogs on messages made by the
//! test and passed to the call: Tab, Shift+Tab (by key state), arrows in a
//! radio group, Enter (the default button: WM_COMMAND IDOK to the dialog),
//! Escape (IDCANCEL), mnemonic characters; messages that are not for the
//! dialog are not processed (FALSE).
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
type WPARAM = usize;
type LPARAM = isize;
type BOOL = i32;
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

const WM_NULL: u32 = 0x0000;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_SYSCHAR: u32 = 0x0106;
const WM_COMMAND: u32 = 0x0111;
const WM_MOUSEMOVE: u32 = 0x0200;
const GWLP_USERDATA: i32 = -21;
const VK_RETURN: usize = 0x0D;
const VK_ESCAPE: usize = 0x1B;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetDesktopWindow() -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetFocus() -> HWND;
    fn SetFocus(hwnd: HWND) -> HWND;
    fn DialogBoxParamW(instance: HINSTANCE, name: *const u16, parent: HWND, proc_: DLGPROC, param: isize) -> isize;
    fn EndDialog(hwnd: HWND, result: isize) -> BOOL;
    fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
    fn GetDlgCtrlID(hwnd: HWND) -> i32;
    fn IsDlgButtonChecked(hwnd: HWND, id: i32) -> u32;
    fn IsDialogMessageA(hwnd: HWND, msg: *mut MSG) -> BOOL;
    fn DialogBoxParamA(instance: HINSTANCE, name: *const u8, parent: HWND, proc_: DLGPROC, param: isize) -> isize;
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
    ".long 2147483792\n",
    ".long 0, 0, 0\n",
    ".short 1, 8\n",
    ".long 2147484192\n",
    ".long 2147483816\n",
    ".long 100\n",
    ".long 2147483840\n",
    ".long 101\n",
    ".long 2147483864\n",
    ".long 102\n",
    ".long 2147483888\n",
    ".long 103\n",
    ".long 2147483912\n",
    ".long 104\n",
    ".long 2147483936\n",
    ".long 110\n",
    ".long 2147483960\n",
    ".long 111\n",
    ".long 2147483984\n",
    ".long 112\n",
    ".long 2147484008\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 384\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 400\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 416\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 432\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 448\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 464\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 480\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 496\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 512\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 528\n",
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
    ".rva .Lrsrc_d7\n",
    ".long 390, 0, 0\n",
    ".rva .Lrsrc_d8\n",
    ".long 246, 0, 0\n",
    ".rva .Lrsrc_d9\n",
    ".long 246, 0, 0\n",
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
    ".balign 4\n",
    ".Lrsrc_d7:\n",
    ".byte 192, 0, 200, 128, 0, 0, 0, 0, 9, 0, 10, 0, 10, 0, 160, 0, 90, 0, 0, 0, 0, 0, 77, 0\n",
    ".byte 110, 0, 101, 0, 109, 0, 111, 0, 110, 0, 105, 0, 99, 0, 115, 0, 32, 0, 233, 0, 45, 78, 0, 0\n",
    ".byte 8, 0, 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0\n",
    ".byte 103, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 10, 0, 209, 7, 255, 255\n",
    ".byte 130, 0, 38, 0, 220, 0, 98, 0, 101, 0, 114, 0, 58, 0, 0, 0, 0, 0, 0, 0, 128, 0, 129, 80\n",
    ".byte 0, 0, 0, 0, 70, 0, 5, 0, 60, 0, 12, 0, 210, 7, 255, 255, 129, 0, 111, 0, 110, 0, 101, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 20, 0, 60, 0, 10, 0, 211, 7, 255, 255\n",
    ".byte 130, 0, 38, 0, 233, 0, 116, 0, 233, 0, 58, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0\n",
    ".byte 70, 0, 20, 0, 60, 0, 12, 0, 212, 7, 255, 255, 129, 0, 116, 0, 119, 0, 111, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 80, 0, 0, 0, 0, 5, 0, 35, 0, 60, 0, 10, 0, 213, 7, 255, 255, 130, 0, 38, 0\n",
    ".byte 45, 78, 58, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 70, 0, 35, 0, 60, 0, 12, 0\n",
    ".byte 214, 7, 255, 255, 129, 0, 116, 0, 104, 0, 114, 0, 101, 0, 101, 0, 0, 0, 0, 0, 3, 0, 1, 80\n",
    ".byte 0, 0, 0, 0, 5, 0, 50, 0, 60, 0, 10, 0, 215, 7, 255, 255, 128, 0, 38, 0, 67, 0, 97, 0\n",
    ".byte 102, 0, 233, 0, 0, 0, 0, 0, 1, 0, 3, 80, 0, 0, 0, 0, 100, 0, 70, 0, 50, 0, 14, 0\n",
    ".byte 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0\n",
    ".byte 45, 0, 70, 0, 50, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0\n",
    ".byte 108, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d8:\n",
    ".byte 192, 0, 192, 128, 0, 0, 0, 0, 4, 0, 10, 0, 10, 0, 120, 0, 60, 0, 0, 0, 68, 0, 108, 0\n",
    ".byte 103, 0, 80, 0, 114, 0, 111, 0, 99, 0, 84, 0, 101, 0, 115, 0, 116, 0, 65, 0, 0, 0, 79, 0\n",
    ".byte 119, 0, 110, 0, 32, 0, 99, 0, 108, 0, 97, 0, 115, 0, 115, 0, 32, 0, 65, 0, 0, 0, 8, 0\n",
    ".byte 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0\n",
    ".byte 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 12, 0, 185, 11, 255, 255\n",
    ".byte 129, 0, 102, 0, 105, 0, 114, 0, 115, 0, 116, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0\n",
    ".byte 5, 0, 20, 0, 60, 0, 12, 0, 186, 11, 255, 255, 129, 0, 115, 0, 101, 0, 99, 0, 111, 0, 110, 0\n",
    ".byte 100, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 70, 0, 40, 0, 40, 0, 14, 0\n",
    ".byte 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0\n",
    ".byte 25, 0, 40, 0, 40, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0\n",
    ".byte 108, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d9:\n",
    ".byte 192, 0, 192, 128, 0, 0, 0, 0, 4, 0, 10, 0, 10, 0, 120, 0, 60, 0, 0, 0, 68, 0, 108, 0\n",
    ".byte 103, 0, 80, 0, 114, 0, 111, 0, 99, 0, 84, 0, 101, 0, 115, 0, 116, 0, 87, 0, 0, 0, 79, 0\n",
    ".byte 119, 0, 110, 0, 32, 0, 99, 0, 108, 0, 97, 0, 115, 0, 115, 0, 32, 0, 87, 0, 0, 0, 8, 0\n",
    ".byte 77, 0, 83, 0, 32, 0, 83, 0, 104, 0, 101, 0, 108, 0, 108, 0, 32, 0, 68, 0, 108, 0, 103, 0\n",
    ".byte 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0, 5, 0, 5, 0, 60, 0, 12, 0, 185, 11, 255, 255\n",
    ".byte 129, 0, 102, 0, 105, 0, 114, 0, 115, 0, 116, 0, 0, 0, 0, 0, 128, 0, 129, 80, 0, 0, 0, 0\n",
    ".byte 5, 0, 20, 0, 60, 0, 12, 0, 186, 11, 255, 255, 129, 0, 115, 0, 101, 0, 99, 0, 111, 0, 110, 0\n",
    ".byte 100, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 80, 0, 0, 0, 0, 70, 0, 40, 0, 40, 0, 14, 0\n",
    ".byte 1, 0, 255, 255, 128, 0, 79, 0, 75, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 80, 0, 0, 0, 0\n",
    ".byte 25, 0, 40, 0, 40, 0, 14, 0, 2, 0, 255, 255, 128, 0, 67, 0, 97, 0, 110, 0, 99, 0, 101, 0\n",
    ".byte 108, 0, 0, 0, 0, 0\n",
);

static GOT: Global<[u32; 4]> = Global::new([0; 4]);

fn focus_id(t: &mut Test, label: &str) {
    // SAFETY: plain query.
    let f = unsafe { GetFocus() };
    say!(t, "{label}: focus on control {}", if f.is_null() { -1 } else { unsafe { GetDlgCtrlID(f) } });
}

fn key(t: &mut Test, d: HWND, to: i32, msg: u32, w: usize, label: &str) {
    // SAFETY: calls on the dialog's windows with a message of our own.
    unsafe {
        let mut m: MSG = zeroed();
        m.hwnd = if to == 0 { d } else { GetDlgItem(d, to) };
        m.message = msg;
        m.wparam = w;
        m.lparam = 0x0001_0001;
        log_clear();
        let r = IsDialogMessageA(d, &mut m);
        say!(t, "{label}: IsDialogMessageA = {r}");
        flush_log(t, "     ");
    }
}

/// The dialog logs the commands it gets (a hook on the procedure is not possible: the
/// WM_COMMAND goes to the dialog procedure of this test, run_cmd_proc).
unsafe extern "system" fn cmd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> isize {
    if msg == WM_COMMAND {
        logln!("WM_COMMAND id {} notify {} from {}", w & 0xFFFF, (w >> 16) & 0xFFFF, if l == 0 { 0 } else { unsafe { GetDlgCtrlID(l as HWND) } });
        return 1;
    }
    if msg == WM_INITDIALOG {
        // SAFETY: the test body runs inside the dialog.
        let t = unsafe { &mut *(*CUR_T.get() as *mut Test) };
        body(t, hwnd);
        unsafe { EndDialog(hwnd, 1) };
        return 0;
    }
    0
}

fn body(t: &mut Test, d: HWND) {
    // SAFETY: calls on the dialog's windows.
    unsafe {
        SetFocus(GetDlgItem(d, 1002));
        focus_id(t, "start");
        key(t, d, 1002, WM_KEYDOWN, 0x09, "Tab on the edit");
        focus_id(t, "after Tab");
        key(t, d, 1003, WM_KEYDOWN, 0x09, "Tab on the check box");
        focus_id(t, "after Tab");
        key(t, d, 1004, WM_KEYDOWN, 0x09, "Tab on the first radio");
        focus_id(t, "after Tab (the next tab stop after the radio group)");
        // arrow keys in the radio group
        SetFocus(GetDlgItem(d, 1004));
        key(t, d, 1004, WM_KEYDOWN, 0x28, "Down on radio 1");
        focus_id(t, "after Down");
        say!(t, "radios: {} {} {}", IsDlgButtonChecked(d, 1004), IsDlgButtonChecked(d, 1005), IsDlgButtonChecked(d, 1006));
        key(t, d, 1005, WM_KEYDOWN, 0x27, "Right on radio 2");
        focus_id(t, "after Right");
        key(t, d, 1006, WM_KEYDOWN, 0x28, "Down on the last radio (wraps)");
        focus_id(t, "after Down");
        key(t, d, 1006, WM_KEYDOWN, 0x26, "Up");
        focus_id(t, "after Up");
        // Enter and Escape
        SetFocus(GetDlgItem(d, 1002));
        key(t, d, 1002, WM_KEYDOWN, VK_RETURN as usize, "Enter on the edit");
        key(t, d, 1002, WM_KEYDOWN, VK_ESCAPE as usize, "Escape on the edit");
        SetFocus(GetDlgItem(d, 2));
        key(t, d, 2, WM_KEYDOWN, VK_RETURN as usize, "Enter on the Cancel button");
        focus_id(t, "after");
        key(t, d, 1002, WM_KEYUP, VK_RETURN as usize, "Enter key up");
        // mnemonics
        SetFocus(GetDlgItem(d, 1002));
        key(t, d, 1002, WM_SYSCHAR, b'c' as usize, "Alt+C (the check box)");
        focus_id(t, "after");
        say!(t, "check box state {}", IsDlgButtonChecked(d, 1003));
        key(t, d, 1002, WM_SYSCHAR, b'n' as usize, "Alt+N (the static: focus goes to the next tab stop)");
        focus_id(t, "after");
        key(t, d, 1002, WM_SYSCHAR, b'w' as usize, "Alt+W (radio two)");
        focus_id(t, "after");
        say!(t, "radios: {} {} {}", IsDlgButtonChecked(d, 1004), IsDlgButtonChecked(d, 1005), IsDlgButtonChecked(d, 1006));
        key(t, d, 1002, WM_SYSCHAR, b'x' as usize, "Alt+X (no such mnemonic)");
        key(t, d, 1002, WM_CHAR, b'a' as usize, "a plain character");
        // not for the dialog
        let mut m: MSG = zeroed();
        m.hwnd = core::ptr::null_mut();
        m.message = WM_KEYDOWN;
        m.wparam = 9;
        say!(t, "message with a NULL window: IsDialogMessageA = {}", IsDialogMessageA(d, &mut m));
        m.hwnd = GetDesktopWindow();
        say!(t, "message to the desktop window: IsDialogMessageA = {}", IsDialogMessageA(d, &mut m));
        say!(t, "bad dialog handle: IsDialogMessageA = {}", IsDialogMessageA(0x0bad_0bad as HWND, &mut m));
        call!(t, "WM_NULL to a control", { m.hwnd = GetDlgItem(d, 1002); m.message = WM_NULL; IsDialogMessageA(d, &mut m) });
        call!(t, "WM_MOUSEMOVE to a control", { m.hwnd = GetDlgItem(d, 1002); m.message = WM_MOUSEMOVE; IsDialogMessageA(d, &mut m) });
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    *CUR_T.get() = t as *mut Test as usize;
    // SAFETY: a resource dialog of this executable.
    let r = unsafe { DialogBoxParamA(instance(), mira(100), core::ptr::null_mut(), Some(cmd_proc), 0) };
    say!(t, "dialog result {r}");
    flush_log(t, "  ");
}
