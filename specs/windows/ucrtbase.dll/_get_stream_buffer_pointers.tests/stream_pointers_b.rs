//! ucrtbase `_get_stream_buffer_pointers(FILE*, char*** base, char*** ptr, int** cnt)`: what the three returned pointers point
//! at. They point at the stream's own buffer fields (inside the FILE object's internal record, not into the buffer), so
//! reading them shows the buffer state of a `FILE*` opened by `fopen`: nothing before the first read, the buffer start, the
//! read position and the bytes left after reads, the buffer size of `setvbuf`, the one-byte internal buffer of an unbuffered
//! stream, and a write buffer. Only offsets, differences and counts are printed, never addresses.
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 2 of the observations, the others write the rest.)
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

// ---- thread synchronisation helpers (declarations from Learn / winnt.h / synchapi.h / winternl.h) ----
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, Ordering::SeqCst};

/// A static that threads share; access through `p()` with raw pointers only.
struct Shared<T>(UnsafeCell<T>);
// SAFETY: the tests synchronise access themselves (that is what they test).
unsafe impl<T> Sync for Shared<T> {}
impl<T> Shared<T> {
    const fn new(v: T) -> Self {
        Shared(UnsafeCell::new(v))
    }
    fn p(&self) -> *mut T {
        self.0.get()
    }
}

/// winnt.h `RTL_SRWLOCK` / `SRWLOCK`: one pointer-sized word.
#[repr(C)]
struct SRWLOCK {
    p: usize,
}
/// winnt.h `RTL_CONDITION_VARIABLE`: one pointer-sized word.
#[repr(C)]
struct CONDVAR {
    p: usize,
}
/// winnt.h `RTL_RUN_ONCE` / `INIT_ONCE`: one pointer-sized word.
#[repr(C)]
struct RUNONCE {
    p: usize,
}
/// winnt.h `RTL_CRITICAL_SECTION` (x64: 40 bytes).
#[repr(C)]
struct CSECT {
    debug: *mut c_void,
    lock_count: i32,
    recursion: i32,
    owner: usize,
    sem: usize,
    spin: usize,
}

const SRW_INIT: SRWLOCK = SRWLOCK { p: 0 };
const CV_INIT: CONDVAR = CONDVAR { p: 0 };
const ONCE_INIT: RUNONCE = RUNONCE { p: 0 };
const CS_INIT: CSECT = CSECT { debug: core::ptr::null_mut(), lock_count: 0, recursion: 0, owner: 0, sem: 0, spin: 0 };

const STATUS_SUCCESS_: i32 = 0;
const STATUS_TIMEOUT_: i32 = 0x102;
const STATUS_PENDING_: i32 = 0x103;
const STATUS_UNSUCCESSFUL_: i32 = 0xC000_0001u32 as i32;
const STATUS_INVALID_PARAMETER_: i32 = 0xC000_000Du32 as i32;
const ERROR_TIMEOUT: u32 = 1460;
const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_INVALID_FLAGS: u32 = 1004;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_TIMEOUT: u32 = 258;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;
const CONDITION_VARIABLE_LOCKMODE_SHARED: u32 = 1;
const RTL_RUN_ONCE_CHECK_ONLY: u32 = 1;
const RTL_RUN_ONCE_ASYNC: u32 = 2;
const RTL_RUN_ONCE_INIT_FAILED: u32 = 4;

type ThreadProc = unsafe extern "system" fn(*mut c_void) -> u32;
type OnceFn = unsafe extern "system" fn(*mut RUNONCE, *mut c_void, *mut *mut c_void) -> u32;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlInitializeSRWLock(l: *mut SRWLOCK);
    fn RtlAcquireSRWLockExclusive(l: *mut SRWLOCK);
    fn RtlAcquireSRWLockShared(l: *mut SRWLOCK);
    fn RtlReleaseSRWLockExclusive(l: *mut SRWLOCK);
    fn RtlReleaseSRWLockShared(l: *mut SRWLOCK);
    fn RtlTryAcquireSRWLockExclusive(l: *mut SRWLOCK) -> u8;
    fn RtlTryAcquireSRWLockShared(l: *mut SRWLOCK) -> u8;
    fn RtlInitializeConditionVariable(c: *mut CONDVAR);
    fn RtlSleepConditionVariableSRW(c: *mut CONDVAR, l: *mut SRWLOCK, timeout: *const i64, flags: u32) -> i32;
    fn RtlSleepConditionVariableCS(c: *mut CONDVAR, cs: *mut CSECT, timeout: *const i64) -> i32;
    fn RtlWakeConditionVariable(c: *mut CONDVAR);
    fn RtlWakeAllConditionVariable(c: *mut CONDVAR);
    fn RtlRunOnceInitialize(o: *mut RUNONCE);
    fn RtlRunOnceExecuteOnce(o: *mut RUNONCE, f: OnceFn, param: *mut c_void, ctx: *mut *mut c_void) -> i32;
    fn RtlRunOnceBeginInitialize(o: *mut RUNONCE, flags: u32, ctx: *mut *mut c_void) -> i32;
    fn RtlRunOnceComplete(o: *mut RUNONCE, flags: u32, ctx: *mut c_void) -> i32;
    fn RtlInitializeCriticalSection(cs: *mut CSECT) -> i32;
    fn RtlEnterCriticalSection(cs: *mut CSECT) -> i32;
    fn RtlLeaveCriticalSection(cs: *mut CSECT) -> i32;
    fn RtlDeleteCriticalSection(cs: *mut CSECT) -> i32;
    fn RtlTryEnterCriticalSection(cs: *mut CSECT) -> u8;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(
        sa: *const SECURITY_ATTRIBUTES,
        stack: usize,
        f: ThreadProc,
        arg: *mut c_void,
        flags: u32,
        tid: *mut u32,
    ) -> HANDLE;
    fn QueryPerformanceCounter(v: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(v: *mut i64) -> BOOL;
    fn WaitForMultipleObjects(n: u32, h: *const HANDLE, all: BOOL, ms: u32) -> u32;
    fn ResetEvent(h: HANDLE) -> BOOL;
    fn SwitchToThread() -> BOOL;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
    fn InitializeSRWLock(l: *mut SRWLOCK);
    fn AcquireSRWLockExclusive(l: *mut SRWLOCK);
    fn AcquireSRWLockShared(l: *mut SRWLOCK);
    fn ReleaseSRWLockExclusive(l: *mut SRWLOCK);
    fn ReleaseSRWLockShared(l: *mut SRWLOCK);
    fn TryAcquireSRWLockExclusive(l: *mut SRWLOCK) -> u8;
    fn TryAcquireSRWLockShared(l: *mut SRWLOCK) -> u8;
    fn InitializeConditionVariable(c: *mut CONDVAR);
    fn SleepConditionVariableSRW(c: *mut CONDVAR, l: *mut SRWLOCK, ms: u32, flags: u32) -> BOOL;
    fn SleepConditionVariableCS(c: *mut CONDVAR, cs: *mut CSECT, ms: u32) -> BOOL;
    fn WakeConditionVariable(c: *mut CONDVAR);
    fn WakeAllConditionVariable(c: *mut CONDVAR);
    fn InitializeCriticalSection(cs: *mut CSECT);
    fn EnterCriticalSection(cs: *mut CSECT);
    fn LeaveCriticalSection(cs: *mut CSECT);
    fn DeleteCriticalSection(cs: *mut CSECT);
}

/// A manual-reset (or auto-reset) unnamed event.
fn new_event(manual: bool, initial: bool) -> HANDLE {
    // SAFETY: no name, default security.
    unsafe { CreateEventW(core::ptr::null(), BOOL::from(manual), BOOL::from(initial), core::ptr::null()) }
}

/// Start a thread running `f(arg)`; NULL when it could not start.
fn spawn(f: ThreadProc, arg: usize) -> HANDLE {
    // SAFETY: `f` is a thread procedure of this image; the arguments are plain values.
    unsafe { CreateThread(core::ptr::null(), 0, f, arg as *mut c_void, 0, core::ptr::null_mut()) }
}

/// Wait up to `ms` for the thread to end, then close its handle. True when it ended in time.
fn join(h: HANDLE, ms: u32) -> bool {
    // SAFETY: a handle returned by `spawn`, closed once.
    unsafe {
        let r = WaitForSingleObject(h, ms);
        CloseHandle(h);
        r == WAIT_OBJECT_0
    }
}

/// Wait for every thread (the same total budget for all), true when all ended.
fn join_all(hs: &[HANDLE], ms: u32) -> bool {
    let mut ok = true;
    for &h in hs {
        ok &= join(h, ms);
    }
    ok
}

fn sleep(ms: u32) {
    // SAFETY: plain call.
    unsafe { Sleep(ms) }
}

fn qpc() -> i64 {
    let mut v = 0i64;
    // SAFETY: a live out pointer.
    unsafe { QueryPerformanceCounter(&mut v) };
    v
}

/// Milliseconds since the counter value `t0`.
fn ms_since(t0: i64) -> i64 {
    let mut f = 1i64;
    // SAFETY: a live out pointer.
    unsafe { QueryPerformanceFrequency(&mut f) };
    (qpc() - t0) * 1000 / f
}

/// Wait (polling) until `flag` reaches `want` or `ms` pass; the final value.
fn wait_for(flag: &AtomicU32, want: u32, ms: u32) -> u32 {
    let t0 = qpc();
    while flag.load(SeqCst) != want && ms_since(t0) < i64::from(ms) {
        sleep(1);
    }
    flag.load(SeqCst)
}

/// A relative timeout for the Rtl calls: `ms` milliseconds as a negative 100 ns count.
fn rel(ms: i64) -> i64 {
    -ms * 10_000
}

fn yes(b: u8) -> u32 {
    u32::from(b != 0)
}

fn st(s: i32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    let _ = write!(b, "{:#x}", s as u32);
    b
}

/// A pointer-sized value as text without showing an address: `null`, `ptr` or the small number.
fn val(p: usize) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if p == 0 {
        let _ = write!(b, "0");
    } else if p < 0x1_0000 {
        let _ = write!(b, "{p:#x}");
    } else {
        let _ = write!(b, "big");
    }
    b
}

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

type FILE = c_void;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetErrorMode(mode: u32) -> u32;
    fn DeleteFileA(name: *const u8) -> BOOL;
}

#[link(name = "ucrtbase", kind = "raw-dylib")]
unsafe extern "C" {
    fn fopen(name: *const u8, mode: *const u8) -> *mut FILE;
    fn fclose(f: *mut FILE) -> i32;
    fn fread(buf: *mut c_void, size: usize, n: usize, f: *mut FILE) -> usize;
    fn fwrite(buf: *const c_void, size: usize, n: usize, f: *mut FILE) -> usize;
    fn fgetc(f: *mut FILE) -> i32;
    fn fflush(f: *mut FILE) -> i32;
    fn fseek(f: *mut FILE, off: i32, whence: i32) -> i32;
    fn setvbuf(f: *mut FILE, buf: *mut u8, mode: i32, size: usize) -> i32;
    fn __acrt_iob_func(i: u32) -> *mut FILE;
    fn _get_stream_buffer_pointers(
        f: *mut FILE,
        base: *mut *mut *mut u8,
        ptr: *mut *mut *mut u8,
        cnt: *mut *mut i32,
    ) -> i32;
}

const NAME: &[u8] = b"poly-stream-test.bin\0";

struct Ptrs {
    ret: i32,
    base: *mut *mut u8,
    ptr: *mut *mut u8,
    cnt: *mut i32,
}

fn get(f: *mut FILE) -> Ptrs {
    let mut p = Ptrs { ret: -1, base: core::ptr::null_mut(), ptr: core::ptr::null_mut(), cnt: core::ptr::null_mut() };
    // SAFETY: a stream this test opened and live out pointers.
    p.ret = unsafe { _get_stream_buffer_pointers(f, &mut p.base, &mut p.ptr, &mut p.cnt) };
    p
}

/// Where a slot lies relative to the FILE object: the offset when it is near, else "far".
fn at(slot: *mut u8, f: *mut FILE) -> Buf<24> {
    let d = slot as isize - f as isize;
    let mut b = Buf::<24>::new();
    if (-0x400..0x400).contains(&d) {
        let _ = write!(b, "{d:#x}");
    } else {
        let _ = write!(b, "far");
    }
    b
}

/// The state of a stream as lines: return value, slot offsets, buffer start null or not, read position within the buffer, bytes left.
fn show(t: &mut Test, what: &str, f: *mut FILE) {
    let p = get(f);
    // SAFETY: the slots belong to the stream, which is open.
    unsafe {
        let base = *p.base;
        let ptr = *p.ptr;
        let diff = if base.is_null() { 0 } else { ptr as isize - base as isize };
        let first = if base.is_null() || *p.cnt <= 0 { -1 } else { i32::from(*ptr) };
        say!(
            t,
            "{what}: returns {} slots at {} {} {} buffer {} position-in-buffer {} count {} next byte {}",
            p.ret,
            at(p.base as *mut u8, f),
            at(p.ptr as *mut u8, f),
            at(p.cnt as *mut u8, f),
            if base.is_null() { "null" } else { "set" },
            diff,
            *p.cnt,
            first
        );
    }
}

fn child(args: &str) -> u32 {
    // SAFETY: each case passes the arguments its name says; a fault ends the child (its exit code is the report).
    unsafe {
        SetErrorMode(0x8003);
        let f = fopen(NAME.as_ptr(), b"rb\0".as_ptr());
        let mut base = core::ptr::null_mut();
        let mut ptr = core::ptr::null_mut();
        let mut cnt = core::ptr::null_mut();
        let r = match args {
            "null_stream" => _get_stream_buffer_pointers(core::ptr::null_mut(), &mut base, &mut ptr, &mut cnt),
            "null_base" => _get_stream_buffer_pointers(f, core::ptr::null_mut(), &mut ptr, &mut cnt),
            "null_ptr" => _get_stream_buffer_pointers(f, &mut base, core::ptr::null_mut(), &mut cnt),
            "null_cnt" => _get_stream_buffer_pointers(f, &mut base, &mut ptr, core::ptr::null_mut()),
            "closed" => {
                fclose(f);
                _get_stream_buffer_pointers(f, &mut base, &mut ptr, &mut cnt)
            }
            _ => return 0xEEEE,
        };
        0x6000_0000 | (r as u32 & 0xFFFF)
    }
}

main_with_child!(child, test);

fn test(t: &mut Test) {
    SAY_PART.store(2, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: the calls follow the C runtime's contract on streams this test opened and closes once.
    unsafe {
        // a 10000 byte file whose byte i is i % 251
        let w = fopen(NAME.as_ptr(), b"wb\0".as_ptr());
        say!(t, "fopen for writing: {}", u32::from(!w.is_null()));
        let mut data = [0u8; 10_000];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        say!(t, "write buffer, fresh stream:");
        show(t, "  before any write", w);
        fwrite(data.as_ptr() as *const c_void, 1, 10, w);
        show(t, "  after fwrite of 10 bytes", w);
        fwrite(data.as_ptr().add(10) as *const c_void, 1, 90, w);
        show(t, "  after 100 bytes in all", w);
        fflush(w);
        show(t, "  after fflush", w);
        fwrite(data.as_ptr().add(100) as *const c_void, 1, 9_900, w);
        show(t, "  after 10000 bytes in all", w);
        fclose(w);

        let r = fopen(NAME.as_ptr(), b"rb\0".as_ptr());
        say!(t, "read buffer, default buffering:");
        show(t, "  fresh stream", r);
        let first = fgetc(r);
        show(t, "  after fgetc", r);
        say!(t, "  fgetc returned {first}");
        let mut buf = [0u8; 200];
        fread(buf.as_mut_ptr() as *mut c_void, 1, 99, r);
        show(t, "  after reading 100 bytes in all", r);
        let p1 = get(r);
        let p2 = get(r);
        say!(
            t,
            "  two calls give the same slots: {}",
            u32::from(p1.base == p2.base && p1.ptr == p2.ptr && p1.cnt == p2.cnt)
        );
        // the position of the bytes in the buffer matches what was read: the byte at ptr is the next one
        fread(buf.as_mut_ptr() as *mut c_void, 1, 200, r);
        show(t, "  after 300 bytes in all", r);
        let mut skip = [0u8; 4000];
        fread(skip.as_mut_ptr() as *mut c_void, 1, 3_800, r);
        show(t, "  after 4100 bytes in all (the first buffer refilled)", r);
        fseek(r, 0, 0);
        show(t, "  after fseek to 0", r);
        fgetc(r);
        show(t, "  after one more fgetc", r);
        fclose(r);

        let r = fopen(NAME.as_ptr(), b"rb\0".as_ptr());
        let sv = setvbuf(r, core::ptr::null_mut(), 0, 8192);
        say!(t, "setvbuf(_IOFBF, NULL, 8192): {sv}");
        show(t, "  before a read", r);
        fgetc(r);
        show(t, "  after fgetc", r);
        fclose(r);

        let r = fopen(NAME.as_ptr(), b"rb\0".as_ptr());
        let mut own = [0u8; 512];
        setvbuf(r, own.as_mut_ptr(), 0, 512);
        fgetc(r);
        let p = get(r);
        say!(
            t,
            "setvbuf with the program's own 512 byte buffer: buffer start is that buffer {} count {}",
            u32::from(*p.base == own.as_mut_ptr()),
            *p.cnt
        );
        fclose(r);

        let r = fopen(NAME.as_ptr(), b"rb\0".as_ptr());
        let sv = setvbuf(r, core::ptr::null_mut(), 4, 0);
        say!(t, "setvbuf(_IONBF): {sv}");
        show(t, "  before a read", r);
        let b = fgetc(r);
        show(t, "  after fgetc", r);
        let p = get(r);
        say!(
            t,
            "  the buffer start is inside the FILE record: {} byte read {b}",
            u32::from((*p.base as isize - r as isize).abs() < 0x400)
        );
        fclose(r);

        // text mode
        let r = fopen(NAME.as_ptr(), b"r\0".as_ptr());
        fgetc(r);
        show(t, "text mode after fgetc", r);
        fclose(r);

        // the standard streams
        for i in 0..3u32 {
            let s = __acrt_iob_func(i);
            let p = get(s);
            say!(t, "standard stream {i}: returns {}", p.ret);
        }

        // refused arguments, one child process each
        for case in ["null_stream", "null_base", "null_ptr", "null_cnt", "closed"] {
            let code = run_child(case).unwrap_or(0xFFFF);
            say!(t, "child {case}: {code:#x}");
        }
        DeleteFileA(NAME.as_ptr());
    }
}
