//! GetClipboardData of text: the handle that was set comes back (the same handle, its size
//! unchanged); a text block without a terminating NUL has its last unit replaced by NUL;
//! embedded NULs stay and convert; an empty string is one NUL; the converted formats have the
//! size of the converted text including the NUL; the same handle is returned on every Get;
//! CF_LOCALE is 4 bytes holding the user's default locale.
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

const WM_NCCREATE: u32 = 0x0081;
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

/// A NUL-terminated UTF-16 string from an ASCII literal, at compile time.
macro_rules! wz {
    ($s:literal) => {{
        const S: &str = concat!($s, "\0");
        const W: [u16; S.len()] = ascii_utf16::<{ S.len() }>(S);
        &W
    }};
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

const CF_TEXT: u32 = 1;
const CF_BITMAP: u32 = 2;
const CF_METAFILEPICT: u32 = 3;
const CF_SYLK: u32 = 4;
const CF_DIF: u32 = 5;
const CF_TIFF: u32 = 6;
const CF_OEMTEXT: u32 = 7;
const CF_DIB: u32 = 8;
const CF_PALETTE: u32 = 9;
const CF_PENDATA: u32 = 10;
const CF_RIFF: u32 = 11;
const CF_WAVE: u32 = 12;
const CF_UNICODETEXT: u32 = 13;
const CF_ENHMETAFILE: u32 = 14;
const CF_HDROP: u32 = 15;
const CF_LOCALE: u32 = 16;
const CF_DIBV5: u32 = 17;
const CF_OWNERDISPLAY: u32 = 0x0080;
const CF_DSPTEXT: u32 = 0x0081;
const CF_DSPBITMAP: u32 = 0x0082;
const CF_DSPMETAFILEPICT: u32 = 0x0083;
const CF_DSPENHMETAFILE: u32 = 0x008E;
const GMEM_MOVEABLE: u32 = 0x0002;
const WM_RENDERFORMAT: u32 = 0x0305;
const WM_RENDERALLFORMATS: u32 = 0x0306;
const WM_DESTROYCLIPBOARD: u32 = 0x0307;
const WM_DRAWCLIPBOARD: u32 = 0x0308;
const WM_CHANGECBCHAIN: u32 = 0x030D;
const WM_CLIPBOARDUPDATE: u32 = 0x031D;
/// Open the clipboard for `hwnd` (NULL: no owner window), retrying for up
/// to two seconds: another program on the desktop may hold it for a moment.
fn cl_open(hwnd: HWND) -> bool {
    for _ in 0..40 {
        // SAFETY: plain call.
        if unsafe { OpenClipboard(hwnd) } != 0 {
            return true;
        }
        // SAFETY: plain call.
        unsafe { Sleep(50) };
    }
    false
}

/// A new global memory block with `flags` holding `data`.
fn cl_alloc_bytes(flags: u32, data: &[u8]) -> HANDLE {
    // SAFETY: allocate, lock, copy `data.len()` bytes into the block, unlock.
    unsafe {
        let h = GlobalAlloc(flags, data.len());
        if h.is_null() {
            return h;
        }
        let p = GlobalLock(h) as *mut u8;
        if !p.is_null() {
            core::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        }
        GlobalUnlock(h);
        h
    }
}

/// A new GMEM_MOVEABLE block holding the UTF-16 units of `s` (give the
/// terminating NUL as part of `s` when wanted).
fn cl_alloc_w(s: &[u16]) -> HANDLE {
    // SAFETY: the bytes of a u16 slice.
    let b = unsafe { core::slice::from_raw_parts(s.as_ptr() as *const u8, s.len() * 2) };
    cl_alloc_bytes(GMEM_MOVEABLE, b)
}

/// A new GMEM_MOVEABLE block holding the bytes of `s`.
fn cl_alloc_a(s: &[u8]) -> HANDLE {
    cl_alloc_bytes(GMEM_MOVEABLE, s)
}

/// Copy up to `out.len()` bytes of the block `h` into `out`; returns the
/// block's size (GlobalSize) or 0 when `h` cannot be locked.
fn cl_bytes(h: HANDLE, out: &mut [u8]) -> usize {
    // SAFETY: lock the block, copy at most its size, unlock.
    unsafe {
        let size = GlobalSize(h);
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return 0;
        }
        let n = size.min(out.len());
        core::ptr::copy_nonoverlapping(p, out.as_mut_ptr(), n);
        GlobalUnlock(h);
        size
    }
}

/// A clipboard format printed by name (predefined ones), or by range for
/// private, GDI object and registered ones (never the number of those).
struct ClFmt(u32);

impl core::fmt::Display for ClFmt {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = match self.0 {
            CF_TEXT => "CF_TEXT",
            CF_BITMAP => "CF_BITMAP",
            CF_METAFILEPICT => "CF_METAFILEPICT",
            CF_SYLK => "CF_SYLK",
            CF_DIF => "CF_DIF",
            CF_TIFF => "CF_TIFF",
            CF_OEMTEXT => "CF_OEMTEXT",
            CF_DIB => "CF_DIB",
            CF_PALETTE => "CF_PALETTE",
            CF_PENDATA => "CF_PENDATA",
            CF_RIFF => "CF_RIFF",
            CF_WAVE => "CF_WAVE",
            CF_UNICODETEXT => "CF_UNICODETEXT",
            CF_ENHMETAFILE => "CF_ENHMETAFILE",
            CF_HDROP => "CF_HDROP",
            CF_LOCALE => "CF_LOCALE",
            CF_DIBV5 => "CF_DIBV5",
            CF_OWNERDISPLAY => "CF_OWNERDISPLAY",
            CF_DSPTEXT => "CF_DSPTEXT",
            CF_DSPBITMAP => "CF_DSPBITMAP",
            CF_DSPMETAFILEPICT => "CF_DSPMETAFILEPICT",
            CF_DSPENHMETAFILE => "CF_DSPENHMETAFILE",
            0 => "0",
            0x200..=0x2FF => "private",
            0x300..=0x3FF => "gdiobj",
            0xC000..=0xFFFF => "registered",
            _ => "other",
        };
        f.write_str(s)
    }
}

/// Bytes as text: printable ASCII as is, others as \xNN.
fn cl_show(b: &[u8], out: &mut Buf<300>) {
    for &c in b {
        if (0x20..0x7F).contains(&c) {
            let _ = out.write_char(c as char);
        } else {
            let _ = write!(out, "\\x{:02x}", c);
        }
    }
}

/// Read the data of `format` (clipboard open) as bytes: prints
/// "<label>: size N bytes ...", returns the size or None when no data.
fn cl_say_data(t: &mut Test, label: &str, format: u32) -> Option<usize> {
    // SAFETY: plain call; the handle belongs to the clipboard.
    let h = unsafe { GetClipboardData(format) };
    if h.is_null() {
        say!(t, "{label}: no data error {}", last_error());
        return None;
    }
    let mut buf = [0u8; 64];
    let size = cl_bytes(h, &mut buf);
    let mut b = Buf::<300>::new();
    cl_show(&buf[..size.min(64)], &mut b);
    say!(t, "{label}: size {} bytes \"{}\"", size, b.as_str());
    Some(size)
}

/// Empty the clipboard and close it (open with retries, owner NULL).
fn cl_clean() {
    if cl_open(core::ptr::null_mut()) {
        // SAFETY: the clipboard is open.
        unsafe {
            EmptyClipboard();
            CloseClipboard();
        }
    }
}

/// What a clipboard test window's procedure does on WM_RENDERFORMAT and
/// WM_RENDERALLFORMATS: 0 = nothing (log only), 1 = put a small text
/// (CF_UNICODETEXT "rendered", CF_TEXT "rendered", others 4 bytes) onto the
/// clipboard for the requested format(s).
static CL_SUPPLY: Global<u8> = Global::new(1);

/// The tag the next window created by `cl_window` gets.
static CL_NEXT_TAG: Global<u8> = Global::new(b'?');

/// Messages seen by the window procedure of `cl_window` windows, by number:
/// count of WM_CLIPBOARDUPDATE, WM_DESTROYCLIPBOARD, WM_RENDERFORMAT,
/// WM_RENDERALLFORMATS, WM_DRAWCLIPBOARD, WM_CHANGECBCHAIN.
static CL_COUNTS: Global<[u32; 6]> = Global::new([0; 6]);

fn cl_put_rendered(format: u32) {
    // SAFETY: plain calls; the clipboard is open (the system opened it for
    // the owner during WM_RENDERFORMAT).
    unsafe {
        match format {
            CF_UNICODETEXT => {
                SetClipboardData(format, cl_alloc_w(wz!("rendered")));
            }
            CF_TEXT => {
                SetClipboardData(format, cl_alloc_a(b"rendered\0"));
            }
            _ => {
                SetClipboardData(format, cl_alloc_a(&[1, 2, 3, 4]));
            }
        }
    }
}

/// The window procedure of the clipboard test windows: logs the clipboard
/// messages under the window's tag; WM_RENDERFORMAT / WM_RENDERALLFORMATS
/// supply data when `CL_SUPPLY` is 1.
unsafe extern "system" fn cl_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        // SAFETY: the window is being created on this thread.
        unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, *CL_NEXT_TAG.get() as isize) };
    }
    // SAFETY: a plain query of a window of this thread.
    let tag = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as u8 as char;
    match msg {
        WM_CLIPBOARDUPDATE => {
            CL_COUNTS.get()[0] += 1;
            logln!("{tag} WM_CLIPBOARDUPDATE");
            return 0;
        }
        WM_DESTROYCLIPBOARD => {
            CL_COUNTS.get()[1] += 1;
            logln!("{tag} WM_DESTROYCLIPBOARD");
            return 0;
        }
        WM_RENDERFORMAT => {
            CL_COUNTS.get()[2] += 1;
            logln!("{tag} WM_RENDERFORMAT {}", ClFmt(w as u32));
            if *CL_SUPPLY.get() == 1 {
                cl_put_rendered(w as u32);
            }
            return 0;
        }
        WM_RENDERALLFORMATS => {
            CL_COUNTS.get()[3] += 1;
            logln!("{tag} WM_RENDERALLFORMATS");
            if *CL_SUPPLY.get() == 1 {
                // SAFETY: plain calls: open as the owner, supply, close.
                unsafe {
                    if OpenClipboard(hwnd) != 0 {
                        SetClipboardData(CF_UNICODETEXT, cl_alloc_w(wz!("rendered")));
                        CloseClipboard();
                    }
                }
            }
            return 0;
        }
        WM_DRAWCLIPBOARD => {
            CL_COUNTS.get()[4] += 1;
            logln!("{tag} WM_DRAWCLIPBOARD");
            return 0;
        }
        WM_CHANGECBCHAIN => {
            CL_COUNTS.get()[5] += 1;
            logln!("{tag} WM_CHANGECBCHAIN");
            return 0;
        }
        _ => {}
    }
    // SAFETY: the default handling of the message just received.
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}

/// A hidden popup window (class `PolyClipWnd`, registered on first use)
/// whose procedure is `cl_proc`; `tag` names it in the log.
fn cl_window(tag: u8) -> HWND {
    static REGISTERED: Global<bool> = Global::new(false);
    if !*REGISTERED.get() {
        register(wz!("PolyClipWnd"), Some(cl_proc), 0, 0, core::ptr::null_mut());
        *REGISTERED.get() = true;
    }
    *CL_NEXT_TAG.get() = tag;
    let h = create(0, wz!("PolyClipWnd"), &[], WS_POPUP, 10, 10, 100, 60, core::ptr::null_mut(), 0);
    *CL_NEXT_TAG.get() = b'?';
    h
}

/// Open the clipboard for `w` and empty it; it stays open.
fn cl_begin(w: HWND) {
    cl_open(w);
    // SAFETY: open.
    unsafe { EmptyClipboard() };
}

/// The CF_LOCALE data, as "equals the user default LCID" (the value
/// differs from machine to machine).
fn cl_say_locale(t: &mut Test) {
    // SAFETY: plain calls; the clipboard is open.
    unsafe {
        let h = GetClipboardData(CF_LOCALE);
        if h.is_null() {
            say!(t, "  CF_LOCALE: no data err {}", last_error());
        } else {
            let mut b = [0u8; 8];
            let n = cl_bytes(h, &mut b);
            let v = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            say!(t, "  CF_LOCALE: size {} equals GetUserDefaultLCID {}", n, v == GetUserDefaultLCID());
        }
    }
}

/// The three text formats and the locale as the open clipboard gives them.
fn cl_say_text_views(t: &mut Test) {
    cl_say_data(t, "  UNI", CF_UNICODETEXT);
    cl_say_data(t, "  TEXT", CF_TEXT);
    cl_say_data(t, "  OEM", CF_OEMTEXT);
    cl_say_locale(t);
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn Sleep(ms: u32);
    fn GlobalAlloc(flags: u32, bytes: usize) -> HANDLE;
    fn GlobalLock(mem: HANDLE) -> *mut c_void;
    fn GlobalUnlock(mem: HANDLE) -> BOOL;
    fn GlobalSize(mem: HANDLE) -> usize;
    fn GetUserDefaultLCID() -> u32;
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
    fn SetWindowLongPtrW(hwnd: HWND, index: i32, value: isize) -> isize;
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn OpenClipboard(hwnd: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn SetClipboardData(format: u32, mem: HANDLE) -> HANDLE;
    fn GetClipboardData(format: u32) -> HANDLE;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    cl_clean();
    let a = cl_window(b'A');
    // SAFETY: plain calls.
    unsafe {
        cl_begin(a);
        let h = cl_alloc_w(wz!("Hi there"));
        SetClipboardData(CF_UNICODETEXT, h);
        let g1 = GetClipboardData(CF_UNICODETEXT);
        let g2 = GetClipboardData(CF_UNICODETEXT);
        say!(t, "set handle returned by Get: {} twice the same {}", g1 == h, g1 == g2);
        say!(t, "  GlobalSize {}", GlobalSize(g1));
        CloseClipboard();
        cl_open(core::ptr::null_mut());
        let g3 = GetClipboardData(CF_UNICODETEXT);
        say!(t, "after close and reopen: same handle {}", g3 == h);
        let t1 = GetClipboardData(CF_TEXT);
        let t2 = GetClipboardData(CF_TEXT);
        say!(t, "synthesized CF_TEXT: twice the same {} distinct from the unicode block {}", t1 == t2, t1 != g3);
        cl_say_text_views(t);
        CloseClipboard();

        // no terminating NUL
        cl_begin(a);
        SetClipboardData(CF_UNICODETEXT, cl_alloc_w(&[0x41, 0x42]));
        cl_say_data(t, "unicode 'AB', no NUL, as set", CF_UNICODETEXT);
        CloseClipboard();
        cl_open(core::ptr::null_mut());
        cl_say_data(t, "  after the close", CF_UNICODETEXT);
        cl_say_data(t, "  TEXT", CF_TEXT);
        CloseClipboard();
        cl_begin(a);
        SetClipboardData(CF_TEXT, cl_alloc_a(b"AB"));
        CloseClipboard();
        cl_open(core::ptr::null_mut());
        cl_say_data(t, "ansi 'AB', no NUL, after the close", CF_TEXT);
        cl_say_data(t, "  UNI", CF_UNICODETEXT);
        CloseClipboard();

        // empty string, embedded NUL
        cl_begin(a);
        SetClipboardData(CF_UNICODETEXT, cl_alloc_w(&[0]));
        CloseClipboard();
        cl_open(core::ptr::null_mut());
        cl_say_data(t, "unicode empty string", CF_UNICODETEXT);
        cl_say_data(t, "  TEXT", CF_TEXT);
        CloseClipboard();
        cl_begin(a);
        SetClipboardData(CF_UNICODETEXT, cl_alloc_w(&[0x41, 0, 0x42, 0]));
        CloseClipboard();
        cl_open(core::ptr::null_mut());
        cl_say_data(t, "unicode 'A',NUL,'B',NUL", CF_UNICODETEXT);
        cl_say_data(t, "  TEXT", CF_TEXT);
        CloseClipboard();

        // a long text
        let mut long = [0x78u16; 1001];
        long[1000] = 0;
        cl_begin(a);
        SetClipboardData(CF_UNICODETEXT, cl_alloc_w(&long));
        CloseClipboard();
        cl_open(core::ptr::null_mut());
        let g = GetClipboardData(CF_UNICODETEXT);
        let tx = GetClipboardData(CF_TEXT);
        say!(t, "1000 characters: unicode size {} ansi size {}", GlobalSize(g), GlobalSize(tx));
        CloseClipboard();
        DestroyWindow(a);
    }
    cl_clean();
    t.check("done", true, 0);
}
