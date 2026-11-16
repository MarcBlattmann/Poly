//! CreateWaitableTimerExW: the flags (manual reset 1, high resolution 2), the access mask, names; and the timer's behaviour through
//! SetWaitableTimer / SetWaitableTimerEx / CancelWaitableTimer / OpenWaitableTimerW (signal, auto and manual reset, period, cancel,
//! completion routine, absolute and relative due times).
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

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
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

poly_wtest::main!(test);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateWaitableTimerExW(sa: *const c_void, name: *const u16, flags: u32, access: u32) -> HANDLE;
    fn CreateWaitableTimerW(sa: *const c_void, manual: BOOL, name: *const u16) -> HANDLE;
    fn OpenWaitableTimerW(access: u32, inherit: BOOL, name: *const u16) -> HANDLE;
    fn SetWaitableTimer(
        h: HANDLE,
        due: *const i64,
        period: i32,
        f: *const c_void,
        a: *const c_void,
        resume: BOOL,
    ) -> BOOL;
    fn SetWaitableTimerEx(
        h: HANDLE,
        due: *const i64,
        period: i32,
        f: *const c_void,
        a: *const c_void,
        ctx: *const c_void,
        delay: u32,
    ) -> BOOL;
    fn CancelWaitableTimer(h: HANDLE) -> BOOL;
    fn SleepEx(ms: u32, alertable: BOOL) -> u32;
    fn QueryPerformanceCounter(v: *mut i64) -> BOOL;
    fn QueryPerformanceFrequency(v: *mut i64) -> BOOL;
}

const TIMER_ALL_ACCESS: u32 = 0x1F0003;
const TIMER_MODIFY_STATE: u32 = 2;
const TIMER_QUERY_STATE: u32 = 1;
const SYNCHRONIZE: u32 = 0x100000;
const MANUAL: u32 = 1;
const HIGH_RES: u32 = 2;
static FIRED: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static FIRED_ARG: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

unsafe extern "system" fn routine(arg: *mut c_void, _lo: u32, _hi: u32) {
    FIRED.fetch_add(1, core::sync::atomic::Ordering::SeqCst);
    FIRED_ARG.store(arg as usize, core::sync::atomic::Ordering::SeqCst);
}

fn ms_now() -> i64 {
    let (mut v, mut f) = (0i64, 1i64);
    // SAFETY: live out pointers.
    unsafe {
        QueryPerformanceCounter(&mut v);
        QueryPerformanceFrequency(&mut f);
    }
    v * 1000 / f
}

fn wait(h: HANDLE, ms: u32) -> u32 {
    // SAFETY: a handle of this test.
    unsafe { WaitForSingleObject(h, ms) }
}

fn test(t: &mut Test) {
    SAY_PART.store(1, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: plain calls with live arguments.
    unsafe {
        // flags and access
        for (flags, access) in [
            (0u32, TIMER_ALL_ACCESS),
            (MANUAL, TIMER_ALL_ACCESS),
            (HIGH_RES, TIMER_ALL_ACCESS),
            (MANUAL | HIGH_RES, TIMER_ALL_ACCESS),
            (0, TIMER_MODIFY_STATE | SYNCHRONIZE),
            (4, TIMER_ALL_ACCESS),
            (0x80, TIMER_ALL_ACCESS),
            (0, 0),
            (0, TIMER_QUERY_STATE | SYNCHRONIZE),
        ] {
            mark_error();
            let h = CreateWaitableTimerExW(core::ptr::null(), core::ptr::null(), flags, access);
            let e = last_error();
            say!(
                t,
                "CreateWaitableTimerExW(flags {flags:#x}, access {access:#x}): handle {} err {}",
                opened(h),
                if h.is_null() { e } else { 0 }
            );
            if !h.is_null() {
                let due: i64 = -10_000;
                mark_error();
                let r = SetWaitableTimer(h, &due, 0, core::ptr::null(), core::ptr::null(), 0);
                let e = last_error();
                let w = if r != 0 { wait(h, 2000) } else { 0xFFFF_FFFF };
                say!(t, "   SetWaitableTimer(1 ms) = {} err {} wait {}", r, if r == 0 { e } else { 0 }, w);
                close(h);
            }
        }
        // auto reset: one signal, then not signalled; manual reset stays signalled
        let auto = CreateWaitableTimerExW(core::ptr::null(), core::ptr::null(), 0, TIMER_ALL_ACCESS);
        let manual = CreateWaitableTimerExW(core::ptr::null(), core::ptr::null(), MANUAL, TIMER_ALL_ACCESS);
        let due: i64 = -200_000;
        SetWaitableTimer(auto, &due, 0, core::ptr::null(), core::ptr::null(), 0);
        SetWaitableTimer(manual, &due, 0, core::ptr::null(), core::ptr::null(), 0);
        let t0 = ms_now();
        let w1 = wait(auto, 3000);
        let el = ms_now() - t0;
        say!(t, "20 ms auto-reset timer: first wait {} elapsed in 15..2000 ms {}", w1, (15..2000).contains(&el));
        say!(t, "  second wait (0 ms) {}", wait(auto, 0));
        say!(t, "manual-reset timer: waits {} {} {}", wait(manual, 3000), wait(manual, 0), wait(manual, 0));
        // setting again resets the signalled state
        let far: i64 = -600_000_000;
        say!(
            t,
            "SetWaitableTimer(60 s) on the signalled manual timer = {}",
            SetWaitableTimer(manual, &far, 0, core::ptr::null(), core::ptr::null(), 0)
        );
        say!(t, "  then wait(0) {}", wait(manual, 0));
        say!(t, "CancelWaitableTimer = {}", CancelWaitableTimer(manual));
        say!(t, "  then wait(50 ms) {}", wait(manual, 50));
        say!(t, "CancelWaitableTimer of a timer that is not set = {}", CancelWaitableTimer(manual));
        // period
        let due: i64 = -100_000;
        SetWaitableTimer(auto, &due, 20, core::ptr::null(), core::ptr::null(), 0);
        let mut got = 0;
        for _ in 0..4 {
            if wait(auto, 2000) == 0 {
                got += 1;
            }
        }
        say!(t, "periodic timer (10 ms, then every 20 ms): four waits all signalled {}", got == 4);
        CancelWaitableTimer(auto);
        // absolute due time in the past
        let past: i64 = 1;
        SetWaitableTimer(auto, &past, 0, core::ptr::null(), core::ptr::null(), 0);
        say!(t, "absolute due time in the past: wait {}", wait(auto, 1000));
        // due 0
        let zero: i64 = 0;
        SetWaitableTimer(auto, &zero, 0, core::ptr::null(), core::ptr::null(), 0);
        say!(t, "due time 0: wait {}", wait(auto, 1000));
        // bad arguments
        mark_error();
        say!(
            t,
            "SetWaitableTimer(NULL due) = {} err {}",
            SetWaitableTimer(auto, core::ptr::null(), 0, core::ptr::null(), core::ptr::null(), 0),
            last_error()
        );
        mark_error();
        let due: i64 = -10_000;
        say!(
            t,
            "SetWaitableTimer(period -1) = {} err {}",
            SetWaitableTimer(auto, &due, -1, core::ptr::null(), core::ptr::null(), 0),
            last_error()
        );
        mark_error();
        let ev = CreateEventW(core::ptr::null(), 0, 0, core::ptr::null());
        say!(
            t,
            "SetWaitableTimer(an event handle) = {} err {}",
            SetWaitableTimer(ev, &due, 0, core::ptr::null(), core::ptr::null(), 0),
            last_error()
        );
        mark_error();
        say!(t, "CancelWaitableTimer(an event handle) = {} err {}", CancelWaitableTimer(ev), last_error());
        close(ev);
        mark_error();
        say!(
            t,
            "SetWaitableTimer(NULL handle) = {} err {}",
            SetWaitableTimer(core::ptr::null_mut(), &due, 0, core::ptr::null(), core::ptr::null(), 0),
            last_error()
        );
        // a handle without TIMER_MODIFY_STATE
        let ro = CreateWaitableTimerExW(core::ptr::null(), core::ptr::null(), 0, TIMER_QUERY_STATE | SYNCHRONIZE);
        mark_error();
        say!(
            t,
            "SetWaitableTimer without TIMER_MODIFY_STATE = {} err {}",
            SetWaitableTimer(ro, &due, 0, core::ptr::null(), core::ptr::null(), 0),
            last_error()
        );
        mark_error();
        say!(t, "CancelWaitableTimer without TIMER_MODIFY_STATE = {} err {}", CancelWaitableTimer(ro), last_error());
        close(ro);
        // SetWaitableTimerEx
        let due: i64 = -100_000;
        mark_error();
        say!(
            t,
            "SetWaitableTimerEx(10 ms, tolerable delay 5) = {} err {}",
            SetWaitableTimerEx(auto, &due, 0, core::ptr::null(), core::ptr::null(), core::ptr::null(), 5),
            if last_error() == MARK { 0 } else { last_error() }
        );
        say!(t, "  wait {}", wait(auto, 1000));
        // completion routine
        FIRED.store(0, core::sync::atomic::Ordering::SeqCst);
        let r = SetWaitableTimer(auto, &due, 0, routine as *const c_void, 0x1234usize as *const c_void, 0);
        let s = SleepEx(2000, 1);
        say!(
            t,
            "completion routine: set {} SleepEx {} calls {} argument {:#x}",
            r,
            s,
            FIRED.load(core::sync::atomic::Ordering::SeqCst),
            FIRED_ARG.load(core::sync::atomic::Ordering::SeqCst)
        );
        say!(t, "  the timer object is also signalled: {}", wait(auto, 100));
        // names
        let mut nm = Buf::<64>::new();
        let _ = write!(nm, "poly-timer-{:x}", GetCurrentProcessId());
        let w = wide(nm.as_str());
        mark_error();
        let a = CreateWaitableTimerExW(core::ptr::null(), w.as_ptr(), 0, TIMER_ALL_ACCESS);
        say!(t, "named timer: handle {} err {}", opened(a), err_text());
        mark_error();
        let b = CreateWaitableTimerExW(core::ptr::null(), w.as_ptr(), 0, TIMER_ALL_ACCESS);
        say!(t, "  same name again: handle {} err {}", opened(b), err_text());
        mark_error();
        let c = OpenWaitableTimerW(TIMER_ALL_ACCESS, 0, w.as_ptr());
        say!(t, "  OpenWaitableTimerW: handle {} err {}", opened(c), err_text());
        let none = wide("poly-timer-no-such-name");
        mark_error();
        let d = OpenWaitableTimerW(TIMER_ALL_ACCESS, 0, none.as_ptr());
        say!(t, "  OpenWaitableTimerW(missing) handle {} err {}", opened(d), err_text());
        // the name of an event
        let ev2 = CreateEventW(core::ptr::null(), 0, 0, wide("poly-timer-event-name").as_ptr());
        mark_error();
        let e2 = CreateWaitableTimerExW(core::ptr::null(), wide("poly-timer-event-name").as_ptr(), 0, TIMER_ALL_ACCESS);
        say!(t, "  a timer named like an event: handle {} err {}", opened(e2), err_text());
        close(ev2);
        close(a);
        close(b);
        close(c);
        // the older CreateWaitableTimerW agrees
        let old = CreateWaitableTimerW(core::ptr::null(), 1, core::ptr::null());
        SetWaitableTimer(old, &due, 0, core::ptr::null(), core::ptr::null(), 0);
        say!(t, "CreateWaitableTimerW(manual): wait {} then {}", wait(old, 1000), wait(old, 0));
        close(old);
        close(auto);
        close(manual);
    }
}
