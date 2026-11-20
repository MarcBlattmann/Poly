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

// RegisterWaitForSingleObject the way Chrome's ObjectWatcher calls it: an infinite wait on an event, flags 12
// (WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE), the callback running on the pool's wait thread, hundreds of registrations
// alive at once, each cancelled by UnregisterWaitEx. Observed: where callbacks run, how they serialise, how many threads
// serve many waits, and what the unregistering call returns while a callback runs.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegisterWaitForSingleObject(out: *mut HANDLE, obj: HANDLE, cb: unsafe extern "system" fn(*mut c_void, u8), ctx: *mut c_void, ms: u32, flags: u32) -> BOOL;
    fn UnregisterWaitEx(h: HANDLE, completion: HANDLE) -> BOOL;
    fn UnregisterWait(h: HANDLE) -> BOOL;
}

static mut GATE3: HANDLE = core::ptr::null_mut();

unsafe extern "system" fn releaser(_: *mut c_void) -> u32 {
    // SAFETY: GATE3 is set before this thread starts.
    unsafe {
        Sleep(150);
        SetEvent(GATE3);
    }
    0
}

const WT_EXECUTEONLYONCE: u32 = 8;
const WT_EXECUTEINWAITTHREAD: u32 = 4;
const FLAGS: u32 = WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE;

#[repr(C)]
struct Slot {
    ran: i32,
    thread: u32,
    timed_out: u8,
    gate: HANDLE,
    entered: HANDLE,
    finished: i32,
}

const SLOT0: Slot = Slot { ran: 0, thread: 0, timed_out: 9, gate: core::ptr::null_mut(), entered: core::ptr::null_mut(), finished: 0 };
static mut SLOTS: [Slot; 256] = [SLOT0; 256];

unsafe extern "system" fn cb(ctx: *mut c_void, timed_out: u8) {
    // SAFETY: ctx is a pointer to a slot of SLOTS, alive for the whole test.
    unsafe {
        let s = &mut *(ctx as *mut Slot);
        s.thread = GetCurrentThreadId();
        s.timed_out = timed_out;
        if !s.entered.is_null() {
            SetEvent(s.entered);
        }
        if !s.gate.is_null() {
            WaitForSingleObject(s.gate, 10_000);
        }
        core::ptr::write_volatile(&mut s.ran, s.ran + 1);
        s.finished = 1;
    }
}

fn slot(i: usize) -> *mut Slot {
    // SAFETY: index in range; the array lives for the whole test.
    unsafe { (&raw mut SLOTS as *mut Slot).add(i) }
}

fn ran(i: usize) -> i32 {
    // SAFETY: valid slot.
    unsafe { core::ptr::read_volatile(&(*slot(i)).ran) }
}

fn wait_until(mut f: impl FnMut() -> bool, ms: u32) -> bool {
    let mut waited = 0;
    while waited < ms {
        if f() {
            return true;
        }
        // SAFETY: plain call.
        unsafe { Sleep(5) };
        waited += 5;
    }
    f()
}

fn register(ev: HANDLE, i: usize) -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid event, callback and slot.
    let ok = unsafe { RegisterWaitForSingleObject(&mut h, ev, cb, slot(i) as *mut c_void, INFINITE, FLAGS) };
    if ok == 0 { core::ptr::null_mut() } else { h }
}

fn test(t: &mut Test) {
    // SAFETY: plain call.
    let me = unsafe { GetCurrentThreadId() };

    // 1: one wait: not before the event, once after it (the manual-reset event stays signalled)
    let ev = manual_event();
    let h = register(ev, 0);
    say!(t, "registered: handle returned {}", yn(!h.is_null()));
    // SAFETY: plain call.
    unsafe { Sleep(150) };
    say!(t, "no callback before the event: {}", yn(ran(0) == 0));
    // SAFETY: valid event.
    unsafe { SetEvent(ev) };
    say!(t, "a callback after SetEvent: {}", yn(wait_until(|| ran(0) >= 1, 3000)));
    // SAFETY: plain call.
    unsafe { Sleep(200) };
    // SAFETY: valid slot.
    let (th, to) = unsafe { ((*slot(0)).thread, (*slot(0)).timed_out) };
    say!(t, "it ran {} time(s) with the event still signalled; on another thread {}; timedOut {}", ran(0), yn(th != me), to);
    // SAFETY: valid handle.
    let r = unsafe { UnregisterWaitEx(h, core::ptr::null_mut()) };
    say!(t, "UnregisterWaitEx(h, NULL) after the callback ran: {} (error {})", yn(r != 0), if r != 0 { 0 } else { gle() });

    // 2: unregistered before the event: it never runs
    let ev2 = manual_event();
    let h2 = register(ev2, 1);
    // SAFETY: valid handle.
    let r = unsafe { UnregisterWaitEx(h2, INVALID_HANDLE) };
    // SAFETY: valid event.
    unsafe { SetEvent(ev2) };
    // SAFETY: plain call.
    unsafe { Sleep(200) };
    say!(t, "UnregisterWaitEx(h, INVALID_HANDLE_VALUE) before the event: {}; the callback ran after SetEvent: {}", yn(r != 0), yn(ran(1) != 0));

    // 3: a callback that blocks serialises the others on the same wait thread; unregistering while it runs
    let gate = manual_event();
    let entered = manual_event();
    let evg = manual_event();
    let evb = manual_event();
    // SAFETY: valid slots.
    unsafe {
        (*slot(2)).gate = gate;
        (*slot(2)).entered = entered;
    }
    let hg = register(evg, 2);
    let hb = register(evb, 3);
    // SAFETY: valid events.
    unsafe {
        SetEvent(evg);
        WaitForSingleObject(entered, 3000);
        SetEvent(evb);
        Sleep(200);
    }
    say!(t, "while the first callback blocks, the second one ran: {}", yn(ran(3) != 0));
    // SAFETY: valid handle.
    let r = unsafe { UnregisterWaitEx(hg, core::ptr::null_mut()) };
    let e = gle();
    say!(t, "UnregisterWaitEx(h, NULL) while its callback runs: {} (error {})", yn(r != 0), if r != 0 { 0 } else { e });
    // SAFETY: valid event.
    unsafe { SetEvent(gate) };
    say!(t, "after the callback is released: the first callback finished {}, the second ran {}", yn(wait_until(|| ran(2) != 0, 3000)), yn(wait_until(|| ran(3) != 0, 3000)));
    // the completion-event form, with another blocked callback
    let gate2 = manual_event();
    let entered2 = manual_event();
    let evg2 = manual_event();
    // SAFETY: valid slots.
    unsafe {
        (*slot(4)).gate = gate2;
        (*slot(4)).entered = entered2;
    }
    let hg2 = register(evg2, 4);
    let done = manual_event();
    // SAFETY: valid events.
    unsafe {
        SetEvent(evg2);
        WaitForSingleObject(entered2, 3000);
    }
    // SAFETY: valid handle and event.
    let r = unsafe { UnregisterWaitEx(hg2, done) };
    let e = gle();
    say!(t, "UnregisterWaitEx(h, event) while its callback runs: {} (error {}), event signalled at once {}", yn(r != 0), if r != 0 { 0 } else { e }, yn(is_set(done)));
    // SAFETY: valid event.
    unsafe { SetEvent(gate2) };
    // SAFETY: valid event.
    let w = unsafe { WaitForSingleObject(done, 3000) };
    say!(t, "the completion event is signalled after the callback returned: {}", yn(w == WAIT_OBJECT_0));
    say!(t, "the callback had finished by then: {}", yn(ran(4) == 1));
    // the blocking form: a helper releases the callback after 150 ms
    let gate3 = manual_event();
    let entered3 = manual_event();
    let evg3 = manual_event();
    // SAFETY: valid slots.
    unsafe {
        (*slot(5)).gate = gate3;
        (*slot(5)).entered = entered3;
        GATE3 = gate3;
    }
    let hg3 = register(evg3, 5);
    let mut tid = 0u32;
    // SAFETY: valid events and function.
    unsafe {
        SetEvent(evg3);
        WaitForSingleObject(entered3, 3000);
        CreateThread(core::ptr::null(), 0, releaser, core::ptr::null_mut(), 0, &mut tid);
    }
    // SAFETY: valid handle.
    let r = unsafe { UnregisterWaitEx(hg3, INVALID_HANDLE) };
    say!(t, "UnregisterWaitEx(h, INVALID_HANDLE_VALUE) while its callback runs: {}; the callback had finished when it returned {}", yn(r != 0), yn(ran(5) == 1));
    // SAFETY: valid handle.
    unsafe { UnregisterWaitEx(hb, INVALID_HANDLE) };

    // 4: many registrations at once, all signalled
    const N: usize = 200;
    let mut evs = [core::ptr::null_mut::<c_void>(); N];
    let mut hs = [core::ptr::null_mut::<c_void>(); N];
    let mut all = true;
    for i in 0..N {
        evs[i] = manual_event();
        hs[i] = register(evs[i], 10 + i);
        all &= !hs[i].is_null();
    }
    say!(t, "{N} registrations alive: all returned a handle {}", yn(all));
    // SAFETY: valid events.
    unsafe {
        for e in evs.iter() {
            SetEvent(*e);
        }
    }
    let ok = wait_until(|| (0..N).all(|i| ran(10 + i) == 1), 10_000);
    say!(t, "all {N} callbacks ran, each once: {}", yn(ok));
    let mut threads = [0u32; N];
    for i in 0..N {
        // SAFETY: valid slot.
        threads[i] = unsafe { (*slot(10 + i)).thread };
    }
    let mut distinct = 0;
    for i in 0..N {
        if !threads[..i].contains(&threads[i]) {
            distinct += 1;
        }
    }
    say!(t, "they ran on {} distinct thread(s), none of them this one: {}", distinct, yn(!threads.contains(&me)));
    let mut all_unreg = true;
    for i in 0..N {
        // SAFETY: valid handle.
        all_unreg &= unsafe { UnregisterWaitEx(hs[i], INVALID_HANDLE) } != 0;
    }
    say!(t, "UnregisterWaitEx(h, INVALID_HANDLE_VALUE) for all {N}: all TRUE {}", yn(all_unreg));

    // 5: UnregisterWait (no Ex) of a wait that never fired
    let ev5 = manual_event();
    let h5 = register(ev5, 250);
    // SAFETY: valid handle.
    let r = unsafe { UnregisterWait(h5) };
    say!(t, "UnregisterWait of a wait that never fired: {} (error {})", yn(r != 0), if r != 0 { 0 } else { gle() });
}
