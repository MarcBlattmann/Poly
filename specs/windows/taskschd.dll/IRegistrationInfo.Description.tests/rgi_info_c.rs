//! IRegistrationInfo: the text properties (description, author, version, documentation, source, URI, date and security
//! descriptor), defaults, what is accepted, how each is written in the XML, and what registering fills in.
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

// Generated from the public SDK header taskschd.h (IIDs, vtable orders): consts only.

const IID_ITaskService: GUID = guid(0x2faba4c7, 0x4da9, 0x4013, [0x96, 0x97, 0x20, 0xcc, 0x3f, 0xd4, 0x0f, 0x85]);
const IID_ITaskFolder: GUID = guid(0x8cfac062, 0xa080, 0x4c15, [0x9a, 0x88, 0xaa, 0x7c, 0x2a, 0xf8, 0x0d, 0xfc]);
const IID_ITaskFolderCollection: GUID =
    guid(0x79184a66, 0x8664, 0x423f, [0x97, 0xf1, 0x63, 0x73, 0x56, 0xa5, 0xd8, 0x12]);
const IID_IRegisteredTask: GUID = guid(0x9c86f320, 0xdee3, 0x4dd1, [0xb9, 0x72, 0xa3, 0x03, 0xf2, 0x6b, 0x06, 0x1e]);
const IID_IRegisteredTaskCollection: GUID =
    guid(0x86627eb4, 0x42a7, 0x41e4, [0xa4, 0xd9, 0xac, 0x33, 0xa7, 0x2f, 0x2d, 0x52]);
const IID_ITaskDefinition: GUID = guid(0xf5bc8fc5, 0x536d, 0x4f77, [0xb8, 0x52, 0xfb, 0xc1, 0x35, 0x6f, 0xde, 0xb6]);
const IID_ITaskSettings: GUID = guid(0x8fd4711d, 0x2d02, 0x4c8c, [0x87, 0xe3, 0xef, 0xf6, 0x99, 0xde, 0x12, 0x7e]);
const IID_ITaskSettings2: GUID = guid(0x2c05c3f0, 0x6eed, 0x4c05, [0xa1, 0x5f, 0xed, 0x7d, 0x7a, 0x98, 0xa3, 0x69]);
const IID_ITaskSettings3: GUID = guid(0x0ad9d0d7, 0x0c7f, 0x4ebb, [0x9a, 0x5f, 0xd1, 0xc6, 0x48, 0xdc, 0xa5, 0x28]);
const IID_ITriggerCollection: GUID = guid(0x85df5081, 0x1b24, 0x4f32, [0x87, 0x8a, 0xd9, 0xd1, 0x4d, 0xf4, 0xcb, 0x77]);
const IID_ITrigger: GUID = guid(0x09941815, 0xea89, 0x4b5b, [0x89, 0xe0, 0x2a, 0x77, 0x38, 0x01, 0xfa, 0xc3]);
const IID_ITimeTrigger: GUID = guid(0xb45747e0, 0xeba7, 0x4276, [0x9f, 0x29, 0x85, 0xc5, 0xbb, 0x30, 0x00, 0x06]);
const IID_IDailyTrigger: GUID = guid(0x126c5cd8, 0xb288, 0x41d5, [0x8d, 0xbf, 0xe4, 0x91, 0x44, 0x6a, 0xdc, 0x5c]);
const IID_ILogonTrigger: GUID = guid(0x72dade38, 0xfae4, 0x4b3e, [0xba, 0xf4, 0x5d, 0x00, 0x9a, 0xf0, 0x2b, 0x1c]);
const IID_IBootTrigger: GUID = guid(0x2a9c35da, 0xd357, 0x41f4, [0xbb, 0xc1, 0x20, 0x7a, 0xc1, 0xb1, 0xf3, 0xcb]);
const IID_IIdleTrigger: GUID = guid(0xd537d2b0, 0x9fb3, 0x4d34, [0x97, 0x39, 0x1f, 0xf5, 0xce, 0x7b, 0x1e, 0xf3]);
const IID_IRegistrationTrigger: GUID =
    guid(0x4c8fec3a, 0xc218, 0x4e0c, [0xb2, 0x3d, 0x62, 0x90, 0x24, 0xdb, 0x91, 0xa2]);
const IID_IEventTrigger: GUID = guid(0xd45b0167, 0x9653, 0x4eef, [0xb9, 0x4f, 0x07, 0x32, 0xca, 0x7a, 0xf2, 0x51]);
const IID_IWeeklyTrigger: GUID = guid(0x5038fc98, 0x82ff, 0x436d, [0x87, 0x28, 0xa5, 0x12, 0xa5, 0x7c, 0x9d, 0xc1]);
const IID_IMonthlyTrigger: GUID = guid(0x97c45ef1, 0x6b02, 0x4a1a, [0x9c, 0x0e, 0x1e, 0xbf, 0xba, 0x15, 0x00, 0xac]);
const IID_IActionCollection: GUID = guid(0x02820e19, 0x7b98, 0x4ed2, [0xb2, 0xe8, 0xfd, 0xcc, 0xce, 0xff, 0x61, 0x9b]);
const IID_IAction: GUID = guid(0xbae54997, 0x48b1, 0x4cbe, [0x99, 0x65, 0xd6, 0xbe, 0x26, 0x3e, 0xbe, 0xa4]);
const IID_IExecAction: GUID = guid(0x4c3d624d, 0xfd6b, 0x49a3, [0xb9, 0xb7, 0x09, 0xcb, 0x3c, 0xd3, 0xf0, 0x47]);
const IID_IExecAction2: GUID = guid(0xf2a82542, 0xbda5, 0x4e6b, [0x91, 0x43, 0xe2, 0xbf, 0x4f, 0x89, 0x87, 0xb6]);
const IID_IPrincipal: GUID = guid(0xd98d51e5, 0xc9b4, 0x496a, [0xa9, 0xc1, 0x18, 0x98, 0x02, 0x61, 0xcf, 0x0f]);
const IID_IPrincipal2: GUID = guid(0x248919ae, 0xe345, 0x4a6d, [0x8a, 0xeb, 0xe0, 0xd3, 0x16, 0x5c, 0x90, 0x4e]);
const IID_IRegistrationInfo: GUID = guid(0x416d8b73, 0xcb41, 0x4ea1, [0x80, 0x5c, 0x9b, 0xe9, 0xa5, 0xac, 0x4a, 0x74]);
const IID_IRepetitionPattern: GUID = guid(0x7fb9acf1, 0x26be, 0x400e, [0x85, 0xb5, 0x29, 0x4b, 0x9c, 0x75, 0xdf, 0xd6]);
const IID_IIdleSettings: GUID = guid(0x84594461, 0x0053, 0x4342, [0xa8, 0xfd, 0x08, 0x8f, 0xab, 0xf1, 0x1f, 0x32]);
const IID_INetworkSettings: GUID = guid(0x9f7dea84, 0xc30b, 0x4245, [0x80, 0xb6, 0x00, 0xe9, 0xf6, 0x46, 0xf1, 0xb4]);
const IID_IRunningTask: GUID = guid(0x653758fb, 0x7b9a, 0x4f1e, [0xa4, 0x71, 0xbe, 0xeb, 0x8e, 0x9b, 0x83, 0x4e]);
const IID_IRunningTaskCollection: GUID =
    guid(0x6a67614b, 0x6828, 0x4fec, [0xaa, 0x54, 0x6d, 0x52, 0xe8, 0xf1, 0xf2, 0xdb]);
const IID_ITaskNamedValueCollection: GUID =
    guid(0xb4ef826b, 0x63c3, 0x46e4, [0xa5, 0x04, 0xef, 0x69, 0xe4, 0xf7, 0xea, 0x4d]);
const IID_ITaskNamedValuePair: GUID =
    guid(0x39038068, 0x2b46, 0x4afd, [0x86, 0x62, 0x7b, 0xb6, 0xf8, 0x68, 0xd2, 0x21]);
const IID_IMaintenanceSettings: GUID =
    guid(0xa6024fa8, 0x9652, 0x4adb, [0xa6, 0xbf, 0x5c, 0xfc, 0xd8, 0x77, 0xa7, 0xba]);
const IID_IComHandlerAction: GUID = guid(0x6d2fd252, 0x75c5, 0x4f66, [0x90, 0xba, 0x2a, 0x7d, 0x8c, 0xc3, 0x03, 0x9f]);
const IID_IEmailAction: GUID = guid(0x10f62c64, 0x7e16, 0x4314, [0xa0, 0xc2, 0x0c, 0x36, 0x83, 0xf9, 0x9d, 0x40]);
const IID_IShowMessageAction: GUID = guid(0x505e9e68, 0xaf89, 0x46b8, [0xa3, 0x0f, 0x56, 0x16, 0x2a, 0x83, 0xd5, 0x37]);
const ITASKSERVICE_GETFOLDER: usize = 7;
const ITASKSERVICE_GETRUNNINGTASKS: usize = 8;
const ITASKSERVICE_NEWTASK: usize = 9;
const ITASKSERVICE_CONNECT: usize = 10;
const ITASKSERVICE_GET_CONNECTED: usize = 11;
const ITASKSERVICE_GET_TARGETSERVER: usize = 12;
const ITASKSERVICE_GET_CONNECTEDUSER: usize = 13;
const ITASKSERVICE_GET_CONNECTEDDOMAIN: usize = 14;
const ITASKSERVICE_GET_HIGHESTVERSION: usize = 15;
const ITASKFOLDER_GET_NAME: usize = 7;
const ITASKFOLDER_GET_PATH: usize = 8;
const ITASKFOLDER_GETFOLDER: usize = 9;
const ITASKFOLDER_GETFOLDERS: usize = 10;
const ITASKFOLDER_CREATEFOLDER: usize = 11;
const ITASKFOLDER_DELETEFOLDER: usize = 12;
const ITASKFOLDER_GETTASK: usize = 13;
const ITASKFOLDER_GETTASKS: usize = 14;
const ITASKFOLDER_DELETETASK: usize = 15;
const ITASKFOLDER_REGISTERTASK: usize = 16;
const ITASKFOLDER_REGISTERTASKDEFINITION: usize = 17;
const ITASKFOLDER_GETSECURITYDESCRIPTOR: usize = 18;
const ITASKFOLDER_SETSECURITYDESCRIPTOR: usize = 19;
const ITASKFOLDERCOLLECTION_GET_COUNT: usize = 7;
const ITASKFOLDERCOLLECTION_GET_ITEM: usize = 8;
const ITASKFOLDERCOLLECTION_GET__NEWENUM: usize = 9;
const IREGISTEREDTASK_GET_NAME: usize = 7;
const IREGISTEREDTASK_GET_PATH: usize = 8;
const IREGISTEREDTASK_GET_STATE: usize = 9;
const IREGISTEREDTASK_GET_ENABLED: usize = 10;
const IREGISTEREDTASK_PUT_ENABLED: usize = 11;
const IREGISTEREDTASK_RUN: usize = 12;
const IREGISTEREDTASK_RUNEX: usize = 13;
const IREGISTEREDTASK_GETINSTANCES: usize = 14;
const IREGISTEREDTASK_GET_LASTRUNTIME: usize = 15;
const IREGISTEREDTASK_GET_LASTTASKRESULT: usize = 16;
const IREGISTEREDTASK_GET_NUMBEROFMISSEDRUNS: usize = 17;
const IREGISTEREDTASK_GET_NEXTRUNTIME: usize = 18;
const IREGISTEREDTASK_GET_DEFINITION: usize = 19;
const IREGISTEREDTASK_GET_XML: usize = 20;
const IREGISTEREDTASK_GETSECURITYDESCRIPTOR: usize = 21;
const IREGISTEREDTASK_SETSECURITYDESCRIPTOR: usize = 22;
const IREGISTEREDTASK_STOP: usize = 23;
const IREGISTEREDTASK_GETRUNTIMES: usize = 24;
const IREGISTEREDTASKCOLLECTION_GET_COUNT: usize = 7;
const IREGISTEREDTASKCOLLECTION_GET_ITEM: usize = 8;
const IREGISTEREDTASKCOLLECTION_GET__NEWENUM: usize = 9;
const ITASKDEFINITION_GET_REGISTRATIONINFO: usize = 7;
const ITASKDEFINITION_PUT_REGISTRATIONINFO: usize = 8;
const ITASKDEFINITION_GET_TRIGGERS: usize = 9;
const ITASKDEFINITION_PUT_TRIGGERS: usize = 10;
const ITASKDEFINITION_GET_SETTINGS: usize = 11;
const ITASKDEFINITION_PUT_SETTINGS: usize = 12;
const ITASKDEFINITION_GET_DATA: usize = 13;
const ITASKDEFINITION_PUT_DATA: usize = 14;
const ITASKDEFINITION_GET_PRINCIPAL: usize = 15;
const ITASKDEFINITION_PUT_PRINCIPAL: usize = 16;
const ITASKDEFINITION_GET_ACTIONS: usize = 17;
const ITASKDEFINITION_PUT_ACTIONS: usize = 18;
const ITASKDEFINITION_GET_XMLTEXT: usize = 19;
const ITASKDEFINITION_PUT_XMLTEXT: usize = 20;
const ITASKSETTINGS_GET_ALLOWDEMANDSTART: usize = 7;
const ITASKSETTINGS_PUT_ALLOWDEMANDSTART: usize = 8;
const ITASKSETTINGS_GET_RESTARTINTERVAL: usize = 9;
const ITASKSETTINGS_PUT_RESTARTINTERVAL: usize = 10;
const ITASKSETTINGS_GET_RESTARTCOUNT: usize = 11;
const ITASKSETTINGS_PUT_RESTARTCOUNT: usize = 12;
const ITASKSETTINGS_GET_MULTIPLEINSTANCES: usize = 13;
const ITASKSETTINGS_PUT_MULTIPLEINSTANCES: usize = 14;
const ITASKSETTINGS_GET_STOPIFGOINGONBATTERIES: usize = 15;
const ITASKSETTINGS_PUT_STOPIFGOINGONBATTERIES: usize = 16;
const ITASKSETTINGS_GET_DISALLOWSTARTIFONBATTERIES: usize = 17;
const ITASKSETTINGS_PUT_DISALLOWSTARTIFONBATTERIES: usize = 18;
const ITASKSETTINGS_GET_ALLOWHARDTERMINATE: usize = 19;
const ITASKSETTINGS_PUT_ALLOWHARDTERMINATE: usize = 20;
const ITASKSETTINGS_GET_STARTWHENAVAILABLE: usize = 21;
const ITASKSETTINGS_PUT_STARTWHENAVAILABLE: usize = 22;
const ITASKSETTINGS_GET_XMLTEXT: usize = 23;
const ITASKSETTINGS_PUT_XMLTEXT: usize = 24;
const ITASKSETTINGS_GET_RUNONLYIFNETWORKAVAILABLE: usize = 25;
const ITASKSETTINGS_PUT_RUNONLYIFNETWORKAVAILABLE: usize = 26;
const ITASKSETTINGS_GET_EXECUTIONTIMELIMIT: usize = 27;
const ITASKSETTINGS_PUT_EXECUTIONTIMELIMIT: usize = 28;
const ITASKSETTINGS_GET_ENABLED: usize = 29;
const ITASKSETTINGS_PUT_ENABLED: usize = 30;
const ITASKSETTINGS_GET_DELETEEXPIREDTASKAFTER: usize = 31;
const ITASKSETTINGS_PUT_DELETEEXPIREDTASKAFTER: usize = 32;
const ITASKSETTINGS_GET_PRIORITY: usize = 33;
const ITASKSETTINGS_PUT_PRIORITY: usize = 34;
const ITASKSETTINGS_GET_COMPATIBILITY: usize = 35;
const ITASKSETTINGS_PUT_COMPATIBILITY: usize = 36;
const ITASKSETTINGS_GET_HIDDEN: usize = 37;
const ITASKSETTINGS_PUT_HIDDEN: usize = 38;
const ITASKSETTINGS_GET_IDLESETTINGS: usize = 39;
const ITASKSETTINGS_PUT_IDLESETTINGS: usize = 40;
const ITASKSETTINGS_GET_RUNONLYIFIDLE: usize = 41;
const ITASKSETTINGS_PUT_RUNONLYIFIDLE: usize = 42;
const ITASKSETTINGS_GET_WAKETORUN: usize = 43;
const ITASKSETTINGS_PUT_WAKETORUN: usize = 44;
const ITASKSETTINGS_GET_NETWORKSETTINGS: usize = 45;
const ITASKSETTINGS_PUT_NETWORKSETTINGS: usize = 46;
const ITASKSETTINGS2_GET_DISALLOWSTARTONREMOTEAPPSESSION: usize = 7;
const ITASKSETTINGS2_PUT_DISALLOWSTARTONREMOTEAPPSESSION: usize = 8;
const ITASKSETTINGS2_GET_USEUNIFIEDSCHEDULINGENGINE: usize = 9;
const ITASKSETTINGS2_PUT_USEUNIFIEDSCHEDULINGENGINE: usize = 10;
const ITASKSETTINGS3_GET_ALLOWDEMANDSTART: usize = 7;
const ITASKSETTINGS3_PUT_ALLOWDEMANDSTART: usize = 8;
const ITASKSETTINGS3_GET_RESTARTINTERVAL: usize = 9;
const ITASKSETTINGS3_PUT_RESTARTINTERVAL: usize = 10;
const ITASKSETTINGS3_GET_RESTARTCOUNT: usize = 11;
const ITASKSETTINGS3_PUT_RESTARTCOUNT: usize = 12;
const ITASKSETTINGS3_GET_MULTIPLEINSTANCES: usize = 13;
const ITASKSETTINGS3_PUT_MULTIPLEINSTANCES: usize = 14;
const ITASKSETTINGS3_GET_STOPIFGOINGONBATTERIES: usize = 15;
const ITASKSETTINGS3_PUT_STOPIFGOINGONBATTERIES: usize = 16;
const ITASKSETTINGS3_GET_DISALLOWSTARTIFONBATTERIES: usize = 17;
const ITASKSETTINGS3_PUT_DISALLOWSTARTIFONBATTERIES: usize = 18;
const ITASKSETTINGS3_GET_ALLOWHARDTERMINATE: usize = 19;
const ITASKSETTINGS3_PUT_ALLOWHARDTERMINATE: usize = 20;
const ITASKSETTINGS3_GET_STARTWHENAVAILABLE: usize = 21;
const ITASKSETTINGS3_PUT_STARTWHENAVAILABLE: usize = 22;
const ITASKSETTINGS3_GET_XMLTEXT: usize = 23;
const ITASKSETTINGS3_PUT_XMLTEXT: usize = 24;
const ITASKSETTINGS3_GET_RUNONLYIFNETWORKAVAILABLE: usize = 25;
const ITASKSETTINGS3_PUT_RUNONLYIFNETWORKAVAILABLE: usize = 26;
const ITASKSETTINGS3_GET_EXECUTIONTIMELIMIT: usize = 27;
const ITASKSETTINGS3_PUT_EXECUTIONTIMELIMIT: usize = 28;
const ITASKSETTINGS3_GET_ENABLED: usize = 29;
const ITASKSETTINGS3_PUT_ENABLED: usize = 30;
const ITASKSETTINGS3_GET_DELETEEXPIREDTASKAFTER: usize = 31;
const ITASKSETTINGS3_PUT_DELETEEXPIREDTASKAFTER: usize = 32;
const ITASKSETTINGS3_GET_PRIORITY: usize = 33;
const ITASKSETTINGS3_PUT_PRIORITY: usize = 34;
const ITASKSETTINGS3_GET_COMPATIBILITY: usize = 35;
const ITASKSETTINGS3_PUT_COMPATIBILITY: usize = 36;
const ITASKSETTINGS3_GET_HIDDEN: usize = 37;
const ITASKSETTINGS3_PUT_HIDDEN: usize = 38;
const ITASKSETTINGS3_GET_IDLESETTINGS: usize = 39;
const ITASKSETTINGS3_PUT_IDLESETTINGS: usize = 40;
const ITASKSETTINGS3_GET_RUNONLYIFIDLE: usize = 41;
const ITASKSETTINGS3_PUT_RUNONLYIFIDLE: usize = 42;
const ITASKSETTINGS3_GET_WAKETORUN: usize = 43;
const ITASKSETTINGS3_PUT_WAKETORUN: usize = 44;
const ITASKSETTINGS3_GET_NETWORKSETTINGS: usize = 45;
const ITASKSETTINGS3_PUT_NETWORKSETTINGS: usize = 46;
const ITASKSETTINGS3_GET_DISALLOWSTARTONREMOTEAPPSESSION: usize = 47;
const ITASKSETTINGS3_PUT_DISALLOWSTARTONREMOTEAPPSESSION: usize = 48;
const ITASKSETTINGS3_GET_USEUNIFIEDSCHEDULINGENGINE: usize = 49;
const ITASKSETTINGS3_PUT_USEUNIFIEDSCHEDULINGENGINE: usize = 50;
const ITASKSETTINGS3_GET_MAINTENANCESETTINGS: usize = 51;
const ITASKSETTINGS3_PUT_MAINTENANCESETTINGS: usize = 52;
const ITASKSETTINGS3_CREATEMAINTENANCESETTINGS: usize = 53;
const ITASKSETTINGS3_GET_VOLATILE: usize = 54;
const ITASKSETTINGS3_PUT_VOLATILE: usize = 55;
const ITRIGGERCOLLECTION_GET_COUNT: usize = 7;
const ITRIGGERCOLLECTION_GET_ITEM: usize = 8;
const ITRIGGERCOLLECTION_GET__NEWENUM: usize = 9;
const ITRIGGERCOLLECTION_CREATE: usize = 10;
const ITRIGGERCOLLECTION_REMOVE: usize = 11;
const ITRIGGERCOLLECTION_CLEAR: usize = 12;
const ITRIGGER_GET_TYPE: usize = 7;
const ITRIGGER_GET_ID: usize = 8;
const ITRIGGER_PUT_ID: usize = 9;
const ITRIGGER_GET_REPETITION: usize = 10;
const ITRIGGER_PUT_REPETITION: usize = 11;
const ITRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const ITRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const ITRIGGER_GET_STARTBOUNDARY: usize = 14;
const ITRIGGER_PUT_STARTBOUNDARY: usize = 15;
const ITRIGGER_GET_ENDBOUNDARY: usize = 16;
const ITRIGGER_PUT_ENDBOUNDARY: usize = 17;
const ITRIGGER_GET_ENABLED: usize = 18;
const ITRIGGER_PUT_ENABLED: usize = 19;
const ITIMETRIGGER_GET_TYPE: usize = 7;
const ITIMETRIGGER_GET_ID: usize = 8;
const ITIMETRIGGER_PUT_ID: usize = 9;
const ITIMETRIGGER_GET_REPETITION: usize = 10;
const ITIMETRIGGER_PUT_REPETITION: usize = 11;
const ITIMETRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const ITIMETRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const ITIMETRIGGER_GET_STARTBOUNDARY: usize = 14;
const ITIMETRIGGER_PUT_STARTBOUNDARY: usize = 15;
const ITIMETRIGGER_GET_ENDBOUNDARY: usize = 16;
const ITIMETRIGGER_PUT_ENDBOUNDARY: usize = 17;
const ITIMETRIGGER_GET_ENABLED: usize = 18;
const ITIMETRIGGER_PUT_ENABLED: usize = 19;
const ITIMETRIGGER_GET_RANDOMDELAY: usize = 20;
const ITIMETRIGGER_PUT_RANDOMDELAY: usize = 21;
const IDAILYTRIGGER_GET_TYPE: usize = 7;
const IDAILYTRIGGER_GET_ID: usize = 8;
const IDAILYTRIGGER_PUT_ID: usize = 9;
const IDAILYTRIGGER_GET_REPETITION: usize = 10;
const IDAILYTRIGGER_PUT_REPETITION: usize = 11;
const IDAILYTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IDAILYTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IDAILYTRIGGER_GET_STARTBOUNDARY: usize = 14;
const IDAILYTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IDAILYTRIGGER_GET_ENDBOUNDARY: usize = 16;
const IDAILYTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IDAILYTRIGGER_GET_ENABLED: usize = 18;
const IDAILYTRIGGER_PUT_ENABLED: usize = 19;
const IDAILYTRIGGER_GET_DAYSINTERVAL: usize = 20;
const IDAILYTRIGGER_PUT_DAYSINTERVAL: usize = 21;
const IDAILYTRIGGER_GET_RANDOMDELAY: usize = 22;
const IDAILYTRIGGER_PUT_RANDOMDELAY: usize = 23;
const ILOGONTRIGGER_GET_TYPE: usize = 7;
const ILOGONTRIGGER_GET_ID: usize = 8;
const ILOGONTRIGGER_PUT_ID: usize = 9;
const ILOGONTRIGGER_GET_REPETITION: usize = 10;
const ILOGONTRIGGER_PUT_REPETITION: usize = 11;
const ILOGONTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const ILOGONTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const ILOGONTRIGGER_GET_STARTBOUNDARY: usize = 14;
const ILOGONTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const ILOGONTRIGGER_GET_ENDBOUNDARY: usize = 16;
const ILOGONTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const ILOGONTRIGGER_GET_ENABLED: usize = 18;
const ILOGONTRIGGER_PUT_ENABLED: usize = 19;
const ILOGONTRIGGER_GET_DELAY: usize = 20;
const ILOGONTRIGGER_PUT_DELAY: usize = 21;
const ILOGONTRIGGER_GET_USERID: usize = 22;
const ILOGONTRIGGER_PUT_USERID: usize = 23;
const IBOOTTRIGGER_GET_TYPE: usize = 7;
const IBOOTTRIGGER_GET_ID: usize = 8;
const IBOOTTRIGGER_PUT_ID: usize = 9;
const IBOOTTRIGGER_GET_REPETITION: usize = 10;
const IBOOTTRIGGER_PUT_REPETITION: usize = 11;
const IBOOTTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IBOOTTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IBOOTTRIGGER_GET_STARTBOUNDARY: usize = 14;
const IBOOTTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IBOOTTRIGGER_GET_ENDBOUNDARY: usize = 16;
const IBOOTTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IBOOTTRIGGER_GET_ENABLED: usize = 18;
const IBOOTTRIGGER_PUT_ENABLED: usize = 19;
const IBOOTTRIGGER_GET_DELAY: usize = 20;
const IBOOTTRIGGER_PUT_DELAY: usize = 21;
const IIDLETRIGGER_GET_TYPE: usize = 7;
const IIDLETRIGGER_GET_ID: usize = 8;
const IIDLETRIGGER_PUT_ID: usize = 9;
const IIDLETRIGGER_GET_REPETITION: usize = 10;
const IIDLETRIGGER_PUT_REPETITION: usize = 11;
const IIDLETRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IIDLETRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IIDLETRIGGER_GET_STARTBOUNDARY: usize = 14;
const IIDLETRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IIDLETRIGGER_GET_ENDBOUNDARY: usize = 16;
const IIDLETRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IIDLETRIGGER_GET_ENABLED: usize = 18;
const IIDLETRIGGER_PUT_ENABLED: usize = 19;
const IREGISTRATIONTRIGGER_GET_TYPE: usize = 7;
const IREGISTRATIONTRIGGER_GET_ID: usize = 8;
const IREGISTRATIONTRIGGER_PUT_ID: usize = 9;
const IREGISTRATIONTRIGGER_GET_REPETITION: usize = 10;
const IREGISTRATIONTRIGGER_PUT_REPETITION: usize = 11;
const IREGISTRATIONTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IREGISTRATIONTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IREGISTRATIONTRIGGER_GET_STARTBOUNDARY: usize = 14;
const IREGISTRATIONTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IREGISTRATIONTRIGGER_GET_ENDBOUNDARY: usize = 16;
const IREGISTRATIONTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IREGISTRATIONTRIGGER_GET_ENABLED: usize = 18;
const IREGISTRATIONTRIGGER_PUT_ENABLED: usize = 19;
const IREGISTRATIONTRIGGER_GET_DELAY: usize = 20;
const IREGISTRATIONTRIGGER_PUT_DELAY: usize = 21;
const IEVENTTRIGGER_GET_TYPE: usize = 7;
const IEVENTTRIGGER_GET_ID: usize = 8;
const IEVENTTRIGGER_PUT_ID: usize = 9;
const IEVENTTRIGGER_GET_REPETITION: usize = 10;
const IEVENTTRIGGER_PUT_REPETITION: usize = 11;
const IEVENTTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IEVENTTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IEVENTTRIGGER_GET_STARTBOUNDARY: usize = 14;
const IEVENTTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IEVENTTRIGGER_GET_ENDBOUNDARY: usize = 16;
const IEVENTTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IEVENTTRIGGER_GET_ENABLED: usize = 18;
const IEVENTTRIGGER_PUT_ENABLED: usize = 19;
const IEVENTTRIGGER_GET_SUBSCRIPTION: usize = 20;
const IEVENTTRIGGER_PUT_SUBSCRIPTION: usize = 21;
const IEVENTTRIGGER_GET_DELAY: usize = 22;
const IEVENTTRIGGER_PUT_DELAY: usize = 23;
const IEVENTTRIGGER_GET_VALUEQUERIES: usize = 24;
const IEVENTTRIGGER_PUT_VALUEQUERIES: usize = 25;
const IWEEKLYTRIGGER_GET_TYPE: usize = 7;
const IWEEKLYTRIGGER_GET_ID: usize = 8;
const IWEEKLYTRIGGER_PUT_ID: usize = 9;
const IWEEKLYTRIGGER_GET_REPETITION: usize = 10;
const IWEEKLYTRIGGER_PUT_REPETITION: usize = 11;
const IWEEKLYTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IWEEKLYTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IWEEKLYTRIGGER_GET_STARTBOUNDARY: usize = 14;
const IWEEKLYTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IWEEKLYTRIGGER_GET_ENDBOUNDARY: usize = 16;
const IWEEKLYTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IWEEKLYTRIGGER_GET_ENABLED: usize = 18;
const IWEEKLYTRIGGER_PUT_ENABLED: usize = 19;
const IWEEKLYTRIGGER_GET_DAYSOFWEEK: usize = 20;
const IWEEKLYTRIGGER_PUT_DAYSOFWEEK: usize = 21;
const IWEEKLYTRIGGER_GET_WEEKSINTERVAL: usize = 22;
const IWEEKLYTRIGGER_PUT_WEEKSINTERVAL: usize = 23;
const IWEEKLYTRIGGER_GET_RANDOMDELAY: usize = 24;
const IWEEKLYTRIGGER_PUT_RANDOMDELAY: usize = 25;
const IMONTHLYTRIGGER_GET_TYPE: usize = 7;
const IMONTHLYTRIGGER_GET_ID: usize = 8;
const IMONTHLYTRIGGER_PUT_ID: usize = 9;
const IMONTHLYTRIGGER_GET_REPETITION: usize = 10;
const IMONTHLYTRIGGER_PUT_REPETITION: usize = 11;
const IMONTHLYTRIGGER_GET_EXECUTIONTIMELIMIT: usize = 12;
const IMONTHLYTRIGGER_PUT_EXECUTIONTIMELIMIT: usize = 13;
const IMONTHLYTRIGGER_GET_STARTBOUNDARY: usize = 14;
const IMONTHLYTRIGGER_PUT_STARTBOUNDARY: usize = 15;
const IMONTHLYTRIGGER_GET_ENDBOUNDARY: usize = 16;
const IMONTHLYTRIGGER_PUT_ENDBOUNDARY: usize = 17;
const IMONTHLYTRIGGER_GET_ENABLED: usize = 18;
const IMONTHLYTRIGGER_PUT_ENABLED: usize = 19;
const IMONTHLYTRIGGER_GET_DAYSOFMONTH: usize = 20;
const IMONTHLYTRIGGER_PUT_DAYSOFMONTH: usize = 21;
const IMONTHLYTRIGGER_GET_MONTHSOFYEAR: usize = 22;
const IMONTHLYTRIGGER_PUT_MONTHSOFYEAR: usize = 23;
const IMONTHLYTRIGGER_GET_RUNONLASTDAYOFMONTH: usize = 24;
const IMONTHLYTRIGGER_PUT_RUNONLASTDAYOFMONTH: usize = 25;
const IMONTHLYTRIGGER_GET_RANDOMDELAY: usize = 26;
const IMONTHLYTRIGGER_PUT_RANDOMDELAY: usize = 27;
const IACTIONCOLLECTION_GET_COUNT: usize = 7;
const IACTIONCOLLECTION_GET_ITEM: usize = 8;
const IACTIONCOLLECTION_GET__NEWENUM: usize = 9;
const IACTIONCOLLECTION_GET_XMLTEXT: usize = 10;
const IACTIONCOLLECTION_PUT_XMLTEXT: usize = 11;
const IACTIONCOLLECTION_CREATE: usize = 12;
const IACTIONCOLLECTION_REMOVE: usize = 13;
const IACTIONCOLLECTION_CLEAR: usize = 14;
const IACTIONCOLLECTION_GET_CONTEXT: usize = 15;
const IACTIONCOLLECTION_PUT_CONTEXT: usize = 16;
const IACTION_GET_ID: usize = 7;
const IACTION_PUT_ID: usize = 8;
const IACTION_GET_TYPE: usize = 9;
const IEXECACTION_GET_ID: usize = 7;
const IEXECACTION_PUT_ID: usize = 8;
const IEXECACTION_GET_TYPE: usize = 9;
const IEXECACTION_GET_PATH: usize = 10;
const IEXECACTION_PUT_PATH: usize = 11;
const IEXECACTION_GET_ARGUMENTS: usize = 12;
const IEXECACTION_PUT_ARGUMENTS: usize = 13;
const IEXECACTION_GET_WORKINGDIRECTORY: usize = 14;
const IEXECACTION_PUT_WORKINGDIRECTORY: usize = 15;
const IEXECACTION2_GET_ID: usize = 7;
const IEXECACTION2_PUT_ID: usize = 8;
const IEXECACTION2_GET_TYPE: usize = 9;
const IEXECACTION2_GET_PATH: usize = 10;
const IEXECACTION2_PUT_PATH: usize = 11;
const IEXECACTION2_GET_ARGUMENTS: usize = 12;
const IEXECACTION2_PUT_ARGUMENTS: usize = 13;
const IEXECACTION2_GET_WORKINGDIRECTORY: usize = 14;
const IEXECACTION2_PUT_WORKINGDIRECTORY: usize = 15;
const IEXECACTION2_GET_HIDEAPPWINDOW: usize = 16;
const IEXECACTION2_PUT_HIDEAPPWINDOW: usize = 17;
const IPRINCIPAL_GET_ID: usize = 7;
const IPRINCIPAL_PUT_ID: usize = 8;
const IPRINCIPAL_GET_DISPLAYNAME: usize = 9;
const IPRINCIPAL_PUT_DISPLAYNAME: usize = 10;
const IPRINCIPAL_GET_USERID: usize = 11;
const IPRINCIPAL_PUT_USERID: usize = 12;
const IPRINCIPAL_GET_LOGONTYPE: usize = 13;
const IPRINCIPAL_PUT_LOGONTYPE: usize = 14;
const IPRINCIPAL_GET_GROUPID: usize = 15;
const IPRINCIPAL_PUT_GROUPID: usize = 16;
const IPRINCIPAL_GET_RUNLEVEL: usize = 17;
const IPRINCIPAL_PUT_RUNLEVEL: usize = 18;
const IPRINCIPAL2_GET_PROCESSTOKENSIDTYPE: usize = 7;
const IPRINCIPAL2_PUT_PROCESSTOKENSIDTYPE: usize = 8;
const IPRINCIPAL2_GET_REQUIREDPRIVILEGECOUNT: usize = 9;
const IPRINCIPAL2_GET_REQUIREDPRIVILEGE: usize = 10;
const IPRINCIPAL2_ADDREQUIREDPRIVILEGE: usize = 11;
const IREGISTRATIONINFO_GET_DESCRIPTION: usize = 7;
const IREGISTRATIONINFO_PUT_DESCRIPTION: usize = 8;
const IREGISTRATIONINFO_GET_AUTHOR: usize = 9;
const IREGISTRATIONINFO_PUT_AUTHOR: usize = 10;
const IREGISTRATIONINFO_GET_VERSION: usize = 11;
const IREGISTRATIONINFO_PUT_VERSION: usize = 12;
const IREGISTRATIONINFO_GET_DATE: usize = 13;
const IREGISTRATIONINFO_PUT_DATE: usize = 14;
const IREGISTRATIONINFO_GET_DOCUMENTATION: usize = 15;
const IREGISTRATIONINFO_PUT_DOCUMENTATION: usize = 16;
const IREGISTRATIONINFO_GET_XMLTEXT: usize = 17;
const IREGISTRATIONINFO_PUT_XMLTEXT: usize = 18;
const IREGISTRATIONINFO_GET_URI: usize = 19;
const IREGISTRATIONINFO_PUT_URI: usize = 20;
const IREGISTRATIONINFO_GET_SECURITYDESCRIPTOR: usize = 21;
const IREGISTRATIONINFO_PUT_SECURITYDESCRIPTOR: usize = 22;
const IREGISTRATIONINFO_GET_SOURCE: usize = 23;
const IREGISTRATIONINFO_PUT_SOURCE: usize = 24;
const IREPETITIONPATTERN_GET_INTERVAL: usize = 7;
const IREPETITIONPATTERN_PUT_INTERVAL: usize = 8;
const IREPETITIONPATTERN_GET_DURATION: usize = 9;
const IREPETITIONPATTERN_PUT_DURATION: usize = 10;
const IREPETITIONPATTERN_GET_STOPATDURATIONEND: usize = 11;
const IREPETITIONPATTERN_PUT_STOPATDURATIONEND: usize = 12;
const IIDLESETTINGS_GET_IDLEDURATION: usize = 7;
const IIDLESETTINGS_PUT_IDLEDURATION: usize = 8;
const IIDLESETTINGS_GET_WAITTIMEOUT: usize = 9;
const IIDLESETTINGS_PUT_WAITTIMEOUT: usize = 10;
const IIDLESETTINGS_GET_STOPONIDLEEND: usize = 11;
const IIDLESETTINGS_PUT_STOPONIDLEEND: usize = 12;
const IIDLESETTINGS_GET_RESTARTONIDLE: usize = 13;
const IIDLESETTINGS_PUT_RESTARTONIDLE: usize = 14;
const INETWORKSETTINGS_GET_NAME: usize = 7;
const INETWORKSETTINGS_PUT_NAME: usize = 8;
const INETWORKSETTINGS_GET_ID: usize = 9;
const INETWORKSETTINGS_PUT_ID: usize = 10;
const IRUNNINGTASK_GET_NAME: usize = 7;
const IRUNNINGTASK_GET_INSTANCEGUID: usize = 8;
const IRUNNINGTASK_GET_PATH: usize = 9;
const IRUNNINGTASK_GET_STATE: usize = 10;
const IRUNNINGTASK_GET_CURRENTACTION: usize = 11;
const IRUNNINGTASK_STOP: usize = 12;
const IRUNNINGTASK_REFRESH: usize = 13;
const IRUNNINGTASK_GET_ENGINEPID: usize = 14;
const IRUNNINGTASKCOLLECTION_GET_COUNT: usize = 7;
const IRUNNINGTASKCOLLECTION_GET_ITEM: usize = 8;
const IRUNNINGTASKCOLLECTION_GET__NEWENUM: usize = 9;
const ITASKNAMEDVALUECOLLECTION_GET_COUNT: usize = 7;
const ITASKNAMEDVALUECOLLECTION_GET_ITEM: usize = 8;
const ITASKNAMEDVALUECOLLECTION_GET__NEWENUM: usize = 9;
const ITASKNAMEDVALUECOLLECTION_CREATE: usize = 10;
const ITASKNAMEDVALUECOLLECTION_REMOVE: usize = 11;
const ITASKNAMEDVALUECOLLECTION_CLEAR: usize = 12;
const ITASKNAMEDVALUEPAIR_GET_NAME: usize = 7;
const ITASKNAMEDVALUEPAIR_PUT_NAME: usize = 8;
const ITASKNAMEDVALUEPAIR_GET_VALUE: usize = 9;
const ITASKNAMEDVALUEPAIR_PUT_VALUE: usize = 10;
const IMAINTENANCESETTINGS_PUT_PERIOD: usize = 7;
const IMAINTENANCESETTINGS_GET_PERIOD: usize = 8;
const IMAINTENANCESETTINGS_PUT_DEADLINE: usize = 9;
const IMAINTENANCESETTINGS_GET_DEADLINE: usize = 10;
const IMAINTENANCESETTINGS_PUT_EXCLUSIVE: usize = 11;
const IMAINTENANCESETTINGS_GET_EXCLUSIVE: usize = 12;
const ICOMHANDLERACTION_GET_ID: usize = 7;
const ICOMHANDLERACTION_PUT_ID: usize = 8;
const ICOMHANDLERACTION_GET_TYPE: usize = 9;
const ICOMHANDLERACTION_GET_CLASSID: usize = 10;
const ICOMHANDLERACTION_PUT_CLASSID: usize = 11;
const ICOMHANDLERACTION_GET_DATA: usize = 12;
const ICOMHANDLERACTION_PUT_DATA: usize = 13;
const IEMAILACTION_GET_ID: usize = 7;
const IEMAILACTION_PUT_ID: usize = 8;
const IEMAILACTION_GET_TYPE: usize = 9;
const IEMAILACTION_GET_SERVER: usize = 10;
const IEMAILACTION_PUT_SERVER: usize = 11;
const IEMAILACTION_GET_SUBJECT: usize = 12;
const IEMAILACTION_PUT_SUBJECT: usize = 13;
const IEMAILACTION_GET_TO: usize = 14;
const IEMAILACTION_PUT_TO: usize = 15;
const IEMAILACTION_GET_CC: usize = 16;
const IEMAILACTION_PUT_CC: usize = 17;
const IEMAILACTION_GET_BCC: usize = 18;
const IEMAILACTION_PUT_BCC: usize = 19;
const IEMAILACTION_GET_REPLYTO: usize = 20;
const IEMAILACTION_PUT_REPLYTO: usize = 21;
const IEMAILACTION_GET_FROM: usize = 22;
const IEMAILACTION_PUT_FROM: usize = 23;
const IEMAILACTION_GET_HEADERFIELDS: usize = 24;
const IEMAILACTION_PUT_HEADERFIELDS: usize = 25;
const IEMAILACTION_GET_BODY: usize = 26;
const IEMAILACTION_PUT_BODY: usize = 27;
const IEMAILACTION_GET_ATTACHMENTS: usize = 28;
const IEMAILACTION_PUT_ATTACHMENTS: usize = 29;
const ISHOWMESSAGEACTION_GET_ID: usize = 7;
const ISHOWMESSAGEACTION_PUT_ID: usize = 8;
const ISHOWMESSAGEACTION_GET_TYPE: usize = 9;
const ISHOWMESSAGEACTION_GET_TITLE: usize = 10;
const ISHOWMESSAGEACTION_PUT_TITLE: usize = 11;
const ISHOWMESSAGEACTION_GET_MESSAGEBODY: usize = 12;
const ISHOWMESSAGEACTION_PUT_MESSAGEBODY: usize = 13;

// ---------------------------------------------------------------- COM and Automation basics
// Prototypes: Microsoft Learn (ole32, oleaut32, "Task Scheduler 2.0 interfaces"); IIDs, method orders and
// enumerations: the public SDK header taskschd.h (the next block is generated from it); winerror.h for codes.

#[repr(C)]
#[derive(Clone, Copy)]
struct GUID {
    a: u32,
    b: u16,
    c: u16,
    d: [u8; 8],
}

const fn guid(a: u32, b: u16, c: u16, d: [u8; 8]) -> GUID {
    GUID { a, b, c, d }
}

const CLSID_TaskScheduler: GUID = guid(0x0f87369f, 0xa4e5, 0x4cfc, [0xbd, 0x3e, 0x73, 0xe6, 0x15, 0x45, 0x72, 0xdd]);
const IID_IUnknown: GUID = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDispatch: GUID = guid(0x20400, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IClassFactory: GUID = guid(1, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IMarshal: GUID = guid(3, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IProvideClassInfo: GUID = guid(0xb196b283, 0xbab4, 0x101a, [0xb6, 0x9c, 0x00, 0xaa, 0x00, 0x34, 0x1d, 0x07]);
const IID_IErrorInfo: GUID = guid(0x1cf2b120, 0x547d, 0x101b, [0x8e, 0x65, 0x08, 0x00, 0x2b, 0x2b, 0xd1, 0x19]);

const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
const E_POINTER: i32 = 0x8000_4003_u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
const CLASS_E_NOAGGREGATION: i32 = 0x8004_0110_u32 as i32;
const REGDB_E_CLASSNOTREG: i32 = 0x8004_0154_u32 as i32;
const CLSCTX_INPROC_SERVER: u32 = 1;
const CLSCTX_LOCAL_SERVER: u32 = 4;
const CLSCTX_ALL: u32 = 0x17;
const COINIT_MULTITHREADED: u32 = 0;
const COINIT_APARTMENTTHREADED: u32 = 2;
const VT_EMPTY: u16 = 0;
const VT_I4: u16 = 3;
const VT_BSTR: u16 = 8;
const VT_ERROR: u16 = 10;
const VT_BOOL: u16 = 11;
const VT_UNKNOWN: u16 = 13;

/// oaidl.h VARIANT (24 bytes on x64): the type, three reserved words, 16 bytes of value.
#[repr(C)]
#[derive(Clone, Copy)]
struct VARIANT {
    vt: u16,
    r: [u16; 3],
    v: [u64; 2],
}

fn v_empty() -> VARIANT {
    VARIANT { vt: VT_EMPTY, r: [0; 3], v: [0; 2] }
}
fn v_bstr(b: *mut u16) -> VARIANT {
    VARIANT { vt: VT_BSTR, r: [0; 3], v: [b as u64, 0] }
}
fn v_i4(x: i32) -> VARIANT {
    VARIANT { vt: VT_I4, r: [0; 3], v: [x as u32 as u64, 0] }
}
fn v_bool(b: bool) -> VARIANT {
    VARIANT { vt: VT_BOOL, r: [0; 3], v: [if b { 0xFFFF } else { 0 }, 0] }
}
fn v_null_bstr() -> VARIANT {
    VARIANT { vt: VT_BSTR, r: [0; 3], v: [0, 0] }
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, flags: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(
        clsid: *const GUID,
        outer: *mut c_void,
        ctx: u32,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> i32;
    fn CoGetClassObject(
        clsid: *const GUID,
        ctx: u32,
        server: *mut c_void,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> i32;
}

#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SysAllocStringLen(p: *const u16, n: u32) -> *mut u16;
    fn SysFreeString(b: *mut u16);
    fn SysStringLen(b: *const u16) -> u32;
}

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
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn DeleteFileW(name: *const u16) -> BOOL;
}

/// `hr=0x80070002`-style text of an HRESULT.
fn hx(h: i32) -> Buf<12> {
    let mut b = Buf::<12>::new();
    let _ = write!(b, "0x{:08X}", h as u32);
    b
}

/// A new BSTR holding `s` (ASCII; up to 3000 characters).
fn bs(s: &str) -> *mut u16 {
    let mut w = [0u16; 3000];
    let mut n = 0;
    for c in s.chars().take(3000) {
        w[n] = c as u32 as u16;
        n += 1;
    }
    // SAFETY: `w` holds `n` units; the call copies them.
    unsafe { SysAllocStringLen(w.as_ptr(), n as u32) }
}

/// A new BSTR holding the UTF-16 units `u`.
fn bs_units(u: &[u16]) -> *mut u16 {
    // SAFETY: the call copies `u.len()` units from `u`.
    unsafe { SysAllocStringLen(u.as_ptr(), u.len() as u32) }
}

fn bs_free(b: *mut u16) {
    if !b.is_null() {
        // SAFETY: a BSTR this test allocated or received from a call, freed once.
        unsafe { SysFreeString(b) };
    }
}

fn bs_len(b: *const u16) -> u32 {
    // SAFETY: a BSTR (or NULL, whose length is 0).
    unsafe { SysStringLen(b) }
}

/// The units of a BSTR (empty for NULL).
fn bs_units_of<'a>(b: *const u16) -> &'a [u16] {
    if b.is_null() {
        return &[];
    }
    // SAFETY: a BSTR returned by the system holds `SysStringLen` units.
    unsafe { core::slice::from_raw_parts(b, SysStringLen(b) as usize) }
}

/// A BSTR as one line of text: `<null>` for NULL, else `[text]` (anything but printable ASCII as `<xxxx>`; the path of
/// this test's image and the machine's own account SIDs are replaced, they differ from run to run).
fn bq(b: *const u16) -> Buf<620> {
    let mut o = Buf::<620>::new();
    if b.is_null() {
        let _ = write!(o, "<null>");
    } else {
        let t = units_text_masked(bs_units_of(b));
        let _ = write!(o, "[{}]", t.as_str());
    }
    o
}

/// A COM interface pointer (possibly NULL).
#[derive(Clone, Copy)]
struct Ob(*mut c_void);

impl Ob {
    fn nul() -> Ob {
        Ob(core::ptr::null_mut())
    }
    fn ok(self) -> bool {
        !self.0.is_null()
    }
    /// Entry `i` of the object's vtable.
    fn ent(self, i: usize) -> usize {
        // SAFETY: a live interface pointer: its first word points to the table; `i` is within the interface's table.
        unsafe { *(*(self.0 as *const *const usize)).add(i) }
    }
}

/// What a call returns when the object pointer is NULL (a test's own slip, never a Windows behaviour).
trait NoObj {
    fn no() -> Self;
}
impl NoObj for i32 {
    fn no() -> i32 {
        0x7FFF_0001
    }
}
impl NoObj for u32 {
    fn no() -> u32 {
        0
    }
}

/// Call vtable entry `$i` of `$o` with the signature `fn($t...) -> $r` after the `this` pointer.
macro_rules! vc {
    ($o:expr, $i:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o: Ob = $o;
        if o.ok() {
            // SAFETY: the entry is a COM method of the interface with exactly this signature (taskschd.h).
            let f: extern "system" fn(*mut c_void $(, $t)*) -> $r = unsafe { core::mem::transmute(o.ent($i)) };
            f(o.0 $(, $a)*)
        } else {
            <$r as NoObj>::no()
        }
    }};
}

fn addref(o: Ob) -> u32 {
    vc!(o, 1, fn() -> u32)
}
fn rel(o: Ob) -> u32 {
    if o.ok() { vc!(o, 2, fn() -> u32) } else { 0 }
}
fn qi(o: Ob, iid: &GUID) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(o, 0, fn(*const GUID, *mut *mut c_void) -> i32, iid, &mut p);
    (h, Ob(p))
}

/// `get_X(BSTR*)`: the result code and the BSTR (caller frees).
fn g_bstr(o: Ob, i: usize) -> (i32, *mut u16) {
    let mut b: *mut u16 = core::ptr::null_mut();
    let h = vc!(o, i, fn(*mut *mut u16) -> i32, &mut b);
    (h, b)
}
/// `get_X(LONG*)`.
fn g_i32(o: Ob, i: usize) -> (i32, i32) {
    let mut v = 0x5A5A_5A5Ai32;
    let h = vc!(o, i, fn(*mut i32) -> i32, &mut v);
    (h, v)
}
/// `get_X(SHORT*)`.
fn g_i16(o: Ob, i: usize) -> (i32, i32) {
    let mut v = 0x5A5Ai16;
    let h = vc!(o, i, fn(*mut i16) -> i32, &mut v);
    (h, i32::from(v))
}
/// `put_X(SHORT)`.
fn p_i16(o: Ob, i: usize, v: i32) -> i32 {
    vc!(o, i, fn(i16) -> i32, v as i16)
}
/// `get_X(VARIANT_BOOL*)` (the 16-bit value as -1, 0 or another).
fn g_bool(o: Ob, i: usize) -> (i32, i32) {
    let mut v = 0x5A5Ai16;
    let h = vc!(o, i, fn(*mut i16) -> i32, &mut v);
    (h, i32::from(v))
}
/// `get_X(DATE*)` as the raw f64 bits.
fn g_date(o: Ob, i: usize) -> (i32, f64) {
    let mut v = -1.0f64;
    let h = vc!(o, i, fn(*mut f64) -> i32, &mut v);
    (h, v)
}
/// `get_X(IFoo**)`.
fn g_obj(o: Ob, i: usize) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(o, i, fn(*mut *mut c_void) -> i32, &mut p);
    (h, Ob(p))
}
/// `put_X(BSTR)` with a new BSTR made from `s` (freed again).
fn p_str(o: Ob, i: usize, s: &str) -> i32 {
    let b = bs(s);
    let h = vc!(o, i, fn(*mut u16) -> i32, b);
    bs_free(b);
    h
}
/// `put_X(BSTR)` with NULL.
fn p_null(o: Ob, i: usize) -> i32 {
    vc!(o, i, fn(*mut u16) -> i32, core::ptr::null_mut())
}
/// `put_X(LONG)`.
fn p_i32(o: Ob, i: usize, v: i32) -> i32 {
    vc!(o, i, fn(i32) -> i32, v)
}
/// `put_X(VARIANT_BOOL)`.
fn p_bool(o: Ob, i: usize, v: bool) -> i32 {
    vc!(o, i, fn(i16) -> i32, if v { -1 } else { 0 })
}

/// Text for a `VARIANT_BOOL`.
fn bt(v: i32) -> &'static str {
    match v {
        -1 => "true",
        0 => "false",
        _ => "other",
    }
}

/// Print one line `label: hr=… [text]` for a `get_X(BSTR*)` call and free the string.
fn say_bstr(t: &mut Test, label: &str, o: Ob, i: usize) -> (i32, Buf<620>) {
    let (h, b) = g_bstr(o, i);
    let q = bq(b);
    say!(t, "{label}: hr={} {}", hx(h).as_str(), q.as_str());
    bs_free(b);
    (h, q)
}

/// `units_text` with the path of this test's image replaced by `<image>` (it differs from run to run) and any
/// `S-1-5-21-...` account SID (the machine's own) by `<user-sid>`.
fn units_text_masked(u: &[u16]) -> Buf<600> {
    let img = side_path("");
    let n = img.iter().position(|&x| x == 0).unwrap_or(0);
    let key: [u16; 9] = [0x53, 0x2D, 0x31, 0x2D, 0x35, 0x2D, 0x32, 0x31, 0x2D];
    let mut o = Buf::<600>::new();
    let mut i = 0;
    while i < u.len() {
        if n > 0 && i + n <= u.len() && eq_nocase(&u[i..i + n], &img[..n]) {
            let _ = write!(o, "<image>");
            i += n;
            continue;
        }
        if i + 9 <= u.len() && u[i..i + 9] == key {
            let mut j = i + 9;
            while j < u.len() && ((0x30..=0x39).contains(&u[j]) || u[j] == 0x2D) {
                j += 1;
            }
            let _ = write!(o, "<user-sid>");
            i = j;
            continue;
        }
        let c = u[i];
        if (0x20..0x7f).contains(&c) {
            let _ = write!(o, "{}", c as u8 as char);
        } else {
            let _ = write!(o, "<{c:04x}>");
        }
        i += 1;
    }
    o
}

/// Print a multi-line BSTR (XML) as `label|line` lines; returns the number of lines.
fn say_lines(t: &mut Test, label: &str, b: *const u16) -> usize {
    let u = bs_units_of(b);
    let mut n = 0;
    let mut start = 0;
    for k in 0..=u.len() {
        if k == u.len() || u[k] == 0x0A {
            let mut e = k;
            if e > start && u[e - 1] == 0x0D {
                e -= 1;
            }
            if e > start || k < u.len() {
                let line = units_text_masked(&u[start..e]);
                let lt = line.as_str().trim_start();
                let indent = line.as_str().len() - lt.len();
                if lt.starts_with("<Date>") {
                    say!(t, "{label}|{}<Date>(a time)</Date>", &line.as_str()[..indent]);
                } else if lt.starts_with("<Author>") && !lt.contains("SYSTEM") {
                    say!(t, "{label}|{}<Author>(an account)</Author>", &line.as_str()[..indent]);
                } else {
                    say!(t, "{label}|{}", line.as_str());
                }
                n += 1;
            }
            start = k + 1;
        }
    }
    n
}

/// Whether `h` is a success code.
fn succ(h: i32) -> bool {
    h >= 0
}

// ---------------------------------------------------------------- the service, folders, definitions

fn co_init() {
    // SAFETY: initialises this thread for COM (multithreaded apartment).
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_MULTITHREADED) };
}

/// A new, unconnected `ITaskService`.
fn new_service() -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    // SAFETY: live out pointer and IID.
    let h = unsafe {
        CoCreateInstance(&CLSID_TaskScheduler, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_ITaskService, &mut p)
    };
    (h, Ob(p))
}

/// `ITaskService::Connect` with four empty variants.
fn svc_connect(s: Ob) -> i32 {
    vc!(
        s,
        ITASKSERVICE_CONNECT,
        fn(VARIANT, VARIANT, VARIANT, VARIANT) -> i32,
        v_empty(),
        v_empty(),
        v_empty(),
        v_empty()
    )
}

/// A connected `ITaskService`.
fn service() -> Ob {
    let (h, s) = new_service();
    if !succ(h) {
        return Ob::nul();
    }
    let _ = svc_connect(s);
    s
}

/// `ITaskService::GetFolder(path)`.
fn get_folder_of(s: Ob, path: &str) -> (i32, Ob) {
    let b = bs(path);
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(s, ITASKSERVICE_GETFOLDER, fn(*mut u16, *mut *mut c_void) -> i32, b, &mut p);
    bs_free(b);
    (h, Ob(p))
}

/// `ITaskFolder::GetFolder(path)`.
fn folder_get_folder(f: Ob, path: &str) -> (i32, Ob) {
    let b = bs(path);
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(f, ITASKFOLDER_GETFOLDER, fn(*mut u16, *mut *mut c_void) -> i32, b, &mut p);
    bs_free(b);
    (h, Ob(p))
}

/// `ITaskFolder::CreateFolder(path, sddl)`; `sddl` None = an empty variant.
fn create_folder(f: Ob, path: &str, sddl: Option<&str>) -> (i32, Ob) {
    let b = bs(path);
    let sb = sddl.map(bs);
    let sv = match sb {
        Some(x) => v_bstr(x),
        None => v_empty(),
    };
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(f, ITASKFOLDER_CREATEFOLDER, fn(*mut u16, VARIANT, *mut *mut c_void) -> i32, b, sv, &mut p);
    bs_free(b);
    if let Some(x) = sb {
        bs_free(x);
    }
    (h, Ob(p))
}

fn delete_folder(f: Ob, path: &str) -> i32 {
    let b = bs(path);
    let h = vc!(f, ITASKFOLDER_DELETEFOLDER, fn(*mut u16, i32) -> i32, b, 0);
    bs_free(b);
    h
}

fn get_task(f: Ob, path: &str) -> (i32, Ob) {
    let b = bs(path);
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(f, ITASKFOLDER_GETTASK, fn(*mut u16, *mut *mut c_void) -> i32, b, &mut p);
    bs_free(b);
    (h, Ob(p))
}

fn delete_task(f: Ob, path: &str) -> i32 {
    let b = bs(path);
    let h = vc!(f, ITASKFOLDER_DELETETASK, fn(*mut u16, i32) -> i32, b, 0);
    bs_free(b);
    h
}

/// `ITaskService::NewTask(0)`.
fn new_task(s: Ob) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(s, ITASKSERVICE_NEWTASK, fn(u32, *mut *mut c_void) -> i32, 0, &mut p);
    (h, Ob(p))
}

/// `ITaskFolder::RegisterTaskDefinition(path, def, flags, user, password, logon, sddl)`; `user` None = empty variant.
fn register(f: Ob, path: &str, def: Ob, flags: i32, user: Option<&str>, logon: i32) -> (i32, Ob) {
    let b = bs(path);
    let ub = user.map(bs);
    let uv = match ub {
        Some(x) => v_bstr(x),
        None => v_empty(),
    };
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(
        f,
        ITASKFOLDER_REGISTERTASKDEFINITION,
        fn(*mut u16, *mut c_void, i32, VARIANT, VARIANT, i32, VARIANT, *mut *mut c_void) -> i32,
        b,
        def.0,
        flags,
        uv,
        v_empty(),
        logon,
        v_empty(),
        &mut p
    );
    bs_free(b);
    if let Some(x) = ub {
        bs_free(x);
    }
    (h, Ob(p))
}

/// `ITaskFolder::RegisterTask(path, xml, flags, user, password, logon, sddl)`.
fn register_xml(f: Ob, path: &str, xml: *mut u16, flags: i32, user: Option<&str>, logon: i32) -> (i32, Ob) {
    let b = bs(path);
    let ub = user.map(bs);
    let uv = match ub {
        Some(x) => v_bstr(x),
        None => v_empty(),
    };
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(
        f,
        ITASKFOLDER_REGISTERTASK,
        fn(*mut u16, *mut u16, i32, VARIANT, VARIANT, i32, VARIANT, *mut *mut c_void) -> i32,
        b,
        xml,
        flags,
        uv,
        v_empty(),
        logon,
        v_empty(),
        &mut p
    );
    bs_free(b);
    if let Some(x) = ub {
        bs_free(x);
    }
    (h, Ob(p))
}

/// A child collection's `get_Count`.
fn coll_count(c: Ob) -> i32 {
    g_i32(c, 7).1
}

/// `get_Item(VARIANT)` of a folder or task collection with the 1-based index `i` (a VT_I4 variant).
fn coll_item(c: Ob, i: i32) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(c, 8, fn(VARIANT, *mut *mut c_void) -> i32, v_i4(i), &mut p);
    (h, Ob(p))
}

/// `get_Item(VARIANT)` with a name.
fn coll_item_name(c: Ob, name: &str) -> (i32, Ob) {
    let b = bs(name);
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(c, 8, fn(VARIANT, *mut *mut c_void) -> i32, v_bstr(b), &mut p);
    bs_free(b);
    (h, Ob(p))
}

/// `get_Item(LONG)` of a trigger or action collection with the 1-based index `i`.
fn coll_item_l(c: Ob, i: i32) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(c, 8, fn(i32, *mut *mut c_void) -> i32, i, &mut p);
    (h, Ob(p))
}

/// The folders of `f` below it, `GetFolders(0)`.
fn get_folders(f: Ob) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(f, ITASKFOLDER_GETFOLDERS, fn(i32, *mut *mut c_void) -> i32, 0, &mut p);
    (h, Ob(p))
}

fn get_tasks(f: Ob, flags: i32) -> (i32, Ob) {
    let mut p: *mut c_void = core::ptr::null_mut();
    let h = vc!(f, ITASKFOLDER_GETTASKS, fn(i32, *mut *mut c_void) -> i32, flags, &mut p);
    (h, Ob(p))
}

/// The name (`get_Name`, slot 7) of a folder or a task.
fn name_of(o: Ob) -> Buf<620> {
    let (_, b) = g_bstr(o, 7);
    let q = bq(b);
    bs_free(b);
    q
}

/// Delete everything under `\<top>` (tasks, then folders, deepest first) and the folder itself; the result of the last step.
fn purge(root: Ob, top: &str) -> i32 {
    let (h, f) = folder_get_folder(root, top);
    if !succ(h) {
        return h;
    }
    purge_in(f);
    rel(f);
    delete_folder(root, top)
}

fn purge_in(f: Ob) {
    for _round in 0..12 {
        let mut any = false;
        let (h, c) = get_tasks(f, 1);
        if succ(h) {
            let n = coll_count(c);
            let mut names: [[u16; 100]; 12] = [[0; 100]; 12];
            let mut k = 0;
            for i in 1..=n.min(12) {
                let (hh, tk) = coll_item(c, i);
                if succ(hh) {
                    let (_, b) = g_bstr(tk, 7);
                    for (j, &x) in bs_units_of(b).iter().take(99).enumerate() {
                        names[k][j] = x;
                    }
                    k += 1;
                    bs_free(b);
                    rel(tk);
                }
            }
            rel(c);
            for nm in names.iter().take(k) {
                let len = nm.iter().position(|&x| x == 0).unwrap_or(100);
                let b = bs_units(&nm[..len]);
                let _ = vc!(f, ITASKFOLDER_DELETETASK, fn(*mut u16, i32) -> i32, b, 0);
                bs_free(b);
                any = true;
            }
        }
        let (h, c) = get_folders(f);
        if succ(h) {
            let n = coll_count(c);
            let mut names: [[u16; 100]; 8] = [[0; 100]; 8];
            let mut k = 0;
            for i in 1..=n.min(8) {
                let (hh, sf) = coll_item(c, i);
                if succ(hh) {
                    let (_, b) = g_bstr(sf, 7);
                    for (j, &x) in bs_units_of(b).iter().take(99).enumerate() {
                        names[k][j] = x;
                    }
                    k += 1;
                    bs_free(b);
                    purge_in(sf);
                    rel(sf);
                }
            }
            rel(c);
            for nm in names.iter().take(k) {
                let len = nm.iter().position(|&x| x == 0).unwrap_or(100);
                let b = bs_units(&nm[..len]);
                let _ = vc!(f, ITASKFOLDER_DELETEFOLDER, fn(*mut u16, i32) -> i32, b, 0);
                bs_free(b);
                any = true;
            }
        }
        if !any {
            break;
        }
    }
}

// ---------------------------------------------------------------- definitions

/// The path of this test's image with `ext` appended (a marker file the task's process writes), NUL-terminated.
fn side_path(ext: &str) -> [u16; 700] {
    let mut p = [0u16; 700];
    // SAFETY: the buffer holds 700 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 640) } as usize;
    for (k, c) in ext.bytes().enumerate() {
        p[n + k] = u16::from(c);
    }
    p
}

fn file_exists(p: &[u16]) -> bool {
    // SAFETY: a NUL-terminated path.
    unsafe { GetFileAttributesW(p.as_ptr()) != 0xFFFF_FFFF }
}

fn file_delete(p: &[u16]) {
    // SAFETY: a NUL-terminated path.
    unsafe { DeleteFileW(p.as_ptr()) };
}

/// Write a small file (the child's marker): `text` as the content.
fn file_write(p: &[u16], text: &str) -> bool {
    // SAFETY: a NUL-terminated path; CREATE_ALWAYS (2), GENERIC_WRITE, share read/write.
    let h = unsafe { CreateFileW(p.as_ptr(), 0x4000_0000, 3, core::ptr::null(), 2, 0x80, core::ptr::null_mut()) };
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut put = 0u32;
    // SAFETY: writes the bytes of `text` to the handle just opened, then closes it.
    unsafe {
        WriteFile(h, text.as_ptr(), text.len() as u32, &mut put, core::ptr::null_mut());
        CloseHandle(h);
    }
    true
}

/// Wait up to `ms` for the file to exist.
fn wait_file(p: &[u16], ms: u32) -> bool {
    let mut w = 0;
    while w <= ms {
        if file_exists(p) {
            return true;
        }
        // SAFETY: sleeps this thread.
        unsafe { Sleep(50) };
        w += 50;
    }
    false
}

/// The path of this test's image as a BSTR with quotes around it, for an exec action's `Path`.
fn image_bstr_quoted() -> *mut u16 {
    let p = side_path("");
    let n = p.iter().position(|&x| x == 0).unwrap_or(0);
    let mut q = [0u16; 702];
    q[0] = 0x22;
    q[1..=n].copy_from_slice(&p[..n]);
    q[n + 1] = 0x22;
    bs_units(&q[..n + 2])
}

fn image_bstr() -> *mut u16 {
    let p = side_path("");
    let n = p.iter().position(|&x| x == 0).unwrap_or(0);
    bs_units(&p[..n])
}

/// Add an exec action that starts this test's image with `--child <mode>`; returns the `IExecAction` (caller releases).
fn add_exec(d: Ob, mode: &str) -> Ob {
    let (_, ac) = g_obj(d, ITASKDEFINITION_GET_ACTIONS);
    let mut p: *mut c_void = core::ptr::null_mut();
    let _ = vc!(ac, IACTIONCOLLECTION_CREATE, fn(i32, *mut *mut c_void) -> i32, 0, &mut p);
    rel(ac);
    let a = Ob(p);
    let (_, ex) = qi(a, &IID_IExecAction);
    rel(a);
    let path = image_bstr_quoted();
    let _ = vc!(ex, IEXECACTION_PUT_PATH, fn(*mut u16) -> i32, path);
    bs_free(path);
    let mut args = Buf::<64>::new();
    let _ = write!(args, "--child {mode}");
    p_str(ex, IEXECACTION_PUT_ARGUMENTS, args.as_str());
    ex
}

/// A new definition that runs as SYSTEM (`principal`, logon type SERVICE_ACCOUNT) and starts this test's image with `--child <mode>`.
fn sys_def(s: Ob, mode: &str) -> Ob {
    let (h, d) = new_task(s);
    if !succ(h) {
        return Ob::nul();
    }
    let (_, pr) = g_obj(d, ITASKDEFINITION_GET_PRINCIPAL);
    p_str(pr, IPRINCIPAL_PUT_USERID, "SYSTEM");
    p_i32(pr, IPRINCIPAL_PUT_LOGONTYPE, 5);
    rel(pr);
    let ex = add_exec(d, mode);
    rel(ex);
    d
}

/// The child process of a task: `mark` writes `<image>.mark` and ends with 0; `exit7` writes it and ends with 7;
/// `hold` writes `<image>.run`, waits up to 20 s for `<image>.go` and ends with 0.
fn child(a: &str) -> u32 {
    let mark = side_path(".mark");
    match a {
        "mark" => {
            file_write(&mark, "ran");
            0
        }
        "exit7" => {
            file_write(&mark, "ran");
            7
        }
        "hold" => {
            file_write(&side_path(".run"), "up");
            let go = side_path(".go");
            wait_file(&go, 20_000);
            file_write(&mark, "done");
            0
        }
        _ => 99,
    }
}

// ---------------------------------------------------------------- registered tasks

const NEVER_RUN: f64 = 36494.0; // 1999-11-30, what LastRunTime holds for a task that never ran

/// One line with what a registered task reports about itself (times as relations, never values).
fn say_task(t: &mut Test, label: &str, task: Ob) {
    if !task.ok() {
        say!(t, "{label}: no task");
        return;
    }
    let (_, name) = g_bstr(task, IREGISTEREDTASK_GET_NAME);
    let (_, path) = g_bstr(task, IREGISTEREDTASK_GET_PATH);
    let (hs, state) = g_i32(task, IREGISTEREDTASK_GET_STATE);
    let (he, en) = g_bool(task, IREGISTEREDTASK_GET_ENABLED);
    let (hl, last) = g_i32(task, IREGISTEREDTASK_GET_LASTTASKRESULT);
    let (_, lr) = g_date(task, IREGISTEREDTASK_GET_LASTRUNTIME);
    let (_, miss) = g_i32(task, IREGISTEREDTASK_GET_NUMBEROFMISSEDRUNS);
    say!(
        t,
        "{label}: name={} path={} state={} (hr={}) enabled={} (hr={}) lastresult=0x{:08X} (hr={}) ever ran={} missed={}",
        bq(name).as_str(),
        bq(path).as_str(),
        state,
        hx(hs).as_str(),
        bt(en),
        hx(he).as_str(),
        last as u32,
        hx(hl).as_str(),
        (lr > NEVER_RUN + 1.0) as u32,
        miss
    );
    bs_free(name);
    bs_free(path);
}

/// A definition that runs `mode` as SYSTEM, registered at `path` (relative to `root`) with CREATE_OR_UPDATE.
fn reg_sys(root: Ob, s: Ob, path: &str, mode: &str) -> (i32, Ob) {
    let d = sys_def(s, mode);
    let r = register(root, path, d, 6, None, 5);
    rel(d);
    r
}

/// Wait until the task reports `want` (TASK_STATE) or `ms` have passed; the state at the end.
fn wait_state(task: Ob, want: i32, ms: u32) -> i32 {
    let mut w = 0;
    loop {
        let (_, st) = g_i32(task, IREGISTEREDTASK_GET_STATE);
        if st == want || w >= ms {
            return st;
        }
        // SAFETY: sleeps this thread.
        unsafe { Sleep(50) };
        w += 50;
    }
}

/// Names of the tasks in a collection, in the order the collection gives them, joined with `|`.
fn names_of(c: Ob) -> Buf<620> {
    let mut o = Buf::<620>::new();
    let n = coll_count(c);
    for i in 1..=n.min(20) {
        let (h, it) = coll_item(c, i);
        if succ(h) {
            let nm = name_of(it);
            let _ = write!(o, "{}{}", if i > 1 { "|" } else { "" }, nm.as_str());
            rel(it);
        }
    }
    o
}

main_with_child!(child, test);

const BASE: &str = "PolySpec\\trgi";

fn put_get(t: &mut Test, label: &str, o: Ob, i: usize, s: &str) {
    let h = p_str(o, i, s);
    let (_, b) = g_bstr(o, i - 1);
    say!(t, "{label} = {s:?}: put hr={} reads back {}", hx(h).as_str(), bq(b).as_str());
    bs_free(b);
}

fn registration_xml(d: Ob) -> Buf<620> {
    let (_, x) = g_bstr(d, ITASKDEFINITION_GET_XMLTEXT);
    let u = bs_units_of(x);
    let mut o = Buf::<620>::new();
    let mut on = false;
    let mut start = 0;
    for k in 0..=u.len() {
        if k == u.len() || u[k] == 0x0A {
            let line = units_text(&u[start..k]);
            let l = line.as_str().trim();
            if l.starts_with("<RegistrationInfo") {
                on = true;
            }
            if on {
                let _ = write!(o, "{} ", l);
            }
            if l.starts_with("</RegistrationInfo") || (l.starts_with("<RegistrationInfo") && l.ends_with("/>")) {
                on = false;
            }
            start = k + 1;
        }
    }
    bs_free(x);
    o
}

fn test(t: &mut Test) {
    SAY_PART.store(3, core::sync::atomic::Ordering::Relaxed);
    co_init();
    let s = service();
    t.check("service", s.ok(), 0);
    let (_, root) = get_folder_of(s, "\\");
    purge(root, BASE);
    let (_, d) = new_task(s);
    let (_, ri) = g_obj(d, ITASKDEFINITION_GET_REGISTRATIONINFO);
    for (label, i) in [
        ("Description", IREGISTRATIONINFO_GET_DESCRIPTION),
        ("Author", IREGISTRATIONINFO_GET_AUTHOR),
        ("Version", IREGISTRATIONINFO_GET_VERSION),
        ("Date", IREGISTRATIONINFO_GET_DATE),
        ("Documentation", IREGISTRATIONINFO_GET_DOCUMENTATION),
        ("URI", IREGISTRATIONINFO_GET_URI),
        ("Source", IREGISTRATIONINFO_GET_SOURCE),
    ] {
        let mut l = Buf::<64>::new();
        let _ = write!(l, "default {label}");
        say_bstr(t, l.as_str(), ri, i);
    }
    // SecurityDescriptor is a VARIANT.
    let mut v = v_empty();
    let h = vc!(ri, IREGISTRATIONINFO_GET_SECURITYDESCRIPTOR, fn(*mut VARIANT) -> i32, &mut v);
    say!(t, "default SecurityDescriptor: hr={} variant type {}", hx(h).as_str(), v.vt);

    for v in [
        "PolySpec test task",
        "",
        "with <angle> & \"quotes\"",
        "line one\nline two",
        "unicode-free text with spaces   and  runs",
    ] {
        put_get(t, "Description", ri, IREGISTRATIONINFO_PUT_DESCRIPTION, v);
    }
    for v in ["NT AUTHORITY\\SYSTEM", "Somebody", ""] {
        put_get(t, "Author", ri, IREGISTRATIONINFO_PUT_AUTHOR, v);
    }
    for v in ["1.0", "v 2", ""] {
        put_get(t, "Version", ri, IREGISTRATIONINFO_PUT_VERSION, v);
    }
    for v in ["2026-11-17T10:00:00", "2026-11-17T10:00:00Z", "2026-11-17", "last week", ""] {
        put_get(t, "Date", ri, IREGISTRATIONINFO_PUT_DATE, v);
    }
    put_get(t, "Documentation", ri, IREGISTRATIONINFO_PUT_DOCUMENTATION, "https://example.invalid/doc");
    put_get(t, "Source", ri, IREGISTRATIONINFO_PUT_SOURCE, "PolySpec");
    for v in ["\\Some\\Path", "plain", ""] {
        put_get(t, "URI", ri, IREGISTRATIONINFO_PUT_URI, v);
    }
    let sddl = bs("D:(A;;FA;;;SY)");
    let h = vc!(ri, IREGISTRATIONINFO_PUT_SECURITYDESCRIPTOR, fn(VARIANT) -> i32, v_bstr(sddl));
    bs_free(sddl);
    let mut v = v_empty();
    let hg = vc!(ri, IREGISTRATIONINFO_GET_SECURITYDESCRIPTOR, fn(*mut VARIANT) -> i32, &mut v);
    say!(t, "SecurityDescriptor = SDDL string: put hr={} get hr={} type {}", hx(h).as_str(), hx(hg).as_str(), v.vt);
    let bad = bs("garbage");
    let h = vc!(ri, IREGISTRATIONINFO_PUT_SECURITYDESCRIPTOR, fn(VARIANT) -> i32, v_bstr(bad));
    bs_free(bad);
    say!(t, "SecurityDescriptor = a bad string: hr={}", hx(h).as_str());
    say!(t, "xml with everything set: {}", registration_xml(d).as_str());

    // What registering fills in.
    let (_, d2) = new_task(s);
    let (_, ri2) = g_obj(d2, ITASKDEFINITION_GET_REGISTRATIONINFO);
    p_str(ri2, IREGISTRATIONINFO_PUT_DESCRIPTION, "only a description");
    let ex = add_exec(d2, "mark");
    rel(ex);
    let (_, pr) = g_obj(d2, ITASKDEFINITION_GET_PRINCIPAL);
    p_str(pr, IPRINCIPAL_PUT_USERID, "SYSTEM");
    p_i32(pr, IPRINCIPAL_PUT_LOGONTYPE, 5);
    rel(pr);
    let (h, task) = register(root, "PolySpec\\trgi\\r1", d2, 2, None, 5);
    say!(t, "registered: hr={}", hx(h).as_str());
    if task.ok() {
        let (_, td) = g_obj(task, IREGISTEREDTASK_GET_DEFINITION);
        say!(t, "registration xml after registering: {}", registration_xml(td).as_str());
        rel(td);
    }
    rel(task);
    // With author and date set.
    p_str(ri2, IREGISTRATIONINFO_PUT_AUTHOR, "NT AUTHORITY\\SYSTEM");
    p_str(ri2, IREGISTRATIONINFO_PUT_DATE, "2026-11-17T10:00:00");
    p_str(ri2, IREGISTRATIONINFO_PUT_URI, "\\Elsewhere\\Name");
    let (h, task) = register(root, "PolySpec\\trgi\\r2", d2, 2, None, 5);
    say!(t, "registered with author, date and a URI: hr={}", hx(h).as_str());
    if task.ok() {
        let (_, td) = g_obj(task, IREGISTEREDTASK_GET_DEFINITION);
        say!(t, "registration xml after registering: {}", registration_xml(td).as_str());
        rel(td);
    }
    rel(task);
    // The object's own XML text.
    let (h, x) = g_bstr(ri2, IREGISTRATIONINFO_GET_XMLTEXT);
    say!(t, "get_XmlText of the registration info: hr={}", hx(h).as_str());
    say_lines(t, "ri xml", x);
    bs_free(x);
    rel(ri2);
    rel(d2);
    rel(ri);
    rel(d);
    purge(root, BASE);
    rel(root);
    rel(s);
}
