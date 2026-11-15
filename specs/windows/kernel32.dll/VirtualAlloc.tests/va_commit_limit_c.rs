//! Error codes of VirtualAlloc and NtAllocateVirtualMemory for requests that cannot be met: reserving more than the address
//! space, committing more than the system could ever back (the commit limit, 1455 / STATUS_COMMITMENT_LIMIT, which V8 checks
//! for), committing inside a reserved range, a zero size, and a process under a job object memory limit (limit hit by a
//! commit gives which error, a reserve beyond the limit still works, committed memory freed makes room again).
//! (Kept in three files that make the same calls: this one writes every third line, starting with line 3 of the observations, the others write the rest.)
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

main_with_child!(child, test);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn VirtualAlloc(addr: *mut c_void, size: usize, kind: u32, protect: u32) -> *mut c_void;
    fn VirtualFree(addr: *mut c_void, size: usize, kind: u32) -> BOOL;
    fn CreateJobObjectW(sa: *const SECURITY_ATTRIBUTES, name: *const u16) -> HANDLE;
    fn SetInformationJobObject(job: HANDLE, class: u32, info: *const c_void, len: u32) -> BOOL;
    fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> BOOL;
    fn SetErrorMode(mode: u32) -> u32;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtAllocateVirtualMemory(
        process: HANDLE,
        base: *mut *mut c_void,
        zero_bits: usize,
        size: *mut usize,
        kind: u32,
        protect: u32,
    ) -> i32;
    fn NtFreeVirtualMemory(process: HANDLE, base: *mut *mut c_void, size: *mut usize, kind: u32) -> i32;
}

const MEM_COMMIT: u32 = 0x1000;
const MEM_RESERVE: u32 = 0x2000;
const MEM_RELEASE: u32 = 0x8000;
const PAGE_READWRITE: u32 = 4;
const PAGE_NOACCESS: u32 = 1;
const MB: usize = 1 << 20;
const TB: usize = 1 << 40;

fn va(size: usize, kind: u32, prot: u32) -> (usize, u32) {
    // SAFETY: allocation requests only; the result is freed by the caller.
    unsafe {
        mark_error();
        let p = VirtualAlloc(core::ptr::null_mut(), size, kind, prot);
        (p as usize, last_error())
    }
}

fn nt(size: usize, kind: u32, prot: u32) -> (i32, usize, usize) {
    let mut base: *mut c_void = core::ptr::null_mut();
    let mut sz = size;
    // SAFETY: allocation requests only; the result is freed by the caller.
    let s = unsafe { NtAllocateVirtualMemory(NT_CURRENT_PROCESS, &mut base, 0, &mut sz, kind, prot) };
    (s, base as usize, sz)
}

fn release(p: usize) {
    if p != 0 {
        // SAFETY: a region this test allocated, released once.
        unsafe { VirtualFree(p as *mut c_void, 0, MEM_RELEASE) };
    }
}

fn outcome(r: (usize, u32)) -> Buf<40> {
    let mut b = Buf::<40>::new();
    if r.0 != 0 {
        let _ = write!(b, "ok");
        release(r.0);
    } else {
        let _ = write!(b, "NULL error {}", r.1);
    }
    b
}

fn nt_outcome(r: (i32, usize, usize), asked: usize) -> Buf<64> {
    let mut b = Buf::<64>::new();
    if r.0 == 0 {
        let _ = write!(b, "ok (size {})", if r.2 == asked { "as asked" } else { "rounded" });
        // SAFETY: the region this call just allocated.
        unsafe {
            let mut base = r.1 as *mut c_void;
            let mut sz = 0usize;
            NtFreeVirtualMemory(NT_CURRENT_PROCESS, &mut base, &mut sz, MEM_RELEASE);
        }
    } else {
        let _ = write!(b, "status {:#x} size {}", r.0 as u32, if r.2 == asked { "unchanged" } else { "changed" });
    }
    b
}

fn okerr(ok: bool, e: u32) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if ok {
        let _ = write!(b, "ok");
    } else {
        let _ = write!(b, "error {e}");
    }
    b
}

fn child(args: &str) -> u32 {
    let hout = arg_handle(args, 1);
    let mut out = ChildOut { h: hout };
    let which = args.split(' ').next().unwrap_or("");
    // SAFETY: the job object calls pass live structures; the allocations are freed where they succeed.
    unsafe {
        SetErrorMode(0x8003);
        let job = CreateJobObjectW(core::ptr::null(), core::ptr::null());
        let mut info = [0u8; 144];
        // JOBOBJECT_EXTENDED_LIMIT_INFORMATION: LimitFlags at 16, ProcessMemoryLimit at 112, JobMemoryLimit at 120
        let (flag, off) = if which == "jobproc" { (0x100u32, 112usize) } else { (0x200u32, 120usize) };
        info[16..20].copy_from_slice(&flag.to_le_bytes());
        info[off..off + 8].copy_from_slice(&((64 * MB) as u64).to_le_bytes());
        say!(out, "SetInformationJobObject: {}", SetInformationJobObject(job, 9, info.as_ptr() as *const c_void, 144));
        say!(out, "AssignProcessToJobObject: {}", AssignProcessToJobObject(job, GetCurrentProcess()));
        let r = va(16 * MB, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        say!(out, "commit 16 MB under the 64 MB limit: {}", if r.0 != 0 { "ok" } else { "failed" });
        let keep = r.0;
        let r = va(32 * MB, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        say!(out, "commit 32 MB more (48 MB in all): {}", if r.0 != 0 { "ok" } else { "failed" });
        let keep2 = r.0;
        let r = va(32 * MB, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        say!(out, "commit 32 MB more (80 MB in all): {}", okerr(r.0 != 0, r.1));
        let s = nt(32 * MB, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        say!(out, "NtAllocateVirtualMemory the same: {}", nt_outcome(s, 32 * MB));
        let s = nt(256 * MB, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        say!(out, "NtAllocateVirtualMemory commit 256 MB: {}", nt_outcome(s, 256 * MB));
        let r = va(1 << 30, MEM_RESERVE, PAGE_NOACCESS);
        say!(out, "reserve 1 GB (no commit): {}", if r.0 != 0 { "ok" } else { "failed" });
        let big = r.0;
        if big != 0 {
            // commit inside the reservation, over the limit
            mark_error();
            let p = VirtualAlloc(big as *mut c_void, 128 * MB, MEM_COMMIT, PAGE_READWRITE);
            say!(out, "commit 128 MB inside the reservation: {}", okerr(!p.is_null(), last_error()));
            mark_error();
            let p = VirtualAlloc(big as *mut c_void, 8 * MB, MEM_COMMIT, PAGE_READWRITE);
            say!(out, "commit 8 MB inside the reservation (56 MB in all): {}", okerr(!p.is_null(), last_error()));
        }
        release(big);
        release(keep2);
        let r = va(32 * MB, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        say!(out, "after freeing 32 MB, commit 32 MB again: {}", if r.0 != 0 { "ok" } else { "failed" });
        release(r.0);
        release(keep);
    }
    0x600D
}

fn test(t: &mut Test) {
    SAY_PART.store(3, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: allocation requests only; every success is freed at once.
    unsafe {
        say!(t, "reserve 64 KB: {}", outcome(va(0x10000, MEM_RESERVE, PAGE_NOACCESS)));
        say!(t, "reserve 0 bytes: {}", outcome(va(0, MEM_RESERVE, PAGE_NOACCESS)));
        say!(t, "commit 0 bytes: {}", outcome(va(0, MEM_COMMIT, PAGE_READWRITE)));
        for (name, size) in [
            ("1 TB", TB),
            ("32 TB", 32 * TB),
            ("100 TB", 100 * TB),
            ("128 TB less 64 KB", 128 * TB - 0x10000),
            ("128 TB", 128 * TB),
            ("256 TB", 256 * TB),
            ("2^63", 1usize << 63),
            ("2^64 - 1 page", usize::MAX - 0xFFF),
            ("2^64 - 1", usize::MAX),
        ] {
            say!(t, "reserve {name}: {}", outcome(va(size, MEM_RESERVE, PAGE_NOACCESS)));
        }
        for (name, size) in [
            ("4 TB", 4 * TB),
            ("16 TB", 16 * TB),
            ("64 TB", 64 * TB),
            ("100 TB", 100 * TB),
            ("128 TB", 128 * TB),
            ("2^63", 1usize << 63),
            ("2^64 - 1 page", usize::MAX - 0xFFF),
        ] {
            say!(t, "reserve and commit {name}: {}", outcome(va(size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE)));
        }
        // NtAllocateVirtualMemory: the status and whether the size is written back
        for (name, size, kind) in [
            ("reserve 128 TB", 128 * TB, MEM_RESERVE),
            ("reserve 256 TB", 256 * TB, MEM_RESERVE),
            ("reserve 2^64 - 1 page", usize::MAX - 0xFFF, MEM_RESERVE),
            ("reserve and commit 4 TB", 4 * TB, MEM_COMMIT | MEM_RESERVE),
            ("reserve and commit 64 TB", 64 * TB, MEM_COMMIT | MEM_RESERVE),
            ("reserve and commit 128 TB", 128 * TB, MEM_COMMIT | MEM_RESERVE),
            ("reserve and commit 2^64 - 1 page", usize::MAX - 0xFFF, MEM_COMMIT | MEM_RESERVE),
            ("size 0", 0, MEM_RESERVE),
        ] {
            say!(
                t,
                "Nt {name}: {}",
                nt_outcome(nt(size, kind, if kind & MEM_COMMIT != 0 { PAGE_READWRITE } else { PAGE_NOACCESS }), size)
            );
        }

        // reserve a large range, then commit more than the system could back inside it
        let (base, e) = va(64 * TB, MEM_RESERVE, PAGE_NOACCESS);
        say!(t, "reserve 64 TB: {}", okerr(base != 0, e));
        if base != 0 {
            mark_error();
            let p = VirtualAlloc(base as *mut c_void, 32 * TB, MEM_COMMIT, PAGE_READWRITE);
            say!(t, "  commit 32 TB inside it: {}", okerr(!p.is_null(), last_error()));
            let mut b2: *mut c_void = base as *mut c_void;
            let mut sz = 32 * TB;
            let s = NtAllocateVirtualMemory(NT_CURRENT_PROCESS, &mut b2, 0, &mut sz, MEM_COMMIT, PAGE_READWRITE);
            say!(t, "  Nt commit 32 TB inside it: status {:#x}", s as u32);
            let p = VirtualAlloc(base as *mut c_void, 0x10000, MEM_COMMIT, PAGE_READWRITE);
            say!(t, "  commit 64 KB inside it: {}", if !p.is_null() { "ok" } else { "failed" });
            release(base);
        }

        // under a job object
        for which in ["jobproc", "jobjob"] {
            let rep = Report::new();
            let mut a = Buf::<64>::new();
            let _ = write!(a, "{which} {:x}", rep.write as usize);
            let ch = start_child(a.as_str(), 0);
            let code = match ch {
                Some(c) => c.finish(),
                None => 0xFFFF,
            };
            say!(t, "{which}: child exit {code:#x}");
            rep.print(t);
        }
    }
}
