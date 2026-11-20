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

// ---------------------------------------------------------------- WaitForInputIdle helpers
//
// One image, several roles. `<image> --child <kind> <event name> <delay ms>` is a CHILD of the test: it sleeps `delay` ms, signals the
// named event right BEFORE its first message call (the child's "idle point") and then makes that call. The parent creates the event and
// looks, after WaitForInputIdle has returned, whether the event is set: set = the call returned after the child's idle point, not set =
// it returned before it. user32 is loaded by the children at run time (LoadLibraryW), so the image does not import it and a child that
// never needs it never loads it.

use core::sync::atomic::{AtomicU32, Ordering};

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

#[repr(C)]
#[derive(Clone, Copy)]
struct Msg {
    hwnd: HANDLE,
    message: u32,
    wparam: usize,
    lparam: isize,
    time: u32,
    pt: [i32; 2],
}

const WAIT_TIMEOUT: u32 = 0x102;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;
const INFINITE: u32 = 0xFFFF_FFFF;
const SENTINEL: u32 = 0x12345;
const CREATE_SUSPENDED: u32 = 4;
const SYNCHRONIZE_: u32 = 0x0010_0000;
const PROCESS_QUERY_LIMITED: u32 = 0x1000;

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
    fn ResumeThread(h: HANDLE) -> u32;
    fn CreateEventW(sa: *const c_void, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn OpenEventW(access: u32, inherit: BOOL, name: *const u16) -> HANDLE;
    fn SetEvent(h: HANDLE) -> BOOL;
    fn Sleep(ms: u32);
    fn LoadLibraryW(name: *const u16) -> HANDLE;
    fn GetProcAddress(m: HANDLE, name: *const u8) -> usize;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn GetCurrentThread() -> HANDLE;
    fn GetCurrentThreadId() -> u32;
    fn CreateThread(sa: *const c_void, stack: usize, start: unsafe extern "system" fn(*mut c_void) -> u32, arg: *mut c_void, flags: u32, tid: *mut u32) -> HANDLE;
    fn CreateFileW(name: *const u16, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, tmpl: HANDLE) -> HANDLE;
    fn ReadFile(h: HANDLE, buf: *mut u8, n: u32, read: *mut u32, ov: *mut c_void) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn GetTickCount64() -> u64;
}

fn ascii_arg(s: &[u16]) -> Buf<160> {
    let mut b = Buf::<160>::new();
    for &c in s {
        let _ = write!(b, "{}", (c as u8) as char);
    }
    b
}

/// The text after ` --child ` on this process's command line.
fn child_args() -> Option<Buf<160>> {
    // SAFETY: the command line is a NUL-terminated string owned by the process.
    let cl = wstr_at(unsafe { GetCommandLineW() }, 600);
    let key = b" --child ";
    let mut i = 0;
    while i + key.len() <= cl.len() {
        if key.iter().enumerate().all(|(k, &c)| cl[i + k] == u16::from(c)) {
            return Some(ascii_arg(&cl[i + key.len()..]));
        }
        i += 1;
    }
    None
}

fn self_path() -> ([u16; 300], usize) {
    let mut p = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 299) } as usize;
    (p, n)
}

/// The path of a file beside this image.
fn beside(name: &str) -> [u16; 300] {
    let (p, n) = self_path();
    let mut last = 0;
    for (i, &u) in p[..n].iter().enumerate() {
        if u == b'\\' as u16 {
            last = i + 1;
        }
    }
    let mut out = [0u16; 300];
    out[..last].copy_from_slice(&p[..last]);
    for (i, b) in name.bytes().enumerate() {
        out[last + i] = u16::from(b);
    }
    out
}

// ---- user32 at run time

type FnGetMessage = unsafe extern "system" fn(*mut Msg, HANDLE, u32, u32) -> i32;
type FnPeekMessage = unsafe extern "system" fn(*mut Msg, HANDLE, u32, u32, u32) -> BOOL;
type FnWaitMessage = unsafe extern "system" fn() -> BOOL;
type FnMwfmo = unsafe extern "system" fn(u32, *const HANDLE, BOOL, u32, u32) -> u32;
type FnWfii = unsafe extern "system" fn(HANDLE, u32) -> u32;
type FnIsGui = unsafe extern "system" fn(BOOL) -> BOOL;
type FnPostThread = unsafe extern "system" fn(u32, u32, usize, isize) -> BOOL;
type FnCreateWindow = unsafe extern "system" fn(u32, *const u16, *const u16, u32, i32, i32, i32, i32, HANDLE, HANDLE, HANDLE, *const c_void) -> HANDLE;

fn user32(name: &str) -> usize {
    let dll = wide("user32.dll");
    let mut n = [0u8; 64];
    n[..name.len()].copy_from_slice(name.as_bytes());
    // SAFETY: valid NUL-terminated names.
    unsafe {
        let m = LoadLibraryW(dll.as_ptr());
        if m.is_null() { 0 } else { GetProcAddress(m, n.as_ptr()) }
    }
}

fn get_message_loop() {
    let f: FnGetMessage = unsafe { core::mem::transmute::<usize, FnGetMessage>(user32("GetMessageW")) };
    let mut m = Msg { hwnd: core::ptr::null_mut(), message: 0, wparam: 0, lparam: 0, time: 0, pt: [0; 2] };
    // SAFETY: valid message buffer.
    while unsafe { f(&mut m, core::ptr::null_mut(), 0, 0) } > 0 {}
}

fn peek_message(remove: bool) -> BOOL {
    let f: FnPeekMessage = unsafe { core::mem::transmute::<usize, FnPeekMessage>(user32("PeekMessageW")) };
    let mut m = Msg { hwnd: core::ptr::null_mut(), message: 0, wparam: 0, lparam: 0, time: 0, pt: [0; 2] };
    // SAFETY: valid message buffer.
    unsafe { f(&mut m, core::ptr::null_mut(), 0, 0, if remove { 1 } else { 0 }) }
}

fn wfii(h: HANDLE, ms: u32) -> u32 {
    let f: FnWfii = unsafe { core::mem::transmute::<usize, FnWfii>(user32("WaitForInputIdle")) };
    // SAFETY: the handle is whatever the caller passes (that is the point of some tests).
    unsafe { f(h, ms) }
}

/// WaitForInputIdle with the sentinel last error set right before the call (the function is looked up first): the result and the last error.
fn wfii_call(h: HANDLE, ms: u32) -> (u32, u32) {
    let f: FnWfii = unsafe { core::mem::transmute::<usize, FnWfii>(user32("WaitForInputIdle")) };
    // SAFETY: the handle is whatever the caller passes (that is the point of some tests).
    unsafe {
        SetLastError(SENTINEL);
        let r = f(h, ms);
        (r, GetLastError())
    }
}

fn make_gui_thread() {
    let f: FnIsGui = unsafe { core::mem::transmute::<usize, FnIsGui>(user32("IsGUIThread")) };
    // SAFETY: plain call; TRUE converts the thread to a GUI thread (a message queue).
    unsafe { f(1) };
}

fn create_hidden_window() -> HANDLE {
    let f: FnCreateWindow = unsafe { core::mem::transmute::<usize, FnCreateWindow>(user32("CreateWindowExW")) };
    let class = wide("STATIC");
    let title = wide("poly");
    // SAFETY: valid strings; WS_POPUP 0x80000000, not visible.
    unsafe { f(0, class.as_ptr(), title.as_ptr(), 0x8000_0000, 0, 0, 10, 10, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null()) }
}

// ---- children

static COUNTER: AtomicU32 = AtomicU32::new(0);

struct Child {
    process: HANDLE,
    thread: HANDLE,
    ev: HANDLE,
    late: HANDLE,
    pid: u32,
    file: Option<[u16; 300]>,
}

fn read_self(buf: &mut [u8]) -> usize {
    let (me, _) = self_path();
    // SAFETY: valid path; GENERIC_READ, share read, OPEN_EXISTING.
    let h = unsafe { CreateFileW(me.as_ptr(), 0x8000_0000, 1, core::ptr::null(), 3, 0x80, core::ptr::null_mut()) };
    if h as isize == -1 {
        return 0;
    }
    let mut n = 0u32;
    // SAFETY: valid buffer and handle.
    let ok = unsafe { ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, core::ptr::null_mut()) };
    // SAFETY: handle from CreateFileW.
    unsafe { CloseHandle(h) };
    if ok == 0 { 0 } else { n as usize }
}

fn write_bytes(path: &[u16; 300], data: &[u8]) -> bool {
    // SAFETY: valid NUL-terminated path; CREATE_ALWAYS 2, GENERIC_WRITE.
    let h = unsafe { CreateFileW(path.as_ptr(), 0x4000_0000, 0, core::ptr::null(), 2, 0x80, core::ptr::null_mut()) };
    if h as isize == -1 {
        return false;
    }
    let mut w = 0u32;
    // SAFETY: valid buffer and handle.
    let ok = unsafe { WriteFile(h, data.as_ptr(), data.len() as u32, &mut w, core::ptr::null_mut()) };
    // SAFETY: handle from CreateFileW.
    unsafe { CloseHandle(h) };
    ok != 0 && w as usize == data.len()
}

/// A copy of this image with the subsystem changed to Windows GUI (2), written beside it as `name`.
fn gui_copy(name: &str) -> Option<[u16; 300]> {
    let mut buf = [0u8; 65536];
    let n = read_self(&mut buf);
    if n == 0 || n == buf.len() {
        return None;
    }
    let pe = u32::from_le_bytes([buf[0x3c], buf[0x3d], buf[0x3e], buf[0x3f]]) as usize;
    let sub = pe + 24 + 68;
    buf[sub] = 2;
    buf[sub + 1] = 0;
    let path = beside(name);
    if write_bytes(&path, &buf[..n]) { Some(path) } else { None }
}

/// Start a child of kind `kind` that signals its idle event after `delay` ms. `gui`: the copy with the GUI subsystem.
fn spawn(kind: &str, delay: u32, gui: bool, flags: u32) -> Child {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // SAFETY: plain call.
    let pid = unsafe { GetCurrentProcessId() };
    let mut name = Buf::<60>::new();
    let _ = write!(name, "PolyWfii{pid}x{n}");
    let evname = wide(name.as_str());
    // SAFETY: valid name.
    let ev = unsafe { CreateEventW(core::ptr::null(), 1, 0, evname.as_ptr()) };
    let latename = wide(late_name(name.as_str()).as_str());
    // SAFETY: valid name.
    let late = unsafe { CreateEventW(core::ptr::null(), 1, 0, latename.as_ptr()) };
    let (exe, file) = if gui {
        let mut f = Buf::<40>::new();
        let _ = write!(f, "wiigui{n}.exe");
        match gui_copy(f.as_str()) {
            Some(p) => (p, Some(p)),
            None => (self_path().0, None),
        }
    } else {
        (self_path().0, None)
    };
    let mut cmd = [0u16; 600];
    let mut k = 0;
    cmd[k] = b'"' as u16;
    k += 1;
    for &u in wstr_at(exe.as_ptr(), 299) {
        cmd[k] = u;
        k += 1;
    }
    let mut tail = Buf::<160>::new();
    let _ = write!(tail, "\" --child {kind} {} {delay}", name.as_str());
    for b in tail.as_str().bytes() {
        cmd[k] = u16::from(b);
        k += 1;
    }
    let mut si = StartupInfoW { cb: 104, pad0: 0, rest: [0; 12] };
    let mut pi = ProcInfo { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 };
    // SAFETY: valid buffers.
    let ok = unsafe { CreateProcessW(exe.as_ptr(), cmd.as_mut_ptr(), core::ptr::null(), core::ptr::null(), 0, flags, core::ptr::null(), core::ptr::null(), &mut si, &mut pi) };
    if ok == 0 {
        return Child { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), ev, late, pid: 0, file };
    }
    Child { process: pi.process, thread: pi.thread, ev, late, pid: pi.pid, file }
}

fn late_name(name: &str) -> Buf<70> {
    let mut b = Buf::<70>::new();
    let _ = write!(b, "{name}L");
    b
}

/// Where the call returned relative to the child's timeline: before its idle point, within 2 s after it, or later than that.
fn phase(ev: HANDLE, late: HANDLE) -> &'static str {
    // SAFETY: handles from CreateEventW.
    unsafe {
        if WaitForSingleObject(late, 0) == 0 {
            "more than 2 s after"
        } else if WaitForSingleObject(ev, 0) == 0 {
            "within 2 s after"
        } else {
            "before"
        }
    }
}

fn idle_passed(c: &Child) -> bool {
    // SAFETY: handle from CreateEventW.
    unsafe { WaitForSingleObject(c.ev, 0) == 0 }
}

/// Wait (up to 20 s) for the child's idle event.
fn wait_idle_event(c: &Child) -> bool {
    // SAFETY: handle from CreateEventW.
    unsafe { WaitForSingleObject(c.ev, 20000) == 0 }
}

fn running(c: &Child) -> bool {
    // SAFETY: handle from CreateProcessW.
    unsafe { WaitForSingleObject(c.process, 0) == WAIT_TIMEOUT }
}

fn kill(c: Child) {
    // SAFETY: handles from CreateProcessW / CreateEventW.
    unsafe {
        if !c.process.is_null() {
            TerminateProcess(c.process, 0xDEAD);
            WaitForSingleObject(c.process, 5000);
            CloseHandle(c.process);
            CloseHandle(c.thread);
        }
        CloseHandle(c.ev);
        CloseHandle(c.late);
        if let Some(f) = c.file {
            DeleteFileW(f.as_ptr());
        }
    }
}

fn result_text(r: u32) -> Buf<40> {
    let mut b = Buf::<40>::new();
    match r {
        0 => {
            let _ = write!(b, "0 (satisfied)");
        }
        WAIT_TIMEOUT => {
            let _ = write!(b, "WAIT_TIMEOUT");
        }
        WAIT_FAILED => {
            let _ = write!(b, "WAIT_FAILED");
        }
        _ => {
            let _ = write!(b, "{r:#x}");
        }
    }
    b
}

fn err_text(e: u32) -> Buf<30> {
    let mut b = Buf::<30>::new();
    if e == SENTINEL {
        let _ = write!(b, "unchanged");
    } else {
        let _ = write!(b, "{e}");
    }
    b
}

/// WaitForInputIdle(child, timeout) with a sentinel last error; the line says what came back and whether the child's idle point had passed.
fn case(t: &mut Test, label: &str, c: &Child, timeout: u32) {
    let (r, e) = wfii_call(c.process, timeout);
    say!(t, "{label}: result {} error {} returned {} the child's idle point", result_text(r).as_str(), err_text(e).as_str(), phase(c.ev, c.late));
}

// ---- the child side

unsafe extern "system" fn watchdog(_: *mut c_void) -> u32 {
    // SAFETY: plain call. A child that is still there after 150 s is a leftover of a dead parent.
    unsafe { Sleep(150000) };
    exit(0xDEAD)
}

unsafe extern "system" fn second_thread(arg: *mut c_void) -> u32 {
    let delay = arg as usize as u32;
    // SAFETY: plain call.
    unsafe { Sleep(delay) };
    signal_idle_event();
    get_message_loop();
    0
}

static EVNAME: AtomicU32 = AtomicU32::new(0);
static mut EVBUF: [u16; 80] = [0; 80];

unsafe extern "system" fn late_thread(_: *mut c_void) -> u32 {
    // SAFETY: plain calls; EVBUF holds the NUL-terminated event name (written once).
    unsafe {
        Sleep(2000);
        let src = core::ptr::addr_of!(EVBUF) as *const u16;
        let mut name = [0u16; 84];
        let mut n = 0;
        while *src.add(n) != 0 && n < 78 {
            name[n] = *src.add(n);
            n += 1;
        }
        name[n] = b'L' as u16;
        let ev = OpenEventW(2, 0, name.as_ptr());
        if !ev.is_null() {
            SetEvent(ev);
            CloseHandle(ev);
        }
    }
    0
}

fn signal_idle_event() {
    // SAFETY: EVBUF is written once by the child's main thread before any use.
    unsafe {
        let name = core::ptr::addr_of!(EVBUF) as *const u16;
        let ev = OpenEventW(2, 0, name);
        if !ev.is_null() {
            SetEvent(ev);
            CloseHandle(ev);
        }
        let mut tid = 0u32;
        CreateThread(core::ptr::null(), 0, late_thread, core::ptr::null_mut(), 0, &mut tid);
    }
}

fn child_main(args: &str) -> u32 {
    let mut it = args.split(' ');
    let kind = it.next().unwrap_or("");
    let evname = it.next().unwrap_or("");
    let delay: u32 = it.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    // SAFETY: single-threaded at this point.
    unsafe {
        let b = core::ptr::addr_of_mut!(EVBUF) as *mut u16;
        for (i, c) in evname.bytes().enumerate().take(78) {
            *b.add(i) = u16::from(c);
        }
    }
    EVNAME.store(1, Ordering::Relaxed);
    let mut tid = 0u32;
    // SAFETY: valid start routine.
    unsafe { CreateThread(core::ptr::null(), 0, watchdog, core::ptr::null_mut(), 0, &mut tid) };
    // SAFETY: plain calls throughout.
    unsafe {
        match kind {
            // never uses user32
            "sleep" => Sleep(4500),
            "exit" => {
                Sleep(delay);
                return 7;
            }
            "spin" => {
                let end = GetTickCount64() + 4500;
                while GetTickCount64() < end {}
            }
            "loaduser" => {
                user32("GetMessageW");
                Sleep(4500);
            }
            "get" => {
                Sleep(delay);
                signal_idle_event();
                get_message_loop();
            }
            "window" => {
                Sleep(delay);
                create_hidden_window();
                signal_idle_event();
                get_message_loop();
            }
            "windowbusyvlong" => {
                Sleep(delay);
                create_hidden_window();
                Sleep(110000);
            }
            "windowbusylong" => {
                Sleep(delay);
                create_hidden_window();
                Sleep(14000);
            }
            "windowbusy" => {
                Sleep(delay);
                create_hidden_window();
                Sleep(4500);
            }
            "peek" | "peekkeep" => {
                Sleep(delay);
                signal_idle_event();
                peek_message(kind == "peek");
                Sleep(4500);
            }
            "waitmsg" => {
                Sleep(delay);
                signal_idle_event();
                let f: FnWaitMessage = core::mem::transmute::<usize, FnWaitMessage>(user32("WaitMessage"));
                f();
                Sleep(4500);
            }
            "mwfmo" => {
                Sleep(delay);
                signal_idle_event();
                let f: FnMwfmo = core::mem::transmute::<usize, FnMwfmo>(user32("MsgWaitForMultipleObjects"));
                f(0, core::ptr::null(), 0, 20000, 0x4FF);
                Sleep(4500);
            }
            "pending" => {
                // a queue with a message in it: the first call returns at once (not idle); the second blocks (idle)
                Sleep(delay);
                make_gui_thread();
                let post: FnPostThread = core::mem::transmute::<usize, FnPostThread>(user32("PostThreadMessageW"));
                post(GetCurrentThreadId(), 0x400, 0, 0);
                get_one_message();
                Sleep(1500);
                signal_idle_event();
                get_message_loop();
            }
            "queueonly" => {
                // a queue (IsGUIThread) and no message call at all
                make_gui_thread();
                Sleep(4500);
            }
            "threads" => {
                // the main thread stays busy; a second thread becomes idle after `delay`
                CreateThread(core::ptr::null(), 0, second_thread, delay as usize as *mut c_void, 0, &mut tid);
                Sleep(4500);
            }
            "once" => {
                // idle for 0.5 s (a blocking wait with a timeout), then busy for 4.5 s
                Sleep(delay);
                signal_idle_event();
                let f: FnMwfmo = core::mem::transmute::<usize, FnMwfmo>(user32("MsgWaitForMultipleObjects"));
                f(0, core::ptr::null(), 0, 500, 0x4FF);
                Sleep(4500);
            }
            "idleexit" => {
                Sleep(delay);
                signal_idle_event();
                let f: FnMwfmo = core::mem::transmute::<usize, FnMwfmo>(user32("MsgWaitForMultipleObjects"));
                f(0, core::ptr::null(), 0, 500, 0x4FF);
                return 5;
            }
            _ => return 0x77,
        }
    }
    0
}

fn get_one_message() {
    let f: FnGetMessage = unsafe { core::mem::transmute::<usize, FnGetMessage>(user32("GetMessageW")) };
    let mut m = Msg { hwnd: core::ptr::null_mut(), message: 0, wparam: 0, lparam: 0, time: 0, pt: [0; 2] };
    // SAFETY: valid buffer.
    unsafe { f(&mut m, core::ptr::null_mut(), 0, 0) };
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
}

macro_rules! main_wfii {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = child_args() {
                exit(child_main(a.as_str()))
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

fn started(t: &mut Test, label: &str, c: &Child) -> bool {
    if c.process.is_null() {
        say!(t, "{label}: the child could not be started, error {}", gle());
        return false;
    }
    true
}

/// Start a child, call WaitForInputIdle on it with `timeout`, end it.
fn run(t: &mut Test, label: &str, kind: &str, delay: u32, gui: bool, timeout: u32) {
    let c = spawn(kind, delay, gui, 0);
    if started(t, label, &c) {
        case(t, label, &c, timeout);
    }
    kill(c);
}

main_wfii!(test);

unsafe extern "system" fn waiter(arg: *mut c_void) -> u32 {
    wfii(arg, 15000)
}

fn test(t: &mut Test) {
    // input already pending: a posted message is taken first (not idle); the idle point is the next, blocking call
    run(t, "message pending, then blocking GetMessage", "pending", 0, true, 10000);
    // several threads: only the second thread of the child becomes idle
    run(t, "second thread idle, main thread busy", "threads", 1500, true, 10000);
    // two waiters at once on one child
    let c = spawn("get", 1500, true, 0);
    if started(t, "two waiters", &c) {
        let mut tid = 0u32;
        // SAFETY: valid start routine; the handle is passed as the argument.
        let h = unsafe { CreateThread(core::ptr::null(), 0, waiter, c.process, 0, &mut tid) };
        let r1 = wfii(c.process, 15000);
        let mut r2 = 0xAAAAu32;
        // SAFETY: handle from CreateThread.
        unsafe {
            WaitForSingleObject(h, 20000);
            GetExitCodeThread(h, &mut r2);
        }
        say!(t, "two waiters: main thread {} second thread {} returned {} the child's idle point", result_text(r1).as_str(), result_text(r2).as_str(), phase(c.ev, c.late));
    }
    kill(c);
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetExitCodeThread(h: HANDLE, code: *mut u32) -> BOOL;
}
