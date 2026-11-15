//! SendDlgItemMessageA to the standard controls of a dialog: messages that
//! carry strings are converted for the control (WM_SETTEXT, WM_GETTEXT,
//! LB_ADDSTRING, LB_INSERTSTRING, LB_GETTEXT, LB_FINDSTRING, EM_REPLACESEL,
//! EM_GETLINE, CB-less), sizes in bytes, and the results are the same as the
//! wide call's; messages without strings behave as with SendDlgItemMessageW;
//! absent ids give 0.
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
const BM_GETCHECK: u32 = 0x00F0;
const BM_SETCHECK: u32 = 0x00F1;
const EM_SETSEL: u32 = 0x00B1;
const LB_ADDSTRING: u32 = 0x0180;
const LB_INSERTSTRING: u32 = 0x0181;
const LB_SELECTSTRING: u32 = 0x018C;
const LB_FINDSTRING: u32 = 0x018F;
const LB_FINDSTRINGEXACT: u32 = 0x01A2;
const LB_GETCOUNT: u32 = 0x018B;
const LB_GETCURSEL: u32 = 0x0188;
const LB_GETTEXT: u32 = 0x0189;
const LB_GETTEXTLEN: u32 = 0x018A;

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
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn GetWindowTextW(hwnd: HWND, buf: *mut u16, max: i32) -> i32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetWindowTextA(hwnd: HWND, buf: *mut u8, max: i32) -> i32;
    fn DialogBoxParamW(instance: HINSTANCE, name: *const u16, parent: HWND, proc_: DLGPROC, param: isize) -> isize;
    fn EndDialog(hwnd: HWND, result: isize) -> BOOL;
    fn GetDlgItem(hwnd: HWND, id: i32) -> HWND;
    fn SendDlgItemMessageW(hwnd: HWND, id: i32, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendDlgItemMessageA(hwnd: HWND, id: i32, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn RegisterClassExA(class: *const c_void) -> u16;
    fn CreateWindowExA(ex: u32, class: *const u8, name: *const u8, style: u32, x: i32, y: i32, w: i32, h: i32, parent: HWND, menu: HMENU, instance: HINSTANCE, param: *mut c_void) -> HWND;
    fn DefWindowProcA(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
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

fn body(t: &mut Test, d: HWND) {
    // SAFETY: calls on the dialog's windows; buffers are as large as the sizes given.
    unsafe {
        say!(t, "ANSI code page {}", GetACP());
        call!(t, "WM_SETTEXT edit with ANSI bytes", SendDlgItemMessageA(d, 1002, WM_SETTEXT, 0, A_HIGH.as_ptr() as LPARAM));
        say!(t, "  the edit's text as UTF-16: '{}'", wtext_of(GetDlgItem(d, 1002)).as_str());
        let mut buf = [0xEEu8; 100];
        call!(t, "WM_GETTEXT size 100", SendDlgItemMessageA(d, 1002, WM_GETTEXT, 100, buf.as_mut_ptr() as LPARAM));
        say!(t, "  bytes '{}'", abytes(&buf).as_str());
        let mut buf = [0xEEu8; 100];
        call!(t, "WM_GETTEXT size 5", SendDlgItemMessageA(d, 1002, WM_GETTEXT, 5, buf.as_mut_ptr() as LPARAM));
        say!(t, "  bytes '{}'", abytes(&buf).as_str());
        SendDlgItemMessageW(d, 1002, WM_SETTEXT, 0, W_MIXED.as_ptr() as LPARAM);
        call!(t, "WM_GETTEXTLENGTH of a text with U+00E9 U+4E2D U+0416 U+20AC", SendDlgItemMessageA(d, 1002, WM_GETTEXTLENGTH, 0, 0));
        let mut buf = [0xEEu8; 100];
        call!(t, "WM_GETTEXT of that text", SendDlgItemMessageA(d, 1002, WM_GETTEXT, 100, buf.as_mut_ptr() as LPARAM));
        say!(t, "  bytes '{}'", abytes(&buf).as_str());
        // the edit control's own messages
        SendDlgItemMessageA(d, 1002, WM_SETTEXT, 0, b"line one\0".as_ptr() as LPARAM);
        call!(t, "EM_SETSEL all", SendDlgItemMessageA(d, 1002, 0x00B1, 0, -1));
        call!(t, "EM_REPLACESEL with an ANSI text", SendDlgItemMessageA(d, 1002, 0x00C2, 1, b"repl\xE9\0".as_ptr() as LPARAM));
        say!(t, "  the edit's text as UTF-16: '{}'", wtext_of(GetDlgItem(d, 1002)).as_str());
        let mut line = [0u8; 40];
        line[0] = 30;
        line[1] = 0;
        call!(t, "EM_GETLINE 0", SendDlgItemMessageA(d, 1002, 0x00C4, 0, line.as_mut_ptr() as LPARAM));
        say!(t, "  line bytes '{}'", abytes(&line).as_str());
        // the list box
        call!(t, "LB_ADDSTRING 'beta'", SendDlgItemMessageA(d, 1008, LB_ADDSTRING, 0, b"beta\0".as_ptr() as LPARAM));
        call!(t, "LB_ADDSTRING with high bytes", SendDlgItemMessageA(d, 1008, LB_ADDSTRING, 0, b"alpha\xE9\0".as_ptr() as LPARAM));
        call!(t, "LB_INSERTSTRING at 0", SendDlgItemMessageA(d, 1008, LB_INSERTSTRING, 0, b"first\0".as_ptr() as LPARAM));
        call!(t, "LB_GETCOUNT", SendDlgItemMessageA(d, 1008, LB_GETCOUNT, 0, 0));
        for i in 0..3 {
            let mut b = [0xEEu8; 40];
            let mut l = Buf::<80>::new();
            let _ = write!(l, "LB_GETTEXT {i}");
            let n = call!(t, l.as_str(), SendDlgItemMessageA(d, 1008, LB_GETTEXT, i, b.as_mut_ptr() as LPARAM));
            say!(t, "  {n} bytes '{}'", abytes(&b).as_str());
            let mut l = Buf::<80>::new();
            let _ = write!(l, "LB_GETTEXTLEN {i}");
            call!(t, l.as_str(), SendDlgItemMessageA(d, 1008, LB_GETTEXTLEN, i, 0));
        }
        call!(t, "LB_FINDSTRING from -1 'al'", SendDlgItemMessageA(d, 1008, LB_FINDSTRING, usize::MAX, b"al\0".as_ptr() as LPARAM));
        call!(t, "LB_FINDSTRINGEXACT 'beta'", SendDlgItemMessageA(d, 1008, LB_FINDSTRINGEXACT, usize::MAX, b"beta\0".as_ptr() as LPARAM));
        call!(t, "LB_SELECTSTRING 'be'", SendDlgItemMessageA(d, 1008, LB_SELECTSTRING, usize::MAX, b"be\0".as_ptr() as LPARAM));
        call!(t, "LB_GETCURSEL", SendDlgItemMessageA(d, 1008, LB_GETCURSEL, 0, 0));
        // messages without strings, and absent ids
        call!(t, "BM_SETCHECK on the check box", SendDlgItemMessageA(d, 1003, BM_SETCHECK, 1, 0));
        call!(t, "BM_GETCHECK", SendDlgItemMessageA(d, 1003, BM_GETCHECK, 0, 0));
        set_error(0xDEAD);
        call!(t, "WM_GETTEXTLENGTH on an absent id", SendDlgItemMessageA(d, 3000, WM_GETTEXTLENGTH, 0, 0));
        call!(t, "bad dialog handle", SendDlgItemMessageA(0x0bad_0bad as HWND, 1002, WM_GETTEXTLENGTH, 0, 0));
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    with_dialog(t, 100, body);
}
