//! WSAAsyncSelect: the window messages of a TCP connection on the loopback
//! interface: FD_CONNECT and FD_WRITE for the connecting side, FD_ACCEPT for
//! the listening side, FD_READ when data arrives, FD_CLOSE when the peer
//! closes; wParam is the socket and the low and high words of lParam the
//! event and the error. The sockets turn non-blocking.
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
const GWLP_USERDATA: i32 = -21;
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

/// Dispatch everything waiting, then keep waiting up to `ms` for anything
/// further (input arrives asynchronously), dispatching as it comes.
fn settle(ms: u32) -> u32 {
    let mut n = pump();
    // SAFETY: plain timing call.
    let start = unsafe { GetTickCount() };
    loop {
        // SAFETY: plain timing call.
        let elapsed = unsafe { GetTickCount() }.wrapping_sub(start);
        if elapsed >= ms {
            break;
        }
        // SAFETY: no handles; wakes on any input or message.
        unsafe { MsgWaitForMultipleObjects(0, core::ptr::null(), 0, ms - elapsed, QS_ALLINPUT) };
        n += pump();
    }
    n
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

type SOCKET = usize;
/// winsock2.h
#[repr(C)]
#[derive(Clone, Copy)]
struct WSADATA {
    version: u16,
    high_version: u16,
    max_sockets: u16,
    max_udp_dg: u16,
    vendor_info: *mut u8,
    description: [u8; 257],
    system_status: [u8; 129],
}

/// winsock2.h
#[repr(C)]
#[derive(Clone, Copy)]
struct SOCKADDR_IN {
    family: u16,
    port: u16,
    addr: u32,
    zero: [u8; 8],
}

const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;
const IPPROTO_TCP: i32 = 6;
const INVALID_SOCKET: usize = !0;
const FD_READ: i32 = 0x01;
const FD_WRITE: i32 = 0x02;
const FD_OOB: i32 = 0x04;
const FD_ACCEPT: i32 = 0x08;
const FD_CONNECT: i32 = 0x10;
const FD_CLOSE: i32 = 0x20;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> HINSTANCE;
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetCurrentProcessId() -> u32;
    fn Sleep(ms: u32);
    fn GetTickCount() -> u32;
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
    fn GetWindowThreadProcessId(hwnd: HWND, pid: *mut u32) -> u32;
    fn PeekMessageW(msg: *mut MSG, hwnd: HWND, min: u32, max: u32, remove: u32) -> BOOL;
    fn TranslateMessage(msg: *const MSG) -> BOOL;
    fn DispatchMessageW(msg: *const MSG) -> LRESULT;
    fn DefWindowProcW(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT;
    fn MsgWaitForMultipleObjects(n: u32, handles: *const HANDLE, all: BOOL, ms: u32, mask: u32) -> u32;
    fn GetSystemMetrics(index: i32) -> i32;
    fn SendInput(n: u32, inputs: *const INPUT, size: i32) -> u32;
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut WSADATA) -> i32;
    fn WSACleanup() -> i32;
    fn WSAGetLastError() -> i32;
    fn socket(af: i32, ty: i32, proto: i32) -> SOCKET;
    fn bind(s: SOCKET, name: *const SOCKADDR_IN, len: i32) -> i32;
    fn listen(s: SOCKET, backlog: i32) -> i32;
    fn accept(s: SOCKET, addr: *mut SOCKADDR_IN, len: *mut i32) -> SOCKET;
    fn connect(s: SOCKET, name: *const SOCKADDR_IN, len: i32) -> i32;
    #[link_name = "send"]
    fn wsend(s: SOCKET, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: SOCKET, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn closesocket(s: SOCKET) -> i32;
    fn getsockname(s: SOCKET, name: *mut SOCKADDR_IN, len: *mut i32) -> i32;
    fn WSAAsyncSelect(s: SOCKET, hwnd: HWND, msg: u32, events: i32) -> i32;
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

// ---- Winsock window-message helpers of these tests ----

static EVS: Global<[(u32, usize, u32, u32); 256]> = Global::new([(0, 0, 0, 0); 256]);
static EVN: Global<usize> = Global::new(0);
static LABELS: Global<[(usize, &'static str); 12]> = Global::new([(0, ""); 12]);

unsafe extern "system" fn ws_wp(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if (0x8001..0x8010).contains(&m) {
        let n = *EVN.get();
        if n < 256 {
            EVS.get()[n] = (m, w, (l as u32) & 0xFFFF, ((l as u32) >> 16) & 0xFFFF);
            *EVN.get() = n + 1;
        }
        return 0;
    }
    // SAFETY: default processing.
    unsafe { DefWindowProcW(h, m, w, l) }
}

/// A window of the test that is never shown.
fn ws_window() -> HWND {
    register(wz!("WsaTestWnd"), Some(ws_wp), 0, 0, core::ptr::null_mut());
    create(0, wz!("WsaTestWnd"), wz!("wsa"), 0, 0, 0, 10, 10, core::ptr::null_mut(), 0)
}

/// Remember a name for a socket (its number is not printed).
fn ws_label(s: SOCKET, name: &'static str) {
    for e in LABELS.get().iter_mut() {
        if e.0 == 0 || e.0 == s {
            *e = (s, name);
            return;
        }
    }
}

fn label_of(s: usize) -> &'static str {
    for e in LABELS.get().iter() {
        if e.0 == s && s != 0 {
            return e.1;
        }
    }
    "unknown socket"
}

fn ev_name(e: u32) -> &'static str {
    match e {
        1 => "FD_READ",
        2 => "FD_WRITE",
        4 => "FD_OOB",
        8 => "FD_ACCEPT",
        0x10 => "FD_CONNECT",
        0x20 => "FD_CLOSE",
        _ => "FD_?",
    }
}

/// Dispatch messages for `ms`.
fn ws_wait(ms: u32) {
    settle(ms);
}

/// Print and remove the notifications of message `m` (arrival order).
fn ws_dump(t: &mut Test, what: &str, m: u32) -> usize {
    let n = *EVN.get();
    let mut keep = 0usize;
    let mut shown = 0usize;
    for i in 0..n {
        let e = EVS.get()[i];
        if e.0 == m {
            say!(t, "{what}: {} for {} error {}", ev_name(e.2), label_of(e.1), e.3);
            shown += 1;
        } else {
            EVS.get()[keep] = e;
            keep += 1;
        }
    }
    *EVN.get() = keep;
    if shown == 0 {
        say!(t, "{what}: none");
    }
    shown
}

/// Number of waiting notifications of message `m` with event `ev` (0: any).
fn ws_count(m: u32, ev: i32) -> usize {
    let mut c = 0;
    for i in 0..*EVN.get() {
        let e = EVS.get()[i];
        if e.0 == m && (ev == 0 || e.2 as i32 == ev) {
            c += 1;
        }
    }
    c
}

fn ws_clear() {
    *EVN.get() = 0;
}

fn ws_err() -> i32 {
    // SAFETY: plain call.
    unsafe { WSAGetLastError() }
}

fn wsa_init() -> i32 {
    let mut d: WSADATA = zeroed();
    // SAFETY: d is writable.
    unsafe { WSAStartup(0x0202, &mut d) }
}

fn loopback(port: u16) -> SOCKADDR_IN {
    SOCKADDR_IN { family: 2, port: port.to_be(), addr: 0x0100_007F, zero: [0; 8] }
}

/// A listening socket on a loopback port: (socket, port).
fn new_listener() -> (SOCKET, u16) {
    // SAFETY: plain socket calls on our own sockets.
    unsafe {
        let s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        let a = loopback(0);
        bind(s, &a, 16);
        let mut b: SOCKADDR_IN = zeroed();
        let mut n = 16;
        getsockname(s, &mut b, &mut n);
        listen(s, 4);
        (s, u16::from_be(b.port))
    }
}

fn new_socket() -> SOCKET {
    // SAFETY: plain call.
    unsafe { socket(AF_INET, SOCK_STREAM, IPPROTO_TCP) }
}

/// recv of up to `n` bytes: (result, error).
fn do_recv(s: SOCKET, n: usize) -> (i32, i32) {
    let mut b = [0u8; 512];
    // SAFETY: b has 512 bytes.
    let r = unsafe { recv(s, b.as_mut_ptr(), n.min(512) as i32, 0) };
    (r, if r < 0 { ws_err() } else { 0 })
}

fn do_send(s: SOCKET, n: usize) -> (i32, i32) {
    let b = [b'x'; 512];
    // SAFETY: b has 512 bytes.
    let r = unsafe { wsend(s, b.as_ptr(), n.min(512) as i32, 0) };
    (r, if r < 0 { ws_err() } else { 0 })
}

/// Connect `c` to the listener at `port` and accept: the accepted socket.
fn connect_accept(c: SOCKET, l: SOCKET, port: u16) -> SOCKET {
    let a = loopback(port);
    // SAFETY: plain socket calls.
    unsafe {
        connect(c, &a, 16);
        let mut tries = 0;
        loop {
            let s = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
            if s != INVALID_SOCKET {
                return s;
            }
            tries += 1;
            if tries > 200 {
                return INVALID_SOCKET;
            }
            Sleep(10);
        }
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    say!(t, "WSAStartup = {}", wsa_init());
    let w = ws_window();
    // SAFETY: calls on our own sockets and window.
    unsafe {
        let (l, port) = new_listener();
        ws_label(l, "listener");
        say!(t, "WSAAsyncSelect(listener, 0x8001, FD_ACCEPT) = {}", WSAAsyncSelect(l, w, 0x8001, FD_ACCEPT));
        let c = new_socket();
        ws_label(c, "client");
        let cm = FD_CONNECT | FD_READ | FD_WRITE | FD_CLOSE;
        say!(t, "WSAAsyncSelect(client, 0x8002, FD_CONNECT|FD_READ|FD_WRITE|FD_CLOSE) = {}", WSAAsyncSelect(c, w, 0x8002, cm));
        let a = loopback(port);
        let r = connect(c, &a, 16);
        say!(t, "connect = {} error {}", r, ws_err());
        ws_wait(300);
        ws_dump(t, "listener window messages", 0x8001);
        ws_dump(t, "client window messages", 0x8002);
        // accept: the new socket has the listener's registration (message 0x8001, FD_ACCEPT) and is not blocking
        let s = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
        say!(t, "accept gave a socket: {}", s != INVALID_SOCKET);
        ws_label(s, "accepted");
        let (rr, re) = do_recv(s, 10);
        say!(t, "recv on the accepted socket with nothing to read: {rr} error {re}");
        ws_wait(100);
        ws_dump(t, "listener window messages after accept", 0x8001);
        // the accepted socket gets its own registration
        say!(t, "WSAAsyncSelect(accepted, 0x8003, FD_READ|FD_WRITE|FD_CLOSE) = {}", WSAAsyncSelect(s, w, 0x8003, FD_READ | FD_WRITE | FD_CLOSE));
        ws_wait(200);
        ws_dump(t, "accepted socket messages after registering", 0x8003);
        // data from the client
        let (sr, se) = do_send(c, 5);
        say!(t, "client sends 5 bytes: {sr} error {se}");
        ws_wait(200);
        ws_dump(t, "accepted socket messages after the data", 0x8003);
        ws_dump(t, "client messages after sending", 0x8002);
        let (rr, _) = do_recv(s, 100);
        say!(t, "accepted socket reads {rr} bytes");
        let (rr, re) = do_recv(s, 100);
        say!(t, "and then {rr} error {re}");
        ws_wait(100);
        ws_dump(t, "accepted socket messages after reading everything", 0x8003);
        // the other direction
        do_send(s, 3);
        ws_wait(200);
        ws_dump(t, "client messages after data from the accepted socket", 0x8002);
        do_recv(c, 100);
        // an orderly close by the accepted side
        say!(t, "closesocket(accepted) = {}", closesocket(s));
        ws_wait(300);
        ws_dump(t, "client messages after the peer closed", 0x8002);
        let (rr, re) = do_recv(c, 100);
        say!(t, "client recv after the close: {rr} error {re}");
        say!(t, "closesocket(client) = {}", closesocket(c));
        // a connection to a port nobody listens on
        let dead = new_socket();
        ws_label(dead, "refused");
        WSAAsyncSelect(dead, w, 0x8004, FD_CONNECT | FD_WRITE | FD_READ | FD_CLOSE);
        let x = loopback(port);
        closesocket(l);
        let r = connect(dead, &x, 16);
        say!(t, "connect to a closed port = {} error {}", r, ws_err());
        ws_wait(4000);
        ws_dump(t, "refused connection messages", 0x8004);
        closesocket(dead);
        DestroyWindow(w);
        say!(t, "WSACleanup = {}", WSACleanup());
    }
}
