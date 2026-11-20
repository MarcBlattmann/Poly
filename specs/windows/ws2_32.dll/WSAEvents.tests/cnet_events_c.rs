#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, unused_imports, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;
type DWORD = u32;

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

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; no characters outside the BMP).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<700> {
    let mut b = Buf::<700>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// The units of a NUL-terminated UTF-16 string at `p` (at most `max`).
fn wstr_at<'a>(p: *const u16, max: usize) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string the system returned; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// Whether two UTF-16 strings are equal without regard to ASCII case.
fn eq_nocase(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(&x, &y)| {
            let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
            l(x) == l(y)
        })
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
}

fn gle() -> i64 {
    // SAFETY: plain call.
    i64::from(unsafe { GetLastError() })
}

fn clear_error() {
    // SAFETY: plain call.
    unsafe { SetLastError(0xDEAD) };
}

/// The last error after a failed call, else -2 (success leaves it unspecified).
fn gle_if(ok: BOOL) -> i64 {
    if ok == 0 { gle() } else { -2 }
}

const PART: u32 = 3;

// ---- the observations are written in three parts (as the other multi-file entries do): this program writes every third line ----
static SAY_N: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let n = SAY_N.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        if PART == 0 || n % 3 == PART - 1 {
            $t.line(b.as_str());
        }
    }};
}

// ---- file helpers (kernel32; prototypes from Learn) ----
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn RemoveDirectoryW(name: *const u16) -> BOOL;
    fn CreateDirectoryW(name: *const u16, sa: *mut c_void) -> BOOL;
    fn GetFileSize(h: HANDLE, hi: *mut u32) -> u32;
    fn GetFileAttributesW(name: *const u16) -> u32;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

/// The path of `name` in the directory of the running image (a fresh scratch directory per run on Windows), as UTF-16.
fn scratch(name: &str) -> [u16; 300] {
    let img = poly_wtest::image_path();
    let mut out = [0u16; 300];
    let mut last = 0;
    for (i, &c) in img.iter().enumerate() {
        if c == 0x5c {
            last = i + 1;
        }
    }
    let mut n = 0;
    for &c in &img[..last] {
        if n < 299 {
            out[n] = c;
            n += 1;
        }
    }
    for c in name.chars() {
        if n < 299 {
            out[n] = c as u32 as u16;
            n += 1;
        }
    }
    out
}

/// Reads a whole file (up to N bytes); `None` when it cannot be opened.
fn read_all<const N: usize>(path: &[u16]) -> Option<([u8; N], usize)> {
    let mut buf = [0u8; N];
    let mut got = 0u32;
    // SAFETY: valid NUL-terminated path and buffer.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_READ, 7, core::ptr::null_mut(), OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            return None;
        }
        ReadFile(h, buf.as_mut_ptr(), N as u32, &mut got, core::ptr::null_mut());
        CloseHandle(h);
    }
    Some((buf, got as usize))
}

/// Writes a whole file; false on failure.
fn write_all(path: &[u16], data: &[u8]) -> bool {
    let mut got = 0u32;
    // SAFETY: valid NUL-terminated path and data.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            return false;
        }
        let ok = WriteFile(h, data.as_ptr(), data.len() as u32, &mut got, core::ptr::null_mut());
        CloseHandle(h);
        ok != 0 && got as usize == data.len()
    }
}

fn exists(path: &[u16]) -> bool {
    // SAFETY: valid NUL-terminated path.
    unsafe { GetFileAttributesW(path.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

fn hexdump(t: &mut Test, d: &[u8]) {
    for (i, ch) in d.chunks(32).enumerate() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "{:04x}:", i * 32);
        for x in ch {
            let _ = write!(b, " {:02x}", x);
        }
        t.line(b.as_str());
    }
}

// ---- overlapped I/O, named pipes and completion ports (kernel32; prototypes from Learn, constants from the public headers) ----

#[repr(C)]
#[derive(Clone, Copy)]
struct OVERLAPPED {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    h_event: HANDLE,
}

impl OVERLAPPED {
    const fn new() -> OVERLAPPED {
        OVERLAPPED { internal: 0, internal_high: 0, offset: 0, offset_high: 0, h_event: core::ptr::null_mut() }
    }
    fn with_event(ev: HANDLE) -> OVERLAPPED {
        OVERLAPPED { h_event: ev, ..OVERLAPPED::new() }
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buf: u32,
        in_buf: u32,
        default_timeout: u32,
        sa: *mut c_void,
    ) -> HANDLE;
    fn ConnectNamedPipe(h: HANDLE, ov: *mut OVERLAPPED) -> BOOL;
    fn DisconnectNamedPipe(h: HANDLE) -> BOOL;
    fn CreateEventW(sa: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn ResetEvent(h: HANDLE) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetOverlappedResult(h: HANDLE, ov: *mut OVERLAPPED, n: *mut u32, wait: BOOL) -> BOOL;
    fn CreateIoCompletionPort(file: HANDLE, port: HANDLE, key: usize, threads: u32) -> HANDLE;
    fn GetQueuedCompletionStatus(port: HANDLE, n: *mut u32, key: *mut usize, ov: *mut *mut OVERLAPPED, ms: u32) -> BOOL;
    fn PostQueuedCompletionStatus(port: HANDLE, n: u32, key: usize, ov: *mut OVERLAPPED) -> BOOL;
    fn CancelIoEx(h: HANDLE, ov: *mut OVERLAPPED) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn CreateThread(
        sa: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn Sleep(ms: u32);
}

const PIPE_ACCESS_DUPLEX: u32 = 3;
const FILE_FLAG_OVERLAPPED: u32 = 0x4000_0000;
const PIPE_WAIT: u32 = 0;
const ERROR_IO_PENDING: u32 = 997;
const ERROR_PIPE_CONNECTED: u32 = 535;
const ERROR_BROKEN_PIPE: u32 = 109;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_NO_DATA: u32 = 232;
const ERROR_OPERATION_ABORTED: u32 = 995;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 0x102;
const INFINITE: u32 = 0xFFFF_FFFF;
const INVALID_HANDLE: HANDLE = -1isize as HANDLE;

/// A pipe name `\\.\pipe\PolySpecB4-<tag>-<pid>` as UTF-16 (unique per process).
fn pipe_name(tag: &str) -> [u16; 80] {
    let mut s = Buf::<80>::new();
    // SAFETY: plain call.
    let pid = unsafe { GetCurrentProcessId() };
    let _ = write!(s, "\\\\.\\pipe\\PolySpecB4-{tag}-{pid}");
    wide80(s.as_str())
}

fn wide80(s: &str) -> [u16; 80] {
    let mut w = [0u16; 80];
    for (i, c) in s.chars().enumerate().take(79) {
        w[i] = c as u32 as u16;
    }
    w
}

/// A connected overlapped byte pipe: (server end, client end). Both ends are opened FILE_FLAG_OVERLAPPED.
fn pipe_pair(tag: &str, out_buf: u32, in_buf: u32) -> (HANDLE, HANDLE) {
    let n = pipe_name(tag);
    // SAFETY: plain calls with live buffers.
    unsafe {
        let s = CreateNamedPipeW(n.as_ptr(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED, PIPE_WAIT, 1, out_buf, in_buf, 0, core::ptr::null_mut());
        let c = CreateFileW(n.as_ptr(), GENERIC_READ | GENERIC_WRITE, 0, core::ptr::null_mut(), OPEN_EXISTING, FILE_FLAG_OVERLAPPED, core::ptr::null_mut());
        // the client connected first: the server's connect reports it
        let mut ov = OVERLAPPED::new();
        ConnectNamedPipe(s, &mut ov);
        (s, c)
    }
}

fn manual_event() -> HANDLE {
    // SAFETY: plain call.
    unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) }
}

/// Whether the event is signalled now (a zero-time wait).
fn is_set(ev: HANDLE) -> bool {
    // SAFETY: valid handle.
    unsafe { WaitForSingleObject(ev, 0) == WAIT_OBJECT_0 }
}

/// What an overlapped call left: (returned TRUE, last error if FALSE else 0xFFFF).
fn outcome(ok: BOOL) -> (bool, u32) {
    if ok != 0 { (true, 0xFFFF) } else { (false, gle() as u32) }
}

fn yn(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

// ---- Winsock (ws2_32; prototypes from Learn, constants from winsock2.h / ws2def.h / mstcpip.h) ----

type SOCKET = usize;
const INVALID_SOCKET: SOCKET = !0;
const SOCKET_ERROR: i32 = -1;
const AF_INET: i32 = 2;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_TCP: i32 = 6;
const IPPROTO_UDP: i32 = 17;
const WSA_FLAG_OVERLAPPED: u32 = 1;
const FIONBIO: i32 = 0x8004_667Eu32 as i32;
const SIO_KEEPALIVE_VALS: u32 = 0x9800_0004;
const FD_READ: i32 = 1;
const FD_WRITE: i32 = 2;
const FD_ACCEPT: i32 = 8;
const FD_CONNECT: i32 = 0x10;
const FD_CLOSE: i32 = 0x20;
const MSG_PEEK: i32 = 2;
const SOL_SOCKET: i32 = 0xFFFF;
const SO_REUSEADDR: i32 = 4;
const TCP_NODELAY: i32 = 1;
const SD_SEND: i32 = 1;
const WSAEWOULDBLOCK: i32 = 10035;
const WSA_IO_PENDING: i32 = 997;

#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrIn {
    family: u16,
    port: u16,
    addr: [u8; 4],
    zero: [u8; 8],
}

impl SockAddrIn {
    fn loopback(port_host_order: u16) -> SockAddrIn {
        SockAddrIn { family: 2, port: port_host_order.to_be(), addr: [127, 0, 0, 1], zero: [0; 8] }
    }
    fn port(&self) -> u16 {
        u16::from_be(self.port)
    }
}

#[repr(C)]
struct WSANETWORKEVENTS {
    events: i32,
    errors: [i32; 10],
}

#[repr(C)]
struct WSABUF {
    len: u32,
    buf: *mut u8,
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(ver: u16, data: *mut u8) -> i32;
    fn WSAGetLastError() -> i32;
    fn WSASocketW(af: i32, ty: i32, proto: i32, info: *const c_void, group: u32, flags: u32) -> SOCKET;
    fn closesocket(s: SOCKET) -> i32;
    fn shutdown(s: SOCKET, how: i32) -> i32;
    fn bind(s: SOCKET, addr: *const SockAddrIn, len: i32) -> i32;
    fn listen(s: SOCKET, backlog: i32) -> i32;
    fn accept(s: SOCKET, addr: *mut SockAddrIn, len: *mut i32) -> SOCKET;
    fn connect(s: SOCKET, addr: *const SockAddrIn, len: i32) -> i32;
    fn getsockname(s: SOCKET, addr: *mut SockAddrIn, len: *mut i32) -> i32;
    fn setsockopt(s: SOCKET, level: i32, name: i32, val: *const u8, len: i32) -> i32;
    fn ioctlsocket(s: SOCKET, cmd: i32, arg: *mut u32) -> i32;
    fn WSAIoctl(s: SOCKET, code: u32, inb: *const u8, inlen: u32, outb: *mut u8, outlen: u32, bytes: *mut u32, ov: *mut c_void, cr: *mut c_void) -> i32;
    fn send(s: SOCKET, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: SOCKET, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn WSASend(s: SOCKET, bufs: *const WSABUF, count: u32, sent: *mut u32, flags: u32, ov: *mut OVERLAPPED, cr: *mut c_void) -> i32;
    fn WSAEventSelect(s: SOCKET, ev: HANDLE, mask: i32) -> i32;
    fn WSAEnumNetworkEvents(s: SOCKET, ev: HANDLE, out: *mut WSANETWORKEVENTS) -> i32;
    fn WSACreateEvent() -> HANDLE;
    fn WSAResetEvent(ev: HANDLE) -> BOOL;
    fn WSASetEvent(ev: HANDLE) -> BOOL;
    fn WSACloseEvent(ev: HANDLE) -> BOOL;
    fn htons(v: u16) -> u16;
    fn ntohs(v: u16) -> u16;
    fn htonl(v: u32) -> u32;
    fn ntohl(v: u32) -> u32;
}

fn wsa_init() {
    let mut d = [0u8; 512];
    // SAFETY: a buffer larger than WSADATA.
    unsafe { WSAStartup(0x0202, d.as_mut_ptr()) };
}

fn wsae() -> i32 {
    // SAFETY: plain call.
    unsafe { WSAGetLastError() }
}

/// A listening loopback TCP socket on an ephemeral port: (socket, port).
fn listener() -> (SOCKET, u16) {
    // SAFETY: plain calls with live buffers.
    unsafe {
        let s = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, core::ptr::null(), 0, WSA_FLAG_OVERLAPPED);
        let a = SockAddrIn::loopback(0);
        bind(s, &a, 16);
        listen(s, 8);
        let mut b = SockAddrIn::loopback(0);
        let mut l = 16;
        getsockname(s, &mut b, &mut l);
        (s, b.port())
    }
}

poly_wtest::main!(test);

// WSACreateEvent, WSAResetEvent, WSASetEvent, WSACloseEvent: Chrome makes and destroys one event per socket operation
// (about 70 per page); what do the calls return, what state do they leave, which errors do bad handles give?

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DuplicateHandle(sp: HANDLE, sh: HANDLE, tp: HANDLE, th: *mut HANDLE, access: u32, inherit: BOOL, opts: u32) -> BOOL;
}

fn test(t: &mut Test) {
    wsa_init();
    // SAFETY: plain call.
    let ev = unsafe { WSACreateEvent() };
    say!(t, "WSACreateEvent: a handle {}", yn(!ev.is_null() && ev != INVALID_HANDLE));
    say!(t, "it starts non-signalled: {}", yn(!is_set(ev)));

    // manual reset: SetEvent stays signalled through waits, WSAResetEvent clears it
    // SAFETY: valid handle.
    let r = unsafe { WSASetEvent(ev) };
    say!(t, "WSASetEvent: {}; signalled {}; still signalled after a wait {}", yn(r != 0), yn(is_set(ev)), yn(is_set(ev)));
    // SAFETY: valid handle.
    let r = unsafe { WSAResetEvent(ev) };
    say!(t, "WSAResetEvent: {}; signalled {}", yn(r != 0), yn(is_set(ev)));
    // SAFETY: valid handle.
    let r = unsafe { WSAResetEvent(ev) };
    say!(t, "WSAResetEvent of an event that is not set: {}", yn(r != 0));

    // an event made by CreateEventW works the same, and an auto-reset one too
    // SAFETY: plain call.
    let auto = unsafe { CreateEventW(core::ptr::null(), 0, 1, core::ptr::null()) };
    say!(t, "an auto-reset event created set: signalled {}", yn(is_set(auto)));
    say!(t, "(the wait consumed it) signalled now {}", yn(is_set(auto)));
    // SAFETY: valid handle.
    unsafe { SetEvent(auto) };
    // SAFETY: valid handle.
    let r = unsafe { WSAResetEvent(auto) };
    say!(t, "WSAResetEvent of a set auto-reset event: {}; signalled {}", yn(r != 0), yn(is_set(auto)));

    // bad handles
    // SAFETY: bad handles on purpose.
    let r = unsafe { WSAResetEvent(core::ptr::null_mut()) };
    say!(t, "WSAResetEvent(NULL): {} WSAGetLastError {}", yn(r != 0), wsae());
    // SAFETY: bad handles on purpose.
    let r = unsafe { WSAResetEvent(INVALID_HANDLE) };
    say!(t, "WSAResetEvent(INVALID_HANDLE_VALUE): {} WSAGetLastError {}", yn(r != 0), wsae());
    // SAFETY: bad handles on purpose.
    let r = unsafe { WSASetEvent(core::ptr::null_mut()) };
    say!(t, "WSASetEvent(NULL): {} WSAGetLastError {}", yn(r != 0), wsae());
    // SAFETY: bad handles on purpose.
    let r = unsafe { WSACloseEvent(core::ptr::null_mut()) };
    say!(t, "WSACloseEvent(NULL): {} WSAGetLastError {}", yn(r != 0), wsae());
    // SAFETY: bad handles on purpose.
    let r = unsafe { WSACloseEvent(INVALID_HANDLE) };
    say!(t, "WSACloseEvent(INVALID_HANDLE_VALUE): {}", yn(r != 0));

    // closing
    // SAFETY: valid handle.
    let r = unsafe { WSACloseEvent(ev) };
    say!(t, "WSACloseEvent: {}", yn(r != 0));
    // SAFETY: the handle is closed on purpose.
    let r = unsafe { WSAResetEvent(ev) };
    say!(t, "WSAResetEvent of the closed event: {} WSAGetLastError {}", yn(r != 0), wsae());
    // SAFETY: the handle is closed on purpose.
    let r = unsafe { WSACloseEvent(ev) };
    say!(t, "WSACloseEvent again: {} WSAGetLastError {}", yn(r != 0), wsae());
    // SAFETY: plain call; closing a CreateEventW handle with WSACloseEvent.
    let r = unsafe { WSACloseEvent(auto) };
    say!(t, "WSACloseEvent of a CreateEventW event: {}", yn(r != 0));
    // SAFETY: valid handle, never WSA-created: a duplicate keeps working after the original is closed
    let ev2 = unsafe { WSACreateEvent() };
    let mut dup: HANDLE = core::ptr::null_mut();
    // SAFETY: plain calls.
    unsafe {
        DuplicateHandle(GetCurrentProcess(), ev2, GetCurrentProcess(), &mut dup, 0, 0, 2);
        WSACloseEvent(ev2);
        WSASetEvent(dup);
    }
    say!(t, "a duplicate of a closed WSA event still works: signalled {}", yn(is_set(dup)));
    // SAFETY: valid handle.
    unsafe { WSACloseEvent(dup) };
}
