//! PathIsUNCServerShareW on relative paths, dots and slashes.
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

// shlwapi.h: path helpers. Test strings are written with `|` for a backslash (`bsw`).
#[link(name = "shlwapi", kind = "raw-dylib")]
unsafe extern "system" {
    fn PathRemoveFileSpecW(path: *mut u16) -> BOOL;
    fn PathAppendW(path: *mut u16, more: *const u16) -> BOOL;
    fn PathCombineW(dst: *mut u16, dir: *const u16, file: *const u16) -> *mut u16;
    fn PathCanonicalizeW(dst: *mut u16, src: *const u16) -> BOOL;
    fn PathIsRelativeW(path: *const u16) -> BOOL;
    fn PathIsRootW(path: *const u16) -> BOOL;
    fn PathIsUNCW(path: *const u16) -> BOOL;
    fn PathIsUNCServerW(path: *const u16) -> BOOL;
    fn PathIsUNCServerShareW(path: *const u16) -> BOOL;
    fn PathIsNetworkPathW(path: *const u16) -> BOOL;
    fn PathIsFileSpecW(path: *const u16) -> BOOL;
    fn PathIsDirectoryW(path: *const u16) -> BOOL;
    fn PathFileExistsW(path: *const u16) -> BOOL;
    fn PathStripPathW(path: *mut u16);
    fn PathStripToRootW(path: *mut u16) -> BOOL;
    fn PathQuoteSpacesW(path: *mut u16);
    fn PathUnquoteSpacesW(path: *mut u16);
    fn PathAddBackslashW(path: *mut u16) -> *mut u16;
    fn PathRemoveBackslashW(path: *mut u16) -> *mut u16;
    fn PathRemoveBlanksW(path: *mut u16);
    fn PathRemoveArgsW(path: *mut u16);
    fn PathGetArgsW(path: *const u16) -> *mut u16;
    fn PathSkipRootW(path: *const u16) -> *mut u16;
    fn PathGetDriveNumberW(path: *const u16) -> i32;
    fn PathCommonPrefixW(a: *const u16, b: *const u16, common: *mut u16) -> i32;
    fn PathRenameExtensionW(path: *mut u16, ext: *const u16) -> BOOL;
    fn PathAddExtensionW(path: *mut u16, ext: *const u16) -> BOOL;
    fn PathIsPrefixW(prefix: *const u16, path: *const u16) -> BOOL;
    fn PathIsSameRootW(a: *const u16, b: *const u16) -> BOOL;
    fn PathBuildRootW(dst: *mut u16, drive: i32) -> *mut u16;
}

/// A test string as a NUL-terminated buffer of 400 units: `|` stands for a backslash.
fn bsw(s: &str) -> [u16; 400] {
    let mut w = [0u16; 400];
    for (i, c) in s.chars().enumerate().take(399) {
        w[i] = if c == '|' { 0x5c } else { c as u32 as u16 };
    }
    w
}

/// A string for a line: as it is up to 70 units, else its first 12 and its length.
fn shown(u: &[u16]) -> Buf<600> {
    if u.len() <= 70 {
        return units_text(u);
    }
    let mut b = Buf::<600>::new();
    let _ = write!(b, "{}...({} units)", units_text(&u[..12]).as_str(), u.len());
    b
}

fn upto_nul(w: &[u16]) -> &[u16] {
    let n = w.iter().position(|&c| c == 0).unwrap_or(w.len());
    &w[..n]
}

/// The buffer's text, or `<n>` when the pointer returned is where a string would start: offset form.
fn off(base: *const u16, p: *const u16) -> Buf<30> {
    let mut b = Buf::<30>::new();
    if p.is_null() {
        let _ = write!(b, "NULL");
    } else {
        let d = (p as usize).wrapping_sub(base as usize) / 2;
        let _ = write!(b, "+{d}");
    }
    b
}

/// A function that works on a string in place: one line per input, `"in" -> "out" ret`.
fn inplace(t: &mut Test, name: &str, inputs: &[&str], f: impl Fn(*mut u16) -> i64) {
    for s in inputs {
        let mut w = bsw(s);
        let shown_in = shown(upto_nul(&bsw(s)));
        mark_error();
        let r = f(w.as_mut_ptr());
        let e = last_error();
        let tail = if e == MARK {
            Buf::<30>::new()
        } else {
            let mut b = Buf::<30>::new();
            let _ = write!(b, " (last error {e})");
            b
        };
        say!(t, "{name} \"{}\" -> \"{}\" ret {r}{}", shown_in.as_str(), shown(upto_nul(&w)).as_str(), tail.as_str());
    }
}

/// A function that returns a pointer into its argument or the argument itself: the offset, and the string it points at.
fn inplace_ptr(t: &mut Test, name: &str, inputs: &[&str], f: impl Fn(*mut u16) -> *mut u16) {
    for s in inputs {
        let mut w = bsw(s);
        let shown_in = shown(upto_nul(&bsw(s)));
        mark_error();
        let p = f(w.as_mut_ptr());
        let e = last_error();
        let tail = if e == MARK {
            Buf::<30>::new()
        } else {
            let mut b = Buf::<30>::new();
            let _ = write!(b, " (last error {e})");
            b
        };
        let at = if p.is_null() { Buf::<600>::new() } else { shown(wstr_at(p, 399)) };
        let mut o = Buf::<600>::new();
        let _ = write!(o, "{} \"{}\"", off(w.as_ptr(), p).as_str(), at.as_str());
        say!(
            t,
            "{name} \"{}\" -> {} string now \"{}\"{}",
            shown_in.as_str(),
            o.as_str(),
            shown(upto_nul(&w)).as_str(),
            tail.as_str()
        );
    }
}

/// A predicate or other function of one read-only string: `"in" -> ret`.
fn pred(t: &mut Test, name: &str, inputs: &[&str], f: impl Fn(*const u16) -> i64) {
    for s in inputs {
        let w = bsw(s);
        mark_error();
        let r = f(w.as_ptr());
        let e = last_error();
        if e == MARK {
            say!(t, "{name} \"{}\" -> {r}", shown(upto_nul(&w)).as_str());
        } else {
            say!(t, "{name} \"{}\" -> {r} (last error {e})", shown(upto_nul(&w)).as_str());
        }
    }
}

/// A function of two strings and a destination buffer: `"a" , "b" -> "dst" ret`. `dst` starts with 0xAAAA.
fn dst2(t: &mut Test, name: &str, pairs: &[(&str, &str)], f: impl Fn(*mut u16, *const u16, *const u16) -> *mut u16) {
    for (a, b) in pairs {
        let wa = bsw(a);
        let wb = bsw(b);
        let mut d = [0xAAAAu16; 400];
        mark_error();
        let p = f(d.as_mut_ptr(), wa.as_ptr(), wb.as_ptr());
        let e = last_error();
        let tail = if e == MARK {
            Buf::<30>::new()
        } else {
            let mut b = Buf::<30>::new();
            let _ = write!(b, " (last error {e})");
            b
        };
        let n = d.iter().position(|&c| c == 0xAAAA).unwrap_or(400);
        let res = if p.is_null() { "NULL" } else { "dst" };
        say!(
            t,
            "{name} \"{}\" , \"{}\" -> {res}, dst \"{}\"{}",
            shown(upto_nul(&wa)).as_str(),
            shown(upto_nul(&wb)).as_str(),
            shown(upto_nul(&d[..n])).as_str(),
            tail.as_str()
        );
    }
}

/// A destination-buffer function of one string (PathCanonicalizeW): `"src" -> "dst" ret`.
fn dst1(t: &mut Test, name: &str, inputs: &[&str], f: impl Fn(*mut u16, *const u16) -> i64) {
    for a in inputs {
        let wa = bsw(a);
        let mut d = [0xAAAAu16; 400];
        mark_error();
        let r = f(d.as_mut_ptr(), wa.as_ptr());
        let e = last_error();
        let tail = if e == MARK {
            Buf::<30>::new()
        } else {
            let mut b = Buf::<30>::new();
            let _ = write!(b, " (last error {e})");
            b
        };
        let n = d.iter().position(|&c| c == 0xAAAA).unwrap_or(400);
        say!(
            t,
            "{name} \"{}\" -> {r}, dst \"{}\"{}",
            shown(upto_nul(&wa)).as_str(),
            shown(upto_nul(&d[..n])).as_str(),
            tail.as_str()
        );
    }
}

/// An in-place function with a second read-only string (PathAppendW, PathRenameExtensionW ...): `"a" + "b" -> "a'" ret`.
fn inplace2(t: &mut Test, name: &str, pairs: &[(&str, &str)], f: impl Fn(*mut u16, *const u16) -> i64) {
    for (a, b) in pairs {
        let mut wa = bsw(a);
        let wb = bsw(b);
        mark_error();
        let r = f(wa.as_mut_ptr(), wb.as_ptr());
        let e = last_error();
        let tail = if e == MARK {
            Buf::<30>::new()
        } else {
            let mut b = Buf::<30>::new();
            let _ = write!(b, " (last error {e})");
            b
        };
        say!(
            t,
            "{name} \"{}\" + \"{}\" -> \"{}\" ret {r}{}",
            shown(upto_nul(&bsw(a))).as_str(),
            shown(upto_nul(&wb)).as_str(),
            shown(upto_nul(&wa)).as_str(),
            tail.as_str()
        );
    }
}

/// A string of `n` copies of `c` as text, up to 400.
fn rep(c: char, n: usize) -> Buf<400> {
    let mut b = Buf::<400>::new();
    for _ in 0..n.min(399) {
        let _ = write!(b, "{c}");
    }
    b
}

// Input sets (`|` is a backslash).
const REL: &[&str] = &[
    "",
    "a",
    "a|",
    "a|b",
    "a|b|",
    "a|b|c.txt",
    "a|b|c.txt|",
    "a||b",
    "a|||b|",
    "|",
    "||",
    "|a",
    "|a|b",
    "|a|b|",
    "|a||b",
    ".",
    "..",
    "a|.",
    "a|..",
    "a|.|b",
    "a|..|b",
    ".|a",
    "..|a",
    "a b",
    "a b|c d",
    " a",
    "a ",
    " ",
    "a| |b",
    "a.b",
    "a.b|c.d",
    ".txt",
    "a|.txt",
    "a|b.",
    "a|b..",
    "a/b",
    "a/b/c",
    "a/b|c",
    "a|b/c",
];
const DRIVE: &[&str] = &[
    "C:",
    "C:|",
    "C:||",
    "C:|a",
    "C:|a|",
    "C:|a|b",
    "C:|a|b|",
    "C:|a|b.txt",
    "C:|a||b",
    "C:||a",
    "C:a",
    "C:a|b",
    "C:a|",
    "c:|a",
    "c:|A|B",
    "1:|a",
    "::|a",
    "C:/",
    "C:/a",
    "C:/a/b",
    "C:/a|b",
    "C:|a/b",
    "CC:|a",
    "C::|a",
    "C:|.",
    "C:|..",
    "C:|a|.",
    "C:|a|..",
    "C:|a|..|b",
    "C:|a|.|b",
    "C:|a|b|..",
    "C:|..|a",
    "C:|a b|c d",
    "C:|a|b ",
    "C:|a|b.",
    "C:|a|b..",
];
const UNC: &[&str] = &[
    "||",
    "|||",
    "||s",
    "||s|",
    "||s||",
    "||s|sh",
    "||s|sh|",
    "||s|sh||",
    "||s|sh|f",
    "||s|sh|f|",
    "||s|sh|f|g.txt",
    "||s|sh|f|g|",
    "||s|sh|f||g",
    "|||s|sh",
    "||s||sh",
    "||s|sh|.",
    "||s|sh|..",
    "||s|.|f",
    "||s|..|f",
    "||.",
    "||.|",
    "||.|C:",
    "||.|C:|a",
    "||.|pipe|x",
    "||?",
    "||?|",
    "//s/sh/f",
    "/|s|sh",
    "||server|share|dir|file.txt",
    "||server|share|dir|",
    "||server|share",
    "||server|share|",
    "||server",
    "||ab",
    "||abc",
    "||abcd",
    "||abcdefgh",
    "||a|b",
    "||ab|cd",
    "||abc|def|g",
    "|||a",
];
const PREFIX: &[&str] = &[
    "||?|C:",
    "||?|C:|",
    "||?|C:|a",
    "||?|C:|a|",
    "||?|C:|a|b",
    "||?|C:|a|b.txt",
    "||?|c:|a",
    "||?|C:||a",
    "||?|",
    "||?|a",
    "||?|a|b",
    "||?|UNC",
    "||?|UNC|",
    "||?|UNC|s",
    "||?|UNC|s|",
    "||?|UNC|s|sh",
    "||?|UNC|s|sh|",
    "||?|UNC|s|sh|f",
    "||?|UNC|s|sh|f|g",
    "||?|unc|s|sh|f",
    "||?|Volume{12345678-1234-1234-1234-123456789abc}|",
    "||?|Volume{12345678-1234-1234-1234-123456789abc}",
    "||?|Volume{12345678-1234-1234-1234-123456789abc}|a",
    "||?|Volume{12345678-1234-1234-1234-123456789abc}|a|b",
    "||?|C:|a|.|b",
    "||?|C:|a|..|b",
    "||?|C:|a/b",
    "||?|pipe|x",
    "||.|C:",
    "||.|C:|",
    "||.|C:|a|b",
    "|??|C:|a|b",
    "|??|",
    "|??|UNC|s|sh|f",
];

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn PathCchSkipRoot(path: *const u16, root_end: *mut *const u16) -> i32;
    fn PathCchCombineEx(out: *mut u16, cch: usize, a: *const u16, b: *const u16, flags: u32) -> i32;
    fn PathCchCanonicalizeEx(out: *mut u16, cch: usize, input: *const u16, flags: u32) -> i32;
}

/// PathCchSkipRoot over a set: `"in" -> hr, rest "..."` (the part after the root).
fn skip_root(t: &mut Test, inputs: &[&str]) {
    for s in inputs {
        let w = bsw(s);
        let mut end: *const u16 = core::ptr::null();
        // SAFETY: NUL-terminated input and a live out pointer.
        let hr = unsafe { PathCchSkipRoot(w.as_ptr(), &mut end) };
        if hr == 0 {
            say!(
                t,
                "PathCchSkipRoot \"{}\" -> {hr:#x}, rest at {} \"{}\"",
                shown(upto_nul(&w)).as_str(),
                off(w.as_ptr(), end).as_str(),
                shown(wstr_at(end, 399)).as_str()
            );
        } else {
            say!(t, "PathCchSkipRoot \"{}\" -> {hr:#x}", shown(upto_nul(&w)).as_str());
        }
    }
}

/// PathCchCanonicalizeEx over a set with `flags`, destination of `cch` units.
fn pcce_canon(t: &mut Test, inputs: &[&str], cch: usize, flags: u32) {
    for s in inputs {
        let w = bsw(s);
        let mut d = [0xAAAAu16; 400];
        // SAFETY: NUL-terminated input; the destination holds 400 units and `cch` is at most that.
        let hr = unsafe { PathCchCanonicalizeEx(d.as_mut_ptr(), cch, w.as_ptr(), flags) };
        let n = d.iter().position(|&c| c == 0xAAAA).unwrap_or(400);
        say!(
            t,
            "PathCchCanonicalizeEx cch {cch} flags {flags:#x} \"{}\" -> {hr:#x}, \"{}\"",
            shown(upto_nul(&w)).as_str(),
            shown(upto_nul(&d[..n])).as_str()
        );
    }
}

/// PathCchCombineEx over pairs with `flags`.
fn pcce_combine(t: &mut Test, pairs: &[(&str, &str)], cch: usize, flags: u32) {
    for (a, b) in pairs {
        let wa = bsw(a);
        let wb = bsw(b);
        let mut d = [0xAAAAu16; 400];
        // SAFETY: NUL-terminated inputs; the destination holds 400 units and `cch` is at most that.
        let hr = unsafe { PathCchCombineEx(d.as_mut_ptr(), cch, wa.as_ptr(), wb.as_ptr(), flags) };
        let n = d.iter().position(|&c| c == 0xAAAA).unwrap_or(400);
        say!(
            t,
            "PathCchCombineEx cch {cch} flags {flags:#x} \"{}\" , \"{}\" -> {hr:#x}, \"{}\"",
            shown(upto_nul(&wa)).as_str(),
            shown(upto_nul(&wb)).as_str(),
            shown(upto_nul(&d[..n])).as_str()
        );
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    pred(t, "PathIsUNCServerShareW", REL, |p| unsafe { PathIsUNCServerShareW(p) as i64 });
}
