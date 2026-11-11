//! GetProcessInformation as Chrome calls it: ProcessPowerThrottling with a 12-byte buffer, on
//! the own process and on a child; what a fresh process answers, that the caller has to fill
//! in the version, other buffer sizes, and the access the handle needs.
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

// processthreadsapi.h: BOOL GetProcessInformation(HANDLE hProcess, PROCESS_INFORMATION_CLASS ProcessInformationClass,
// LPVOID ProcessInformation, DWORD ProcessInformationSize); BOOL SetProcessInformation(...) with the same arguments;
// PROCESS_INFORMATION_CLASS, PROCESS_POWER_THROTTLING_STATE, MEMORY_PRIORITY_INFORMATION,
// PROCESS_PROTECTION_LEVEL_INFORMATION, PROCESS_LEAP_SECOND_INFO, PROCESS_MACHINE_INFORMATION, APP_MEMORY_INFORMATION.
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessInformation(process: HANDLE, class: u32, info: *mut c_void, size: u32) -> BOOL;
    fn SetProcessInformation(process: HANDLE, class: u32, info: *const c_void, size: u32) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
}

const ProcessMemoryPriority: u32 = 0;
const ProcessMemoryExhaustionInfo: u32 = 1;
const ProcessAppMemoryInfo: u32 = 2;
const ProcessInPrivateInfo: u32 = 3;
const ProcessPowerThrottling: u32 = 4;
const ProcessReservedValue1: u32 = 5;
const ProcessTelemetryCoverageInfo: u32 = 6;
const ProcessProtectionLevelInfo: u32 = 7;
const ProcessLeapSecondInfo: u32 = 8;
const ProcessMachineTypeInfo: u32 = 9;
const PROCESS_POWER_THROTTLING_EXECUTION_SPEED: u32 = 1;
const PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION: u32 = 4;
const PROCESS_SET_INFORMATION: u32 = 0x0200;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

/// 64 bytes of scratch, 8-byte aligned, filled with 0xAA before every call.
#[repr(C, align(8))]
struct Info {
    b: [u8; 64],
}

impl Info {
    fn new() -> Info {
        Info { b: [0xAA; 64] }
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn put_u32(&mut self, off: usize, v: u32) {
        self.b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    /// One past the last byte that is not 0xAA any more.
    fn written(&self) -> usize {
        self.b.iter().rposition(|&x| x != 0xAA).map_or(0, |i| i + 1)
    }
}

/// GetProcessInformation into fresh scratch: (result, last error text, the scratch). For
/// ProcessPowerThrottling the caller's Version field (the first four bytes) is set to `version` first.
fn get_info(h: HANDLE, class: u32, size: u32, version: Option<u32>) -> (BOOL, Buf<24>, Info) {
    let mut i = Info::new();
    if let Some(v) = version {
        i.put_u32(0, v);
    }
    mark_error();
    // SAFETY: 64 bytes of scratch; the tests pass sizes up to 64.
    let ok = unsafe { GetProcessInformation(h, class, i.b.as_mut_ptr().cast(), size) };
    (ok, err_text(), i)
}

/// One line for ProcessPowerThrottling, asked the way it has to be (Version 1 in the buffer): the three fields.
fn say_throttling(t: &mut Test, what: &str, h: HANDLE) {
    let (ok, e, i) = get_info(h, ProcessPowerThrottling, 12, Some(1));
    if ok != 0 {
        say!(
            t,
            "{what}: GetProcessInformation(ProcessPowerThrottling, 12) = {ok}: Version {} ControlMask {:#x} StateMask {:#x}",
            i.u32_at(0),
            i.u32_at(4),
            i.u32_at(8)
        );
    } else {
        say!(t, "{what}: GetProcessInformation(ProcessPowerThrottling, 12) = {ok} last error {}", e.as_str());
    }
}

/// One line: SetProcessInformation(ProcessPowerThrottling, {version, control, state}).
fn set_throttling(t: &mut Test, h: HANDLE, version: u32, control: u32, state: u32) {
    let v = [version, control, state];
    mark_error();
    // SAFETY: 12 bytes.
    let ok = unsafe { SetProcessInformation(h, ProcessPowerThrottling, v.as_ptr().cast(), 12) };
    say!(
        t,
        "SetProcessInformation(ProcessPowerThrottling, Version {version} ControlMask {control:#x} StateMask {state:#x}) = {ok} last error {}",
        err_text().as_str()
    );
}

main_with_child!(child, test);

fn child(_: &str) -> u32 {
    // SAFETY: no pointers.
    unsafe { Sleep(8000) };
    0
}

fn test(t: &mut Test) {
    // SAFETY: no arguments.
    let me = unsafe { GetCurrentProcess() };
    // Chrome: GetProcessInformation(process, ProcessPowerThrottling (4), &state, 12) = TRUE.
    say_throttling(t, "own process, fresh", me);
    // The caller has to say which version of the structure it has: the first field, before the call.
    for (what, version) in [
        ("Version 1 in the buffer", Some(1u32)),
        ("Version 0 in the buffer", Some(0)),
        ("Version 2 in the buffer", Some(2)),
        ("the buffer not filled in (0xAAAAAAAA)", None),
    ] {
        let (ok, e, i) = get_info(me, ProcessPowerThrottling, 12, version);
        if ok != 0 {
            say!(
                t,
                "{what}: = {ok}: Version {} ControlMask {:#x} StateMask {:#x}",
                i.u32_at(0),
                i.u32_at(4),
                i.u32_at(8)
            );
        } else {
            say!(
                t,
                "{what}: = {ok} last error {}; ControlMask and StateMask untouched {}",
                e.as_str(),
                u32::from(i.u32_at(4) == 0xAAAA_AAAA && i.u32_at(8) == 0xAAAA_AAAA)
            );
        }
    }
    // Other sizes (Version 1 in the buffer where four bytes fit).
    for size in [0u32, 4, 8, 11, 13, 16, 24] {
        let (ok, e, i) = get_info(me, ProcessPowerThrottling, size, if size >= 4 { Some(1) } else { None });
        say!(
            t,
            "size {size}: = {ok} last error {}; ControlMask untouched {}",
            e.as_str(),
            u32::from(i.u32_at(4) == 0xAAAA_AAAA)
        );
    }

    // A child process, through handles with each right.
    let Some(c) = start_child("wait", 0) else {
        say!(t, "the child did not start");
        return;
    };
    say_throttling(t, "child, the handle from CreateProcessW", c.process);
    for (name, access) in [
        ("PROCESS_QUERY_LIMITED_INFORMATION", PROCESS_QUERY_LIMITED_INFORMATION),
        ("PROCESS_QUERY_INFORMATION", PROCESS_QUERY_INFORMATION),
        ("PROCESS_SET_INFORMATION", PROCESS_SET_INFORMATION),
        ("SYNCHRONIZE", SYNCHRONIZE),
    ] {
        // SAFETY: opens the child by its id.
        let h = unsafe { OpenProcess(access, 0, c.pid) };
        let mut what = Buf::<100>::new();
        let _ = write!(what, "child, handle with {name} (opened {})", opened(h));
        say_throttling(t, what.as_str(), h);
        close(h);
    }
    say_throttling(t, "the child's thread handle", c.thread);
    say_throttling(t, "NULL handle", core::ptr::null_mut());
    c.kill();
}
