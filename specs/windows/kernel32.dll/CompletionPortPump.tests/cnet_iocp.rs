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

const PART: u32 = 1;

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

poly_wtest::main!(test);

// A completion port as Chrome's message pump uses it (GetQueuedCompletionStatus, never the Ex form; PostQueuedCompletionStatus
// from other threads to wake it; overlapped pipe handles associated with a key): what packets arrive, in which order, with which
// key, count and OVERLAPPED, for inline completions, pending ones, failures and cancellation.

static mut PORT: HANDLE = core::ptr::null_mut();
static mut WAITER_RESULT: [usize; 4] = [0; 4];

fn get(port: HANDLE, ms: u32) -> (bool, u32, u32, usize, usize) {
    let mut n = 0xAAAAu32;
    let mut key = 0xAAAAusize;
    let mut ov: *mut OVERLAPPED = core::ptr::null_mut();
    // SAFETY: live out-parameters.
    let ok = unsafe { GetQueuedCompletionStatus(port, &mut n, &mut key, &mut ov, ms) };
    let e = if ok == 0 { gle() as u32 } else { 0 };
    (ok != 0, e, n, key, ov as usize)
}

fn packet(t: &mut Test, label: &str, port: HANDLE, ms: u32, expect_ov: &OVERLAPPED) {
    let (ok, e, n, key, ov) = get(port, ms);
    let kt = if key == 0xAAAA { String3("untouched") } else { key_text(key) };
    say!(
        t,
        "{label}: returned {} (error {e}), count {n}, key {}, OVERLAPPED is the call's {}",
        yn(ok),
        kt.as_str(),
        yn(ov == expect_ov as *const OVERLAPPED as usize)
    );
}

struct String3(&'static str);
impl String3 {
    fn as_str(&self) -> &str {
        self.0
    }
}
fn key_text(k: usize) -> String3 {
    match k {
        7 => String3("7"),
        9 => String3("9"),
        99 => String3("99"),
        0 => String3("0"),
        _ => String3("other"),
    }
}

unsafe extern "system" fn waiter(_: *mut c_void) -> u32 {
    // SAFETY: the port is set before the thread starts and lives to the end of the test.
    unsafe {
        let (ok, _, n, key, _) = get(PORT, INFINITE);
        WAITER_RESULT = [ok as usize, n as usize, key, GetCurrentThreadId() as usize];
    }
    0
}

fn test(t: &mut Test) {
    // a new port, concurrency 1
    // SAFETY: plain call.
    let port = unsafe { CreateIoCompletionPort(INVALID_HANDLE, core::ptr::null_mut(), 0, 1) };
    say!(t, "new port: handle {}", yn(!port.is_null() && port != INVALID_HANDLE));

    // an empty port with a zero timeout
    let mut dummy = OVERLAPPED::new();
    packet(t, "empty port, 0 ms", port, 0, &dummy);
    say!(t, "last error after it: {}", gle());
    let (ok, e, n, key, ov) = get(port, 50);
    say!(t, "empty port, 50 ms: returned {} (error {e}), count {n:#x}, key {key:#x}, OVERLAPPED null {}", yn(ok), yn(ov == 0));

    // posted packets come out in order, with what was posted
    // SAFETY: plain calls.
    unsafe {
        PostQueuedCompletionStatus(port, 1, 99, &mut dummy);
        PostQueuedCompletionStatus(port, 2, 99, core::ptr::null_mut());
        PostQueuedCompletionStatus(port, 3, 0, core::ptr::null_mut());
    }
    packet(t, "first posted (count 1, key 99, an OVERLAPPED)", port, 0, &dummy);
    let (ok, e, n, key, ov) = get(port, 0);
    say!(t, "second posted: returned {} (error {e}), count {n}, key {}, OVERLAPPED null {}", yn(ok), key_text(key).as_str(), yn(ov == 0));
    let (ok, e, n, key, ov) = get(port, 0);
    say!(t, "third posted: returned {} (error {e}), count {n}, key {}, OVERLAPPED null {}", yn(ok), key_text(key).as_str(), yn(ov == 0));

    // pipe ends associated with keys
    let (s, c) = pipe_pair("iocp", 4096, 4096);
    // SAFETY: plain calls.
    let (ps, pc) = unsafe { (CreateIoCompletionPort(s, port, 7, 1), CreateIoCompletionPort(c, port, 9, 1)) };
    say!(t, "associating both ends returns the port: {}", yn(ps == port && pc == port));
    // SAFETY: plain call; a second association of the same handle.
    let again = unsafe { CreateIoCompletionPort(s, port, 8, 1) };
    say!(t, "associating the server end again: {} (error {})", if again.is_null() { "NULL" } else { "a handle" }, gle());

    // an inline completion still queues a packet
    let ev = manual_event();
    let mut wov = OVERLAPPED::with_event(ev);
    let mut n = 0u32;
    // SAFETY: live buffer and OVERLAPPED.
    let ok = unsafe { WriteFile(c, b"hello".as_ptr(), 5, &mut n, &mut wov as *mut OVERLAPPED as *mut c_void) };
    say!(t, "write 5 bytes: returned {} count {n}", yn(ok != 0));
    packet(t, "packet of the inline write", port, 100, &wov);
    let (ok, _, _, _, _) = get(port, 50);
    say!(t, "a second packet for it: {}", yn(ok));

    // a pending read is completed by the peer's write: two packets, which first?
    let mut buf = [0u8; 32];
    let mut rov = OVERLAPPED::with_event(manual_event());
    // SAFETY: live buffer and OVERLAPPED. First the read that is inline (5 bytes are waiting), then a pending one.
    let ok = unsafe { ReadFile(s, buf.as_mut_ptr(), 32, &mut n, &mut rov as *mut OVERLAPPED as *mut c_void) };
    say!(t, "read of the waiting 5 bytes: returned {} count {n}", yn(ok != 0));
    packet(t, "packet of the inline read", port, 100, &rov);
    let mut rov2 = OVERLAPPED::with_event(manual_event());
    // SAFETY: live buffer and OVERLAPPED.
    let ok = unsafe { ReadFile(s, buf.as_mut_ptr(), 32, &mut n, &mut rov2 as *mut OVERLAPPED as *mut c_void) };
    let e = gle();
    say!(t, "a read with no data: returned {} (error {})", yn(ok != 0), e);
    let (ok, _, _, _, _) = get(port, 50);
    say!(t, "a packet before the data came: {}", yn(ok));
    let mut wov2 = OVERLAPPED::with_event(manual_event());
    // SAFETY: live buffer and OVERLAPPED.
    unsafe { WriteFile(c, b"0123456789".as_ptr(), 10, &mut n, &mut wov2 as *mut OVERLAPPED as *mut c_void) };
    let (_, _, n1, k1, o1) = get(port, 500);
    let (_, _, n2, k2, o2) = get(port, 500);
    say!(
        t,
        "two packets after the write: first key {} count {n1} is the {}; second key {} count {n2} is the {}",
        key_text(k1).as_str(),
        if o1 == &wov2 as *const OVERLAPPED as usize { "write" } else if o1 == &rov2 as *const OVERLAPPED as usize { "read" } else { "other" },
        key_text(k2).as_str(),
        if o2 == &wov2 as *const OVERLAPPED as usize { "write" } else if o2 == &rov2 as *const OVERLAPPED as usize { "read" } else { "other" }
    );
    let (ok, _, _, _, _) = get(port, 50);
    say!(t, "a third packet: {}", yn(ok));

    // a cancelled read and a read failed by the peer closing: packets with errors
    let mut rov3 = OVERLAPPED::with_event(manual_event());
    // SAFETY: live buffer and OVERLAPPED.
    unsafe { ReadFile(s, buf.as_mut_ptr(), 32, &mut n, &mut rov3 as *mut OVERLAPPED as *mut c_void) };
    // SAFETY: valid handle and OVERLAPPED.
    unsafe { CancelIoEx(s, &mut rov3) };
    packet(t, "packet of the cancelled read", port, 500, &rov3);
    say!(t, "its Internal is STATUS_CANCELLED: {}", yn(rov3.internal == 0xC000_0120));
    let mut rov4 = OVERLAPPED::with_event(manual_event());
    // SAFETY: live buffer and OVERLAPPED.
    unsafe { ReadFile(s, buf.as_mut_ptr(), 32, &mut n, &mut rov4 as *mut OVERLAPPED as *mut c_void) };
    // SAFETY: valid handle.
    unsafe { CloseHandle(c) };
    packet(t, "packet of the read the peer's close ended", port, 500, &rov4);

    // one waiter, one packet from another thread
    // SAFETY: the static is written before the thread starts and read after it ended.
    unsafe { PORT = port };
    let mut id = 0u32;
    // SAFETY: plain call.
    let th = unsafe { CreateThread(core::ptr::null(), 0, waiter, core::ptr::null_mut(), 0, &mut id) };
    // SAFETY: plain calls.
    unsafe {
        Sleep(100);
        let w = WaitForSingleObject(th, 0);
        say!(t, "a waiter blocked in GetQueuedCompletionStatus(INFINITE): still waiting {}", yn(w == WAIT_TIMEOUT));
        PostQueuedCompletionStatus(port, 77, 99, core::ptr::null_mut());
        let w = WaitForSingleObject(th, 5000);
        say!(t, "after one post: the waiter ended {}", yn(w == WAIT_OBJECT_0));
        let r = WAITER_RESULT;
        say!(t, "the waiter got: returned {}, count {}, key {}, on another thread {}", yn(r[0] != 0), r[1], key_text(r[2]).as_str(), yn(r[3] as u32 != GetCurrentThreadId()));
    }
    // SAFETY: valid handles.
    unsafe {
        CloseHandle(s);
        CloseHandle(port);
    }
}
