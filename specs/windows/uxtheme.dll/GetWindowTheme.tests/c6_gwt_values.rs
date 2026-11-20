//! GetWindowTheme (uxtheme.dll): the theme handle of a window; 0 for a NULL handle, a made-up handle and a destroyed window, and (with no
//! desktop theme, in session 0) for the windows of the standard and common controls too.
#![no_std]
#![no_main]
#![allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    non_snake_case,
    unused_imports,
    clippy::all,
    static_mut_refs
)]

// The declarations in this file are from Microsoft Learn (prototypes) and the public SDK headers named beside them
// (commctrl.h, winuser.h, winbase.h, winerror.h, uxtheme.h, vsstyle.h); the helpers are this test's own.
// The program carries a manifest (an RT_MANIFEST resource, at the end of this file) that asks for version 6.0.0.0 of
// Microsoft.Windows.Common-Controls: it runs with the side-by-side comctl32 version 6.

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_wtest::*;

type BOOL = i32;
type HWND = usize;
type HRESULT = u32;

// --- commctrl.h: ICC_* ---
const ICC_LISTVIEW_CLASSES: u32 = 0x1;
const ICC_TREEVIEW_CLASSES: u32 = 0x2;
const ICC_BAR_CLASSES: u32 = 0x4;
const ICC_TAB_CLASSES: u32 = 0x8;
const ICC_UPDOWN_CLASS: u32 = 0x10;
const ICC_PROGRESS_CLASS: u32 = 0x20;
const ICC_HOTKEY_CLASS: u32 = 0x40;
const ICC_ANIMATE_CLASS: u32 = 0x80;
const ICC_WIN95_CLASSES: u32 = 0xFF;
const ICC_DATE_CLASSES: u32 = 0x100;
const ICC_USEREX_CLASSES: u32 = 0x200;
const ICC_COOL_CLASSES: u32 = 0x400;
const ICC_INTERNET_CLASSES: u32 = 0x800;
const ICC_PAGESCROLLER_CLASS: u32 = 0x1000;
const ICC_NATIVEFNTCTL_CLASS: u32 = 0x2000;
const ICC_STANDARD_CLASSES: u32 = 0x4000;
const ICC_LINK_CLASS: u32 = 0x8000;

// --- winuser.h / commctrl.h ---
const WS_CHILD: u32 = 0x4000_0000;
const WS_VISIBLE: u32 = 0x1000_0000;
const WS_POPUP: u32 = 0x8000_0000;
const WS_OVERLAPPED: u32 = 0;
const WS_BORDER: u32 = 0x0080_0000;
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;
const WS_TABSTOP: u32 = 0x0001_0000;
const GWL_STYLE: i32 = -16;
const GWLP_WNDPROC: i32 = -4;
const GWL_EXSTYLE: i32 = -20;
const WM_NULL: u32 = 0;
const WM_USER: u32 = 0x400;
const WM_GETTEXT: u32 = 0xD;
const WM_SETTEXT: u32 = 0xC;
const WM_GETTEXTLENGTH: u32 = 0xE;
const WM_SETFONT: u32 = 0x30;
const WM_GETFONT: u32 = 0x31;
const WM_DESTROY: u32 = 2;
const WM_NCDESTROY: u32 = 0x82;
const WM_INITDIALOG: u32 = 0x110;
const WM_COMMAND: u32 = 0x111;
const DS_SETFONT: u32 = 0x40;
const DS_MODALFRAME: u32 = 0x80;

const PBS_SMOOTH: u32 = 1;
const PBS_VERTICAL: u32 = 4;
const PBS_MARQUEE: u32 = 8;
const PBS_SMOOTHREVERSE: u32 = 0x10;
const PBM_SETRANGE: u32 = WM_USER + 1;
const PBM_SETPOS: u32 = WM_USER + 2;
const PBM_DELTAPOS: u32 = WM_USER + 3;
const PBM_SETSTEP: u32 = WM_USER + 4;
const PBM_STEPIT: u32 = WM_USER + 5;
const PBM_SETRANGE32: u32 = WM_USER + 6;
const PBM_GETRANGE: u32 = WM_USER + 7;
const PBM_GETPOS: u32 = WM_USER + 8;
const PBM_SETBARCOLOR: u32 = WM_USER + 9;
const PBM_SETMARQUEE: u32 = WM_USER + 10;
const PBM_GETSTEP: u32 = WM_USER + 13;
const PBM_GETBKCOLOR: u32 = WM_USER + 14;
const PBM_GETBARCOLOR: u32 = WM_USER + 15;
const PBM_SETSTATE: u32 = WM_USER + 16;
const PBM_GETSTATE: u32 = WM_USER + 17;
const PBM_SETBKCOLOR: u32 = 0x2001;
const PBST_NORMAL: u32 = 1;
const PBST_ERROR: u32 = 2;
const PBST_PAUSED: u32 = 3;
const CLR_DEFAULT: u32 = 0xFF00_0000;

const BM_GETCHECK: u32 = 0xF0;
const BM_SETCHECK: u32 = 0xF1;
const BM_GETSTATE: u32 = 0xF2;
const BM_CLICK: u32 = 0xF5;
const BCM_FIRST: u32 = 0x1600;
const BCM_GETIDEALSIZE: u32 = BCM_FIRST + 1;
const BCM_SETIMAGELIST: u32 = BCM_FIRST + 2;
const BCM_GETIMAGELIST: u32 = BCM_FIRST + 3;
const BCM_SETTEXTMARGIN: u32 = BCM_FIRST + 4;
const BCM_GETTEXTMARGIN: u32 = BCM_FIRST + 5;
const BCM_SETDROPDOWNSTATE: u32 = BCM_FIRST + 6;
const BCM_SETSPLITINFO: u32 = BCM_FIRST + 7;
const BCM_GETSPLITINFO: u32 = BCM_FIRST + 8;
const BCM_SETNOTE: u32 = BCM_FIRST + 9;
const BCM_GETNOTE: u32 = BCM_FIRST + 10;
const BCM_GETNOTELENGTH: u32 = BCM_FIRST + 11;
const BCM_SETSHIELD: u32 = BCM_FIRST + 12;
const BS_PUSHBUTTON: u32 = 0;
const BS_DEFPUSHBUTTON: u32 = 1;
const BS_CHECKBOX: u32 = 2;
const BS_AUTOCHECKBOX: u32 = 3;
const BS_RADIOBUTTON: u32 = 4;
const BS_GROUPBOX: u32 = 7;
const BS_SPLITBUTTON: u32 = 0xC;
const BS_COMMANDLINK: u32 = 0xE;
const BS_DEFCOMMANDLINK: u32 = 0xF;
const EM_FIRST_ECM: u32 = 0x1500;
const EM_SETCUEBANNER: u32 = EM_FIRST_ECM + 1;
const EM_GETCUEBANNER: u32 = EM_FIRST_ECM + 2;
const EM_SHOWBALLOONTIP: u32 = EM_FIRST_ECM + 3;
const EM_HIDEBALLOONTIP: u32 = EM_FIRST_ECM + 4;
const EM_GETLIMITTEXT: u32 = 0xD5;
const EM_SETLIMITTEXT: u32 = 0xC5;
const CB_SETCUEBANNER: u32 = 0x1703;
const CB_GETCUEBANNER: u32 = 0x1704;
const CB_ADDSTRING: u32 = 0x143;
const CB_GETCOUNT: u32 = 0x146;
const CB_GETCURSEL: u32 = 0x147;
const CB_SETCURSEL: u32 = 0x14E;
const LB_ADDSTRING: u32 = 0x180;
const LB_GETCOUNT: u32 = 0x18B;
const SBM_SETPOS: u32 = 0xE0;
const SBM_GETPOS: u32 = 0xE1;
const SBM_SETRANGE: u32 = 0xE2;
const SBM_GETRANGE: u32 = 0xE3;
const SS_LEFT: u32 = 0;
const SS_ICON: u32 = 3;
const SS_SIMPLE: u32 = 0xB;
const SB_HORZ: u32 = 0;
const SBS_HORZ: u32 = 0;
const ES_AUTOHSCROLL: u32 = 0x80;
const CBS_DROPDOWNLIST: u32 = 3;
const LBS_NOTIFY: u32 = 1;

const ERROR_SUCCESS: u32 = 0;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
const ERROR_CANNOT_FIND_WND_CLASS: u32 = 1407;
const ERROR_INVALID_WINDOW_HANDLE: u32 = 1400;
const HWND_MESSAGE: usize = (-3isize) as usize;

#[repr(C)]
struct INITCOMMONCONTROLSEX {
    dwSize: u32,
    dwICC: u32,
}

#[repr(C)]
struct DLLVERSIONINFO {
    cbSize: u32,
    dwMajorVersion: u32,
    dwMinorVersion: u32,
    dwBuildNumber: u32,
    dwPlatformID: u32,
}

#[repr(C)]
struct DLLVERSIONINFO2 {
    info1: DLLVERSIONINFO,
    dwFlags: u32,
    ullVersion: u64,
}

#[repr(C)]
struct WNDCLASSEXW {
    cbSize: u32,
    style: u32,
    lpfnWndProc: usize,
    cbClsExtra: i32,
    cbWndExtra: i32,
    hInstance: usize,
    hIcon: usize,
    hCursor: usize,
    hbrBackground: usize,
    lpszMenuName: *const u16,
    lpszClassName: *const u16,
    hIconSm: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct RECT {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
struct SIZE {
    cx: i32,
    cy: i32,
}

#[link(name = "comctl32", kind = "raw-dylib")]
unsafe extern "system" {
    fn InitCommonControlsEx(p: *const INITCOMMONCONTROLSEX) -> BOOL;
    fn InitCommonControls();
    fn DllGetVersion(p: *mut DLLVERSIONINFO) -> HRESULT;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: usize,
        menu: usize,
        inst: usize,
        param: usize,
    ) -> HWND;
    fn DestroyWindow(h: HWND) -> BOOL;
    fn SendMessageW(h: HWND, m: u32, w: usize, l: isize) -> isize;
    fn GetClassNameW(h: HWND, buf: *mut u16, n: i32) -> i32;
    fn GetClassInfoExW(inst: usize, class: *const u16, wc: *mut WNDCLASSEXW) -> u32;
    fn RegisterClassExW(wc: *const WNDCLASSEXW) -> u16;
    fn UnregisterClassW(class: *const u16, inst: usize) -> BOOL;
    fn GetWindowLongW(h: HWND, i: i32) -> i32;
    fn GetWindowLongPtrW(h: HWND, i: i32) -> isize;
    fn SetWindowLongW(h: HWND, i: i32, v: i32) -> i32;
    fn GetParent(h: HWND) -> HWND;
    fn GetWindowRect(h: HWND, r: *mut RECT) -> BOOL;
    fn GetClientRect(h: HWND, r: *mut RECT) -> BOOL;
    fn MapWindowPoints(from: HWND, to: HWND, pts: *mut RECT, n: u32) -> i32;
    fn IsWindow(h: HWND) -> BOOL;
    fn DefWindowProcW(h: HWND, m: u32, w: usize, l: isize) -> isize;
    fn CallWindowProcW(p: usize, h: HWND, m: u32, w: usize, l: isize) -> isize;
    fn GetWindowTextW(h: HWND, buf: *mut u16, n: i32) -> i32;
    fn SetWindowTextW(h: HWND, s: *const u16) -> BOOL;
    fn CreateDialogIndirectParamW(inst: usize, tpl: *const c_void, parent: HWND, proc_: usize, param: isize) -> HWND;
    fn GetDlgItem(dlg: HWND, id: i32) -> HWND;
    fn PostMessageW(h: HWND, m: u32, w: usize, l: isize) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> usize;
    fn GetModuleHandleExW(flags: u32, name: usize, out: *mut usize) -> BOOL;
    fn GetModuleFileNameW(m: usize, buf: *mut u16, n: u32) -> u32;
    fn GetProcAddress(m: usize, name: *const u8) -> usize;
    fn LoadLibraryW(name: *const u16) -> usize;
    fn FreeLibrary(m: usize) -> BOOL;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn GetCurrentThreadId() -> u32;
    fn CreateThread(
        attr: *const c_void,
        stack: usize,
        f: extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> usize;
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
    fn CloseHandle(h: usize) -> BOOL;
}

// ---------------------------------------------------------------------------------------------------------------

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

macro_rules! say {
    ($t:expr, $($arg:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($arg)*);
        $t.line(b.as_str());
    }};
}

/// An ASCII string as a NUL-terminated UTF-16 array of up to 64 units (kept on the stack).
struct W(pub [u16; 64]);
fn w(s: &str) -> W {
    let mut a = [0u16; 64];
    for (i, c) in s.bytes().enumerate() {
        a[i] = c as u16;
    }
    W(a)
}
impl W {
    fn p(&self) -> *const u16 {
        self.0.as_ptr()
    }
}

fn err_name(e: u32) -> &'static str {
    match e {
        0 => "0",
        5 => "ERROR_ACCESS_DENIED",
        6 => "ERROR_INVALID_HANDLE",
        87 => "ERROR_INVALID_PARAMETER",
        1400 => "ERROR_INVALID_WINDOW_HANDLE",
        1407 => "ERROR_CANNOT_FIND_WND_CLASS",
        1410 => "ERROR_CLASS_ALREADY_EXISTS",
        0xDEAD => "unchanged",
        _ => "other",
    }
}

/// Sentinel for "the last error was not touched".
const MARK_ERR: u32 = 0xDEAD;

/// Create a window of `class` with `style` (0 extended style) under `parent`; 0 when the class does not exist.
fn mk(class: &str, style: u32, parent: HWND) -> HWND {
    let c = w(class);
    let e = w("x");
    // SAFETY: valid strings.
    unsafe { CreateWindowExW(0, c.p(), e.p(), style, 0, 0, 100, 40, parent, 0, 0, 0) }
}

/// A window to be the parent of controls: a message-only window of the STATIC class cannot parent children, so a plain overlapped
/// STATIC window (no visible effect in session 0) is used.
fn parent_window() -> HWND {
    let c = w("STATIC");
    let e = w("p");
    // SAFETY: valid strings.
    unsafe { CreateWindowExW(0, c.p(), e.p(), WS_OVERLAPPED, 0, 0, 300, 200, 0, 0, 0, 0) }
}

fn class_of(h: HWND) -> Buf<64> {
    let mut b = [0u16; 64];
    // SAFETY: valid buffer.
    let n = unsafe { GetClassNameW(h, b.as_mut_ptr(), 64) } as usize;
    let mut s = Buf::<64>::new();
    for i in 0..n {
        let _ = write!(s, "{}", if b[i] < 128 { b[i] as u8 as char } else { '?' });
    }
    s
}

/// Whether a window of `class` can be created (as a child of `parent`), and its class name as the system reports it; the window is destroyed.
fn can_create(class: &str, parent: HWND) -> (bool, u32) {
    // SAFETY: plain call.
    unsafe { SetLastError(MARK_ERR) };
    let h = mk(class, WS_CHILD, parent);
    // SAFETY: plain call.
    let e = unsafe { GetLastError() };
    if h != 0 {
        // SAFETY: valid window.
        unsafe { DestroyWindow(h) };
        (true, e)
    } else {
        (false, e)
    }
}

/// GetClassInfoExW (hinstance NULL, the system class path): whether it exists, its style, extra bytes, and the file name of the module that
/// holds its window procedure (lower case, e.g. "user32.dll", "comctl32.dll").
fn class_info(class: &str) -> (bool, u32, i32, i32, Buf<64>) {
    let c = w(class);
    // SAFETY: plain-old-data, zero is valid.
    let mut wc: WNDCLASSEXW = unsafe { core::mem::zeroed() };
    wc.cbSize = core::mem::size_of::<WNDCLASSEXW>() as u32;
    // SAFETY: valid pointers.
    let r = unsafe { GetClassInfoExW(0, c.p(), &mut wc) };
    if r == 0 {
        return (false, 0, 0, 0, Buf::new());
    }
    (true, wc.style, wc.cbClsExtra, wc.cbWndExtra, module_of(wc.lpfnWndProc))
}

/// The file name (lower case) of the module that contains `addr`.
fn module_of(addr: usize) -> Buf<64> {
    let mut m = 0usize;
    // GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS (4) | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT (2)
    // SAFETY: valid pointer.
    let ok = unsafe { GetModuleHandleExW(6, addr, &mut m) };
    let mut s = Buf::<64>::new();
    if ok == 0 {
        let _ = write!(s, "(none)");
        return s;
    }
    let mut b = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetModuleFileNameW(m, b.as_mut_ptr(), 300) } as usize;
    let mut start = 0;
    for i in 0..n {
        if b[i] == b'\\' as u16 {
            start = i + 1;
        }
    }
    for i in start..n {
        let c = b[i];
        let c = if c >= b'A' as u16 && c <= b'Z' as u16 { c + 32 } else { c };
        let _ = write!(s, "{}", if c < 128 { c as u8 as char } else { '?' });
    }
    s
}

/// The side-by-side directory name part of a loaded module's path (lower case), e.g. true when it contains the common-controls assembly name.
fn module_path_contains(name: &str, needle: &str) -> bool {
    let n = w(name);
    // SAFETY: valid string.
    let m = unsafe { GetModuleHandleW(n.p()) };
    if m == 0 {
        return false;
    }
    let mut b = [0u16; 400];
    // SAFETY: valid buffer.
    let len = unsafe { GetModuleFileNameW(m, b.as_mut_ptr(), 400) } as usize;
    let nb = needle.as_bytes();
    if len < nb.len() {
        return false;
    }
    'outer: for i in 0..=(len - nb.len()) {
        for (k, &c) in nb.iter().enumerate() {
            let x = b[i + k];
            let x = if x >= b'A' as u16 && x <= b'Z' as u16 { x + 32 } else { x };
            if x != c as u16 {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

fn icc(mask: u32) -> (BOOL, u32) {
    let p = INITCOMMONCONTROLSEX { dwSize: 8, dwICC: mask };
    // SAFETY: plain calls.
    unsafe { SetLastError(MARK_ERR) };
    let r = unsafe { InitCommonControlsEx(&p) };
    let e = unsafe { GetLastError() };
    (r, e)
}

fn icc_size(size: u32, mask: u32) -> (BOOL, u32) {
    let p = INITCOMMONCONTROLSEX { dwSize: size, dwICC: mask };
    // SAFETY: plain calls (the structure is 8 bytes; a larger declared size is only read by the system as a number).
    unsafe { SetLastError(MARK_ERR) };
    let r = unsafe { InitCommonControlsEx(&p) };
    let e = unsafe { GetLastError() };
    (r, e)
}

fn send(h: HWND, m: u32, wp: usize, lp: isize) -> isize {
    // SAFETY: plain call.
    unsafe { SendMessageW(h, m, wp, lp) }
}

fn style_of(h: HWND) -> u32 {
    // SAFETY: plain call.
    unsafe { GetWindowLongW(h, GWL_STYLE) as u32 }
}

fn text_of(h: HWND) -> Buf<100> {
    let mut b = [0u16; 100];
    // SAFETY: valid buffer.
    let n = unsafe { GetWindowTextW(h, b.as_mut_ptr(), 100) } as usize;
    let mut s = Buf::<100>::new();
    for i in 0..n {
        let _ = write!(s, "{}", if b[i] < 128 { b[i] as u8 as char } else { '?' });
    }
    s
}

// ---------------------------------------------------------------------------------------------------------------
// The manifest of the program: Microsoft.Windows.Common-Controls 6.0.0.0 (RT_MANIFEST, id 1).
// ---------------------------------------------------------------------------------------------------------------

core::arch::global_asm!(
    ".section .rsrc$01,\"dr\"\n",
    ".balign 4\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 24\n",
    ".long 0x80000018\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 1\n",
    ".long 0x80000030\n",
    ".long 0, 0, 0\n",
    ".short 0, 1\n",
    ".long 0\n",
    ".long 0x48\n",
    ".rva .Lc6_manifest\n",
    ".long .Lc6_manifest_end - .Lc6_manifest\n",
    ".long 0, 0\n",
    ".section .rsrc$02,\"dr\"\n",
    ".balign 4\n",
    ".Lc6_manifest:\n",
    ".ascii \"<?xml version='1.0' encoding='UTF-8' standalone='yes'?>\\r\\n\"\n",
    ".ascii \"<assembly xmlns='urn:schemas-microsoft-com:asm.v1' manifestVersion='1.0'>\\r\\n\"\n",
    ".ascii \"<dependency><dependentAssembly><assemblyIdentity type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'/></dependentAssembly></dependency>\\r\\n\"\n",
    ".ascii \"</assembly>\\r\\n\"\n",
    ".Lc6_manifest_end:\n",
    ".balign 4\n",
);

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let n = w("uxtheme.dll");
    // SAFETY: valid string and name.
    let m = unsafe { LoadLibraryW(n.p()) };
    let gw = unsafe { GetProcAddress(m, b"GetWindowTheme\0".as_ptr()) };
    t.check("GetWindowTheme is exported", gw != 0, 0);
    // SAFETY: the export has this signature.
    let get: extern "system" fn(HWND) -> usize = unsafe { core::mem::transmute(gw) };
    let p = parent_window();
    let s = mk("Static", WS_CHILD, p);
    let b = mk("Button", WS_CHILD, p);
    let g = mk("msctls_progress32", WS_CHILD, p);
    let l = mk("SysListView32", WS_CHILD, p);
    for (name, h) in [("static", s), ("button", b), ("progress bar", g), ("list view", l), ("top-level window", p)] {
        say!(t, "{}: theme {}", name, get(h) != 0);
        t.expect(name, get(h) as i64, 0);
    }
    unsafe { DestroyWindow(s) };
    t.expect("NULL", get(0) as i64, 0);
    t.expect("0x1234", get(0x1234) as i64, 0);
    t.expect("a destroyed window", get(s) as i64, 0);
}
