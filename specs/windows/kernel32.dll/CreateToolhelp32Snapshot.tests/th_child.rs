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

// ---- child process helpers: the test starts itself as `--child <arg>` (kernel32 prototypes from Learn) ----
#[repr(C)]
struct StartupInfoW {
    cb: u32,
    pad0: u32,
    rest: [usize; 12],
}

#[repr(C)]
struct ProcInfo {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleFileNameW(module: HANDLE, buf: *mut u16, size: u32) -> u32;
    fn GetCommandLineW() -> *const u16;
    fn CreateProcessW(
        app: *const u16,
        cmd: *mut u16,
        pa: *const c_void,
        ta: *const c_void,
        inherit: BOOL,
        flags: u32,
        env: *const c_void,
        dir: *const u16,
        si: *mut StartupInfoW,
        pi: *mut ProcInfo,
    ) -> BOOL;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn Sleep(ms: u32);
}

/// The text after ` --child ` on this process's command line (up to 40 ASCII characters).
fn child_arg() -> Option<Buf<40>> {
    // SAFETY: the command line is a NUL-terminated string owned by the process.
    let cl = wstr_at(unsafe { GetCommandLineW() }, 600);
    let key = b" --child ";
    let mut i = 0;
    while i + key.len() <= cl.len() {
        if key.iter().enumerate().all(|(k, &c)| cl[i + k] == u16::from(c)) {
            let mut b = Buf::<40>::new();
            for &c in &cl[i + key.len()..] {
                let _ = write!(b, "{}", (c as u8) as char);
            }
            return Some(b);
        }
        i += 1;
    }
    None
}

/// This image's path (UTF-16, NUL-terminated) in a buffer; the length without the NUL.
fn self_path() -> ([u16; 300], usize) {
    let mut p = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 299) } as usize;
    (p, n)
}

/// The command line `"<this image>" --child <arg>` as a NUL-terminated buffer.
fn child_cmdline(arg: &str) -> [u16; 600] {
    let (p, n) = self_path();
    let mut c = [0u16; 600];
    let mut k = 0;
    c[k] = b'"' as u16;
    k += 1;
    for &u in &p[..n] {
        c[k] = u;
        k += 1;
    }
    c[k] = b'"' as u16;
    k += 1;
    for b in " --child ".bytes().chain(arg.bytes()) {
        c[k] = u16::from(b);
        k += 1;
    }
    c
}

/// Starts this image as `--child <arg>` and returns at once (None when CreateProcessW failed).
fn spawn_self_child(arg: &str) -> Option<ProcInfo> {
    let (path, _) = self_path();
    let mut cmd = child_cmdline(arg);
    let mut si = StartupInfoW { cb: 104, pad0: 0, rest: [0; 12] };
    let mut pi = ProcInfo { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid, NUL-terminated buffers; zeroed STARTUPINFOW of 104 bytes.
    let ok = unsafe {
        CreateProcessW(path.as_ptr(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, 0, core::ptr::null(), core::ptr::null(), &mut si, &mut pi)
    };
    if ok == 0 { None } else { Some(pi) }
}

/// Ends a child started by `spawn_self_child` and closes its handles.
fn end_child(pi: &ProcInfo) {
    // SAFETY: handles from CreateProcessW.
    unsafe {
        TerminateProcess(pi.process, 0xDEAD);
        WaitForSingleObject(pi.process, 5000);
        CloseHandle(pi.process);
        CloseHandle(pi.thread);
    }
}

/// The child half of a test: `--child <arg>` runs `$child(arg) -> exit code` and never opens the observation file.
macro_rules! main_with_child {
    ($child:path, $test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_arg() {
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

/// The standard child: mode "sleep" waits 10 s (the parent ends it earlier).
fn sleep_child(arg: &str) -> u32 {
    if arg == "sleep" {
        // SAFETY: plain call.
        unsafe { Sleep(10000) };
    }
    0
}

// ---- Toolhelp helpers (tlhelp32.h: PROCESSENTRY32W, TH32CS_*; prototypes from Learn) ----
#[repr(C)]
#[derive(Clone, Copy)]
struct ProcessEntry32W {
    size: u32,
    usage: u32,
    pid: u32,
    heap_id: usize,
    module_id: u32,
    threads: u32,
    parent: u32,
    pri: i32,
    flags: u32,
    exe: [u16; 260],
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Process32FirstW(snap: HANDLE, entry: *mut ProcessEntry32W) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn Process32NextW(snap: HANDLE, entry: *mut ProcessEntry32W) -> BOOL;
}

const TH32CS_SNAPPROCESS: u32 = 2;
const TH32CS_SNAPTHREAD: u32 = 4;
const TH32CS_SNAPMODULE: u32 = 8;

fn new_entry(size: u32) -> ProcessEntry32W {
    ProcessEntry32W { size, usage: 0, pid: 0, heap_id: 0, module_id: 0, threads: 0, parent: 0, pri: 0, flags: 0, exe: [0; 260] }
}

#[derive(Clone, Copy)]
struct Row {
    pid: u32,
    parent: u32,
    threads: u32,
    name: [u16; 64],
}

struct Snap {
    rows: [Row; 640],
    n: usize,
}

/// Walks a snapshot with Process32FirstW/NextW (entry size 568).
fn walk(snap: HANDLE) -> Snap {
    let mut s = Snap { rows: [Row { pid: 0, parent: 0, threads: 0, name: [0; 64] }; 640], n: 0 };
    let mut e = new_entry(core::mem::size_of::<ProcessEntry32W>() as u32);
    // SAFETY: valid snapshot handle and entry of the documented size.
    let mut ok = unsafe { Process32FirstW(snap, &mut e) };
    while ok != 0 && s.n < 640 {
        let nm = wstr_at(e.exe.as_ptr(), 260);
        let mut name = [0u16; 64];
        let k = if nm.len() > 63 { 63 } else { nm.len() };
        name[..k].copy_from_slice(&nm[..k]);
        s.rows[s.n] = Row { pid: e.pid, parent: e.parent, threads: e.threads, name };
        s.n += 1;
        // SAFETY: as above.
        ok = unsafe { Process32NextW(snap, &mut e) };
    }
    s
}

fn find(s: &Snap, pid: u32) -> Option<Row> {
    s.rows[..s.n].iter().copied().find(|r| r.pid == pid)
}

fn name_is(r: &Row, text: &str) -> bool {
    let w = wide(text);
    let n = wstr_at(w.as_ptr(), 300).len();
    let m = wstr_at(r.name.as_ptr(), 64).len();
    eq_nocase(&r.name[..m], &w[..n])
}

/// The file name part of this image's path, as UTF-16.
fn image_file_name() -> ([u16; 64], usize) {
    let img = poly_wtest::image_path();
    let mut last = 0;
    for (i, &c) in img.iter().enumerate() {
        if c == 0x5c {
            last = i + 1;
        }
    }
    let mut o = [0u16; 64];
    let tail = &img[last..];
    let k = if tail.len() > 63 { 63 } else { tail.len() };
    o[..k].copy_from_slice(&tail[..k]);
    (o, k)
}

fn name_is_image(r: &Row) -> bool {
    let (n, k) = image_file_name();
    let m = wstr_at(r.name.as_ptr(), 64).len();
    eq_nocase(&r.name[..m], &n[..k])
}
// Toolhelp process snapshots follow a child process: a snapshot is a point in time; the child has our pid as its parent.
main_with_child!(sleep_child, test);

fn snap_walk() -> Snap {
    // SAFETY: plain call.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let s = walk(snap);
    // SAFETY: handle from the call above.
    unsafe { CloseHandle(snap) };
    s
}

fn test(t: &mut Test) {
    // SAFETY: plain call.
    let me = unsafe { GetCurrentProcessId() };
    // SAFETY: plain call.
    let before = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    let Some(pi) = spawn_self_child("sleep") else {
        say!(t, "could not start the child: {}", gle());
        return;
    };
    let after = snap_walk();
    let r = find(&after, pi.pid);
    say!(t, "a snapshot taken after the start lists the child: {}", r.is_some());
    if let Some(r) = r {
        say!(t, "its parent is us: {}", r.parent == me);
        say!(t, "its exe name is this image's file name: {}", name_is_image(&r));
        say!(t, "its thread count is at least 1: {}", r.threads >= 1);
    }
    let old = walk(before);
    say!(t, "the snapshot taken before the start does not list it: {}", find(&old, pi.pid).is_none());
    // SAFETY: handle from the call above.
    unsafe { CloseHandle(before) };

    // end the child; we still hold its process handle
    // SAFETY: handles from CreateProcessW.
    unsafe {
        TerminateProcess(pi.process, 0xDEAD);
        WaitForSingleObject(pi.process, 5000);
    }
    let gone = snap_walk();
    let z = find(&gone, pi.pid);
    say!(t, "after it ended, with our process handle still open, it is listed: {}", z.is_some());
    if let Some(z) = z {
        say!(t, "  its thread count is then {}", z.threads);
    }
    // SAFETY: handles from CreateProcessW.
    unsafe {
        CloseHandle(pi.process);
        CloseHandle(pi.thread);
    }
    // the system removes the process object a moment after the last handle closed
    let mut listed = true;
    for _ in 0..50 {
        let s = snap_walk();
        listed = find(&s, pi.pid).is_some();
        if !listed {
            break;
        }
        // SAFETY: plain call.
        unsafe { Sleep(100) };
    }
    say!(t, "after the last handle was closed it is no longer listed (within 5 s): {}", !listed);
}
