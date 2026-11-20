//! The overflow of the primary thread of a process (stack from the executable's header, 1 MiB in these programs) in a child process each:
//! with the exception caught (the exception code, the faulting page and the layout afterwards), without a handler (the process's exit
//! code), and the last pages: after the overflow the lowest page of the allocation is still reserved and touching it raises an access violation while the page above it can be used.
// poly-arch: x86
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

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
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
        pub extern "C" fn mainCRTStartup() -> ! {
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

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
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

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
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

// ---- stack probing helpers shared by the stack tests (this kit's own code; x64 and x86) ----
// Declarations from Microsoft Learn (VirtualQuery, VirtualProtect, CreateThread, SetThreadStackGuarantee,
// AddVectoredExceptionHandler, GetCurrentThreadStackLimits) and the public SDK headers (winnt.h:
// MEMORY_BASIC_INFORMATION, EXCEPTION_RECORD, EXCEPTION_POINTERS, PAGE_* and MEM_* constants; ntstatus.h:
// STATUS_STACK_OVERFLOW 0xC00000FD, STATUS_GUARD_PAGE_VIOLATION 0x80000001, STATUS_ACCESS_VIOLATION 0xC0000005).

#[cfg(target_pointer_width = "64")]
#[repr(C)]
struct MBI {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    pad: u32,
    size: usize,
    state: u32,
    protect: u32,
    ty: u32,
    pad2: u32,
}

#[cfg(target_pointer_width = "32")]
#[repr(C)]
struct MBI {
    base: usize,
    alloc_base: usize,
    alloc_protect: u32,
    size: usize,
    state: u32,
    protect: u32,
    ty: u32,
}

#[repr(C)]
struct ExRec {
    code: u32,
    flags: u32,
    next: *mut ExRec,
    addr: usize,
    n: u32,
    info: [usize; 15],
}

#[repr(C)]
struct ExPtrs {
    rec: *mut ExRec,
    ctx: *mut u8,
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn VirtualQuery(addr: usize, mbi: *mut MBI, len: usize) -> usize;
    fn VirtualProtect(addr: usize, size: usize, new: u32, old: *mut u32) -> BOOL;
    fn AddVectoredExceptionHandler(first: u32, h: unsafe extern "system" fn(*mut ExPtrs) -> i32) -> *mut c_void;
    fn RemoveVectoredExceptionHandler(h: *mut c_void) -> u32;
    fn SetThreadStackGuarantee(size: *mut u32) -> BOOL;
    fn GetCurrentThreadStackLimits(low: *mut usize, high: *mut usize);
}

#[link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn RtlCreateUserThread(
        process: HANDLE,
        sd: *const c_void,
        suspended: u8,
        zero_bits: usize,
        reserve: usize,
        commit: usize,
        start: unsafe extern "system" fn(*mut c_void) -> u32,
        param: *mut c_void,
        thread: *mut HANDLE,
        cid: *mut [usize; 2],
    ) -> i32;
}

const PAGE: usize = 4096;
const STATUS_STACK_OVERFLOW: u32 = 0xC000_00FD;
const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_GUARD_PAGE_VIOLATION: u32 = 0x8000_0001;

// ------------------------------------------------------------ recovery from an exception on the same thread
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    ".globl stk_catch",
    "stk_catch:",
    "push rbx", "push rbp", "push rdi", "push rsi", "push r12", "push r13", "push r14", "push r15",
    "sub rsp, 0x28",
    "mov [r8], rsp",
    "mov rax, rcx",
    "mov rcx, rdx",
    "call rax",
    "add rsp, 0x28",
    "pop r15", "pop r14", "pop r13", "pop r12", "pop rsi", "pop rdi", "pop rbp", "pop rbx",
    "ret",
    ".globl stk_resume",
    "stk_resume:",
    "mov eax, 0xFFFFFFFF",
    "add rsp, 0x28",
    "pop r15", "pop r14", "pop r13", "pop r12", "pop rsi", "pop rdi", "pop rbp", "pop rbx",
    "ret",
);

#[cfg(target_arch = "x86")]
core::arch::global_asm!(
    ".globl _stk_catch",
    "_stk_catch:",
    "push ebx", "push esi", "push edi", "push ebp",
    "mov eax, [esp + 20]",
    "mov ecx, [esp + 24]",
    "mov edx, [esp + 28]",
    "mov [edx], esp",
    "push ecx",
    "call eax",
    "pop ebp", "pop edi", "pop esi", "pop ebx",
    "ret",
    ".globl _stk_resume",
    "_stk_resume:",
    "mov eax, 0xFFFFFFFF",
    "pop ebp", "pop edi", "pop esi", "pop ebx",
    "ret",
);

unsafe extern "C" {
    fn stk_catch(f: unsafe extern "system" fn(usize) -> u32, arg: usize, save: *mut usize) -> u32;
    fn stk_resume();
}

#[cfg(target_arch = "x86_64")]
const CTX_SP: usize = 0x98;
#[cfg(target_arch = "x86_64")]
const CTX_IP: usize = 0xF8;
#[cfg(target_arch = "x86")]
const CTX_SP: usize = 0xC4;
#[cfg(target_arch = "x86")]
const CTX_IP: usize = 0xB8;

/// What the exception handler saw (one exception per `catch`).
struct Seen {
    armed: bool,
    saved_sp: usize,
    code: u32,
    n: u32,
    info0: usize,
    info1: usize,
    sp: usize,
}

static mut SEEN: Seen = Seen { armed: false, saved_sp: 0, code: 0, n: 0, info0: 0, info1: 0, sp: 0 };

unsafe extern "system" fn stk_veh(p: *mut ExPtrs) -> i32 {
    // SAFETY: the pointers come from the system's exception dispatch; one thread at a time probes.
    unsafe {
        let r = &*(*p).rec;
        let s = &raw mut SEEN;
        if !(*s).armed || !(r.code == STATUS_STACK_OVERFLOW || r.code == STATUS_ACCESS_VIOLATION || r.code == STATUS_GUARD_PAGE_VIOLATION) {
            return 0;
        }
        (*s).code = r.code;
        (*s).n = r.n;
        (*s).info0 = r.info[0];
        (*s).info1 = r.info[1];
        let ctx = (*p).ctx;
        (*s).sp = *(ctx.add(CTX_SP) as *const usize);
        *(ctx.add(CTX_SP) as *mut usize) = (*s).saved_sp;
        *(ctx.add(CTX_IP) as *mut usize) = stk_resume as *const () as usize;
        (*s).armed = false;
    }
    -1
}

/// Run `f(arg)` with exceptions of the three kinds caught: Ok(f's result) or Err(the exception code).
fn catch(f: unsafe extern "system" fn(usize) -> u32, arg: usize) -> Result<u32, u32> {
    // SAFETY: arms the handler for this thread's next exception; `stk_catch` saves and restores the registers the callee must keep.
    unsafe {
        let s = &raw mut SEEN;
        (*s).armed = true;
        let r = stk_catch(f, arg, &raw mut (*s).saved_sp);
        (*s).armed = false;
        if r == 0xFFFF_FFFF { Err((*s).code) } else { Ok(r) }
    }
}

fn seen() -> (u32, u32, usize, usize, usize) {
    // SAFETY: read after `catch` returned.
    unsafe {
        let s = &raw const SEEN;
        ((*s).code, (*s).n, (*s).info0, (*s).info1, (*s).sp)
    }
}

// ------------------------------------------------------------ the layout of a stack allocation
/// The pages of the allocation that holds `addr`, counted by kind (R reserved, G guard, C committed without guard), where the
/// highest guard page ends (`gtop`), the lowest guard page starts (`gbase`) and the allocation starts (`base`).
#[derive(Clone, Copy)]
struct Lay {
    base: usize,
    end: usize,
    r: usize,
    g: usize,
    c: usize,
    gbase: usize,
    gtop: usize,
    /// the lowest committed page (guard or not)
    low_commit: usize,
    /// the number of separate guard parts
    gn: usize,
    /// the lowest page committed without the guard flag
    lowc: usize,
}

fn lay(addr: usize) -> Lay {
    // SAFETY: plain queries.
    let mut m: MBI = unsafe { core::mem::zeroed() };
    unsafe { VirtualQuery(addr, &mut m, core::mem::size_of::<MBI>()) };
    let ab = m.alloc_base;
    let mut l = Lay { base: ab, end: ab, r: 0, g: 0, c: 0, gbase: 0, gtop: 0, low_commit: 0, gn: 0, lowc: 0 };
    let mut cur = ab;
    loop {
        // SAFETY: plain query.
        let mut r: MBI = unsafe { core::mem::zeroed() };
        if unsafe { VirtualQuery(cur, &mut r, core::mem::size_of::<MBI>()) } == 0 || r.alloc_base != ab {
            break;
        }
        let pages = r.size / PAGE;
        if r.state == 0x2000 {
            l.r += pages;
        } else {
            if l.low_commit == 0 {
                l.low_commit = r.base;
            }
            if r.protect & 0x100 != 0 {
                l.g += pages;
                l.gn += 1;
                if l.gbase == 0 {
                    l.gbase = r.base;
                }
                l.gtop = r.base + r.size;
            } else {
                l.c += pages;
                if l.lowc == 0 {
                    l.lowc = r.base;
                }
            }
        }
        cur = r.base + r.size;
    }
    l.end = cur;
    l
}

/// The layout as text, bottom to top: `R12 G3 C1 G1 C4`.
fn lay_text(addr: usize) -> Buf<160> {
    // SAFETY: plain query.
    let mut m: MBI = unsafe { core::mem::zeroed() };
    unsafe { VirtualQuery(addr, &mut m, core::mem::size_of::<MBI>()) };
    let ab = m.alloc_base;
    let mut b = Buf::<160>::new();
    let mut cur = ab;
    let mut first = true;
    loop {
        // SAFETY: plain query.
        let mut r: MBI = unsafe { core::mem::zeroed() };
        if unsafe { VirtualQuery(cur, &mut r, core::mem::size_of::<MBI>()) } == 0 || r.alloc_base != ab {
            break;
        }
        let kind = if r.state == 0x2000 { 'R' } else if r.protect & 0x100 != 0 { 'G' } else { 'C' };
        if !first {
            let _ = write!(b, " ");
        }
        first = false;
        let _ = write!(b, "{}{}", kind, r.size / PAGE);
        cur = r.base + r.size;
    }
    b
}

fn here() -> usize {
    let local = 0u8;
    core::hint::black_box(&local as *const u8 as usize)
}

// ------------------------------------------------------------ touching memory
#[derive(Clone, Copy, PartialEq)]
enum Acc {
    Read,
    Write,
}

unsafe extern "system" fn touch_w(addr: usize) -> u32 {
    // SAFETY: deliberately touches an address near the stack: that is what is observed.
    unsafe { core::ptr::write_volatile(addr as *mut u8, 0x5A) };
    0
}

unsafe extern "system" fn touch_r(addr: usize) -> u32 {
    // SAFETY: as above.
    u32::from(unsafe { core::ptr::read_volatile(addr as *const u8) } == 0x5A)
}

/// A frame without a probe: `rsp = addr` (not aligned), then `push`: the way a function with a large frame that skips
/// `__chkstk` first touches the stack.
#[cfg(target_arch = "x86_64")]
unsafe extern "system" fn touch_push(addr: usize) -> u32 {
    let a = (addr & !7) + 8;
    // SAFETY: deliberately moves the stack pointer to a chosen address and pushes there, then restores it.
    unsafe {
        core::arch::asm!("mov {old}, rsp", "mov rsp, {new}", "push 0", "mov rsp, {old}", old = out(reg) _, new = in(reg) a);
    }
    0
}

#[cfg(target_arch = "x86")]
unsafe extern "system" fn touch_push(addr: usize) -> u32 {
    let a = (addr & !3) + 4;
    // SAFETY: deliberately moves the stack pointer to a chosen address and pushes there, then restores it.
    unsafe {
        core::arch::asm!("mov {old}, esp", "mov esp, {new}", "push 0", "mov esp, {old}", old = out(reg) _, new = in(reg) a);
    }
    0
}

// ------------------------------------------------------------ threads with chosen stacks
#[derive(Clone, Copy)]
enum Make {
    /// CreateThread, dwStackSize with STACK_SIZE_PARAM_IS_A_RESERVATION (0 = the image's default)
    Reserve(usize),
    /// CreateThread, dwStackSize as the initial commit
    Commit(usize),
    /// RtlCreateUserThread with a reserve and a commit size (bytes)
    Native(usize, usize),
}

/// Run `f(arg)` on a new thread with the stack made as `how` says; Some(the thread's result) or None when it could not be made
/// or did not end in 20 s.
fn on_thread(how: Make, f: unsafe extern "system" fn(*mut c_void) -> u32, arg: usize) -> Option<u32> {
    let mut h: HANDLE = core::ptr::null_mut();
    let mut tid = 0u32;
    // SAFETY: plain thread starts with a start routine of this test.
    unsafe {
        match how {
            Make::Reserve(n) => {
                h = CreateThread(core::ptr::null(), n, f, arg as *mut c_void, if n != 0 { 0x10000 } else { 0 }, &mut tid);
            }
            Make::Commit(n) => {
                h = CreateThread(core::ptr::null(), n, f, arg as *mut c_void, 0, &mut tid);
            }
            Make::Native(r, c) => {
                let mut cid = [0usize; 2];
                let st = RtlCreateUserThread(-1isize as HANDLE, core::ptr::null(), 0, 0, r, c, f, arg as *mut c_void, &mut h, &mut cid);
                if st != 0 {
                    return None;
                }
            }
        }
    }
    if h.is_null() {
        return None;
    }
    let (ended, code) = join(h, 20_000);
    if ended { Some(code) } else { None }
}

static mut LINE: Buf<6000> = Buf::new();

fn line_clear() {
    // SAFETY: one probe thread at a time writes, the parent reads after the thread ended.
    unsafe { *(&raw mut LINE) = Buf::new() };
}

fn line_text() -> Buf<6000> {
    // SAFETY: read after the thread ended.
    unsafe { core::ptr::read(&raw const LINE) }
}

macro_rules! lput {
    ($($a:tt)*) => {{
        // SAFETY: one probe thread at a time writes.
        let b = unsafe { &mut *(&raw mut LINE) };
        let _ = write!(b, $($a)*);
    }};
}

/// Describe an exception code by name for the output.
fn code_name(c: u32) -> &'static str {
    match c {
        STATUS_STACK_OVERFLOW => "STATUS_STACK_OVERFLOW",
        STATUS_ACCESS_VIOLATION => "STATUS_ACCESS_VIOLATION",
        STATUS_GUARD_PAGE_VIOLATION => "STATUS_GUARD_PAGE_VIOLATION",
        0 => "none",
        _ => "other",
    }
}

fn install_veh() -> *mut c_void {
    // SAFETY: registers the handler of this file; removed by the caller or at process end.
    unsafe { AddVectoredExceptionHandler(1, stk_veh) }
}

/// Commit about 20 KiB of stack below the caller and give it back: afterwards the exception dispatch of a probe finds
/// committed pages under its stack pointer and does not grow the stack itself.
#[inline(never)]
fn pregrow() {
    let mut a = [0u8; 20000];
    core::hint::black_box(&mut a);
    a[0] = 1;
    core::hint::black_box(&a);
}

// ------------------------------------------------------------ the standard probe
/// What the next `probe_thread` does: touch `gtop - d` once, where `d` is `val` bytes (rel = false) or the size of the guard region
/// in bytes plus `val` (rel = true; 0 is the first byte of the lowest guard page, 1 the last byte of the page below the region).
struct Cfg {
    rel: bool,
    val: isize,
    acc: usize,
    guarantee: u32,
}

static mut CFG: Cfg = Cfg { rel: false, val: 1, acc: 1, guarantee: 0 };

fn cfg(rel: bool, val: isize, acc: usize, guarantee: u32) {
    // SAFETY: before the probe thread starts.
    unsafe { *(&raw mut CFG) = Cfg { rel, val, acc, guarantee } };
}

fn delta(a: usize, b: usize) -> isize {
    a as isize - b as isize
}

/// A new thread's probe: optionally raise the stack guarantee, commit some stack below the caller, then touch one byte and report.
unsafe extern "system" fn probe_thread(_p: *mut c_void) -> u32 {
    // SAFETY: set by the parent before the thread starts.
    let (rel, val, acc, gua) = unsafe {
        let c = &*(&raw const CFG);
        (c.rel, c.val, c.acc, c.guarantee)
    };
    if gua != 0 {
        let mut g = gua;
        // SAFETY: a live out value.
        unsafe { SetThreadStackGuarantee(&mut g) };
    }
    pregrow();
    let _ = lay(here());
    let before = lay(here());
    let d = if rel { (before.g * PAGE) as isize + val } else { val } as usize;
    let addr = before.gtop - d;
    let res = catch(if acc == 1 { touch_w } else { touch_r }, addr);
    let after = lay(here());
    lput!("guard region {} pages; ", before.g);
    match res {
        Ok(_) => lput!("touch succeeded"),
        Err(c) => {
            let (_, n, i0, _, _) = seen();
            lput!("{} (information {}, {} parameters)", code_name(c), i0, n);
        }
    }
    lput!("; pages reserved {:+}, guard {:+}, committed {:+}; guard region now {} pages", delta(after.r, before.r), delta(after.g, before.g), delta(after.c, before.c), after.g);
    0
}

/// Run the standard probe on a thread made as `how`, print the line.
fn probe_line(t: &mut Test, what: &str, how: Make) {
    line_clear();
    let r = on_thread(how, probe_thread, 0);
    let lt = line_text();
    say!(t, "{what}: {}", if r.is_some() { lt.as_str() } else { "the thread did not run" });
}

/// One touch inside a running probe thread, relative to the current layout, with the change it made.
fn step(label: &str, rel: bool, val: isize, acc: usize) {
    let before = lay(here());
    let d = if rel { (before.g * PAGE) as isize + val } else { val } as usize;
    let addr = before.gtop - d;
    let res = catch(if acc == 1 { touch_w } else { touch_r }, addr);
    let after = lay(here());
    lput!("{label}: ");
    match res {
        Ok(_) => lput!("touch succeeded"),
        Err(c) => {
            let (_, n, i0, _, _) = seen();
            lput!("{} (information {}, {} parameters)", code_name(c), i0, n);
        }
    }
    lput!("; pages reserved {:+}, guard {:+}, committed {:+}; guard {} pages in {} parts
", delta(after.r, before.r), delta(after.g, before.g), delta(after.c, before.c), after.g, after.gn);
}

/// Print the lines (separated by newlines) a probe thread collected.
fn print_lines(t: &mut Test, prefix: &str) {
    let lt = line_text();
    for l in lt.as_str().split(char::from(10)) {
        if !l.is_empty() {
            say!(t, "{prefix}{l}");
        }
    }
}

// ------------------------------------------------------------ running the stack out
#[inline(never)]
fn rec(n: usize) -> usize {
    let mut a = [0u8; 2048];
    core::hint::black_box(&mut a);
    a[0] = n as u8;
    let r = rec(n + 1);
    core::hint::black_box(&a);
    r + a[1] as usize
}

/// Calls itself until the stack is used up.
unsafe extern "system" fn rec_run(_a: usize) -> u32 {
    rec(0) as u32
}

/// Write down what one overflow round did: the exception, the stack pointer at the exception in pages above the allocation's start,
/// and the layout after the recovery.
fn round_line(label: &str, res: Result<u32, u32>) {
    let l = lay(here());
    lput!("{label}: ");
    match res {
        Ok(_) => lput!("returned without an exception"),
        Err(c) => {
            let (_, n, i0, i1, sp) = seen();
            let _ = sp;
            lput!("{} (information {}, {} parameters), faulting address {} pages above the start of the allocation", code_name(c), i0, n, (i1.wrapping_sub(l.base)) as isize / PAGE as isize);
        }
    }
    lput!("; afterwards {} pages in all: {} reserved, {} guard, {} committed\n", (l.end - l.base) / PAGE, l.r, l.g, l.c);
}

/// Set the guard flag on the lowest `pages` committed pages (what `_resetstkoflw` does, here with a chosen count).
fn rearm(pages: usize) -> bool {
    let l = lay(here());
    let mut old = 0u32;
    // SAFETY: protects pages of this thread's own stack that nothing is using.
    unsafe { VirtualProtect(l.low_commit, pages * PAGE, 0x104, &mut old) != 0 }
}

// ------------------------------------------------------------ frames with and without a probe
/// A frame of `n` bytes with no probe (what a compiler emits for a small frame or with the probe switched off): the stack pointer is
/// lowered by `n`, one byte at the new bottom is written, the stack pointer is put back.
#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn frame_bare(n: usize) {
    // SAFETY: lowers and restores the stack pointer around one write.
    unsafe {
        core::arch::asm!("mov {old}, rsp", "sub rsp, {n}", "mov byte ptr [rsp], 1", "mov rsp, {old}", old = out(reg) _, n = in(reg) n);
    }
}

#[cfg(target_arch = "x86")]
#[inline(never)]
fn frame_bare(n: usize) {
    // SAFETY: lowers and restores the stack pointer around one write.
    unsafe {
        core::arch::asm!("mov {old}, esp", "sub esp, {n}", "mov byte ptr [esp], 1", "mov esp, {old}", old = out(reg) _, n = in(reg) n);
    }
}

/// The same frame with the probe loop a compiler's `__chkstk` makes: the pages are touched one after the other from the top down.
#[cfg(target_arch = "x86_64")]
#[inline(never)]
fn frame_probed(n: usize) {
    // SAFETY: lowers the stack pointer a page at a time, writing at each step, and restores it.
    unsafe {
        core::arch::asm!(
            "mov {old}, rsp",
            "mov {i}, {n}",
            "2:",
            "sub rsp, 4096",
            "mov byte ptr [rsp], 1",
            "sub {i}, 4096",
            "ja 2b",
            "mov rsp, {old}",
            old = out(reg) _, i = out(reg) _, n = in(reg) n
        );
    }
}

#[cfg(target_arch = "x86")]
#[inline(never)]
fn frame_probed(n: usize) {
    // SAFETY: lowers the stack pointer a page at a time, writing at each step, and restores it.
    unsafe {
        core::arch::asm!(
            "mov {old}, esp",
            "mov {i}, {n}",
            "2:",
            "sub esp, 4096",
            "mov byte ptr [esp], 1",
            "sub {i}, 4096",
            "ja 2b",
            "mov esp, {old}",
            old = out(reg) _, i = out(reg) _, n = in(reg) n
        );
    }
}

main_with_child!(child, test);

fn child(args: &str) -> u32 {
    let f = beside("polyws-stkovfm.txt");
    if args == "bare" {
        // SAFETY: deliberately runs the stack out with no handler at all.
        return unsafe { rec_run(0) };
    }
    let _h = install_veh();
    pregrow();
    let first = lay(here());
    line_clear();
    let res = catch(rec_run, 0);
    round_line("overflow", res);
    let lt = line_text();
    append_line(&f, lt.as_str().trim());
    let l = lay(here());
    let mut b = Buf::<300>::new();
    let _ = write!(b, "guard region at the start {} pages; the allocation has {} pages", first.g, (l.end - l.base) / PAGE);
    append_line(&f, b.as_str());
    for (what, addr) in [("the first byte of the allocation", l.base), ("the first byte of the second page", l.base + PAGE), ("the last byte of the second page", l.base + 2 * PAGE - 1)] {
        let r = catch(touch_w, addr);
        let mut b = Buf::<200>::new();
        let _ = write!(b, "write at {what}: {}", if r.is_ok() { "succeeded" } else { code_name(r.unwrap_err()) });
        append_line(&f, b.as_str());
    }
    0
}

fn test(t: &mut Test) {
    let f = beside("polyws-stkovfm.txt");
    for args in ["caught", "bare"] {
        delete_file(&f);
        let c = start_child(args, 0).expect("child");
        let code = c.finish();
        say!(t, "primary thread, {args}: process ended with {code:#x}");
        let txt = read_text(&f);
        for l in txt.s().split(char::from(10)) {
            if !l.is_empty() {
                say!(t, "    {l}");
            }
        }
    }
    delete_file(&f);
}
