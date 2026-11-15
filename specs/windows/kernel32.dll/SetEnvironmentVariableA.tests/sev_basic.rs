//! SetEnvironmentVariableA: create, replace, delete, empty values; the order of the block.
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

// Helpers for the ANSI (`...A`) entries: byte strings.

/// A NUL-terminated copy of `s` (up to 399 bytes).
fn cs(s: &[u8]) -> [u8; 400] {
    let mut o = [0u8; 400];
    let n = s.len().min(399);
    o[..n].copy_from_slice(&s[..n]);
    o
}

/// Bytes as text: printable ASCII as it is, anything else as `<xx>` (hexadecimal).
fn bshow(b: &[u8]) -> Buf<900> {
    let mut o = Buf::<900>::new();
    for &c in b {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(o, "{}", c as char);
        } else {
            let _ = write!(o, "<{c:02x}>");
        }
    }
    o
}

/// The bytes of a NUL-terminated string at `p` (at most `max`).
fn bstr_at<'a>(p: *const u8, max: usize) -> &'a [u8] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string; the scan stops at `max`.
    unsafe {
        while n < max && *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// A text for a line: as it is up to 40 bytes, else its first 10 and its length.
fn bshort(b: &[u8]) -> Buf<900> {
    if b.len() <= 40 {
        return bshow(b);
    }
    let mut o = Buf::<900>::new();
    let _ = write!(o, "{}...({} bytes)", bshow(&b[..10]).as_str(), b.len());
    o
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetACP() -> u32;
    fn GetOEMCP() -> u32;
}

// processenv.h: environment variables.
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetEnvironmentVariableA(name: *const u8, buf: *mut u8, size: u32) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn SetEnvironmentVariableA(name: *const u8, value: *const u8) -> BOOL;
    fn SetEnvironmentVariableW(name: *const u16, value: *const u16) -> BOOL;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(p: *mut u16) -> BOOL;
}

/// SetEnvironmentVariableW with a text name and value (`None` deletes).
fn setw(name: &str, value: Option<&str>) -> bool {
    let n = wide(name);
    let v = value.map(wide);
    // SAFETY: NUL-terminated strings.
    unsafe { SetEnvironmentVariableW(n.as_ptr(), v.as_ref().map_or(core::ptr::null(), |v| v.as_ptr())) != 0 }
}

/// SetEnvironmentVariableW with raw UTF-16 units.
fn setw_units(name: &[u16], value: Option<&[u16]>) -> bool {
    let mut n = [0u16; 300];
    n[..name.len()].copy_from_slice(name);
    let mut v = [0u16; 300];
    if let Some(x) = value {
        v[..x.len()].copy_from_slice(x);
    }
    // SAFETY: NUL-terminated strings.
    unsafe { SetEnvironmentVariableW(n.as_ptr(), if value.is_some() { v.as_ptr() } else { core::ptr::null() }) != 0 }
}

/// The value of a variable through the W function, as units in `out`: `None` when it is not there.
fn getw_units(name: &[u16], out: &mut [u16; 400]) -> Option<usize> {
    let mut n = [0u16; 300];
    n[..name.len()].copy_from_slice(name);
    // SAFETY: NUL-terminated name; the buffer holds 400 units.
    let r = unsafe { GetEnvironmentVariableW(n.as_ptr(), out.as_mut_ptr(), 400) } as usize;
    if r == 0 || r >= 400 { None } else { Some(r) }
}

/// The text name's value through the W function, shown; or `<none>`.
fn getw_show(name: &str) -> Buf<600> {
    let n = wide(name);
    let mut out = [0u16; 400];
    // SAFETY: NUL-terminated name; the buffer holds 400 units.
    let r = unsafe { GetEnvironmentVariableW(n.as_ptr(), out.as_mut_ptr(), 400) } as usize;
    let mut b = Buf::<600>::new();
    if r == 0 || r >= 400 {
        let _ = write!(b, "<none>");
    } else {
        let _ = write!(b, "\"{}\"", units_text(&out[..r]).as_str());
    }
    b
}

/// Every `NAME=value` entry of the environment block whose name starts with `prefix` (case-insensitive),
/// in block order, one line each (names and values shortened to 70 units).
fn env_entries(t: &mut Test, tag: &str, prefix: &str) {
    // SAFETY: the block is a list of NUL-terminated strings ended by an empty one; freed below.
    unsafe {
        let blk = GetEnvironmentStringsW();
        let mut p = blk;
        let pw = wide(prefix);
        let pl = wstr_at(pw.as_ptr(), 300);
        let mut k = 0;
        loop {
            let s = wstr_at(p, 40000);
            if s.is_empty() {
                break;
            }
            if s.len() >= pl.len() && eq_nocase(&s[..pl.len()], pl) {
                say!(t, "{tag} entry {k}: \"{}\"", units_text(&s[..s.len().min(70)]).as_str());
                k += 1;
            }
            p = p.add(s.len() + 1);
        }
        say!(t, "{tag} entries with prefix \"{prefix}\": {k}");
        FreeEnvironmentStringsW(blk);
    }
}

/// GetEnvironmentVariableA of `name` into a buffer of `size` bytes pre-filled with 0xAA: one line.
fn gva(t: &mut Test, name: &[u8], size: u32) -> u32 {
    let n = cs(name);
    let mut buf = [0xAAu8; 400];
    mark_error();
    // SAFETY: NUL-terminated name; the buffer holds 400 bytes and `size` is at most that.
    let r = unsafe { GetEnvironmentVariableA(n.as_ptr(), buf.as_mut_ptr(), size) };
    let e = err_text();
    // The bytes written: up to the last one that is not the fill.
    let mut touched = 0;
    for (i, &b) in buf.iter().enumerate() {
        if b != 0xAA {
            touched = i + 1;
        }
    }
    say!(
        t,
        "GetEnvironmentVariableA \"{}\" size {size} -> {r}, error {}, wrote {touched} bytes: \"{}\"",
        bshort(name).as_str(),
        e.as_str(),
        bshort(&buf[..touched.min(60)]).as_str()
    );
    r
}

/// SetEnvironmentVariableA: one line.
fn sva(t: &mut Test, name: &[u8], value: Option<&[u8]>) -> bool {
    let n = cs(name);
    let v = value.map(cs);
    mark_error();
    // SAFETY: NUL-terminated strings.
    let r = unsafe { SetEnvironmentVariableA(n.as_ptr(), v.as_ref().map_or(core::ptr::null(), |v| v.as_ptr())) };
    let e = err_text();
    match value {
        Some(x) => say!(
            t,
            "SetEnvironmentVariableA \"{}\" = \"{}\" -> {r}, error {}",
            bshort(name).as_str(),
            bshort(x).as_str(),
            e.as_str()
        ),
        None => say!(t, "SetEnvironmentVariableA \"{}\" = NULL -> {r}, error {}", bshort(name).as_str(), e.as_str()),
    }
    r != 0
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    sva(t, b"PT_SEV", Some(b"first"));
    say!(t, "read through W: {}", getw_show("PT_SEV").as_str());
    gva(t, b"PT_SEV", 100);
    // Replace, with a name in other letter case.
    sva(t, b"pt_sev", Some(b"second value"));
    say!(t, "read through W: {}", getw_show("PT_SEV").as_str());
    env_entries(t, "after replace", "PT_SEV");
    // Delete, and delete again.
    sva(t, b"PT_SEV", None);
    say!(t, "read through W: {}", getw_show("PT_SEV").as_str());
    sva(t, b"PT_SEV", None);
    sva(t, b"PT_SEV_NEVER", None);
    // An empty value creates the variable (it is found, with 0 characters), and a NULL deletes it.
    sva(t, b"PT_SEV_E", Some(b""));
    gva(t, b"PT_SEV_E", 100);
    say!(t, "read through W: {}", getw_show("PT_SEV_E").as_str());
    env_entries(t, "empty", "PT_SEV_E");
    sva(t, b"PT_SEV_E", None);
    env_entries(t, "deleted", "PT_SEV_E");
    // A value with an equal sign, semicolons, percent signs, spaces at both ends.
    sva(t, b"PT_SEV_V", Some(b"  a=b;c%d%  "));
    say!(t, "read through W: {}", getw_show("PT_SEV_V").as_str());
    // A name with spaces, dots and brackets.
    sva(t, b"PT SEV (1).x", Some(b"v"));
    say!(t, "read through W: {}", getw_show("PT SEV (1).x").as_str());
    // The value is stored as given: no expansion.
    sva(t, b"PT_SEV_X", Some(b"%PT_SEV_V%"));
    say!(t, "read through W: {}", getw_show("PT_SEV_X").as_str());
    // The order of the entries in the block, set in a mixed order and letter case.
    for n in [
        &b"PT_ORD_m"[..],
        b"PT_ORD_Z",
        b"PT_ORD_a",
        b"pt_ord_B",
        b"PT_ORD_9",
        b"PT_ORD_",
        b"PT_ORD_a2",
        b"PT_ORD__",
        b"PT_ORD_~",
    ] {
        sva(t, n, Some(b"1"));
    }
    env_entries(t, "order", "PT_ORD");
    // The same variable set twice in other letter cases: one entry, and its name's case.
    sva(t, b"PT_CASE_Abc", Some(b"1"));
    sva(t, b"pt_case_ABC", Some(b"2"));
    sva(t, b"Pt_Case_aBc", Some(b"3"));
    env_entries(t, "case", "PT_CASE");
    say!(t, "read through W: {}", getw_show("PT_CASE_ABC").as_str());
}
