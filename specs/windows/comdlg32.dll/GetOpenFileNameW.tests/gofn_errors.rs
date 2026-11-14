//! GetOpenFileNameW and CommDlgExtendedError: calls that fail before a
//! dialog is shown give FALSE and a common dialog error code: a wrong
//! lStructSize (CDERR_STRUCTSIZE), a hook flag without a hook (CDERR_NOHOOK),
//! a template flag without a template, a missing or too small file buffer;
//! CommDlgExtendedError starts at 0 and keeps the code of the last failing
//! common dialog call.
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
type HDC = *mut c_void;
type WPARAM = usize;
type LPARAM = isize;
type BOOL = i32;
const WM_COMMAND: u32 = 0x0111;
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

type HOOKPROC = Option<unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> usize>;
const WM_INITDIALOG: u32 = 0x0110;
const WM_NOTIFY: u32 = 0x004E;
const IDCANCEL: i32 = 2;
/// commctrl.h
#[repr(C)]
#[derive(Clone, Copy)]
struct NMHDR {
    hwnd_from: HWND,
    id_from: usize,
    code: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowLongPtrW(hwnd: HWND, index: i32) -> isize;
    fn GetParent(hwnd: HWND) -> HWND;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PostMessageW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> BOOL;
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "comdlg32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CommDlgExtendedError() -> u32;
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

// ---- common dialog structures (commdlg.h) ----

#[repr(C)]
struct OPENFILENAMEW {
    l_struct_size: u32,
    hwnd_owner: HWND,
    h_instance: HINSTANCE,
    lpstr_filter: *const u16,
    lpstr_custom_filter: *mut u16,
    n_max_cust_filter: u32,
    n_filter_index: u32,
    lpstr_file: *mut u16,
    n_max_file: u32,
    lpstr_file_title: *mut u16,
    n_max_file_title: u32,
    lpstr_initial_dir: *const u16,
    lpstr_title: *const u16,
    flags: u32,
    n_file_offset: u16,
    n_file_extension: u16,
    lpstr_def_ext: *const u16,
    l_cust_data: isize,
    lpfn_hook: HOOKPROC,
    lp_template_name: *const u16,
    pv_reserved: *mut c_void,
    dw_reserved: u32,
    flags_ex: u32,
}

#[repr(C)]
struct CHOOSECOLORW {
    l_struct_size: u32,
    hwnd_owner: HWND,
    h_instance: HWND,
    rgb_result: u32,
    lp_cust_colors: *mut u32,
    flags: u32,
    l_cust_data: isize,
    lpfn_hook: HOOKPROC,
    lp_template_name: *const u16,
}

#[repr(C)]
struct LOGFONTW {
    height: i32,
    width: i32,
    escapement: i32,
    orientation: i32,
    weight: i32,
    italic: u8,
    underline: u8,
    strike_out: u8,
    char_set: u8,
    out_precision: u8,
    clip_precision: u8,
    quality: u8,
    pitch_and_family: u8,
    face_name: [u16; 32],
}

#[repr(C)]
struct CHOOSEFONTW {
    l_struct_size: u32,
    hwnd_owner: HWND,
    hdc: HDC,
    lp_log_font: *mut LOGFONTW,
    i_point_size: i32,
    flags: u32,
    rgb_colors: u32,
    l_cust_data: isize,
    lpfn_hook: HOOKPROC,
    lp_template_name: *const u16,
    h_instance: HINSTANCE,
    lpsz_style: *mut u16,
    n_font_type: u16,
    pad: u16,
    n_size_min: i32,
    n_size_max: i32,
}

const OFN_READONLY: u32 = 0x1;
const OFN_OVERWRITEPROMPT: u32 = 0x2;
const OFN_HIDEREADONLY: u32 = 0x4;
const OFN_NOCHANGEDIR: u32 = 0x8;
const OFN_ENABLEHOOK: u32 = 0x20;
const OFN_ENABLETEMPLATE: u32 = 0x40;
const OFN_ENABLETEMPLATEHANDLE: u32 = 0x80;
const OFN_NOVALIDATE: u32 = 0x100;
const OFN_ALLOWMULTISELECT: u32 = 0x200;
const OFN_PATHMUSTEXIST: u32 = 0x800;
const OFN_FILEMUSTEXIST: u32 = 0x1000;
const OFN_EXPLORER: u32 = 0x80000;
const CC_RGBINIT: u32 = 0x1;
const CC_FULLOPEN: u32 = 0x2;
const CC_ENABLEHOOK: u32 = 0x10;
const CC_ENABLETEMPLATE: u32 = 0x20;
const CF_SCREENFONTS: u32 = 0x1;
const CF_ENABLEHOOK: u32 = 0x8;
const CF_INITTOLOGFONTSTRUCT: u32 = 0x40;
const CF_NOSTYLESEL: u32 = 0x100000;
const CDERR_STRUCTSIZE: u32 = 0x1;
const CDERR_INITIALIZATION: u32 = 0x2;
const CDERR_NOTEMPLATE: u32 = 0x3;
const CDERR_NOHINSTANCE: u32 = 0x4;
const CDERR_LOADSTRFAILURE: u32 = 0x5;
const CDERR_FINDRESFAILURE: u32 = 0x6;
const CDERR_LOADRESFAILURE: u32 = 0x7;
const CDERR_LOCKRESFAILURE: u32 = 0x8;
const CDERR_MEMALLOCFAILURE: u32 = 0x9;
const CDERR_MEMLOCKFAILURE: u32 = 0xA;
const CDERR_NOHOOK: u32 = 0xB;
const CDERR_REGISTERMSGFAIL: u32 = 0xC;
const CFERR_NOFONTS: u32 = 0x2001;
const CFERR_MAXLESSTHANMIN: u32 = 0x2002;
const FNERR_SUBCLASSFAILURE: u32 = 0x3001;
const FNERR_INVALIDFILENAME: u32 = 0x3002;
const FNERR_BUFFERTOOSMALL: u32 = 0x3003;

#[link(name = "comdlg32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetOpenFileNameW(ofn: *mut OPENFILENAMEW) -> BOOL;
    fn GetSaveFileNameW(ofn: *mut OPENFILENAMEW) -> BOOL;
    fn ChooseColorW(cc: *mut CHOOSECOLORW) -> BOOL;
    fn ChooseFontW(cf: *mut CHOOSEFONTW) -> BOOL;
}

/// The dialog is ended with Cancel as soon as it is up: for the hook of an
/// old style dialog (WM_INITDIALOG: the dialog itself), an Explorer style
/// one (WM_NOTIFY CDN_INITDONE: the child, its parent is the dialog) and the
/// colour and font dialogs.
unsafe extern "system" fn cancel_hook(hdlg: HWND, msg: u32, _w: WPARAM, l: LPARAM) -> usize {
    if msg == WM_INITDIALOG {
        logln!("hook: WM_INITDIALOG");
        // SAFETY: posts to the dialog.
        unsafe { PostMessageW(hdlg, WM_COMMAND, IDCANCEL as usize, 0) };
        // also the parent (Explorer style: hdlg is a child of the dialog)
        let p = unsafe { GetParent(hdlg) };
        if !p.is_null() {
            unsafe { PostMessageW(p, WM_COMMAND, IDCANCEL as usize, 0) };
        }
        return 0;
    }
    if msg == WM_NOTIFY {
        // SAFETY: l points to an NMHDR.
        let code = unsafe { (*(l as *const NMHDR)).code } as i32;
        if code == -604 {
            logln!("hook: CDN_INITDONE");
            let p = unsafe { GetParent(hdlg) };
            unsafe { PostMessageW(if p.is_null() { hdlg } else { p }, WM_COMMAND, IDCANCEL as usize, 0) };
        }
    }
    0
}

fn base(buf: &mut [u16; 260]) -> OPENFILENAMEW {
    let mut o: OPENFILENAMEW = zeroed();
    o.l_struct_size = core::mem::size_of::<OPENFILENAMEW>() as u32;
    o.lpstr_file = buf.as_mut_ptr();
    o.n_max_file = 260;
    o.lpstr_filter = wz!("All\0*.*\0").as_ptr();
    o.flags = OFN_ENABLEHOOK | OFN_EXPLORER;
    o.lpfn_hook = Some(cancel_hook);
    o
}

fn try_open(t: &mut Test, label: &str, o: &mut OPENFILENAMEW) {
    // SAFETY: o is a filled OPENFILENAMEW.
    let r = unsafe { GetOpenFileNameW(o) };
    let e = unsafe { CommDlgExtendedError() };
    say!(t, "{label}: GetOpenFileNameW = {r}, CommDlgExtendedError = {e:#x}");
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    say!(t, "sizeof OPENFILENAMEW {}", core::mem::size_of::<OPENFILENAMEW>());
    say!(t, "CommDlgExtendedError before any call: {:#x}", unsafe { CommDlgExtendedError() });
    let mut buf = [0u16; 260];
    let mut o = base(&mut buf);
    o.l_struct_size = 0;
    try_open(t, "lStructSize 0", &mut o);
    o.l_struct_size = 4;
    try_open(t, "lStructSize 4", &mut o);
    o.l_struct_size = 1000;
    try_open(t, "lStructSize 1000", &mut o);
    let mut o = base(&mut buf);
    o.flags = OFN_ENABLEHOOK;
    o.lpfn_hook = None;
    try_open(t, "OFN_ENABLEHOOK without a hook", &mut o);
    let mut o = base(&mut buf);
    o.flags = OFN_ENABLETEMPLATE;
    o.lpfn_hook = None;
    try_open(t, "OFN_ENABLETEMPLATE without a template name", &mut o);
    let mut o = base(&mut buf);
    o.flags = OFN_ENABLETEMPLATEHANDLE;
    o.lpfn_hook = None;
    try_open(t, "OFN_ENABLETEMPLATEHANDLE without a template", &mut o);
    let mut o = base(&mut buf);
    o.flags = OFN_ENABLETEMPLATE | OFN_ENABLEHOOK;
    o.lp_template_name = wz!("NoSuchTemplate").as_ptr();
    o.lpfn_hook = Some(cancel_hook);
    try_open(t, "a template that is not in the module", &mut o);
    let mut o = base(&mut buf);
    o.lpstr_file = core::ptr::null_mut();
    try_open(t, "lpstrFile NULL", &mut o);
    let mut o = base(&mut buf);
    o.n_max_file = 0;
    try_open(t, "nMaxFile 0", &mut o);
    let mut o = base(&mut buf);
    o.n_max_file = 1;
    try_open(t, "nMaxFile 1", &mut o);
    let mut o = base(&mut buf);
    o.hwnd_owner = 0x0bad_0bad as HWND;
    try_open(t, "a bad owner window handle", &mut o);
    // the code is replaced by the next call
    let mut o = base(&mut buf);
    o.l_struct_size = 0;
    try_open(t, "again lStructSize 0", &mut o);
    say!(t, "CommDlgExtendedError twice in a row: {:#x} {:#x}", unsafe { CommDlgExtendedError() }, unsafe { CommDlgExtendedError() });
}
