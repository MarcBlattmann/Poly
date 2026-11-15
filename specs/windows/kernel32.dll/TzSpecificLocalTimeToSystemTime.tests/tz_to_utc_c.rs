//! TzSpecificLocalTimeToSystemTime as Chrome's browser calls it (123 calls): a fixed zone (UTC-8, daylight time
//! from the second Sunday of March 02:00 to the first Sunday of November 02:00, daylight bias -60) turns local
//! times into UTC times; what the skipped hour of the spring change and the repeated hour of the autumn change
//! give; what bad input gives.
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

// timezoneapi.h: SYSTEMTIME, TIME_ZONE_INFORMATION, DYNAMIC_TIME_ZONE_INFORMATION and the conversions.
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SystemTimeToTzSpecificLocalTime(
        tz: *const TIME_ZONE_INFORMATION,
        utc: *const SYSTEMTIME,
        local: *mut SYSTEMTIME,
    ) -> BOOL;
    fn TzSpecificLocalTimeToSystemTime(
        tz: *const TIME_ZONE_INFORMATION,
        local: *const SYSTEMTIME,
        utc: *mut SYSTEMTIME,
    ) -> BOOL;
    fn GetTimeZoneInformation(tz: *mut TIME_ZONE_INFORMATION) -> u32;
    fn GetDynamicTimeZoneInformation(tz: *mut DYNAMIC_TIME_ZONE_INFORMATION) -> u32;
    fn SystemTimeToFileTime(st: *const SYSTEMTIME, ft: *mut u64) -> BOOL;
}

/// timezoneapi.h `SYSTEMTIME` (16 bytes).
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct SYSTEMTIME {
    year: u16,
    month: u16,
    day_of_week: u16,
    day: u16,
    hour: u16,
    minute: u16,
    second: u16,
    ms: u16,
}

/// timezoneapi.h `TIME_ZONE_INFORMATION` (172 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct TIME_ZONE_INFORMATION {
    bias: i32,
    standard_name: [u16; 32],
    standard_date: SYSTEMTIME,
    standard_bias: i32,
    daylight_name: [u16; 32],
    daylight_date: SYSTEMTIME,
    daylight_bias: i32,
}

/// timezoneapi.h `DYNAMIC_TIME_ZONE_INFORMATION` (432 bytes).
#[repr(C)]
#[derive(Clone, Copy)]
struct DYNAMIC_TIME_ZONE_INFORMATION {
    bias: i32,
    standard_name: [u16; 32],
    standard_date: SYSTEMTIME,
    standard_bias: i32,
    daylight_name: [u16; 32],
    daylight_date: SYSTEMTIME,
    daylight_bias: i32,
    key_name: [u16; 128],
    dynamic_dst_disabled: u8,
}

const fn st(year: u16, month: u16, day: u16, hour: u16, minute: u16, second: u16) -> SYSTEMTIME {
    SYSTEMTIME { year, month, day_of_week: 0, day, hour, minute, second, ms: 0 }
}

/// A rule date: month, day of week (0 = Sunday), occurrence in `day` (1..4, 5 = last), hour, minute.
const fn rule(month: u16, dow: u16, occurrence: u16, hour: u16) -> SYSTEMTIME {
    SYSTEMTIME { year: 0, month, day_of_week: dow, day: occurrence, hour, minute: 0, second: 0, ms: 0 }
}

const NO_DATE: SYSTEMTIME =
    SYSTEMTIME { year: 0, month: 0, day_of_week: 0, day: 0, hour: 0, minute: 0, second: 0, ms: 0 };

fn zone(bias: i32, std_date: SYSTEMTIME, std_bias: i32, dst_date: SYSTEMTIME, dst_bias: i32) -> TIME_ZONE_INFORMATION {
    TIME_ZONE_INFORMATION {
        bias,
        standard_name: [0; 32],
        standard_date: std_date,
        standard_bias: std_bias,
        daylight_name: [0; 32],
        daylight_date: dst_date,
        daylight_bias: dst_bias,
    }
}

/// UTC-8 with daylight time from the second Sunday of March 02:00 to the first Sunday of November 02:00.
fn pacific() -> TIME_ZONE_INFORMATION {
    zone(480, rule(11, 0, 1, 2), 0, rule(3, 0, 2, 2), -60)
}

fn stamp(s: &SYSTEMTIME) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let _ = write!(
        b,
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} dow {}",
        s.year, s.month, s.day, s.hour, s.minute, s.second, s.ms, s.day_of_week
    );
    b
}

fn to_local(t: &mut Test, tz: Option<&TIME_ZONE_INFORMATION>, utc: SYSTEMTIME, label: &str) {
    let mut out = SYSTEMTIME { year: 0xAAAA, ..Default::default() };
    mark_error();
    let p = tz.map_or(core::ptr::null(), |z| z as *const _);
    // SAFETY: live structures.
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(p, &utc, &mut out) };
    if ok != 0 {
        say!(t, "{label}: UTC {} -> local {}", stamp(&utc).as_str(), stamp(&out).as_str());
    } else {
        say!(
            t,
            "{label}: UTC {} -> failed, last error {}, output untouched {}",
            stamp(&utc).as_str(),
            err_text().as_str(),
            u32::from(out.year == 0xAAAA)
        );
    }
}

fn to_utc(t: &mut Test, tz: Option<&TIME_ZONE_INFORMATION>, local: SYSTEMTIME, label: &str) {
    let mut out = SYSTEMTIME { year: 0xAAAA, ..Default::default() };
    mark_error();
    let p = tz.map_or(core::ptr::null(), |z| z as *const _);
    // SAFETY: live structures.
    let ok = unsafe { TzSpecificLocalTimeToSystemTime(p, &local, &mut out) };
    if ok != 0 {
        say!(t, "{label}: local {} -> UTC {}", stamp(&local).as_str(), stamp(&out).as_str());
    } else {
        say!(
            t,
            "{label}: local {} -> failed, last error {}, output untouched {}",
            stamp(&local).as_str(),
            err_text().as_str(),
            u32::from(out.year == 0xAAAA)
        );
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    SAY_PART.store(3, core::sync::atomic::Ordering::Relaxed);
    let z = pacific();
    to_utc(t, Some(&z), st(2020, 1, 15, 4, 0, 0), "winter");
    to_utc(t, Some(&z), st(2020, 7, 15, 5, 0, 0), "summer");
    // spring 2020: local times from 02:00 to 02:59:59 on 8 March do not exist
    to_utc(t, Some(&z), st(2020, 3, 8, 1, 59, 59), "one second before the skipped hour");
    to_utc(t, Some(&z), st(2020, 3, 8, 2, 0, 0), "the start of the skipped hour");
    to_utc(t, Some(&z), st(2020, 3, 8, 2, 30, 0), "inside the skipped hour");
    to_utc(t, Some(&z), st(2020, 3, 8, 2, 59, 59), "the end of the skipped hour");
    to_utc(t, Some(&z), st(2020, 3, 8, 3, 0, 0), "after the skipped hour");
    // autumn 2020: local times from 01:00 to 01:59:59 on 1 November happen twice
    to_utc(t, Some(&z), st(2020, 11, 1, 0, 59, 59), "one second before the repeated hour");
    to_utc(t, Some(&z), st(2020, 11, 1, 1, 0, 0), "the start of the repeated hour");
    to_utc(t, Some(&z), st(2020, 11, 1, 1, 30, 0), "inside the repeated hour");
    to_utc(t, Some(&z), st(2020, 11, 1, 1, 59, 59), "the end of the repeated hour");
    to_utc(t, Some(&z), st(2020, 11, 1, 2, 0, 0), "after the repeated hour");
    to_utc(t, Some(&z), st(2021, 1, 1, 0, 0, 0), "local New Year");
    to_utc(t, Some(&z), st(2024, 2, 28, 19, 0, 0), "a leap-year evening");
    let mut ms = st(2020, 7, 15, 5, 0, 0);
    ms.ms = 999;
    ms.day_of_week = 5;
    to_utc(t, Some(&z), ms, "milliseconds 999 and a wrong day of week in the input");
    let eu = zone(-60, rule(10, 0, 5, 3), 0, rule(3, 0, 5, 2), -60);
    to_utc(t, Some(&eu), st(2020, 3, 29, 1, 59, 59), "last-Sunday zone, before the skipped hour");
    to_utc(t, Some(&eu), st(2020, 3, 29, 2, 30, 0), "last-Sunday zone, inside the skipped hour");
    to_utc(t, Some(&eu), st(2020, 10, 25, 2, 30, 0), "last-Sunday zone, inside the repeated hour");
    let none = zone(-330, NO_DATE, 0, NO_DATE, 0);
    to_utc(t, Some(&none), st(2020, 7, 15, 17, 30, 0), "UTC+5:30 without daylight time");
    // a round trip through the other function
    let utc0 = st(2020, 7, 15, 12, 34, 56);
    let mut local = SYSTEMTIME::default();
    let mut back = SYSTEMTIME::default();
    // SAFETY: live structures.
    let (a, b) = unsafe {
        (SystemTimeToTzSpecificLocalTime(&z, &utc0, &mut local), TzSpecificLocalTimeToSystemTime(&z, &local, &mut back))
    };
    say!(
        t,
        "round trip UTC -> local -> UTC: {a} {b}, same second {}",
        u32::from(
            back.hour == utc0.hour && back.minute == utc0.minute && back.second == utc0.second && back.day == utc0.day
        )
    );
    // bad input
    to_utc(t, Some(&z), st(2020, 13, 1, 0, 0, 0), "month 13");
    to_utc(t, Some(&z), st(2020, 2, 30, 0, 0, 0), "30 February");
    to_utc(t, Some(&z), st(2020, 1, 1, 24, 0, 0), "hour 24");
    to_utc(t, Some(&z), st(1601, 1, 1, 0, 0, 0), "1 January 1601 local (before the epoch in UTC)");
    to_utc(t, Some(&z), st(30827, 12, 31, 23, 59, 59), "the end of the file-time range");
    let mut out = SYSTEMTIME::default();
    let local0 = st(2020, 7, 15, 5, 0, 0);
    // SAFETY: live structures; a NULL zone is allowed.
    let ok = unsafe { TzSpecificLocalTimeToSystemTime(core::ptr::null(), &local0, &mut out) };
    say!(
        t,
        "NULL zone: {ok}, the UTC time is within 24 hours of the local time: {}",
        u32::from(out.day >= 14 && out.day <= 16)
    );
}
