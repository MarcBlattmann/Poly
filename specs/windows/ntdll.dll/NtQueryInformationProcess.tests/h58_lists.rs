//! NtQueryInformationProcess class 0x3a (58) on the calling process (the pseudo-handle and a real handle): the buffer-length
//! rules (below 4 bytes: STATUS_INFO_LENGTH_MISMATCH; from 4 bytes on: success with as many whole 4-byte entries as fit, no
//! "buffer too small" status, the returned length is the bytes written), the content (one 4-byte handle value per open
//! handle of the process, strictly ascending, multiples of 4, as many entries as ProcessHandleCount says), and the
//! argument errors (NULL, misaligned and unreadable buffers, NULL and invalid handles, the thread pseudo-handle).
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

// winternl.h (NtQueryInformationProcess, NtQueryObject prototypes), processthreadsapi.h, handleapi.h, synchapi.h, memoryapi.h,
// ioapiset.h, jobapi2.h: the class this track observes is the number 0x3a (58); no public header names it.
const CLASS_HANDLE_LIST: u32 = 0x3a;
const CLASS_HANDLE_COUNT: u32 = 20;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC0000004u32 as i32;
const STATUS_ACCESS_DENIED: i32 = 0xC0000022u32 as i32;
const STATUS_INVALID_HANDLE: i32 = 0xC0000008u32 as i32;
const STATUS_ACCESS_VIOLATION: i32 = 0xC0000005u32 as i32;
const STATUS_OBJECT_TYPE_MISMATCH: i32 = 0xC0000024u32 as i32;
const STATUS_DATATYPE_MISALIGNMENT: i32 = 0x80000002u32 as i32;
const STATUS_PROCESS_IS_TERMINATING: i32 = 0xC000010Au32 as i32;
const PROCESS_DUP_HANDLE: u32 = 0x40;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_ALL_ACCESS: u32 = 0x1F_FFFF;
const DUPLICATE_SAME_ACCESS: u32 = 2;
const HANDLE_FLAG_PROTECT_FROM_CLOSE: u32 = 2;
const SENTINEL_RET: u32 = 0x7777;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationProcess(p: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtQueryObject(h: HANDLE, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn OpenThread(access: u32, inherit: BOOL, tid: u32) -> HANDLE;
    fn ResumeThread(h: HANDLE) -> u32;
    fn DuplicateHandle(
        sp: HANDLE,
        sh: HANDLE,
        tp: HANDLE,
        th: *mut HANDLE,
        access: u32,
        inherit: BOOL,
        options: u32,
    ) -> BOOL;
    fn SetHandleInformation(h: HANDLE, mask: u32, flags: u32) -> BOOL;
    fn CreateSemaphoreW(sa: *const SECURITY_ATTRIBUTES, initial: i32, max: i32, name: *const u16) -> HANDLE;
    fn CreateMutexW(sa: *const SECURITY_ATTRIBUTES, owner: BOOL, name: *const u16) -> HANDLE;
    fn CreateWaitableTimerW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, name: *const u16) -> HANDLE;
    fn CreateFileMappingW(
        file: HANDLE,
        sa: *const SECURITY_ATTRIBUTES,
        prot: u32,
        hi: u32,
        lo: u32,
        name: *const u16,
    ) -> HANDLE;
    fn CreateIoCompletionPort(file: HANDLE, port: HANDLE, key: usize, threads: u32) -> HANDLE;
    fn CreateJobObjectW(sa: *const SECURITY_ATTRIBUTES, name: *const u16) -> HANDLE;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disp: u32,
        flags: u32,
        tmpl: HANDLE,
    ) -> HANDLE;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: HANDLE, sub: *const u16, options: u32, sam: u32, out: *mut HANDLE) -> i32;
    fn RegCloseKey(key: HANDLE) -> i32;
}

/// The handle list of a query: the entries and the status and returned length.
struct HList {
    v: [u32; 4096],
    n: usize,
    status: i32,
    ret: u32,
}

/// Query class 0x3a of `p` into a buffer of 4096 entries.
fn hlist(p: HANDLE) -> HList {
    let mut l = HList { v: [0; 4096], n: 0, status: 0, ret: SENTINEL_RET };
    // SAFETY: a live buffer of 16384 bytes and a live return length.
    l.status = unsafe { NtQueryInformationProcess(p, CLASS_HANDLE_LIST, l.v.as_mut_ptr().cast(), 16384, &mut l.ret) };
    if l.status == 0 {
        l.n = l.ret as usize / 4;
    }
    l
}

impl HList {
    fn contains(&self, h: HANDLE) -> bool {
        let x = h as usize as u32;
        self.v[..self.n].contains(&x)
    }
    /// Strictly ascending, every value a non-zero multiple of 4.
    fn well_formed(&self) -> bool {
        let s = &self.v[..self.n];
        s.iter().all(|&x| x != 0 && x % 4 == 0) && s.windows(2).all(|w| w[0] < w[1])
    }
}

/// Class 20 (ProcessHandleCount) of `p`: the number of open handles, or `u32::MAX`.
fn handle_count(p: HANDLE) -> u32 {
    let mut n = 0u32;
    let mut r = 0u32;
    // SAFETY: a 4-byte buffer and a live return length.
    let s = unsafe { NtQueryInformationProcess(p, CLASS_HANDLE_COUNT, (&mut n as *mut u32).cast(), 4, &mut r) };
    if s == 0 { n } else { u32::MAX }
}

/// One query with an explicit buffer length: (status, return length (SENTINEL_RET when untouched), bytes of the buffer written).
fn query_len(p: HANDLE, len: u32) -> (i32, u32, usize) {
    let mut buf = [0xAAu8; 16448];
    let mut ret = SENTINEL_RET;
    // SAFETY: a live buffer of at least `len` bytes (len is at most 16384) and a live return length.
    let s = unsafe { NtQueryInformationProcess(p, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), len, &mut ret) };
    let touched = buf.iter().rposition(|&b| b != 0xAA).map_or(0, |i| i + 1);
    (s, ret, touched)
}

/// The type name `NtQueryObject(ObjectTypeInformation)` reports for `h` (printable ASCII), or `<failed 0x...>`.
fn type_name(h: HANDLE) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let mut buf = [0u64; 128];
    let mut ret = 0u32;
    // SAFETY: a live buffer of 1024 bytes and a live return length.
    let s = unsafe { NtQueryObject(h, 2, buf.as_mut_ptr().cast(), 1024, &mut ret) };
    if s != 0 {
        let _ = write!(b, "<failed {:#x}>", s as u32);
        return b;
    }
    // UNICODE_STRING at the start: length (bytes), maximum length, padding, a pointer into this buffer.
    let len = (buf[0] & 0xFFFF) as usize / 2;
    let ptr = buf[1] as usize as *const u16;
    // SAFETY: the system points the string at memory inside the buffer it was given.
    let units = unsafe { core::slice::from_raw_parts(ptr, len.min(60)) };
    let _ = write!(b, "{}", units_text(units).as_str());
    b
}

/// How many handles of `l` answer `NtQueryObject(ObjectTypeInformation)`.
fn typed_count(l: &HList) -> usize {
    l.v[..l.n].iter().filter(|&&x| !type_name(x as usize as HANDLE).as_str().starts_with("<failed")).count()
}

/// Resume the main thread of `c` (a child made with CREATE_SUSPENDED).
fn resume_child(c: &Child) {
    // SAFETY: the thread handle of a process this test started.
    unsafe { ResumeThread(c.thread) };
}

/// A returned length as text: `unchanged` when it is still the sentinel.
fn ret_text(r: u32) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if r == SENTINEL_RET {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{r}");
    }
    b
}

main_with_child!(child, test);

fn child(_: &str) -> u32 {
    0
}

fn test(t: &mut Test) {
    SAY_PART.store(1, core::sync::atomic::Ordering::Relaxed);
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentProcess() };
    for len in [0u32, 1, 2, 3, 4, 5, 6, 7, 8, 9, 11, 12, 13, 16] {
        let (s, ret, touched) = query_len(me, len);
        say!(t, "length {len}: status={s:#x} return length {} bytes written={touched}", ret_text(ret).as_str());
    }

    let n = handle_count(me) as usize;
    let l = hlist(me);
    say!(
        t,
        "length 16384: status={:#x} entries = ProcessHandleCount: {} return length = 4 x entries: {}",
        l.status,
        l.n == n,
        l.ret as usize == 4 * l.n
    );
    say!(t, "entries well formed (non-zero, multiples of 4, strictly ascending): {}", l.well_formed());
    say!(t, "more than 3 entries: {}", l.n > 3);

    // Exact, one entry short, three bytes over, and larger than needed.
    let exact = (4 * l.n) as u32;
    for (what, len) in [
        ("exact", exact),
        ("one entry short", exact - 4),
        ("three bytes over", exact + 3),
        ("one entry over", exact + 4),
        ("0x1068", 0x1068),
        ("0x4000", 0x4000),
    ] {
        let (s, ret, touched) = query_len(me, len);
        let want = len.min(exact) & !3;
        say!(
            t,
            "{what}: status={s:#x} return length = whole entries that fit: {} bytes written = return length: {}",
            ret == want,
            touched == ret as usize
        );
    }

    // The truncated list is a prefix of the full one.
    let mut small = [0u32; 3];
    let mut ret = 0u32;
    // SAFETY: a 12-byte buffer and a live return length.
    let s = unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, small.as_mut_ptr().cast(), 12, &mut ret) };
    say!(
        t,
        "12 bytes: status={s:#x} return length={ret} the three values are the first three of the full list: {}",
        small[..] == l.v[..3]
    );

    // A real handle to this process: the same list, plus the handle itself.
    let mut real: HANDLE = core::ptr::null_mut();
    // SAFETY: live out pointer; the pseudo-handles are valid.
    let ok = unsafe { DuplicateHandle(me, me, me, &mut real, 0, 0, DUPLICATE_SAME_ACCESS) };
    let l2 = hlist(real);
    say!(
        t,
        "real handle (duplicate made: {}): status={:#x} entries = before + 1: {} contains the handle used: {} well formed: {}",
        ok != 0,
        l2.status,
        l2.n == l.n + 1,
        l2.contains(real),
        l2.well_formed()
    );
    say!(t, "every entry of the first list is in the second: {}", l.v[..l.n].iter().all(|x| l2.v[..l2.n].contains(x)));
    say!(t, "ProcessHandleCount through the real handle = entries: {}", handle_count(real) as usize == l2.n);
    // SAFETY: NtClose of a handle this test opened.
    unsafe { NtClose(real) };
    let l3 = hlist(me);
    say!(t, "after closing it: entries back to the first count: {}", l3.n == l.n);

    // Every listed handle is valid: it answers NtQueryObject(ObjectTypeInformation).
    say!(t, "handles of the list that answer NtQueryObject(ObjectTypeInformation): all={}", typed_count(&l3) == l3.n);

    // The returned length is optional; NULL and bad buffers; alignment.
    let mut buf = [0u64; 16];
    // SAFETY: a live buffer of 128 bytes; a NULL return length is the case.
    let s =
        unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 64, core::ptr::null_mut()) };
    say!(t, "NULL return length pointer: status={s:#x}");
    // SAFETY: the buffer is NULL and the length 0: the call must refuse before touching it.
    let s = unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, core::ptr::null_mut(), 0, &mut ret) };
    say!(t, "NULL buffer, length 0: status={s:#x}");
    // SAFETY: a NULL buffer with a length is the case; the call reports it.
    let s = unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, core::ptr::null_mut(), 8, &mut ret) };
    say!(t, "NULL buffer, length 8: status={s:#x}");
    // SAFETY: an address that is not mapped; the call reports an access violation.
    let s = unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, 0x10 as *mut c_void, 64, &mut ret) };
    say!(t, "unmapped buffer: status={s:#x}");
    ret = SENTINEL_RET;
    for off in 1..8usize {
        // SAFETY: the buffer holds 128 bytes; 64 bytes at an offset of up to 7 stay inside it.
        let s = unsafe {
            NtQueryInformationProcess(
                me,
                CLASS_HANDLE_LIST,
                buf.as_mut_ptr().cast::<u8>().add(off).cast(),
                64,
                &mut ret,
            )
        };
        say!(t, "buffer at offset {off} of an 8-byte aligned block: status={s:#x}");
    }
    // SAFETY: a live buffer; the return length pointer is not mapped.
    let s = unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 64, 0x10 as *mut u32) };
    say!(t, "unmapped return length pointer: status={s:#x}");

    // Handles that are not a process.
    // SAFETY: no arguments.
    let th = unsafe { GetCurrentThread() };
    // SAFETY: a live buffer.
    let s = unsafe { NtQueryInformationProcess(th, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 64, &mut ret) };
    say!(t, "the thread pseudo-handle: status={s:#x}");
    // SAFETY: a live buffer.
    let s = unsafe {
        NtQueryInformationProcess(core::ptr::null_mut(), CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 64, &mut ret)
    };
    say!(t, "NULL handle: status={s:#x}");
    // SAFETY: a live buffer; the handle value is not open in this process.
    let s = unsafe {
        NtQueryInformationProcess(0x7654 as HANDLE, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 64, &mut ret)
    };
    say!(t, "a closed handle value: status={s:#x}");
    let ev = opened_event();
    // SAFETY: a live buffer.
    let s = unsafe { NtQueryInformationProcess(ev, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 64, &mut ret) };
    say!(t, "an event handle: status={s:#x}");
    close(ev);

    // Other classes next to it: 0x39 and 0x3b are not this one (their statuses are other classes' business) but class 0x3a with
    // a length below 4 never writes the return length.
    ret = SENTINEL_RET;
    // SAFETY: a live buffer.
    let s = unsafe { NtQueryInformationProcess(me, CLASS_HANDLE_LIST, buf.as_mut_ptr().cast(), 3, &mut ret) };
    say!(t, "length 3 leaves the return length alone: {} (status {s:#x})", ret == SENTINEL_RET);
}

fn opened_event() -> HANDLE {
    // SAFETY: an unnamed auto-reset event.
    unsafe { CreateEventW(core::ptr::null(), 0, 0, core::ptr::null()) }
}
