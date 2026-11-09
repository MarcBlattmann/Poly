//! EmptyClipboard from threads: a thread that did not open the clipboard cannot empty it
//! (ERROR_CLIPBOARD_NOT_OPEN) even while another thread has it open; a thread that opened it with
//! NULL while another also opened it with NULL can.
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

const WM_NULL: u32 = 0x0000;
const WM_CREATE: u32 = 0x0001;
const WM_DESTROY: u32 = 0x0002;
const WM_MOVE: u32 = 0x0003;
const WM_SIZE: u32 = 0x0005;
const WM_ENABLE: u32 = 0x000A;
const WM_SETTEXT: u32 = 0x000C;
const WM_GETTEXT: u32 = 0x000D;
const WM_GETTEXTLENGTH: u32 = 0x000E;
const WM_PAINT: u32 = 0x000F;
const WM_CLOSE: u32 = 0x0010;
const WM_QUIT: u32 = 0x0012;
const WM_ERASEBKGND: u32 = 0x0014;
const WM_SHOWWINDOW: u32 = 0x0018;
const WM_GETMINMAXINFO: u32 = 0x0024;
const WM_WINDOWPOSCHANGING: u32 = 0x0046;
const WM_WINDOWPOSCHANGED: u32 = 0x0047;
const WM_STYLECHANGING: u32 = 0x007C;
const WM_STYLECHANGED: u32 = 0x007D;
const WM_NCCREATE: u32 = 0x0081;
const WM_NCDESTROY: u32 = 0x0082;
const WM_NCCALCSIZE: u32 = 0x0083;
const WM_NCPAINT: u32 = 0x0085;
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_DEADCHAR: u32 = 0x0103;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
const WM_COMMAND: u32 = 0x0111;
const WM_SYSCOMMAND: u32 = 0x0112;
const WM_TIMER: u32 = 0x0113;
const WM_PARENTNOTIFY: u32 = 0x0210;
const WM_CONTEXTMENU: u32 = 0x007B;
const WM_CHILDACTIVATE: u32 = 0x0022;
const WM_USER: u32 = 0x0400;
const WS_POPUP: u32 = 0x8000_0000;
const GWLP_USERDATA: i32 = -21;
const PM_REMOVE: u32 = 0x0001;
const WAIT_OBJECT_0: u32 = 0;
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

/// The documented messages the sequence tests record; everything else
/// (undocumented and environment-dependent traffic) is left out, and so are
/// the painting messages (WM_PAINT, WM_ERASEBKGND, WM_NCPAINT), which come
/// only for windows on a visible desktop.
fn documented(msg: u32) -> bool {
    matches!(
        msg,
        WM_NCCREATE
            | WM_CREATE
            | WM_NCCALCSIZE
            | WM_GETMINMAXINFO
            | WM_SIZE
            | WM_MOVE
            | WM_SHOWWINDOW
            | WM_WINDOWPOSCHANGING
            | WM_WINDOWPOSCHANGED
            | WM_DESTROY
            | WM_NCDESTROY
            | WM_PARENTNOTIFY
            | WM_SETTEXT
            | WM_GETTEXT
            | WM_GETTEXTLENGTH
            | WM_STYLECHANGING
            | WM_STYLECHANGED
            | WM_CLOSE
            | WM_TIMER
            | WM_KEYDOWN
            | WM_KEYUP
            | WM_CHAR
            | WM_SYSKEYDOWN
            | WM_SYSKEYUP
            | WM_SYSCHAR
            | WM_DEADCHAR
            | WM_SYSCOMMAND
            | WM_CONTEXTMENU
            | WM_COMMAND
            | WM_ENABLE
            | WM_CHILDACTIVATE
            | WM_QUIT
            | WM_NULL
    ) || (WM_USER..0xC000).contains(&msg)
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

/// winuser.h: INPUT on x64 (type, then the union at offset 8; 40 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct INPUT {
    kind: u32,
    pad: u32,
    data: [u64; 4],
}

/// SendInput of a list; returns how many were inserted.
fn send(inputs: &[INPUT]) -> u32 {
    // SAFETY: a slice of filled INPUTs of the documented size.
    unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), core::mem::size_of::<INPUT>() as i32) }
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
const CF_PRIVATEFIRST: u32 = 0x0200;
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

/// The formats on the clipboard in enumeration order (the clipboard must be
/// open for a sensible answer); returns how many.
fn cl_formats(out: &mut [u32; 32]) -> usize {
    let mut n = 0;
    let mut f = 0u32;
    loop {
        // SAFETY: plain call.
        f = unsafe { EnumClipboardFormats(f) };
        if f == 0 || n >= 32 {
            break;
        }
        out[n] = f;
        n += 1;
    }
    n
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

/// The formats list as text: names joined with " ", or "(none)".
fn cl_list(fs: &[u32; 32], n: usize, out: &mut Buf<300>) {
    if n == 0 {
        let _ = out.write_str("(none)");
    }
    for i in 0..n {
        if i > 0 {
            let _ = out.write_char(' ');
        }
        let _ = write!(out, "{}", ClFmt(fs[i]));
    }
}

/// Print the formats now on the clipboard (open it first) as one line.
fn cl_say_formats(t: &mut Test, label: &str) {
    let mut fs = [0u32; 32];
    let n = cl_formats(&mut fs);
    let mut b = Buf::<300>::new();
    cl_list(&fs, n, &mut b);
    say!(t, "{label}: {} [{}]", n, b.as_str());
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

/// Run `f` on a new thread and wait for it, dispatching this thread's
/// messages while it waits (the thread may send a message to a window of
/// this one, for instance WM_DESTROYCLIPBOARD to the old owner).
fn cl_in_thread(f: THREADPROC) -> u32 {
    // SAFETY: a thread that runs `f` and ends; waited for and closed.
    unsafe {
        let mut id = 0u32;
        let h = CreateThread(core::ptr::null_mut(), 0, f, core::ptr::null_mut(), 0, &mut id);
        if h.is_null() {
            return 0xFFFF_FFFF;
        }
        for _ in 0..1000 {
            pump();
            if WaitForSingleObject(h, 10) == WAIT_OBJECT_0 {
                break;
            }
        }
        CloseHandle(h);
        id
    }
}

/// Results a helper thread leaves for the test: 8 values (atomics, so the
/// compiler cannot move, merge or drop the accesses across threads).
struct ClOut([core::sync::atomic::AtomicI64; 8]);

impl ClOut {
    const fn new() -> Self {
        ClOut([const { core::sync::atomic::AtomicI64::new(0) }; 8])
    }
    fn set(&self, i: usize, v: i64) {
        self.0[i].store(v, core::sync::atomic::Ordering::SeqCst)
    }
    fn at(&self, i: usize) -> i64 {
        self.0[i].load(core::sync::atomic::Ordering::SeqCst)
    }
    fn fill(&self, v: i64) {
        for a in &self.0 {
            a.store(v, core::sync::atomic::Ordering::SeqCst)
        }
    }
    fn all(&self) -> [i64; 8] {
        core::array::from_fn(|i| self.at(i))
    }
}

static CL_THREAD_OUT: ClOut = ClOut::new();
/// A number shared between threads (a mode, a window handle).
struct ClCell(core::sync::atomic::AtomicUsize);

impl ClCell {
    const fn new(v: usize) -> Self {
        ClCell(core::sync::atomic::AtomicUsize::new(v))
    }
    fn get(&self) -> usize {
        self.0.load(core::sync::atomic::Ordering::SeqCst)
    }
    fn set(&self, v: usize) {
        self.0.store(v, core::sync::atomic::Ordering::SeqCst)
    }
}

/// Open the clipboard for `w` and empty it; it stays open.
fn cl_begin(w: HWND) {
    cl_open(w);
    // SAFETY: open.
    unsafe { EmptyClipboard() };
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
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
    fn GlobalAlloc(flags: u32, bytes: usize) -> HANDLE;
    fn GlobalLock(mem: HANDLE) -> *mut c_void;
    fn GlobalUnlock(mem: HANDLE) -> BOOL;
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
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn SendInput(n: u32, inputs: *const INPUT, size: i32) -> u32;
    fn OpenClipboard(hwnd: HWND) -> BOOL;
    fn CloseClipboard() -> BOOL;
    fn EmptyClipboard() -> BOOL;
    fn SetClipboardData(format: u32, mem: HANDLE) -> HANDLE;
    fn EnumClipboardFormats(format: u32) -> u32;
}

poly_wtest::main!(test);

static MODE: ClCell = ClCell::new(0);

unsafe extern "system" fn thr(_: *mut c_void) -> u32 {
    // SAFETY: plain calls.
    unsafe {
        let o = &CL_THREAD_OUT;
        set_error(0);
        match MODE.get() {
            1 => {
                o.set(0, EmptyClipboard() as i64);
                o.set(1, last_error() as i64);
            }
            2 => {
                let r = OpenClipboard(core::ptr::null_mut());
                o.set(0, r as i64);
                set_error(0);
                o.set(1, EmptyClipboard() as i64);
                o.set(2, last_error() as i64);
                o.set(3, CloseClipboard() as i64);
            }
            _ => {}
        }
    }
    0
}

fn run(mode: u32) -> [i64; 8] {
    MODE.set((mode) as usize);
    CL_THREAD_OUT.fill(-9);
    cl_in_thread(thr);
    CL_THREAD_OUT.all()
}

fn test(t: &mut Test) {
    cl_clean();
    let a = cl_window(b'A');
    // SAFETY: plain calls.
    unsafe {
        cl_begin(core::ptr::null_mut());
        SetClipboardData(CF_PRIVATEFIRST, cl_alloc_a(b"keep"));
        CloseClipboard();
        // closed: the thread cannot empty
        let o = run(1);
        say!(t, "closed, thread Empty: {} err {}", o[0], o[1]);
        // open for A in main: the thread cannot empty
        cl_open(a);
        let o = run(1);
        say!(t, "main has A open, thread Empty: {} err {}", o[0], o[1]);
        cl_say_formats(t, "  formats");
        CloseClipboard();
        // open for NULL in main: the thread opens for NULL and empties
        cl_open(core::ptr::null_mut());
        let o = run(2);
        say!(t, "main has NULL open, thread Open(NULL) {} Empty {} err {} Close {}", o[0], o[1], o[2], o[3]);
        cl_say_formats(t, "  formats");
        set_error(0);
        say!(t, "  main Close: {} err {}", CloseClipboard(), last_error());
        DestroyWindow(a);
    }
    cl_clean();
    t.check("done", true, 0);
}
