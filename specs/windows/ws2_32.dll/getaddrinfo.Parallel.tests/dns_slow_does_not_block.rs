//! getaddrinfo: a slow lookup on one thread (a single-label name, about a second) does not delay lookups on other threads (localhost, a numeric address, a failing fully qualified name).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_snake_case, unused_unsafe, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the
// public SDK headers ws2def.h, ws2tcpip.h, winsock2.h, windns.h, winerror.h
// (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;
type HANDLE = *mut c_void;

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
    fn push(&mut self, c: u8) {
        if self.n < N {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            self.push(c);
        }
        Ok(())
    }
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

#[repr(C)]
struct WSADATA {
    version: u16,
    high_version: u16,
    max_sockets: u16,
    max_udp: u16,
    vendor: *mut u8,
    description: [u8; 257],
    system_status: [u8; 129],
}

/// ws2def.h ADDRINFOA
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

/// ws2def.h ADDRINFOW
#[repr(C)]
struct AddrInfoW {
    flags: i32,
    family: i32,
    socktype: i32,
    protocol: i32,
    addrlen: usize,
    canonname: *mut u16,
    addr: *mut u8,
    next: *mut AddrInfoW,
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAStartup(version: u16, data: *mut WSADATA) -> i32;
    fn WSACleanup() -> i32;
    fn WSAGetLastError() -> i32;
    fn WSASetLastError(e: i32);
    fn getaddrinfo(node: *const u8, service: *const u8, hints: *const AddrInfoA, res: *mut *mut AddrInfoA) -> i32;
    fn freeaddrinfo(res: *mut AddrInfoA);
    fn GetAddrInfoW(node: *const u16, service: *const u16, hints: *const AddrInfoW, res: *mut *mut AddrInfoW) -> i32;
    fn FreeAddrInfoW(res: *mut AddrInfoW);
    fn gethostbyname(name: *const u8) -> *mut c_void;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn QueryPerformanceCounter(v: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(v: *mut i64) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetTickCount64() -> u64;
    fn Sleep(ms: u32);
    fn GetLastError() -> u32;
    fn CreateThread(attr: *mut c_void, stack: usize, f: unsafe extern "system" fn(*mut c_void) -> u32, arg: *mut c_void, flags: u32, id: *mut u32) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn CloseHandle(h: HANDLE) -> BOOL;
}

const AI_PASSIVE: i32 = 0x1;
const AI_CANONNAME: i32 = 0x2;
const AI_NUMERICHOST: i32 = 0x4;
const AI_ALL: i32 = 0x100;
const AI_ADDRCONFIG: i32 = 0x400;
const AI_V4MAPPED: i32 = 0x800;
const AI_DNS_ONLY: i32 = 0x10;
const AF_UNSPEC: i32 = 0;
const AF_INET: i32 = 2;
const AF_INET6: i32 = 23;
const SOCK_STREAM: i32 = 1;

const WSAHOST_NOT_FOUND: i32 = 11001;
const WSATRY_AGAIN: i32 = 11002;
const WSANO_DATA: i32 = 11004;

/// Milliseconds on the performance counter.
fn now_us() -> i64 {
    let mut f = 0i64;
    let mut c = 0i64;
    // SAFETY: both are writable.
    unsafe {
        QueryPerformanceFrequency(&mut f);
        QueryPerformanceCounter(&mut c);
    }
    (c as i128 * 1_000_000 / f as i128) as i64
}

fn start_winsock() {
    let mut d: WSADATA = zeroed_wsa();
    // SAFETY: d is writable.
    unsafe { WSAStartup(0x0202, &mut d) };
}

fn zeroed_wsa() -> WSADATA {
    WSADATA { version: 0, high_version: 0, max_sockets: 0, max_udp: 0, vendor: core::ptr::null_mut(), description: [0; 257], system_status: [0; 129] }
}

/// What a lookup gave: the error code (0 for success), the number of entries, how many are IPv4 and IPv6, and the time in microseconds.
#[derive(Clone, Copy)]
struct Look {
    code: i32,
    last_error: i32,
    entries: u32,
    v4: u32,
    v6: u32,
    us: i64,
}

impl Look {
    fn ms(&self) -> i64 {
        self.us / 1000
    }
}

fn hints(flags: i32, family: i32) -> AddrInfoA {
    AddrInfoA { flags, family, socktype: SOCK_STREAM, protocol: 0, addrlen: 0, canonname: core::ptr::null_mut(), addr: core::ptr::null_mut(), next: core::ptr::null_mut() }
}

/// getaddrinfo(name, NULL, {flags, family, SOCK_STREAM}), timed, the list freed.
fn look(name: &str, flags: i32, family: i32) -> Look {
    let mut n = [0u8; 200];
    for (i, c) in name.bytes().enumerate() {
        n[i] = c;
    }
    let h = hints(flags, family);
    let mut res: *mut AddrInfoA = core::ptr::null_mut();
    // SAFETY: n is NUL terminated, h and res are valid.
    unsafe { WSASetLastError(0) };
    let t0 = now_us();
    // SAFETY: as above.
    let code = unsafe { getaddrinfo(n.as_ptr(), core::ptr::null(), &h, &mut res) };
    let us = now_us() - t0;
    // SAFETY: plain query.
    let le = unsafe { WSAGetLastError() };
    let (mut e, mut v4, mut v6) = (0, 0, 0);
    let mut p = res;
    while !p.is_null() {
        // SAFETY: the list returned by the call.
        unsafe {
            e += 1;
            match (*p).family {
                AF_INET => v4 += 1,
                AF_INET6 => v6 += 1,
                _ => {}
            }
            p = (*p).next;
        }
    }
    if !res.is_null() {
        // SAFETY: the list returned by the call.
        unsafe { freeaddrinfo(res) };
    }
    Look { code, last_error: le, entries: e, v4, v6, us }
}

/// GetAddrInfoW with the same arguments.
fn look_w(name: &str, flags: i32, family: i32) -> Look {
    let mut n = [0u16; 200];
    for (i, c) in name.bytes().enumerate() {
        n[i] = c as u16;
    }
    let h = AddrInfoW { flags, family, socktype: SOCK_STREAM, protocol: 0, addrlen: 0, canonname: core::ptr::null_mut(), addr: core::ptr::null_mut(), next: core::ptr::null_mut() };
    let mut res: *mut AddrInfoW = core::ptr::null_mut();
    // SAFETY: n is NUL terminated, h and res are valid.
    unsafe { WSASetLastError(0) };
    let t0 = now_us();
    // SAFETY: as above.
    let code = unsafe { GetAddrInfoW(n.as_ptr(), core::ptr::null(), &h, &mut res) };
    let us = now_us() - t0;
    // SAFETY: plain query.
    let le = unsafe { WSAGetLastError() };
    let (mut e, mut v4, mut v6) = (0, 0, 0);
    let mut p = res;
    while !p.is_null() {
        // SAFETY: the list returned by the call.
        unsafe {
            e += 1;
            match (*p).family {
                AF_INET => v4 += 1,
                AF_INET6 => v6 += 1,
                _ => {}
            }
            p = (*p).next;
        }
    }
    if !res.is_null() {
        // SAFETY: the list returned by the call.
        unsafe { FreeAddrInfoW(res) };
    }
    Look { code, last_error: le, entries: e, v4, v6, us }
}

/// A name that nobody owns: `<prefix>-<16 hex digits>` plus `suffix` (a different one in every run and call).
fn random_name(prefix: &str, suffix: &str) -> Buf<120> {
    static CALLS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
    let c = CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed) as u64;
    // SAFETY: plain queries.
    let (pid, tick) = unsafe { (GetCurrentProcessId() as u64, GetTickCount64()) };
    let mut x = now_us() as u64 ^ (pid << 32) ^ tick.rotate_left(17) ^ (c << 48) ^ 0x9E37_79B9_7F4A_7C15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    let mut b = Buf::<120>::new();
    let _ = write!(b, "{}-{:016x}{}", prefix, x, suffix);
    b
}

// ---- the hosts file and a helper process ------------------------------------------------

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileA(name: *const u8, access: u32, share: u32, sa: *mut c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: *mut c_void, ta: *mut c_void, inherit: BOOL, flags: u32, env: *mut c_void, dir: *const u16, si: *mut StartupInfoW, pi: *mut ProcessInfo) -> BOOL;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
}

#[repr(C)]
struct StartupInfoW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    xs: u32,
    ys: u32,
    xc: u32,
    yc: u32,
    fill: u32,
    flags: u32,
    show: u16,
    cb2: u16,
    reserved2: *mut u8,
    stdin: HANDLE,
    stdout: HANDLE,
    stderr: HANDLE,
}

#[repr(C)]
struct ProcessInfo {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

/// Run `cmd` (a command line) hidden, wait up to `ms`, return its exit code (None: did not start or timed out).
fn run_cmd(cmd: &str, ms: u32) -> Option<u32> {
    let mut w = [0u16; 600];
    for (i, c) in cmd.bytes().enumerate() {
        w[i] = c as u16;
    }
    let mut si = StartupInfoW { cb: core::mem::size_of::<StartupInfoW>() as u32, reserved: core::ptr::null_mut(), desktop: core::ptr::null_mut(), title: core::ptr::null_mut(), x: 0, y: 0, xs: 0, ys: 0, xc: 0, yc: 0, fill: 0, flags: 1, show: 0, cb2: 0, reserved2: core::ptr::null_mut(), stdin: core::ptr::null_mut(), stdout: core::ptr::null_mut(), stderr: core::ptr::null_mut() };
    let mut pi = ProcessInfo { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: w is a writable NUL-terminated command line; si and pi are valid.
    let ok = unsafe { CreateProcessW(core::ptr::null(), w.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut(), 0, 0x0800_0000, core::ptr::null_mut(), core::ptr::null(), &mut si, &mut pi) };
    if ok == 0 {
        return None;
    }
    // SAFETY: the process handle just created.
    unsafe {
        let r = WaitForSingleObject(pi.process, ms);
        let mut code = 0u32;
        GetExitCodeProcess(pi.process, &mut code);
        CloseHandle(pi.process);
        CloseHandle(pi.thread);
        if r == 0 { Some(code) } else { None }
    }
}

const HOSTS: &[u8] = b"C:\\Windows\\System32\\drivers\\etc\\hosts\0";

/// Rewrite the hosts file: drop every line that contains "polytest-" and add `extra` (lines without the line end). Returns false if the file could not be written.
fn hosts_rewrite(extra: &[&str]) -> bool {
    let mut buf = [0u8; 65536];
    let mut n = 0u32;
    // SAFETY: plain file calls on a buffer of the right size.
    unsafe {
        let h = CreateFileA(HOSTS.as_ptr(), 0x8000_0000, 7, core::ptr::null_mut(), 3, 0x80, core::ptr::null_mut());
        if h as isize == -1 {
            return false;
        }
        ReadFile(h, buf.as_mut_ptr(), 60000, &mut n, core::ptr::null_mut());
        CloseHandle(h);
    }
    let text = &buf[..n as usize];
    let mut out = [0u8; 65536];
    let mut o = 0usize;
    for line in text.split_inclusive(|&c| c == b'\n') {
        let has = line.windows(9).any(|w| w == b"polytest-");
        if !has && o + line.len() < 62000 {
            out[o..o + line.len()].copy_from_slice(line);
            o += line.len();
        }
    }
    if o > 0 && out[o - 1] != b'\n' {
        out[o] = b'\n';
        o += 1;
    }
    for l in extra {
        out[o..o + l.len()].copy_from_slice(l.as_bytes());
        o += l.len();
        out[o] = b'\r';
        out[o + 1] = b'\n';
        o += 2;
    }
    let mut wrote = 0u32;
    // SAFETY: plain file calls.
    unsafe {
        let h = CreateFileA(HOSTS.as_ptr(), 0x4000_0000, 1, core::ptr::null_mut(), 2, 0x80, core::ptr::null_mut());
        if h as isize == -1 {
            return false;
        }
        WriteFile(h, out.as_ptr(), o as u32, &mut wrote, core::ptr::null_mut());
        CloseHandle(h);
    }
    wrote as usize == o
}

// ---- bounds ---------------------------------------------------------------------------------

/// Whether the lookup took at least `lo` and less than `hi` milliseconds.
fn between(l: &Look, lo: i64, hi: i64) -> bool {
    let ms = l.us / 1000;
    ms >= lo && ms < hi
}

/// "code C, last error E, N entries" of a lookup (no times).
fn what(l: &Look) -> Buf<80> {
    let mut b = Buf::<80>::new();
    let _ = write!(b, "code {} last error {} entries {}", l.code, l.last_error, l.entries);
    b
}

poly_wtest::main!(test);

static NAME: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static DONE_US: core::sync::atomic::AtomicI64 = core::sync::atomic::AtomicI64::new(0);
static CODE: core::sync::atomic::AtomicI32 = core::sync::atomic::AtomicI32::new(-1);

unsafe extern "system" fn slow(_p: *mut c_void) -> u32 {
    let n = random_name("polytest", "");
    let l = look(n.as_str(), 0, AF_UNSPEC);
    CODE.store(l.code, core::sync::atomic::Ordering::SeqCst);
    DONE_US.store(l.us, core::sync::atomic::Ordering::SeqCst);
    let _ = NAME.load(core::sync::atomic::Ordering::Relaxed);
    0
}

fn test(t: &mut Test) {
    start_winsock();
    // warm the machinery: the first lookup of a process loads the resolver
    let _ = look("localhost", 0, AF_UNSPEC);
    let _ = look(random_name("polytest", ".invalid").as_str(), 0, AF_UNSPEC);
    let mut id = 0u32;
    // SAFETY: the thread procedure above.
    let h = unsafe { CreateThread(core::ptr::null_mut(), 0, slow, core::ptr::null_mut(), 0, &mut id) };
    // SAFETY: plain call.
    unsafe { Sleep(150) };
    let t0 = now_us();
    let a = look("localhost", 0, AF_UNSPEC);
    let b = look("127.0.0.1", 0, AF_UNSPEC);
    let c = look(random_name("polytest", ".invalid").as_str(), 0, AF_UNSPEC);
    let d = look(random_name("polytest", "").as_str(), AI_DNS_ONLY, AF_UNSPEC);
    let others = (now_us() - t0) / 1000;
    say!(t, "while the slow lookup runs: localhost code {} (entries > 0: {}); a numeric address code {}; a failing fully qualified name code {}; a single-label name with AI_DNS_ONLY code {}", a.code, a.entries > 0, b.code, c.code, d.code);
    say!(t, "the four lookups took under 500 ms together: {}", others < 500);
    // SAFETY: handle of the thread just started.
    unsafe {
        WaitForSingleObject(h, 20000);
        CloseHandle(h);
    }
    let us = DONE_US.load(core::sync::atomic::Ordering::SeqCst);
    say!(t, "the slow lookup: code {}; took 300 to 4000 ms: {}", CODE.load(core::sync::atomic::Ordering::SeqCst), us / 1000 >= 300 && us / 1000 < 4000);
}
