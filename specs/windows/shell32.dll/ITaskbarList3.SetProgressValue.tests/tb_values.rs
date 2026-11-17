//! The taskbar list's progress calls on windows of different kinds: a window that is shown, one that is not,
//! a child window, a message-only window, the desktop window, and a window of another thread's creation; and
//! two taskbar list objects driving one window.
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

// objbase.h, combaseapi.h, unknwn.h; shobjidl_core.h (the order of the methods in the vtables, the interface ids), shtypes.h.
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
    fn CoTaskMemFree(p: *mut c_void);
}

const COINIT_APARTMENTTHREADED: u32 = 2;
const COINIT_MULTITHREADED: u32 = 0;
const CLSCTX_INPROC_SERVER: u32 = 1;

const IID_IUnknown: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IShellItem: GUID = guid(0x43826D1E, 0xE718, 0x42EE, [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE]);
const IID_IShellItem2: GUID = guid(0x7E9FB0D3, 0x919F, 0x4307, [0xAB, 0x2E, 0x9B, 0x18, 0x60, 0x31, 0x0C, 0x93]);
const CLSID_FileOperation: GUID = guid(0x3AD05575, 0x8857, 0x4850, [0x92, 0x77, 0x11, 0xB8, 0x5B, 0xDB, 0x8E, 0x09]);
const IID_IFileOperation: GUID = guid(0x947AAB5F, 0x0A5C, 0x4C13, [0xB4, 0xD6, 0x4B, 0xF7, 0x83, 0x6F, 0xC9, 0xF8]);
const CLSID_TaskbarList: GUID = guid(0x56FDF344, 0xFD6D, 0x11D0, [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90]);
const IID_ITaskbarList3: GUID = guid(0xEA1AFB91, 0x9E28, 0x4B86, [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF]);
const CLSID_FileSaveDialog: GUID = guid(0xC0B4E2F3, 0xBA21, 0x4773, [0x8D, 0xBA, 0x33, 0x5E, 0xC9, 0x46, 0xEB, 0x8B]);
const IID_IFileSaveDialog: GUID = guid(0x84BCCD23, 0x5FDE, 0x4CDB, [0xAE, 0xA4, 0xAF, 0x64, 0xB8, 0x3D, 0x78, 0xAB]);
const IID_IFileDialog: GUID = guid(0x42F85136, 0xDB7E, 0x439C, [0x85, 0xF1, 0xE4, 0x07, 0x5D, 0x13, 0x5F, 0xC8]);
const CLSID_DestinationList: GUID = guid(0x77F10CF0, 0x3DB5, 0x4966, [0xB5, 0x20, 0xB7, 0xC5, 0x4F, 0xD3, 0x5E, 0xD6]);
const IID_ICustomDestinationList: GUID =
    guid(0x6332DEBF, 0x87B5, 0x4670, [0x90, 0xC0, 0x5E, 0x57, 0xB4, 0x08, 0xA4, 0x9E]);
/// A made-up interface id nothing implements.
const IID_Nothing: GUID = guid(0x1234_5678, 0x9abc, 0xdef0, [1, 2, 3, 4, 5, 6, 7, 8]);

/// An HRESULT as `0x%08x`.
struct Hr(i32);
impl core::fmt::Display for Hr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:08x}", self.0 as u32)
    }
}

/// `np(ptr)`: whether a pointer is set.
fn np<T>(p: *const T) -> &'static str {
    if p.is_null() { "null" } else { "set" }
}

/// The function at `slot` of the vtable of the COM object `obj`.
unsafe fn vslot(obj: *mut c_void, slot: usize) -> *const c_void {
    // SAFETY: the caller passes a live COM object and a slot its interface has (the order in the SDK header).
    unsafe { *(*(obj as *const *const *const c_void)).add(slot) }
}

/// `vcall!(obj, slot, fn(A, B) -> R, a, b)`: call the method at `slot`.
macro_rules! vcall {
    ($obj:expr, $slot:expr, fn($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o: *mut c_void = $obj;
        let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(vslot(o, $slot));
        f(o $(, $a)*)
    }};
}

fn release(p: *mut c_void) -> u32 {
    if p.is_null() {
        return 0;
    }
    // SAFETY: Release is slot 2 of every COM interface; `p` is a live object the test holds a reference to.
    unsafe { vcall!(p, 2, fn() -> u32) }
}

fn addref(p: *mut c_void) -> u32 {
    // SAFETY: AddRef is slot 1.
    unsafe { vcall!(p, 1, fn() -> u32) }
}

/// QueryInterface: the result and the interface pointer (null on failure).
fn qi(p: *mut c_void, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: QueryInterface is slot 0.
    let hr = unsafe { vcall!(p, 0, fn(*const GUID, *mut *mut c_void) -> i32, iid, &mut out) };
    (hr, out)
}

/// CoCreateInstance: the result and the interface pointer.
fn create(clsid: &GUID, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: the arguments are live locals.
    let hr = unsafe { CoCreateInstance(clsid, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, iid, &mut out) };
    (hr, out)
}

/// Start COM in a single-threaded apartment (S_OK, or S_FALSE when already started).
fn com_start() -> i32 {
    // SAFETY: no pointers.
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) }
}

// ---- IShellItem (slots after IUnknown: BindToHandler 3, GetParent 4, GetDisplayName 5, GetAttributes 6, Compare 7)
const SIGDN_NORMALDISPLAY: u32 = 0;
const SIGDN_PARENTRELATIVEPARSING: u32 = 0x8001_8001;
const SIGDN_DESKTOPABSOLUTEPARSING: u32 = 0x8002_8000;
const SIGDN_PARENTRELATIVEEDITING: u32 = 0x8003_1001;
const SIGDN_DESKTOPABSOLUTEEDITING: u32 = 0x8004_c000;
const SIGDN_FILESYSPATH: u32 = 0x8005_8000;
const SIGDN_URL: u32 = 0x8006_8000;

/// IShellItem::GetDisplayName: the result and the string (copied, at most 259 units; the caller's copy is freed).
fn item_name(item: *mut c_void, kind: u32) -> (i32, [u16; 260]) {
    let mut p: *mut u16 = core::ptr::null_mut();
    let mut out = [0u16; 260];
    // SAFETY: slot 5 of IShellItem; the string is CoTaskMemAlloc memory freed here.
    unsafe {
        let hr = vcall!(item, 5, fn(u32, *mut *mut u16) -> i32, kind, &mut p);
        if hr >= 0 && !p.is_null() {
            let s = wstr_at(p, 259);
            out[..s.len()].copy_from_slice(s);
            CoTaskMemFree(p.cast());
        }
        (hr, out)
    }
}

fn item_for(path: &[u16]) -> (i32, *mut c_void) {
    let mut item: *mut c_void = core::ptr::null_mut();
    // SAFETY: `path` is NUL-terminated; the result is a COM object the caller releases.
    let hr = unsafe { SHCreateItemFromParsingName(path.as_ptr(), core::ptr::null_mut(), &IID_IShellItem, &mut item) };
    (hr, item)
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHCreateItemFromParsingName(path: *const u16, bc: *mut c_void, iid: *const GUID, out: *mut *mut c_void) -> i32;
}

poly_wtest::main!(test);

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        title: *const u16,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        parent: HANDLE,
        menu: HANDLE,
        inst: HANDLE,
        param: *mut c_void,
    ) -> HANDLE;
    fn DestroyWindow(h: HANDLE) -> BOOL;
    fn ShowWindow(h: HANDLE, cmd: i32) -> BOOL;
    fn GetDesktopWindow() -> HANDLE;
}

fn state(tb: *mut c_void, w: HANDLE, s: u32) -> i32 {
    // SAFETY: slot 10 of ITaskbarList3.
    unsafe { vcall!(tb, 10, fn(HANDLE, u32) -> i32, w, s) }
}
fn value(tb: *mut c_void, w: HANDLE, d: u64, t: u64) -> i32 {
    // SAFETY: slot 9.
    unsafe { vcall!(tb, 9, fn(HANDLE, u64, u64) -> i32, w, d, t) }
}

fn win(style: u32, parent: HANDLE) -> HANDLE {
    let class = wide("STATIC");
    let title = wide("poly taskbar values");
    // SAFETY: windows of a system class, destroyed by the caller.
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            title.as_ptr(),
            style,
            30,
            30,
            120,
            80,
            parent,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    }
}

fn test(t: &mut Test) {
    say!(t, "CoInitializeEx: {}", Hr(com_start()));
    let (hr, tb) = create(&CLSID_TaskbarList, &IID_ITaskbarList3);
    say!(t, "create: {} {}", Hr(hr), np(tb));
    if tb.is_null() {
        return;
    }
    say!(t, "HrInit: {}", Hr(unsafe { vcall!(tb, 3, fn() -> i32) }));
    let shown = win(0x00CF_0000, core::ptr::null_mut());
    let hidden = win(0x00CF_0000, core::ptr::null_mut());
    // SAFETY: SW_SHOWNOACTIVATE (4) and SW_HIDE (0).
    unsafe {
        ShowWindow(shown, 4);
        ShowWindow(hidden, 0);
    }
    let child = win(0x4000_0000, shown);
    let mut msg_only = core::ptr::null_mut();
    {
        let class = wide("STATIC");
        // SAFETY: HWND_MESSAGE is -3 as the parent.
        msg_only = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                -3isize as HANDLE,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
    }
    // SAFETY: no arguments.
    let desktop = unsafe { GetDesktopWindow() };
    for (what, w) in [
        ("a shown top-level window", shown),
        ("a hidden top-level window", hidden),
        ("a child window", child),
        ("a message-only window", msg_only),
        ("the desktop window", desktop),
    ] {
        say!(
            t,
            "{what}: window {}; SetProgressState(NORMAL) {}; SetProgressValue(5 of 10) {}; SetProgressState(NOPROGRESS) {}",
            opened(w),
            Hr(state(tb, w, 2)),
            Hr(value(tb, w, 5, 10)),
            Hr(state(tb, w, 0))
        );
    }
    // A second object on the same window.
    let (_, tb2) = create(&CLSID_TaskbarList, &IID_ITaskbarList3);
    say!(t, "a second object is another object: {}", tb != tb2);
    say!(t, "second HrInit: {}", Hr(unsafe { vcall!(tb2, 3, fn() -> i32) }));
    say!(t, "second object SetProgressValue: {}", Hr(value(tb2, shown, 1, 2)));
    say!(t, "first object SetProgressState(INDETERMINATE): {}", Hr(state(tb, shown, 1)));
    say!(t, "first object SetProgressState(NOPROGRESS): {}", Hr(state(tb, shown, 0)));
    release(tb2);
    // SAFETY: windows made above.
    unsafe {
        DestroyWindow(child);
        DestroyWindow(shown);
        DestroyWindow(hidden);
        DestroyWindow(msg_only);
    }
    say!(t, "last Release: {}", release(tb));
}
