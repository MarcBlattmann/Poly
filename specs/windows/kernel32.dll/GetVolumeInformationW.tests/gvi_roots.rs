//! GetVolumeInformationW on names that are and are not the root of a volume, asking only for the longest
//! component (as Chrome does) and then for the file system name.
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
        self.line(s);
    }
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

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

// guiddef.h `GUID`.
#[repr(C)]
struct GUID {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}

const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> GUID {
    GUID { a, b, c, d }
}

impl GUID {
    fn is(&self, o: &GUID) -> bool {
        self.a == o.a && self.b == o.b && self.c == o.c && self.d == o.d
    }
}

// fileapi.h, winbase.h, winnt.h (constants), minwinbase.h (WIN32_FIND_STREAM_DATA), handleapi.h.
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const c_void,
        disp: u32,
        flags: u32,
        tmpl: HANDLE,
    ) -> HANDLE;
    fn DeleteFileW(n: *const u16) -> BOOL;
    fn MoveFileExW(a: *const u16, b: *const u16, flags: u32) -> BOOL;
    fn CopyFileW(a: *const u16, b: *const u16, fail_if_exists: BOOL) -> BOOL;
    fn GetFileAttributesW(n: *const u16) -> u32;
    fn SetFileAttributesW(n: *const u16, a: u32) -> BOOL;
    fn CreateDirectoryW(n: *const u16, sa: *const c_void) -> BOOL;
    fn RemoveDirectoryW(n: *const u16) -> BOOL;
    fn GetFileSizeEx(h: HANDLE, size: *mut i64) -> BOOL;
    fn FindFirstStreamW(name: *const u16, level: i32, data: *mut c_void, flags: u32) -> HANDLE;
    fn FindNextStreamW(h: HANDLE, data: *mut c_void) -> BOOL;
    fn FindClose(h: HANDLE) -> BOOL;
}

const GENERIC_READ: u32 = 0x8000_0000;
const GENERIC_WRITE: u32 = 0x4000_0000;
const DELETE: u32 = 0x0001_0000;
const SHARE_ALL: u32 = 7;
const CREATE_NEW: u32 = 1;
const CREATE_ALWAYS: u32 = 2;
const OPEN_EXISTING: u32 = 3;
const OPEN_ALWAYS: u32 = 4;
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;

/// The directory of this test's image, then `name` (a file name, possibly with a `:stream` part), as a UTF-16 string.
fn beside(name: &str) -> [u16; 300] {
    let mut full = [0u16; 300];
    // SAFETY: `full` is writable for 300 units.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), full.as_mut_ptr(), 260) } as usize;
    let mut cut = 0;
    for i in 0..n {
        if full[i] == u16::from(b'\\') {
            cut = i + 1;
        }
    }
    let mut k = cut;
    for c in name.chars() {
        if k < 299 {
            full[k] = c as u32 as u16;
            k += 1;
        }
    }
    while k < 300 {
        full[k] = 0;
        k += 1;
    }
    full
}

/// The length of a NUL-terminated buffer.
fn ulen(p: &[u16]) -> usize {
    p.iter().position(|&c| c == 0).unwrap_or(p.len())
}

fn write_file(p: &[u16], data: &[u8]) -> bool {
    // SAFETY: `p` is NUL-terminated; the handle is closed here.
    unsafe {
        let h = CreateFileW(
            p.as_ptr(),
            GENERIC_WRITE,
            SHARE_ALL,
            core::ptr::null(),
            CREATE_ALWAYS,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut put = 0u32;
        let ok = WriteFile(h, data.as_ptr(), data.len() as u32, &mut put, core::ptr::null_mut());
        CloseHandle(h);
        ok != 0 && put as usize == data.len()
    }
}

/// The content of a file (at most 64 bytes), or None when it cannot be opened.
fn read_file(p: &[u16]) -> Option<([u8; 64], usize)> {
    let mut buf = [0u8; 64];
    // SAFETY: `p` is NUL-terminated; the handle is closed here.
    unsafe {
        let h = CreateFileW(
            p.as_ptr(),
            GENERIC_READ,
            SHARE_ALL,
            core::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            core::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut got = 0u32;
        ReadFile(h, buf.as_mut_ptr(), 64, &mut got, core::ptr::null_mut());
        CloseHandle(h);
        Some((buf, got as usize))
    }
}

fn exists(p: &[u16]) -> bool {
    // SAFETY: `p` is NUL-terminated.
    unsafe { GetFileAttributesW(p.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

fn size_of_file(p: &[u16]) -> i64 {
    // SAFETY: `p` is NUL-terminated; the handle is closed here.
    unsafe {
        let h =
            CreateFileW(p.as_ptr(), 0, SHARE_ALL, core::ptr::null(), OPEN_EXISTING, 0x0200_0000, core::ptr::null_mut());
        if h == INVALID_HANDLE_VALUE {
            return -1;
        }
        let mut s = -1i64;
        GetFileSizeEx(h, &mut s);
        CloseHandle(h);
        s
    }
}

fn rm(p: &[u16]) {
    // SAFETY: `p` is NUL-terminated.
    unsafe {
        if DeleteFileW(p.as_ptr()) == 0 {
            RemoveDirectoryW(p.as_ptr());
        }
    }
}

/// What a file looks like: absent, or its size and what its first bytes say (a text of up to 12 characters).
fn describe(p: &[u16]) -> Buf<80> {
    let mut b = Buf::<80>::new();
    if !exists(p) {
        let _ = write!(b, "absent");
        return b;
    }
    let _ = write!(b, "present size={}", size_of_file(p));
    if let Some((c, n)) = read_file(p) {
        let _ = write!(b, " text=\"");
        for &x in &c[..n.min(12)] {
            if (0x20..0x7f).contains(&x) {
                let _ = write!(b, "{}", x as char);
            } else {
                let _ = write!(b, "<{x:02x}>");
            }
        }
        let _ = write!(b, "\"");
    }
    b
}

/// Every stream name of a file (`FindFirstStreamW`), as `name:size` separated by spaces.
fn streams_of(p: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    let mut data = [0u8; 600];
    // SAFETY: WIN32_FIND_STREAM_DATA is 8 + 296 * 2 bytes; the handle is closed here.
    unsafe {
        let h = FindFirstStreamW(p.as_ptr(), 0, data.as_mut_ptr().cast(), 0);
        if h == INVALID_HANDLE_VALUE {
            let _ = write!(b, "none (error {})", last_error());
            return b;
        }
        loop {
            let size = i64::from_le_bytes(data[..8].try_into().unwrap_or([0; 8]));
            let name: &[u16] = core::slice::from_raw_parts(data.as_ptr().add(8).cast::<u16>(), 296);
            let n = ulen(name);
            let _ = write!(b, "{}:{} ", units_text(&name[..n]), size);
            if FindNextStreamW(h, data.as_mut_ptr().cast()) == 0 {
                let _ = write!(b, "(end {})", last_error());
                break;
            }
        }
        FindClose(h);
    }
    b
}

#[repr(C)]
struct FindData {
    attrs: u32,
    times: [u32; 6],
    size_hi: u32,
    size_lo: u32,
    reserved: [u32; 2],
    name: [u16; 260],
    alt: [u16; 14],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn FindFirstFileW(name: *const u16, data: *mut FindData) -> HANDLE;
    fn FindNextFileW(h: HANDLE, data: *mut FindData) -> BOOL;
}

/// The names `<prefix>*` a listing of the image's folder gives (`name:size`, folders marked with a slash).
fn list_names(prefix: &str) -> Buf<400> {
    let mut out = Buf::<400>::new();
    let mut pat = [0u16; 300];
    {
        let mut s = beside(prefix);
        let n = ulen(&s);
        s[n] = u16::from(b'*');
        pat = s;
    }
    // SAFETY: a live FindData; the handle is closed here.
    unsafe {
        let mut d: FindData = core::mem::zeroed();
        let h = FindFirstFileW(pat.as_ptr(), &mut d);
        if h == INVALID_HANDLE_VALUE {
            let _ = write!(out, "none");
            return out;
        }
        loop {
            let n = ulen(&d.name);
            if d.attrs & 0x10 != 0 {
                let _ = write!(out, "{}/ ", units_text(&d.name[..n]));
            } else {
                let _ = write!(out, "{}:{} ", units_text(&d.name[..n]), d.size_lo);
            }
            if FindNextFileW(h, &mut d) == 0 {
                break;
            }
        }
        FindClose(h);
    }
    out
}

poly_wtest::main!(test);

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetVolumeInformationW(
        root: *const u16,
        name: *mut u16,
        name_len: u32,
        serial: *mut u32,
        max_component: *mut u32,
        flags: *mut u32,
        fs: *mut u16,
        fs_len: u32,
    ) -> BOOL;
}

fn probe(t: &mut Test, what: &str, root: *const u16) {
    let mut max = 0xAAAA_AAAAu32;
    let mut fs = [0xAAAAu16; 40];
    mark_error();
    // SAFETY: `root` is NULL or NUL-terminated; buffers as long as said.
    let ok = unsafe {
        GetVolumeInformationW(
            root,
            core::ptr::null_mut(),
            0,
            core::ptr::null_mut(),
            &mut max,
            core::ptr::null_mut(),
            fs.as_mut_ptr(),
            40,
        )
    };
    let e = err_text();
    if max == 0xAAAA_AAAA {
        say!(t, "{what}: {ok} error {e}; max component untouched; file system name untouched {}", fs[0] == 0xAAAA);
    } else {
        say!(t, "{what}: {ok} error {e}; max component {max}; file system \"{}\"", units_text(&fs[..ulen(&fs)]));
    }
}

fn test(t: &mut Test) {
    let mut root = beside("");
    root[3] = 0;
    probe(t, "the drive root of the image", root.as_ptr());
    let mut low = root;
    low[0] |= 0x20;
    probe(t, "the same with a lower case letter", low.as_ptr());
    let mut nobs = root;
    nobs[2] = 0;
    probe(t, "the drive without the backslash", nobs.as_ptr());
    probe(t, "NULL", core::ptr::null());
    probe(t, "a backslash", wide("\\").as_ptr());
    probe(t, "an empty string", wide("").as_ptr());
    probe(t, "a drive that is not there", wide("Z:\\").as_ptr());
    probe(t, "a folder", beside("").as_ptr());
    probe(t, "a relative name", wide("x").as_ptr());
    probe(t, "a file name", wide("C:\\x.txt").as_ptr());
    let mut ext = [0u16; 300];
    for (i, c) in "\\\\?\\".chars().enumerate() {
        ext[i] = c as u16;
    }
    let l = ulen(&root);
    ext[4..4 + l].copy_from_slice(&root[..l]);
    probe(t, "the root with the \\\\?\\ prefix", ext.as_ptr());
    let mut dev = [0u16; 300];
    for (i, c) in "\\\\.\\".chars().enumerate() {
        dev[i] = c as u16;
    }
    dev[4..4 + l].copy_from_slice(&root[..l]);
    probe(t, "the root with the \\\\.\\ prefix", dev.as_ptr());
    probe(t, "a UNC name of a server that is not there", wide("\\\\polyserver\\share\\").as_ptr());
}
