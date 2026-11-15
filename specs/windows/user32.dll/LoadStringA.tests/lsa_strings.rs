//! LoadStringA: strings of the string table converted to the ANSI code page
//! (the string with U+00C4, U+00F6, U+00FC, U+20AC and a Chinese character
//! gives their ANSI bytes and '?'), the return value counts bytes without the
//! NUL, the buffer size in bytes truncates, a string longer than the buffer,
//! size 0 and a NULL buffer, the highest ids and the second block.
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
    fn LoadStringA(instance: HINSTANCE, id: u32, buf: *mut u8, max: i32) -> i32;
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
    ".short 1, 8\n",
    ".long 2147484936\n",
    ".long 2147483736\n",
    ".long 1\n",
    ".long 2147483768\n",
    ".long 2\n",
    ".long 2147483808\n",
    ".long 3\n",
    ".long 2147483840\n",
    ".long 6\n",
    ".long 2147483872\n",
    ".long 9\n",
    ".long 2147483920\n",
    ".long 10\n",
    ".long 2147483952\n",
    ".long 12\n",
    ".long 2147483992\n",
    ".long 14\n",
    ".long 2147484024\n",
    ".long 0, 0, 0\n",
    ".short 1, 1\n",
    ".long 2147484950\n",
    ".long 2147484056\n",
    ".long 5\n",
    ".long 2147484080\n",
    ".long 0, 0, 0\n",
    ".short 0, 3\n",
    ".long 11\n",
    ".long 2147484104\n",
    ".long 12\n",
    ".long 2147484128\n",
    ".long 13\n",
    ".long 2147484152\n",
    ".long 0, 0, 0\n",
    ".short 1, 1\n",
    ".long 2147484964\n",
    ".long 2147484176\n",
    ".long 400\n",
    ".long 2147484200\n",
    ".long 0, 0, 0\n",
    ".short 0, 2\n",
    ".long 1\n",
    ".long 2147484224\n",
    ".long 2\n",
    ".long 2147484248\n",
    ".long 0, 0, 0\n",
    ".short 0, 4\n",
    ".long 1\n",
    ".long 2147484272\n",
    ".long 2\n",
    ".long 2147484296\n",
    ".long 19\n",
    ".long 2147484320\n",
    ".long 4096\n",
    ".long 2147484344\n",
    ".long 0, 0, 0\n",
    ".short 1, 1\n",
    ".long 2147484982\n",
    ".long 2147484368\n",
    ".long 300\n",
    ".long 2147484392\n",
    ".long 0, 0, 0\n",
    ".short 1, 2\n",
    ".long 2147484994\n",
    ".long 2147484416\n",
    ".long 77\n",
    ".long 2147484440\n",
    ".long 78\n",
    ".long 2147484464\n",
    ".long 0, 0, 0\n",
    ".short 1, 1\n",
    ".long 2147485008\n",
    ".long 2147484488\n",
    ".long 600\n",
    ".long 2147484512\n",
    ".long 0, 0, 0\n",
    ".short 1, 1\n",
    ".long 2147485026\n",
    ".long 2147484536\n",
    ".long 500\n",
    ".long 2147484560\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 936\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 952\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 968\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 984\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1000\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1016\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1032\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1048\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1064\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1080\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1096\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1112\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1128\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1144\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1160\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1176\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1192\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1031\n",
    ".long 1208\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1224\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1240\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1256\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1033\n",
    ".long 1272\n",
    ".rva .Lrsrc_d0\n",
    ".long 17, 0, 0\n",
    ".rva .Lrsrc_d1\n",
    ".long 4, 0, 0\n",
    ".rva .Lrsrc_d2\n",
    ".long 3244, 0, 0\n",
    ".rva .Lrsrc_d3\n",
    ".long 876, 0, 0\n",
    ".rva .Lrsrc_d4\n",
    ".long 3244, 0, 0\n",
    ".rva .Lrsrc_d5\n",
    ".long 88, 0, 0\n",
    ".rva .Lrsrc_d6\n",
    ".long 76, 0, 0\n",
    ".rva .Lrsrc_d7\n",
    ".long 3240, 0, 0\n",
    ".rva .Lrsrc_d8\n",
    ".long 872, 0, 0\n",
    ".rva .Lrsrc_d9\n",
    ".long 740, 0, 0\n",
    ".rva .Lrsrc_d10\n",
    ".long 86, 0, 0\n",
    ".rva .Lrsrc_d11\n",
    ".long 58, 0, 0\n",
    ".rva .Lrsrc_d12\n",
    ".long 60, 0, 0\n",
    ".rva .Lrsrc_d13\n",
    ".long 8, 0, 0\n",
    ".rva .Lrsrc_d14\n",
    ".long 40, 0, 0\n",
    ".rva .Lrsrc_d15\n",
    ".long 10, 0, 0\n",
    ".rva .Lrsrc_d16\n",
    ".long 5, 0, 0\n",
    ".rva .Lrsrc_d17\n",
    ".long 1, 0, 0\n",
    ".rva .Lrsrc_d18\n",
    ".long 20, 0, 0\n",
    ".rva .Lrsrc_d19\n",
    ".long 34, 0, 0\n",
    ".rva .Lrsrc_d20\n",
    ".long 20, 0, 0\n",
    ".rva .Lrsrc_d21\n",
    ".long 34, 0, 0\n",
    ".short 6\n",
    ".short 77, 89, 84, 89, 80, 69\n",
    ".short 6\n",
    ".short 67, 85, 83, 84, 79, 77\n",
    ".short 8\n",
    ".short 78, 65, 77, 69, 68, 66, 77, 80\n",
    ".short 5\n",
    ".short 65, 67, 67, 69, 76\n",
    ".short 6\n",
    ".short 77, 89, 68, 65, 84, 65\n",
    ".short 8\n",
    ".short 78, 65, 77, 69, 68, 67, 85, 82\n",
    ".short 7\n",
    ".short 65, 80, 80, 73, 67, 79, 78\n",
    ".section .rsrc$02,\"dr\"\n",
    ".balign 4\n",
    ".Lrsrc_d0:\n",
    ".byte 99, 117, 115, 116, 111, 109, 32, 116, 121, 112, 101, 32, 100, 97, 116, 97, 33\n",
    ".balign 4\n",
    ".Lrsrc_d1:\n",
    ".byte 102, 105, 118, 101\n",
    ".balign 4\n",
    ".Lrsrc_d2:\n",
    ".byte 5, 0, 7, 0, 40, 0, 0, 0, 32, 0, 0, 0, 64, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0\n",
    ".byte 128, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d3:\n",
    ".byte 2, 0, 3, 0, 40, 0, 0, 0, 16, 0, 0, 0, 32, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0\n",
    ".byte 64, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d4:\n",
    ".byte 1, 0, 1, 0, 40, 0, 0, 0, 32, 0, 0, 0, 64, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0\n",
    ".byte 128, 12, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d5:\n",
    ".byte 40, 0, 0, 0, 8, 0, 0, 0, 2, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0, 48, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 255\n",
    ".byte 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255\n",
    ".byte 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 255, 0\n",
    ".balign 4\n",
    ".Lrsrc_d6:\n",
    ".byte 40, 0, 0, 0, 4, 0, 0, 0, 3, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0, 36, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0\n",
    ".byte 255, 255, 0, 0, 255, 0, 0, 0, 0, 255, 255, 0, 0, 0, 0, 255, 0, 0, 255, 255, 0, 0, 0, 0\n",
    ".byte 255, 255, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d7:\n",
    ".byte 40, 0, 0, 0, 32, 0, 0, 0, 64, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0, 128, 12, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30\n",
    ".byte 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30\n",
    ".byte 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d8:\n",
    ".byte 40, 0, 0, 0, 16, 0, 0, 0, 32, 0, 0, 0, 1, 0, 24, 0, 0, 0, 0, 0, 64, 3, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200\n",
    ".byte 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 30, 200, 30, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d9:\n",
    ".byte 0, 0, 5, 0, 72, 0, 101, 0, 108, 0, 108, 0, 111, 0, 5, 0, 87, 0, 111, 0, 114, 0, 108, 0\n",
    ".byte 100, 0, 28, 0, 87, 0, 105, 0, 116, 0, 104, 0, 32, 0, 38, 0, 109, 0, 110, 0, 101, 0, 109, 0\n",
    ".byte 111, 0, 110, 0, 105, 0, 99, 0, 32, 0, 97, 0, 110, 0, 100, 0, 32, 0, 37, 0, 115, 0, 32, 0\n",
    ".byte 102, 0, 111, 0, 114, 0, 109, 0, 97, 0, 116, 0, 8, 0, 84, 0, 97, 0, 98, 0, 9, 0, 104, 0\n",
    ".byte 101, 0, 114, 0, 101, 0, 7, 0, 196, 0, 246, 0, 252, 0, 32, 0, 172, 32, 32, 0, 45, 78, 44, 1\n",
    ".byte 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0\n",
    ".byte 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0\n",
    ".byte 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0\n",
    ".byte 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0\n",
    ".byte 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0\n",
    ".byte 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0\n",
    ".byte 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0\n",
    ".byte 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0\n",
    ".byte 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0\n",
    ".byte 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0\n",
    ".byte 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0\n",
    ".byte 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0\n",
    ".byte 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0\n",
    ".byte 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0\n",
    ".byte 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0\n",
    ".byte 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0\n",
    ".byte 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0\n",
    ".byte 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0\n",
    ".byte 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0, 115, 0, 116, 0\n",
    ".byte 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0, 101, 0, 102, 0\n",
    ".byte 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0, 113, 0, 114, 0\n",
    ".byte 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0, 99, 0, 100, 0\n",
    ".byte 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0, 111, 0, 112, 0\n",
    ".byte 113, 0, 114, 0, 115, 0, 116, 0, 117, 0, 118, 0, 119, 0, 120, 0, 121, 0, 122, 0, 97, 0, 98, 0\n",
    ".byte 99, 0, 100, 0, 101, 0, 102, 0, 103, 0, 104, 0, 105, 0, 106, 0, 107, 0, 108, 0, 109, 0, 110, 0\n",
    ".byte 1, 0, 65, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d10:\n",
    ".byte 13, 0, 66, 0, 108, 0, 111, 0, 99, 0, 107, 0, 32, 0, 50, 0, 32, 0, 102, 0, 105, 0, 114, 0\n",
    ".byte 115, 0, 116, 0, 14, 0, 66, 0, 108, 0, 111, 0, 99, 0, 107, 0, 32, 0, 50, 0, 32, 0, 115, 0\n",
    ".byte 101, 0, 99, 0, 111, 0, 110, 0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d11:\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 13, 0, 84, 0, 104, 0, 114, 0, 101, 0, 101, 0, 32, 0, 104, 0, 117, 0, 110, 0, 100, 0, 114, 0\n",
    ".byte 101, 0, 100, 0, 0, 0, 0, 0, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d12:\n",
    ".byte 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0\n",
    ".byte 0, 0, 0, 0, 0, 0, 14, 0, 84, 0, 104, 0, 101, 0, 32, 0, 104, 0, 105, 0, 103, 0, 104, 0\n",
    ".byte 101, 0, 115, 0, 116, 0, 32, 0, 105, 0, 100, 0\n",
    ".balign 4\n",
    ".Lrsrc_d13:\n",
    ".byte 129, 0, 113, 0, 144, 1, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d14:\n",
    ".byte 1, 0, 112, 0, 47, 1, 0, 0, 9, 0, 79, 0, 45, 1, 0, 0, 0, 0, 65, 0, 48, 1, 0, 0\n",
    ".byte 5, 0, 116, 0, 49, 1, 0, 0, 145, 0, 13, 0, 50, 1, 0, 0\n",
    ".balign 4\n",
    ".Lrsrc_d15:\n",
    ".byte 0, 1, 2, 3, 4, 5, 6, 7, 8, 9\n",
    ".balign 4\n",
    ".Lrsrc_d16:\n",
    ".byte 1, 2, 3, 4, 5\n",
    ".balign 4\n",
    ".Lrsrc_d17:\n",
    ".byte 9\n",
    ".balign 4\n",
    ".Lrsrc_d18:\n",
    ".byte 0, 0, 2, 0, 1, 0, 32, 0, 64, 0, 1, 0, 24, 0, 172, 12, 0, 0, 13, 0\n",
    ".balign 4\n",
    ".Lrsrc_d19:\n",
    ".byte 0, 0, 2, 0, 2, 0, 32, 0, 64, 0, 1, 0, 24, 0, 172, 12, 0, 0, 11, 0, 16, 0, 32, 0\n",
    ".byte 1, 0, 24, 0, 108, 3, 0, 0, 12, 0\n",
    ".balign 4\n",
    ".Lrsrc_d20:\n",
    ".byte 0, 0, 1, 0, 1, 0, 32, 32, 0, 0, 1, 0, 24, 0, 168, 12, 0, 0, 1, 0\n",
    ".balign 4\n",
    ".Lrsrc_d21:\n",
    ".byte 0, 0, 1, 0, 2, 0, 32, 32, 0, 0, 1, 0, 24, 0, 168, 12, 0, 0, 1, 0, 16, 16, 0, 0\n",
    ".byte 1, 0, 24, 0, 104, 3, 0, 0, 2, 0\n",
);

fn ls(t: &mut Test, label: &str, id: u32, size: i32) {
    let mut b = [0xEEu8; 400];
    set_error(0xDEAD);
    // SAFETY: b has 400 bytes; size is at most 400.
    let n = unsafe { LoadStringA(instance(), id, b.as_mut_ptr(), size) };
    let e = last_error();
    let written = b.iter().take_while(|&&c| c != 0xEE).count();
    say!(t, "{label}: {n} err={} bytes '{}' written={written}", if n != 0 { -2 } else if e == 0xDEAD { -1 } else { e as i64 }, if written == 0 { Buf::<300>::new() } else { abytes(&b) }.as_str());
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    say!(t, "ANSI code page {}", unsafe { GetACP() });
    ls(t, "id 1", 1, 100);
    ls(t, "id 2", 2, 100);
    ls(t, "id 3 (with & and a format)", 3, 100);
    ls(t, "id 4 (a tab)", 4, 100);
    ls(t, "id 5 (non-ASCII characters)", 5, 100);
    ls(t, "id 6 (300 characters), size 400", 6, 400);
    ls(t, "id 6, size 100", 6, 100);
    ls(t, "id 7 (one character)", 7, 100);
    ls(t, "id 16 (second block)", 16, 100);
    ls(t, "id 17", 17, 100);
    ls(t, "id 18 (a hole in the block)", 18, 100);
    ls(t, "id 300", 300, 100);
    ls(t, "id 65535", 65535, 100);
    ls(t, "id 5 with size 4", 5, 4);
    ls(t, "id 5 with size 6", 5, 6);
    ls(t, "id 1 with size 5 (exactly the length plus NUL)", 1, 6);
    ls(t, "id 1 with size 5", 1, 5);
    ls(t, "id 1 with size 1", 1, 1);
    ls(t, "id 1 with size 0", 1, 0);
    ls(t, "id 1 with size -1", 1, -1);
    ls(t, "id 999 (absent)", 999, 100);
    ls(t, "id 0", 0, 100);
    ls(t, "id 0x10000", 0x10000, 100);
    call!(t, "NULL module (the executable)", { let mut b = [0u8; 20]; unsafe { LoadStringA(core::ptr::null_mut(), 1, b.as_mut_ptr(), 20) } });
    call!(t, "made-up module", { let mut b = [0u8; 20]; unsafe { LoadStringA(0x1000_0000 as HINSTANCE, 1, b.as_mut_ptr(), 20) } });
}
