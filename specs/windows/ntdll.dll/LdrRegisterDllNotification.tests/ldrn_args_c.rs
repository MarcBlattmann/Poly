//! LdrRegisterDllNotification argument rules: the flags word must be 0, the callback must not be NULL, a NULL cookie
//! pointer and bad cookies for the unregister call are refused with statuses (each case runs in a child process whose
//! exit code is the status, so a fault shows as 0xC0000005).
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

// winternl.h / Microsoft Learn "LdrRegisterDllNotification": the callback and its data.
#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn LdrRegisterDllNotification(
        flags: u32,
        callback: Option<NotifyFn>,
        context: *mut c_void,
        cookie: *mut *mut c_void,
    ) -> i32;
    fn LdrUnregisterDllNotification(cookie: *mut c_void) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExW(name: *const u16, file: HANDLE, flags: u32) -> HANDLE;
    fn FreeLibrary(module: HANDLE) -> BOOL;
}

/// winternl.h `UNICODE_STRING`.
#[repr(C)]
struct UNICODE_STRING {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

/// winternl.h `LDR_DLL_LOADED_NOTIFICATION_DATA` (the unloaded form has the same layout).
#[repr(C)]
struct LDR_DLL_NOTIFICATION_DATA {
    flags: u32,
    full_dll_name: *const UNICODE_STRING,
    base_dll_name: *const UNICODE_STRING,
    dll_base: *mut c_void,
    size_of_image: u32,
}

type NotifyFn = unsafe extern "system" fn(u32, *const LDR_DLL_NOTIFICATION_DATA, *mut c_void);

/// One recorded notification.
#[derive(Clone, Copy)]
struct Event {
    reason: u32,
    flags: u32,
    ctx: usize,
    thread: u32,
    base: [u8; 24],
    full_len: u16,
    full_ends_with_base: bool,
    dll_base: usize,
    size: u32,
}

const NO_EVENT: Event = Event {
    reason: 0,
    flags: 0,
    ctx: 0,
    thread: 0,
    base: [0; 24],
    full_len: 0,
    full_ends_with_base: false,
    dll_base: 0,
    size: 0,
};

static EVENTS: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
static mut LOG: [Event; 256] = [NO_EVENT; 256];

fn lower(u: u16) -> u16 {
    if (0x41..=0x5a).contains(&u) { u + 0x20 } else { u }
}

/// The callback: records what it was given. It runs with the loader lock held, so it only copies.
unsafe extern "system" fn record(reason: u32, data: *const LDR_DLL_NOTIFICATION_DATA, ctx: *mut c_void) {
    use core::sync::atomic::Ordering::SeqCst;
    let i = EVENTS.fetch_add(1, SeqCst) as usize;
    if i >= 256 {
        return;
    }
    let mut e = NO_EVENT;
    e.reason = reason;
    e.ctx = ctx as usize;
    // SAFETY: no arguments.
    e.thread = unsafe { GetCurrentThreadId() };
    // SAFETY: the loader passes a live notification structure whose strings are valid during the call.
    unsafe {
        let d = &*data;
        e.flags = d.flags;
        e.dll_base = d.dll_base as usize;
        e.size = d.size_of_image;
        let base = &*d.base_dll_name;
        let full = &*d.full_dll_name;
        let bu = core::slice::from_raw_parts(base.buffer, base.length as usize / 2);
        let fu = core::slice::from_raw_parts(full.buffer, full.length as usize / 2);
        for (k, &u) in bu.iter().take(24).enumerate() {
            e.base[k] = lower(u) as u8;
        }
        e.full_len = full.length / 2;
        e.full_ends_with_base = fu.len() > bu.len()
            && fu[fu.len() - bu.len() - 1] == 0x5c
            && fu[fu.len() - bu.len()..].iter().zip(bu).all(|(&a, &b)| lower(a) == lower(b));
        LOG[i] = e;
    }
}

fn event_count() -> usize {
    (EVENTS.load(core::sync::atomic::Ordering::SeqCst) as usize).min(256)
}

fn event(i: usize) -> Event {
    // SAFETY: indexes below the recorded count were written by the callback; the tests read after the loader call returned.
    unsafe { LOG[i] }
}

fn base_text(e: &Event) -> &str {
    let n = e.base.iter().position(|&b| b == 0).unwrap_or(24);
    core::str::from_utf8(&e.base[..n]).unwrap_or("?")
}

/// The recorded events whose base name is `name` (lower case): (loaded, unloaded).
fn count_for(name: &str) -> (usize, usize) {
    let (mut l, mut u) = (0, 0);
    for i in 0..event_count() {
        let e = event(i);
        if base_text(&e) == name {
            if e.reason == 1 {
                l += 1;
            } else if e.reason == 2 {
                u += 1;
            }
        }
    }
    (l, u)
}

fn load(name: &str, flags: u32) -> HANDLE {
    let w = wide(name);
    // SAFETY: a NUL-terminated name.
    unsafe { LoadLibraryExW(w.as_ptr(), core::ptr::null_mut(), flags) }
}

fn reset_events() {
    EVENTS.store(0, core::sync::atomic::Ordering::SeqCst);
}

main_with_child!(child, test);

fn child(args: &str) -> u32 {
    let mut cookie: *mut c_void = core::ptr::null_mut();
    // SAFETY: each case passes exactly the arguments its name says; a fault ends the child (the parent reports its exit code).
    unsafe {
        match args {
            "flags1" => LdrRegisterDllNotification(1, Some(record), core::ptr::null_mut(), &mut cookie) as u32,
            "flags2" => LdrRegisterDllNotification(2, Some(record), core::ptr::null_mut(), &mut cookie) as u32,
            "flags_big" => {
                LdrRegisterDllNotification(0x8000_0000, Some(record), core::ptr::null_mut(), &mut cookie) as u32
            }
            "nullcb" => LdrRegisterDllNotification(0, None, core::ptr::null_mut(), &mut cookie) as u32,
            "nullcookie" => {
                LdrRegisterDllNotification(0, Some(record), core::ptr::null_mut(), core::ptr::null_mut()) as u32
            }
            "ok" => LdrRegisterDllNotification(0, Some(record), core::ptr::null_mut(), &mut cookie) as u32,
            "unreg_null" => LdrUnregisterDllNotification(core::ptr::null_mut()) as u32,
            "unreg_garbage" => LdrUnregisterDllNotification(0x1234usize as *mut c_void) as u32,
            "unreg_twice" => {
                LdrRegisterDllNotification(0, Some(record), core::ptr::null_mut(), &mut cookie);
                LdrUnregisterDllNotification(cookie);
                LdrUnregisterDllNotification(cookie) as u32
            }
            "cookie_untouched_on_error" => {
                let mut c: *mut c_void = 0x77usize as *mut c_void;
                LdrRegisterDllNotification(1, Some(record), core::ptr::null_mut(), &mut c);
                u32::from(c as usize == 0x77)
            }
            "cookie_written_on_success" => {
                let mut c: *mut c_void = 0x77usize as *mut c_void;
                LdrRegisterDllNotification(0, Some(record), core::ptr::null_mut(), &mut c);
                u32::from(c as usize != 0x77 && !c.is_null())
            }
            _ => 0xEEEE,
        }
    }
}

fn test(t: &mut Test) {
    SAY_PART.store(3, core::sync::atomic::Ordering::Relaxed);
    for case in [
        "ok",
        "flags1",
        "flags2",
        "flags_big",
        "nullcb",
        "nullcookie",
        "unreg_null",
        "unreg_garbage",
        "unreg_twice",
        "cookie_untouched_on_error",
        "cookie_written_on_success",
    ] {
        let code = run_child(case).unwrap_or(0xFFFF);
        say!(t, "{case}: {code:#x}");
    }
}
