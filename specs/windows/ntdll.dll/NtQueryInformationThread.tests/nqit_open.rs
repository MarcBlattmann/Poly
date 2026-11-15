//! NtOpenThread and NtQueryInformationThread (the MSYS2 runtime opens its own thread with access 0x400, THREAD_SET_LIMITED_
//! INFORMATION, and asks about threads): which access masks open a thread, and which classes each mask may query (basic
//! information needs THREAD_QUERY_INFORMATION or THREAD_QUERY_LIMITED_INFORMATION, set-limited only is refused), the
//! values of basic information (exit status STILL_ACTIVE, thread and process ids, priorities), the Win32 start address class
//! (9: asked by the runtime's signal thread on itself), the length rules and bad handles.
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

poly_wtest::main!(test);

#[repr(C)]
struct CLIENT_ID {
    process: usize,
    thread: usize,
}

#[repr(C)]
struct OBJECT_ATTRIBUTES {
    length: u32,
    root: HANDLE,
    name: *const c_void,
    attributes: u32,
    sd: *const c_void,
    qos: *const c_void,
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtOpenThread(h: *mut HANDLE, access: u32, oa: *const OBJECT_ATTRIBUTES, cid: *const CLIENT_ID) -> i32;
    fn NtQueryInformationThread(h: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

static TID: AtomicU32 = AtomicU32::new(0);
static STOP: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn worker(_: *mut c_void) -> u32 {
    // SAFETY: plain calls.
    unsafe {
        TID.store(GetCurrentThreadId(), SeqCst);
        while STOP.load(SeqCst) == 0 {
            Sleep(5);
        }
    }
    0
}

fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn rd64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3], b[at + 4], b[at + 5], b[at + 6], b[at + 7]])
}

fn open(tid: u32, access: u32) -> (i32, HANDLE) {
    let oa = OBJECT_ATTRIBUTES {
        length: 48,
        root: core::ptr::null_mut(),
        name: core::ptr::null(),
        attributes: 0,
        sd: core::ptr::null(),
        qos: core::ptr::null(),
    };
    let cid = CLIENT_ID { process: 0, thread: tid as usize };
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: live structures; the handle is closed by the caller.
    let s = unsafe { NtOpenThread(&mut h, access, &oa, &cid) };
    (s, h)
}

fn rl(r: u32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    if r == 0xAAAA_AAAA {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{r}");
    }
    b
}

fn qt(h: HANDLE, class: u32, len: u32, buf: &mut [u8; 256]) -> (i32, u32) {
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: the buffer holds 256 bytes and `len` stays within it.
    let s = unsafe { NtQueryInformationThread(h, class, buf.as_mut_ptr() as *mut c_void, len, &mut ret) };
    (s, ret)
}

fn test(t: &mut Test) {
    SAY_PART.store(1, core::sync::atomic::Ordering::Relaxed);
    let mut b = [0u8; 256];
    // SAFETY: plain calls on a thread this test started.
    unsafe {
        let h = spawn(worker, 0);
        wait_for(&TID, TID.load(SeqCst).max(1), 100);
        sleep(100);
        let tid = TID.load(SeqCst);
        say!(t, "worker thread id is non-zero: {}", u32::from(tid != 0));

        for (name, access) in [
            ("THREAD_QUERY_INFORMATION 0x40", 0x40u32),
            ("THREAD_QUERY_LIMITED_INFORMATION 0x800", 0x800),
            ("THREAD_SET_LIMITED_INFORMATION 0x400", 0x400),
            ("THREAD_SET_INFORMATION 0x20", 0x20),
            ("SYNCHRONIZE 0x100000", 0x0010_0000),
            ("THREAD_GET_CONTEXT 0x8", 0x8),
            ("THREAD_SUSPEND_RESUME 0x2", 0x2),
            ("THREAD_ALL_ACCESS 0x1fffff", 0x1F_FFFF),
            ("MAXIMUM_ALLOWED", 0x0200_0000),
            ("no access 0", 0),
        ] {
            let (s, th) = open(tid, access);
            say!(t, "NtOpenThread({name}): {}", st(s));
            if s == 0 {
                let (s0, r0) = qt(th, 0, 48, &mut b);
                say!(t, "  class 0 basic information: {} length {}", st(s0), rl(r0));
                let (s9, r9) = qt(th, 9, 8, &mut b);
                say!(t, "  class 9 Win32 start address: {} length {}", st(s9), rl(r9));
                let (s3, r3) = qt(th, 1, 32, &mut b);
                say!(t, "  class 1 times: {} length {}", st(s3), rl(r3));
                let (s6, r6) = qt(th, 3, 4, &mut b);
                say!(t, "  class 3 base priority: {} length {}", st(s6), rl(r6));
                close(th);
            }
        }

        // values
        let (s, th) = open(tid, 0x800);
        let (s0, r0) = qt(th, 0, 48, &mut b);
        say!(
            t,
            "basic information of the worker: {} length {} exit status {:#x} thread id matches {} process id matches {} priority {} base priority {}",
            st(s0),
            rl(r0),
            rd32(&b, 0),
            u32::from(rd64(&b, 24) == tid as u64),
            u32::from(rd64(&b, 16) == u64::from(GetCurrentProcessId())),
            rd32(&b, 40) as i32,
            rd32(&b, 44) as i32
        );
        let _ = s;
        for len in [0u32, 8, 47, 49, 64] {
            let (s, r) = qt(th, 0, len, &mut b);
            say!(t, "class 0 with length {len}: {} returned length {}", st(s), rl(r));
        }
        let (s, r) = qt(th, 9, 8, &mut b);
        say!(
            t,
            "class 9 on the worker: {} length {} address is non-zero {}",
            st(s),
            rl(r),
            u32::from(rd64(&b, 0) != 0)
        );
        for class in [
            1u32, 2, 4, 5, 7, 8, 10, 11, 12, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31,
            32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43,
        ] {
            let (s, r) = qt(th, class, 16, &mut b);
            say!(t, "class {class} with 16 bytes: {} length {}", st(s), rl(r));
        }
        close(th);
        // the pseudo-handle of this thread
        let (s, r) = qt(GetCurrentThread(), 0, 48, &mut b);
        say!(
            t,
            "current thread pseudo-handle, class 0: {} length {} thread id matches {}",
            st(s),
            rl(r),
            u32::from(rd64(&b, 24) == u64::from(GetCurrentThreadId()))
        );
        let (s, r) = qt(GetCurrentThread(), 9, 8, &mut b);
        say!(t, "current thread pseudo-handle, class 9: {} length {}", st(s), rl(r));
        let (s, _) = qt(core::ptr::null_mut(), 0, 48, &mut b);
        say!(t, "NULL handle: {}", st(s));
        let (s, _) = qt(NT_CURRENT_PROCESS, 0, 48, &mut b);
        say!(t, "the process pseudo-handle: {}", st(s));

        // a thread id that does not exist
        let (s, th) = open(0xFFFF_FF00, 0x800);
        say!(t, "NtOpenThread of a thread id that does not exist: {}", st(s));
        close(th);
        let (s, th) = open(0, 0x800);
        say!(t, "NtOpenThread of thread id 0: {}", st(s));
        close(th);

        STOP.store(1, SeqCst);
        join(h, 5000);
    }
}
