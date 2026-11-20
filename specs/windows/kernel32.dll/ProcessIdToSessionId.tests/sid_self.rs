//! ProcessIdToSessionId for processes of this test: its own id (the answer equals the session of its own token), a child
//! process of the same image that is running, the same child after it ended, an id that no process has, process id 0, and
//! a NULL output pointer (in a child process); what happens to the output and to the last error on each failure.
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
    SECURITY_ATTRIBUTES { length: core::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, security_descriptor: core::ptr::null_mut(), inherit_handle: 1 }
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
        cb: core::mem::size_of::<STARTUPINFOW>() as u32,
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
        CreateProcessW(core::ptr::null(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 1, flags, core::ptr::null(), core::ptr::null(), &si, &mut pi)
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
            let ok = unsafe { ReadFile(self.read, all.as_mut_ptr().add(n), (all.len() - n) as u32, &mut got, core::ptr::null_mut()) };
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


// ===================================================================== small file and text helpers (spec-win-updater-services)
// Declarations from Microsoft Learn (fileapi.h, winbase.h, winnt.h); the helpers are this test's own.

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, template: HANDLE) -> HANDLE;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn CreateDirectoryW(name: *const u16, sa: *const c_void) -> BOOL;
    fn RemoveDirectoryW(name: *const u16) -> BOOL;
    fn SetFilePointerEx(h: HANDLE, dist: i64, newpos: *mut i64, method: u32) -> BOOL;
    fn GetFileSizeEx(h: HANDLE, size: *mut i64) -> BOOL;
    fn SetFileAttributesW(name: *const u16, attrs: u32) -> BOOL;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const CREATE_ALWAYS: u32 = 2;
const CREATE_NEW: u32 = 1;
const FILE_SHARE_ALL: u32 = 7;

/// A wide path of a file beside the executable: `<dir of the image>\<name>`.
fn beside(name: &str) -> [u16; 700] {
    let mut p = [0u16; 700];
    // SAFETY: the buffer holds 700 units; a NULL module is the executable.
    let mut n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 600) } as usize;
    while n > 0 && p[n - 1] != 92 {
        n -= 1;
    }
    for b in name.bytes() {
        p[n] = u16::from(b);
        n += 1;
    }
    p[n] = 0;
    p
}

fn wsz(s: &str) -> [u16; 400] {
    let mut w = [0u16; 400];
    for (i, c) in s.chars().enumerate().take(399) {
        w[i] = c as u32 as u16;
    }
    w
}

fn file_exists(path: &[u16]) -> bool {
    // SAFETY: `path` is NUL-terminated.
    unsafe { GetFileAttributesW(path.as_ptr()) != 0xFFFF_FFFF }
}

fn delete_file(path: &[u16]) {
    // SAFETY: `path` is NUL-terminated.
    unsafe { DeleteFileW(path.as_ptr()) };
}

/// Write `data` to a new file (replacing an older one); true when it worked.
fn put_file(path: &[u16], data: &[u8]) -> bool {
    // SAFETY: `path` is NUL-terminated; the handle is closed here.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_WRITE, FILE_SHARE_ALL, core::ptr::null(), CREATE_ALWAYS, 0x80, core::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut put = 0u32;
        let ok = data.is_empty() || WriteFile(h, data.as_ptr(), data.len() as u32, &mut put, core::ptr::null_mut()) != 0;
        CloseHandle(h);
        ok
    }
}

fn append_line(path: &[u16], line: &str) {
    // SAFETY: `path` is NUL-terminated; the handle is closed here.
    unsafe {
        let h = CreateFileW(path.as_ptr(), 4, FILE_SHARE_ALL, core::ptr::null(), OPEN_ALWAYS, 0x80, core::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return;
        }
        let mut put = 0u32;
        let mut buf = [0u8; 400];
        let n = line.len().min(398);
        buf[..n].copy_from_slice(&line.as_bytes()[..n]);
        buf[n] = b'\n';
        WriteFile(h, buf.as_ptr(), (n + 1) as u32, &mut put, core::ptr::null_mut());
        CloseHandle(h);
    }
}

/// The contents of a file (up to 12 KB; empty when missing).
struct Text {
    b: [u8; 12288],
    n: usize,
}

impl Text {
    fn s(&self) -> &str {
        core::str::from_utf8(&self.b[..self.n]).unwrap_or("?")
    }
}

fn read_text(path: &[u16]) -> Text {
    let mut t = Text { b: [0; 12288], n: 0 };
    // SAFETY: `path` is NUL-terminated; reads into `t.b`; the handle is closed here.
    unsafe {
        let h = CreateFileW(path.as_ptr(), GENERIC_READ, FILE_SHARE_ALL, core::ptr::null(), OPEN_EXISTING, 0x80, core::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return t;
        }
        let mut got = 0u32;
        ReadFile(h, t.b.as_mut_ptr(), 12288, &mut got, core::ptr::null_mut());
        t.n = got as usize;
        CloseHandle(h);
    }
    t
}

fn hex8(v: i32) -> Buf<12> {
    let mut b = Buf::<12>::new();
    let _ = write!(b, "{:#010x}", v as u32);
    b
}

/// `<ok>` as text for a BOOL result with the last error: `1` or `0 (error N)`.
fn ok_err(ok: BOOL, err: u32) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if ok != 0 {
        let _ = write!(b, "1");
    } else {
        let _ = write!(b, "0 (error {err})");
    }
    b
}

/// Run `f` as a child of this image (`--child <arg>`), wait, its exit code (0xFFFFFFFE when it could not start).
fn child_code(arg: &str) -> u32 {
    run_child(arg).unwrap_or(0xFFFF_FFFE)
}

use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering::SeqCst};

struct Sync_<T>(UnsafeCell<T>);
// SAFETY: the cells are written before other threads run, or by one thread at a time.
unsafe impl<T> Sync for Sync_<T> {}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateThread(sa: *const c_void, stack: usize, start: unsafe extern "system" fn(*mut c_void) -> u32, param: *mut c_void, flags: u32, tid: *mut u32) -> HANDLE;
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
}

/// Run `f(arg)` on a new thread; its handle.
fn spawn(f: unsafe extern "system" fn(*mut c_void) -> u32, arg: usize) -> HANDLE {
    let mut tid = 0u32;
    // SAFETY: a plain thread start; the start routine is an `extern "system"` function of this test.
    unsafe { CreateThread(core::ptr::null(), 0, f, arg as *mut c_void, 0, &mut tid) }
}

/// Wait for a thread (`ms` at most) and close its handle; whether it ended, and its exit code.
fn join(h: HANDLE, ms: u32) -> (bool, u32) {
    let mut code = 0u32;
    // SAFETY: the handle of a thread this test started.
    let ended = unsafe {
        let ended = WaitForSingleObject(h, ms) == 0;
        GetExitCodeThread(h, &mut code);
        ended
    };
    close(h);
    (ended, code)
}

main_with_child!(child, test);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(process: HANDLE, access: u32, out: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> BOOL;
}

fn token_session() -> u32 {
    let mut tok: HANDLE = core::ptr::null_mut();
    let mut v = [0u8; 4];
    let mut r = 0u32;
    // SAFETY: live out values; the handle is closed here.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), 8, &mut tok) == 0 {
            return 0xFFFF;
        }
        GetTokenInformation(tok, 12, v.as_mut_ptr(), 4, &mut r);
        close(tok);
    }
    u32::from_le_bytes(v)
}

fn child(args: &str) -> u32 {
    match args {
        "null" => {
            // SAFETY: the NULL output is the point of the test.
            let ok = unsafe { ProcessIdToSessionId(GetCurrentProcessId(), core::ptr::null_mut()) };
            last_error() + 1000 * ok as u32
        }
        "wait" => {
            let quit = beside("polyws-quit-sid");
            let mut n = 0;
            while !file_exists(&quit) && n < 300 {
                // SAFETY: a plain sleep.
                unsafe { Sleep(20) };
                n += 1;
            }
            0
        }
        _ => 0,
    }
}

fn ask(t: &mut Test, what: &str, pid: u32) {
    let mut s = 0xDEADu32;
    mark_error();
    // SAFETY: a live out value.
    let ok = unsafe { ProcessIdToSessionId(pid, &mut s) };
    let e = last_error();
    let mut b = Buf::<200>::new();
    let _ = write!(b, "{what}: {}", ok_err(ok, e).as_str());
    if ok != 0 {
        let _ = write!(b, ", session equals the session of this process's token: {}", u32::from(s == token_session()));
    } else {
        let _ = write!(b, ", output {}", if s == 0xDEAD { "untouched" } else { "written" });
    }
    say!(t, "{}", b.as_str());
}

fn test(t: &mut Test) {
    // SAFETY: no arguments.
    ask(t, "this process", unsafe { GetCurrentProcessId() });
    let quit = beside("polyws-quit-sid");
    delete_file(&quit);
    let c = start_child("wait", 0).expect("child");
    let cpid = c.pid;
    ask(t, "a running child process", cpid);
    make_file_here(&quit);
    let code = c.finish();
    say!(t, "the child ended with {code}");
    ask(t, "the child after it ended", cpid);
    delete_file(&quit);
    ask(t, "process id 0", 0);
    ask(t, "an id no process has", 0x00FF_FFF0);
    ask(t, "id 0xFFFFFFFF", 0xFFFF_FFFF);
    let code = child_code("null");
    say!(t, "NULL output pointer: the child reports {code}");
}

fn make_file_here(p: &[u16]) {
    put_file(p, b"x");
}
