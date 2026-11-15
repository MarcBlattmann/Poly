//! A socket made by WSASocketW is a kernel handle: its inheritance flag with and without
//! WSA_FLAG_NO_HANDLE_INHERIT, CloseHandle and DuplicateHandle on it, what GetFileType says, overlapped
//! use on a socket made with and without WSA_FLAG_OVERLAPPED, and closing twice.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 1 of the observations, the others write the rest.)
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers named beside them (constants, layouts); the helpers are
// this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;

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

/// Where observation lines go: the test's output file, or (in a child process) a pipe to the parent.
trait Sink {
    fn put(&mut self, s: &str);
}

impl Sink for Test {
    fn put(&mut self, s: &str) {
        if say_keep() {
            self.line(s);
        }
    }
}

/// Which observations of this run are written: 0 all of them, 1/2/3 every third one starting at the first/second/third
/// (a test that is kept in two files writes the two halves of its observations; every call is made in both).
static SAY_PART: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static SAY_N: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

fn say_keep() -> bool {
    let p = SAY_PART.load(core::sync::atomic::Ordering::Relaxed);
    if p == 0 {
        return true;
    }
    let n = SAY_N.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    n % 3 + 1 == p
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<600>::new();
        let _ = write!(b, $($a)*);
        $t.put(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of a string (up to 299 units; characters outside the BMP are not used).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.chars().enumerate().take(299) {
        w[i] = c as u32 as u16;
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<600> {
    let mut b = Buf::<600>::new();
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
    let low = |c: u16| if (0x41..=0x5a).contains(&c) { c + 0x20 } else { c };
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| low(x) == low(y))
}

const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
const INFINITE: u32 = 0xFFFF_FFFF;
/// A value no call sets as last error: "unchanged" when it is still there afterwards.
const MARK: u32 = 0xDEAD;

/// winbase.h `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: BOOL,
}

fn inheritable() -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES { length: 24, security_descriptor: core::ptr::null_mut(), inherit_handle: 1 }
}

/// processthreadsapi.h `STARTUPINFOW` (104 bytes on x64).
#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    pos: [u32; 7],
    flags: u32,
    show_window: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std: [HANDLE; 3],
}

/// processthreadsapi.h `PROCESS_INFORMATION`.
#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetCurrentProcess() -> HANDLE;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn Sleep(ms: u32);
    fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, put: *mut u32, ov: *mut c_void) -> BOOL;
    fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const SECURITY_ATTRIBUTES,
        ta: *const SECURITY_ATTRIBUTES,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *const STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn GetCommandLineW() -> *const u16;
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, n: u32) -> u32;
}

fn close(h: HANDLE) {
    if !h.is_null() && h != INVALID_HANDLE_VALUE {
        // SAFETY: a handle this test opened, closed once.
        unsafe { CloseHandle(h) };
    }
}

fn last_error() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetLastError() }
}

fn mark_error() {
    // SAFETY: sets this thread's last error.
    unsafe { SetLastError(MARK) };
}

/// The last error as text: `unchanged` when it still is the mark set before the call.
fn err_text() -> Buf<24> {
    let e = last_error();
    let mut b = Buf::<24>::new();
    if e == MARK {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{e}");
    }
    b
}

/// 1 for a usable handle, else 0.
fn opened(h: HANDLE) -> u32 {
    u32::from(!h.is_null() && h != INVALID_HANDLE_VALUE)
}

// ----------------------------------------------------- child processes

/// The ASCII text after ` --child ` on this process's command line.
fn child_args() -> Option<Buf<160>> {
    // SAFETY: GetCommandLineW returns a NUL-terminated string that lives as long as the process.
    let p = unsafe { GetCommandLineW() };
    let units = wstr_at(p, 700);
    let n = units.len();
    let key = b" --child ";
    let at = (0..n.saturating_sub(key.len())).find(|&i| (0..key.len()).all(|j| units[i + j] == u16::from(key[j])))?;
    let mut out = Buf::<160>::new();
    for &u in &units[at + key.len()..n] {
        let _ = write!(out, "{}", u as u8 as char);
    }
    Some(out)
}

/// The `i`th space-separated word of `args` as a hexadecimal handle value.
fn arg_handle(args: &str, i: usize) -> HANDLE {
    let v = args.split(' ').nth(i).and_then(|w| usize::from_str_radix(w, 16).ok()).unwrap_or(0);
    v as HANDLE
}

/// A child process: this test's own image started with `--child <args>`.
struct Child {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

/// Start this test's own image with `--child <args>` (handles inherited, creation `flags`).
fn start_child(args: &str, flags: u32) -> Option<Child> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut cmd = [0u16; 800];
    let mut m = 0;
    let mut push = |u: u16| {
        if m < 799 {
            cmd[m] = u;
            m += 1;
        }
    };
    push(u16::from(b'"'));
    for &u in &exe[..n] {
        push(u);
    }
    push(u16::from(b'"'));
    for b in " --child ".bytes().chain(args.bytes()) {
        push(u16::from(b));
    }
    let si = STARTUPINFOW {
        cb: 104,
        reserved: core::ptr::null_mut(),
        desktop: core::ptr::null_mut(),
        title: core::ptr::null_mut(),
        pos: [0; 7],
        flags: 0,
        show_window: 0,
        cb_reserved2: 0,
        reserved2: core::ptr::null_mut(),
        std: [core::ptr::null_mut(); 3],
    };
    let mut pi = PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: `cmd` is mutable and NUL-terminated; the structures are live locals.
    let ok = unsafe {
        CreateProcessW(
            core::ptr::null(),
            cmd.as_mut_ptr(),
            core::ptr::null(),
            core::ptr::null(),
            1,
            flags,
            core::ptr::null(),
            core::ptr::null(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        return None;
    }
    Some(Child { process: pi.process, thread: pi.thread, pid: pi.pid, tid: pi.tid })
}

impl Child {
    /// Wait for the child (10 s at most, then it is ended with 0xDEAD); close its handles; its exit code.
    fn finish(self) -> u32 {
        let mut code = 0u32;
        // SAFETY: the handles of the process this test started, closed once.
        unsafe {
            if WaitForSingleObject(self.process, 10_000) != 0 {
                TerminateProcess(self.process, 0xDEAD);
                WaitForSingleObject(self.process, 5_000);
            }
            GetExitCodeProcess(self.process, &mut code);
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
        code
    }
    /// End the child now and close its handles.
    fn kill(self) {
        // SAFETY: the handles of the process this test started, closed once.
        unsafe {
            TerminateProcess(self.process, 0xDEAD);
            WaitForSingleObject(self.process, 5_000);
            CloseHandle(self.thread);
            CloseHandle(self.process);
        }
    }
}

/// Start this test's own image with `--child <args>` and wait for it; its exit code.
fn run_child(args: &str) -> Option<u32> {
    start_child(args, 0).map(Child::finish)
}

/// The child's report: lines written to an inherited anonymous pipe. The parent side.
struct Report {
    read: HANDLE,
    write: HANDLE,
}

impl Report {
    fn new() -> Report {
        let mut r = Report { read: core::ptr::null_mut(), write: core::ptr::null_mut() };
        let sa = inheritable();
        // SAFETY: live out pointers and attributes.
        unsafe { CreatePipe(&mut r.read, &mut r.write, &sa, 8192) };
        r
    }
    /// Close the write end and print what the child wrote, each line with `child: ` before it.
    fn print(self, t: &mut Test) {
        close(self.write);
        let mut all = [0u8; 8192];
        let mut n = 0usize;
        loop {
            let mut got = 0u32;
            // SAFETY: reads into the free part of `all`.
            let ok = unsafe {
                ReadFile(self.read, all.as_mut_ptr().add(n), (all.len() - n) as u32, &mut got, core::ptr::null_mut())
            };
            if ok == 0 || got == 0 {
                break;
            }
            n += got as usize;
            if n == all.len() {
                break;
            }
        }
        close(self.read);
        for line in core::str::from_utf8(&all[..n]).unwrap_or("?").split('\n') {
            if !line.is_empty() {
                say!(t, "child: {line}");
            }
        }
    }
}

/// The child side of a report: lines go into the inherited pipe.
struct ChildOut {
    h: HANDLE,
}

impl Sink for ChildOut {
    fn put(&mut self, s: &str) {
        let mut put = 0u32;
        // SAFETY: writes the bytes of `s` and a newline to the inherited pipe handle.
        unsafe {
            WriteFile(self.h, s.as_ptr(), s.len() as u32, &mut put, core::ptr::null_mut());
            WriteFile(self.h, b"\n".as_ptr(), 1, &mut put, core::ptr::null_mut());
        }
    }
}

/// The entry point of a test that starts its own image again: a process started with
/// `--child <args>` runs `$child(args) -> u32` (its exit code) and never opens the observation
/// file (the parent holds it); the other one runs `$test` like `poly_wtest::main!`.
macro_rules! main_with_child {
    ($child:path, $test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_args() {
                exit($child(a.as_str()))
            }
            if open_output().is_err() {
                exit(EXIT_NO_OUTPUT);
            }
            let mut t = Test::new();
            $test(&mut t);
            let code = t.finish();
            exit(code)
        }
    };
}

// ---- Winsock declarations (Microsoft Learn prototypes; winsock2.h, ws2def.h, ws2ipdef.h, mswsock.h constants) ----

type SOCKET = usize;
const INVALID_SOCKET: SOCKET = !0;
const SOCKET_ERROR: i32 = -1;

const AF_INET: i32 = 2;
const AF_INET6: i32 = 23;
const SOCK_STREAM: i32 = 1;
const SOCK_DGRAM: i32 = 2;
const IPPROTO_TCP: i32 = 6;
const IPPROTO_UDP: i32 = 17;
const IPPROTO_IPV6: i32 = 41;
const SOL_SOCKET: i32 = 0xFFFF;
const SO_REUSEADDR: i32 = 4;
const SO_KEEPALIVE: i32 = 8;
const SO_LINGER: i32 = 0x80;
const SO_BROADCAST: i32 = 0x20;
const SO_SNDBUF: i32 = 0x1001;
const SO_RCVBUF: i32 = 0x1002;
const SO_ERROR: i32 = 0x1007;
const SO_TYPE: i32 = 0x1008;
const SO_ACCEPTCONN: i32 = 2;
const SO_EXCLUSIVEADDRUSE: i32 = !4;
const SO_RANDOMIZE_PORT: i32 = 0x3005;
const SO_UPDATE_ACCEPT_CONTEXT: i32 = 0x700B;
const SO_UPDATE_CONNECT_CONTEXT: i32 = 0x7010;
const TCP_NODELAY: i32 = 1;
const IPV6_V6ONLY: i32 = 27;
const MSG_PEEK: i32 = 2;
const SD_RECEIVE: i32 = 0;
const SD_SEND: i32 = 1;
const SD_BOTH: i32 = 2;
const FIONREAD: u32 = 0x4004667F;
const FIONBIO: u32 = 0x8004667E;
const SIO_KEEPALIVE_VALS: u32 = 0x98000004;
const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xC8000006;
const SIO_UDP_CONNRESET: u32 = 0x9800000C;
const SIO_BASE_HANDLE: u32 = 0x48000022;
const FD_READ: i32 = 1;
const FD_WRITE: i32 = 2;
const FD_OOB: i32 = 4;
const FD_ACCEPT: i32 = 8;
const FD_CONNECT: i32 = 0x10;
const FD_CLOSE: i32 = 0x20;
const WSA_FLAG_OVERLAPPED: u32 = 1;
const WSA_FLAG_NO_HANDLE_INHERIT: u32 = 0x80;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_OBJECT_0: u32 = 0;
const WSA_IO_PENDING: i32 = 997;
const WSAEWOULDBLOCK: i32 = 10035;

#[repr(C)]
struct WSADATA {
    version: u16,
    high_version: u16,
    max_sockets: u16,
    max_udp_dg: u16,
    vendor_info: *mut u8,
    description: [u8; 257],
    system_status: [u8; 129],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrIn {
    family: u16,
    port: u16, // network byte order
    addr: [u8; 4],
    zero: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SockaddrIn6 {
    family: u16,
    port: u16, // network byte order
    flowinfo: u32,
    addr: [u8; 16],
    scope_id: u32,
}

#[repr(C)]
struct WSABUF {
    len: u32,
    buf: *mut u8,
}

#[repr(C)]
struct WSANETWORKEVENTS {
    events: i32,
    error: [i32; 10],
}

#[repr(C)]
struct OVERLAPPED {
    internal: usize,
    internal_high: usize,
    offset: u32,
    offset_high: u32,
    event: HANDLE,
}

fn overlapped() -> OVERLAPPED {
    OVERLAPPED { internal: 0, internal_high: 0, offset: 0, offset_high: 0, event: core::ptr::null_mut() }
}

#[repr(C)]
struct Timeval {
    sec: i32,
    usec: i32,
}

#[repr(C)]
struct FdSet {
    count: u32,
    array: [SOCKET; 64],
}

#[repr(C)]
struct Linger {
    onoff: u16,
    linger: u16,
}

#[repr(C)]
struct AddrInfoA {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: usize,
    canonname: *mut u8,
    addr: *mut u8,
    next: *mut AddrInfoA,
}

#[repr(C)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut WSADATA) -> i32;
    fn WSACleanup() -> i32;
    fn WSAGetLastError() -> i32;
    fn WSASetLastError(e: i32);
    fn WSASocketW(af: i32, ty: i32, proto: i32, info: *const c_void, group: u32, flags: u32) -> SOCKET;
    fn socket(af: i32, ty: i32, proto: i32) -> SOCKET;
    fn closesocket(s: SOCKET) -> i32;
    fn bind(s: SOCKET, name: *const c_void, len: i32) -> i32;
    fn listen(s: SOCKET, backlog: i32) -> i32;
    fn accept(s: SOCKET, addr: *mut c_void, len: *mut i32) -> SOCKET;
    fn connect(s: SOCKET, name: *const c_void, len: i32) -> i32;
    fn send(s: SOCKET, buf: *const u8, len: i32, flags: i32) -> i32;
    fn recv(s: SOCKET, buf: *mut u8, len: i32, flags: i32) -> i32;
    fn sendto(s: SOCKET, buf: *const u8, len: i32, flags: i32, to: *const c_void, tolen: i32) -> i32;
    fn recvfrom(s: SOCKET, buf: *mut u8, len: i32, flags: i32, from: *mut c_void, fromlen: *mut i32) -> i32;
    fn shutdown(s: SOCKET, how: i32) -> i32;
    fn getsockname(s: SOCKET, name: *mut c_void, len: *mut i32) -> i32;
    fn getpeername(s: SOCKET, name: *mut c_void, len: *mut i32) -> i32;
    fn setsockopt(s: SOCKET, level: i32, name: i32, val: *const u8, len: i32) -> i32;
    fn getsockopt(s: SOCKET, level: i32, name: i32, val: *mut u8, len: *mut i32) -> i32;
    fn ioctlsocket(s: SOCKET, cmd: u32, arg: *mut u32) -> i32;
    fn WSAIoctl(
        s: SOCKET,
        code: u32,
        inb: *const c_void,
        inlen: u32,
        outb: *mut c_void,
        outlen: u32,
        ret: *mut u32,
        ov: *mut OVERLAPPED,
        routine: *const c_void,
    ) -> i32;
    fn WSAEventSelect(s: SOCKET, ev: HANDLE, mask: i32) -> i32;
    fn WSAEnumNetworkEvents(s: SOCKET, ev: HANDLE, out: *mut WSANETWORKEVENTS) -> i32;
    fn WSACreateEvent() -> HANDLE;
    fn WSACloseEvent(h: HANDLE) -> BOOL;
    fn WSAResetEvent(h: HANDLE) -> BOOL;
    fn WSASetEvent(h: HANDLE) -> BOOL;
    fn WSAWaitForMultipleEvents(n: u32, h: *const HANDLE, all: BOOL, ms: u32, alertable: BOOL) -> u32;
    fn WSASend(
        s: SOCKET,
        b: *const WSABUF,
        n: u32,
        sent: *mut u32,
        flags: u32,
        ov: *mut OVERLAPPED,
        r: *const c_void,
    ) -> i32;
    fn WSARecv(
        s: SOCKET,
        b: *const WSABUF,
        n: u32,
        got: *mut u32,
        flags: *mut u32,
        ov: *mut OVERLAPPED,
        r: *const c_void,
    ) -> i32;
    fn WSAGetOverlappedResult(
        s: SOCKET,
        ov: *mut OVERLAPPED,
        transferred: *mut u32,
        wait: BOOL,
        flags: *mut u32,
    ) -> BOOL;
    fn select(n: i32, r: *mut FdSet, w: *mut FdSet, e: *mut FdSet, t: *const Timeval) -> i32;
    fn getaddrinfo(node: *const u8, service: *const u8, hints: *const AddrInfoA, res: *mut *mut AddrInfoA) -> i32;
    fn freeaddrinfo(res: *mut AddrInfoA);
    fn WSADuplicateSocketW(s: SOCKET, pid: u32, info: *mut u8) -> i32;
    fn htons(v: u16) -> u16;
    fn ntohs(v: u16) -> u16;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(
        sa: *const c_void,
        stack: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        flags: u32,
        id: *mut u32,
    ) -> HANDLE;
    fn CreateIoCompletionPort(file: HANDLE, port: HANDLE, key: usize, threads: u32) -> HANDLE;
    fn GetQueuedCompletionStatus(
        port: HANDLE,
        bytes: *mut u32,
        key: *mut usize,
        ov: *mut *mut OVERLAPPED,
        ms: u32,
    ) -> BOOL;
    fn PostQueuedCompletionStatus(port: HANDLE, bytes: u32, key: usize, ov: *mut OVERLAPPED) -> BOOL;
    fn SetFileCompletionNotificationModes(file: HANDLE, flags: u8) -> BOOL;
    fn GetHandleInformation(h: HANDLE, flags: *mut u32) -> BOOL;
    fn CancelIoEx(h: HANDLE, ov: *mut OVERLAPPED) -> BOOL;
    fn ResetEvent(h: HANDLE) -> BOOL;
    fn GetOverlappedResult(h: HANDLE, ov: *mut OVERLAPPED, n: *mut u32, wait: BOOL) -> BOOL;
}

// ---- helpers ----

fn netinit() {
    // SAFETY: a live WSADATA.
    unsafe {
        let mut d: WSADATA = core::mem::zeroed();
        WSAStartup(0x0202, &mut d);
    }
}

fn lasterr() -> i32 {
    // SAFETY: no arguments.
    unsafe { WSAGetLastError() }
}

/// An IPv4 address and port (host order) as a sockaddr_in.
fn sin(a: [u8; 4], port: u16) -> SockaddrIn {
    // SAFETY: htons has no preconditions.
    SockaddrIn { family: 2, port: unsafe { htons(port) }, addr: a, zero: [0; 8] }
}

fn sin6(a: [u8; 16], port: u16) -> SockaddrIn6 {
    // SAFETY: htons has no preconditions.
    SockaddrIn6 { family: 23, port: unsafe { htons(port) }, flowinfo: 0, addr: a, scope_id: 0 }
}

const LOOP6: [u8; 16] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
const ANY6: [u8; 16] = [0; 16];

fn mapped(a: [u8; 4]) -> [u8; 16] {
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, a[0], a[1], a[2], a[3]]
}

fn p<T>(x: &T) -> *const c_void {
    x as *const T as *const c_void
}

fn pm<T>(x: &mut T) -> *mut c_void {
    x as *mut T as *mut c_void
}

/// A TCP socket (overlapped flag like Chrome's WSASocketW call).
fn tcp4() -> SOCKET {
    // SAFETY: plain call.
    unsafe { WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, core::ptr::null(), 0, WSA_FLAG_OVERLAPPED) }
}

fn udp4() -> SOCKET {
    // SAFETY: plain call.
    unsafe { WSASocketW(AF_INET, SOCK_DGRAM, IPPROTO_UDP, core::ptr::null(), 0, WSA_FLAG_OVERLAPPED) }
}

/// A listening IPv4 loopback socket on a port chosen by the system; `(socket, port in host order)`.
fn listener4() -> (SOCKET, u16) {
    // SAFETY: live local structures.
    unsafe {
        let s = tcp4();
        let a = sin([127, 0, 0, 1], 0);
        bind(s, p(&a), 16);
        listen(s, 8);
        (s, sockport(s))
    }
}

/// The local port (host order) of a bound socket; 0 on failure.
fn sockport(s: SOCKET) -> u16 {
    // SAFETY: live local buffer.
    unsafe {
        let mut a: SockaddrIn = core::mem::zeroed();
        let mut n = 16i32;
        if getsockname(s, pm(&mut a), &mut n) != 0 {
            return 0;
        }
        ntohs(a.port)
    }
}

/// `connect` to 127.0.0.1:port; returns the call's result and the WSA error.
fn connect4(s: SOCKET, port: u16) -> (i32, i32) {
    let a = sin([127, 0, 0, 1], port);
    // SAFETY: live local structure.
    unsafe {
        WSASetLastError(0);
        let r = connect(s, p(&a), 16);
        (r, WSAGetLastError())
    }
}

/// A loopback connection: `(client, accepted server side, listener)`; both blocking.
fn pair4() -> (SOCKET, SOCKET, SOCKET) {
    let (l, port) = listener4();
    let c = tcp4();
    connect4(c, port);
    // SAFETY: plain call.
    let a = unsafe { accept(l, core::ptr::null_mut(), core::ptr::null_mut()) };
    (c, a, l)
}

fn setnb(s: SOCKET, on: u32) -> i32 {
    let mut v = on;
    // SAFETY: live local value.
    unsafe { ioctlsocket(s, FIONBIO, &mut v) }
}

fn avail(s: SOCKET) -> (i32, u32) {
    let mut v = 0u32;
    // SAFETY: live local value.
    let r = unsafe { ioctlsocket(s, FIONREAD, &mut v) };
    (r, v)
}

fn sendbytes(s: SOCKET, b: &[u8]) -> i32 {
    // SAFETY: the slice is live.
    unsafe { send(s, b.as_ptr(), b.len() as i32, 0) }
}

/// Non-blocking style receive: `(result, WSA error)`; the error is 0 when the call succeeded.
fn recvbytes(s: SOCKET, buf: &mut [u8], flags: i32) -> (i32, i32) {
    // SAFETY: the slice is live.
    unsafe {
        WSASetLastError(0);
        let r = recv(s, buf.as_mut_ptr(), buf.len() as i32, flags);
        (r, if r == SOCKET_ERROR { WSAGetLastError() } else { 0 })
    }
}

fn setopt_i32(s: SOCKET, level: i32, name: i32, v: i32) -> (i32, i32) {
    // SAFETY: live local value.
    unsafe {
        WSASetLastError(0);
        let r = setsockopt(s, level, name, &v as *const i32 as *const u8, 4);
        (r, if r != 0 { WSAGetLastError() } else { 0 })
    }
}

/// `getsockopt` of a 4-byte option: `(result, error, value, returned length)`.
fn getopt_i32(s: SOCKET, level: i32, name: i32) -> (i32, i32, i32, i32) {
    let mut v = 0x5A5A5A5Ai32;
    let mut n = 4i32;
    // SAFETY: live local values.
    unsafe {
        WSASetLastError(0);
        let r = getsockopt(s, level, name, &mut v as *mut i32 as *mut u8, &mut n);
        (r, if r != 0 { WSAGetLastError() } else { 0 }, v, n)
    }
}

/// Wait for an event handle at most `ms`; 1 when it was signalled.
fn waitev(h: HANDLE, ms: u32) -> u32 {
    // SAFETY: a live event handle.
    unsafe { u32::from(WaitForSingleObject(h, ms) == 0) }
}

fn netevents(s: SOCKET, ev: HANDLE) -> (i32, i32, WSANETWORKEVENTS) {
    // SAFETY: live local structure.
    unsafe {
        let mut ne = WSANETWORKEVENTS { events: 0x7777, error: [0x7777; 10] };
        WSASetLastError(0);
        let r = WSAEnumNetworkEvents(s, ev, &mut ne);
        (r, if r != 0 { WSAGetLastError() } else { 0 }, ne)
    }
}

/// The bits of an event mask as text: `READ|WRITE|...`.
fn ev_text(m: i32) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let names = [
        (1, "READ"),
        (2, "WRITE"),
        (4, "OOB"),
        (8, "ACCEPT"),
        (0x10, "CONNECT"),
        (0x20, "CLOSE"),
        (0x40, "QOS"),
        (0x80, "GROUP_QOS"),
        (0x100, "ROUTING"),
        (0x200, "ADDRLIST"),
    ];
    let mut first = true;
    for (bit, n) in names {
        if m & bit != 0 {
            let _ = write!(b, "{}{n}", if first { "" } else { "|" });
            first = false;
        }
    }
    if first {
        let _ = write!(b, "none");
    }
    b
}

/// The server thread: a listener, a mode, and a few bytes to say.
struct Srv {
    listener: SOCKET,
    mode: u32,
    data: [u8; 64],
    len: usize,
}

/// Mode 1: accept one connection, send `data`, read until the peer closes, close.
/// Mode 2: accept one connection, send `data`, close at once (a graceful close).
/// Mode 3: accept one connection, close it with SO_LINGER 0 at once (a reset).
/// Mode 4: accept one connection, read until the peer closes (an echo of nothing), close.
unsafe extern "system" fn srv_main(param: *mut c_void) -> u32 {
    // SAFETY: the test passes a live Srv that outlives the thread (it joins it).
    unsafe {
        let srv = &*(param as *const Srv);
        let c = accept(srv.listener, core::ptr::null_mut(), core::ptr::null_mut());
        if c == INVALID_SOCKET {
            return 1;
        }
        match srv.mode {
            1 => {
                send(c, srv.data.as_ptr(), srv.len as i32, 0);
                let mut b = [0u8; 64];
                while recv(c, b.as_mut_ptr(), 64, 0) > 0 {}
            }
            2 => {
                send(c, srv.data.as_ptr(), srv.len as i32, 0);
            }
            3 => {
                let l = Linger { onoff: 1, linger: 0 };
                setsockopt(c, SOL_SOCKET, SO_LINGER, &l as *const Linger as *const u8, 4);
            }
            _ => {
                let mut b = [0u8; 64];
                while recv(c, b.as_mut_ptr(), 64, 0) > 0 {}
            }
        }
        closesocket(c);
        0
    }
}

fn start_srv(srv: &Srv) -> HANDLE {
    // SAFETY: `srv` stays alive until the caller joins the thread.
    unsafe { CreateThread(core::ptr::null(), 0, srv_main, srv as *const Srv as *mut c_void, 0, core::ptr::null_mut()) }
}

fn join(h: HANDLE) -> u32 {
    // SAFETY: a thread handle this test created.
    unsafe {
        let r = WaitForSingleObject(h, 10_000);
        CloseHandle(h);
        r
    }
}

/// The text of a WSA error code that Chrome meets, else its number.
fn wsa_text(e: i32) -> Buf<32> {
    let mut b = Buf::<32>::new();
    let n = match e {
        0 => "0",
        10004 => "WSAEINTR",
        10009 => "WSAEBADF",
        10013 => "WSAEACCES",
        10014 => "WSAEFAULT",
        10022 => "WSAEINVAL",
        10024 => "WSAEMFILE",
        10035 => "WSAEWOULDBLOCK",
        10036 => "WSAEINPROGRESS",
        10037 => "WSAEALREADY",
        10038 => "WSAENOTSOCK",
        10039 => "WSAEDESTADDRREQ",
        10040 => "WSAEMSGSIZE",
        10041 => "WSAEPROTOTYPE",
        10042 => "WSAENOPROTOOPT",
        10043 => "WSAEPROTONOSUPPORT",
        10044 => "WSAESOCKTNOSUPPORT",
        10045 => "WSAEOPNOTSUPP",
        10047 => "WSAEAFNOSUPPORT",
        10048 => "WSAEADDRINUSE",
        10049 => "WSAEADDRNOTAVAIL",
        10050 => "WSAENETDOWN",
        10051 => "WSAENETUNREACH",
        10053 => "WSAECONNABORTED",
        10054 => "WSAECONNRESET",
        10055 => "WSAENOBUFS",
        10056 => "WSAEISCONN",
        10057 => "WSAENOTCONN",
        10058 => "WSAESHUTDOWN",
        10060 => "WSAETIMEDOUT",
        10061 => "WSAECONNREFUSED",
        10065 => "WSAEHOSTUNREACH",
        10091 => "WSASYSNOTREADY",
        10092 => "WSAVERNOTSUPPORTED",
        10093 => "WSANOTINITIALISED",
        10109 => "WSATYPE_NOT_FOUND",
        11001 => "WSAHOST_NOT_FOUND",
        11002 => "WSATRY_AGAIN",
        11003 => "WSANO_RECOVERY",
        11004 => "WSANO_DATA",
        995 => "ERROR_OPERATION_ABORTED",
        997 => "WSA_IO_PENDING",
        996 => "WSA_IO_INCOMPLETE",
        6 => "ERROR_INVALID_HANDLE",
        87 => "ERROR_INVALID_PARAMETER",
        _ => "",
    };
    if n.is_empty() {
        let _ = write!(b, "{e}");
    } else {
        let _ = write!(b, "{n}");
    }
    b
}

/// The result of a socket call as text: `-1/WSAEWOULDBLOCK`, or the number.
fn rr(r: i32, e: i32) -> Buf<48> {
    let mut b = Buf::<48>::new();
    if r == SOCKET_ERROR {
        let _ = write!(b, "-1/{}", wsa_text(e).as_str());
    } else {
        let _ = write!(b, "{r}");
    }
    b
}

poly_wtest::main!(test);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DuplicateHandle(
        src_proc: HANDLE,
        src: HANDLE,
        dst_proc: HANDLE,
        dst: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    fn GetFileType(h: HANDLE) -> u32;
}

fn inherit_flag(s: SOCKET) -> u32 {
    let mut f = 0xFFu32;
    // SAFETY: live local value.
    unsafe { GetHandleInformation(s as HANDLE, &mut f) };
    f
}

fn test(t: &mut Test) {
    SAY_PART.store(1, core::sync::atomic::Ordering::Relaxed);
    netinit();
    // SAFETY: plain calls with live arguments.
    unsafe {
        // handle flags
        for (label, flags) in [
            ("flags 0", 0u32),
            ("WSA_FLAG_OVERLAPPED", WSA_FLAG_OVERLAPPED),
            ("WSA_FLAG_NO_HANDLE_INHERIT", WSA_FLAG_NO_HANDLE_INHERIT),
            ("OVERLAPPED|NO_HANDLE_INHERIT", WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT),
        ] {
            let s = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, core::ptr::null(), 0, flags);
            say!(t, "{label}: handle information flags {} multiple of 4 {}", inherit_flag(s), u32::from(s % 4 == 0));
            closesocket(s);
        }
        let s = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
        say!(t, "socket(): handle information flags {}", inherit_flag(s));
        say!(t, "GetFileType {}", GetFileType(s as HANDLE));
        // CloseHandle on a socket
        SetLastError(0);
        let r = CloseHandle(s as HANDLE);
        say!(t, "CloseHandle(socket) {} error {}", r, GetLastError());
        WSASetLastError(0);
        let r = closesocket(s);
        say!(t, "closesocket after CloseHandle {}", rr(r, WSAGetLastError()).as_str());

        // DuplicateHandle of a socket: the copy is a socket and outlives the original
        let (l, port) = listener4();
        let c = tcp4();
        connect4(c, port);
        let srv = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
        let mut d: HANDLE = core::ptr::null_mut();
        let me = GetCurrentProcess();
        let r = DuplicateHandle(me, c as HANDLE, me, &mut d, 0, 0, 2);
        say!(t, "DuplicateHandle(socket, SAME_ACCESS) {}", r);
        let d = d as SOCKET;
        say!(t, "copy's handle information flags {}", inherit_flag(d));
        say!(t, "the copy sends: {}", sendbytes(d, b"abc"));
        closesocket(c);
        let mut b = [0u8; 8];
        let (r, e) = recvbytes(srv, &mut b, 0);
        say!(t, "the peer receives {}", rr(r, e).as_str());
        say!(t, "the copy sends after the original was closed: {}", sendbytes(d, b"de"));
        let mut buf = [0u8; 16];
        let mut n = 16i32;
        WSASetLastError(0);
        let r = getsockname(d, buf.as_mut_ptr() as *mut c_void, &mut n);
        say!(t, "getsockname on the copy {}", rr(r, WSAGetLastError()).as_str());
        // the peer sees no end of stream while one copy is open
        setnb(srv, 1);
        let (r, e) = recvbytes(srv, &mut b, 0);
        say!(t, "peer recv after the original closed (copy open) {}", rr(r, e).as_str());
        let (r, e) = recvbytes(srv, &mut b, 0);
        say!(t, "peer recv again {}", rr(r, e).as_str());
        closesocket(d);
        Sleep(100);
        let (r, e) = recvbytes(srv, &mut b, 0);
        say!(t, "peer recv after the last copy closed {}", rr(r, e).as_str());
        closesocket(srv);

        // closing twice
        let s = tcp4();
        say!(t, "first closesocket {}", closesocket(s));
        WSASetLastError(0);
        let r = closesocket(s);
        say!(t, "second closesocket {}", rr(r, WSAGetLastError()).as_str());
        WSASetLastError(0);
        let r = closesocket(INVALID_SOCKET);
        say!(t, "closesocket(INVALID_SOCKET) {}", rr(r, WSAGetLastError()).as_str());
        WSASetLastError(0);
        let r = closesocket(0);
        say!(t, "closesocket(0) {}", rr(r, WSAGetLastError()).as_str());

        // overlapped use: a receive with an OVERLAPPED and no data, on sockets made in the three ways
        for (label, mk) in [("WSASocketW(OVERLAPPED)", 1u32), ("socket()", 2)] {
            let (l, port) = listener4();
            let c = if mk == 1 { tcp4() } else { socket(AF_INET, SOCK_STREAM, IPPROTO_TCP) };
            connect4(c, port);
            let srv = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
            let mut buf = [0u8; 8];
            let wb = WSABUF { len: 8, buf: buf.as_mut_ptr() };
            let mut ov = overlapped();
            let ev = WSACreateEvent();
            ov.event = ev;
            let mut got = 0u32;
            let mut fl = 0u32;
            WSASetLastError(0);
            let r = WSARecv(c, &wb, 1, &mut got, &mut fl, &mut ov, core::ptr::null());
            say!(t, "{label}: WSARecv with an OVERLAPPED and no data {}", rr(r, WSAGetLastError()).as_str());
            sendbytes(srv, b"xyz");
            say!(t, "{label}: completion event signalled {}", waitev(ev, 5000));
            let mut n = 0u32;
            let mut f = 0u32;
            let ok = WSAGetOverlappedResult(c, &mut ov, &mut n, 0, &mut f);
            say!(t, "{label}: WSAGetOverlappedResult {} transferred {} flags {}", ok, n, f);
            WSACloseEvent(ev);
            closesocket(srv);
            closesocket(c);
            closesocket(l);
        }
        // a socket made without WSA_FLAG_OVERLAPPED: an overlapped call is carried out synchronously
        let (l, port) = listener4();
        let c = WSASocketW(AF_INET, SOCK_STREAM, IPPROTO_TCP, core::ptr::null(), 0, 0);
        connect4(c, port);
        let srv = accept(l, core::ptr::null_mut(), core::ptr::null_mut());
        sendbytes(srv, b"xyz");
        Sleep(100);
        let mut buf = [0u8; 8];
        let wb = WSABUF { len: 8, buf: buf.as_mut_ptr() };
        let mut ov = overlapped();
        let ev = WSACreateEvent();
        ov.event = ev;
        let mut got = 0u32;
        let mut fl = 0u32;
        WSASetLastError(0);
        let r = WSARecv(c, &wb, 1, &mut got, &mut fl, &mut ov, core::ptr::null());
        say!(
            t,
            "WSASocketW(flags 0): WSARecv with an OVERLAPPED and data waiting {} received {got}",
            rr(r, WSAGetLastError()).as_str()
        );
        say!(t, "WSASocketW(flags 0): the OVERLAPPED's event signalled {}", waitev(ev, 100));
        WSACloseEvent(ev);
        closesocket(srv);
        closesocket(c);
        closesocket(l);
    }
}
