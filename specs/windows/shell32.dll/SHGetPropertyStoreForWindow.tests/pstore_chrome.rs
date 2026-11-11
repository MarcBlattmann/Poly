//! SHGetPropertyStoreForWindow as Chrome calls it (a top-level window of its own,
//! IID_IPropertyStore): the result, then the store used the way a browser uses it: the
//! application user model id and the relaunch properties set, read back and committed, and
//! read again through a second store of the same window.
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

// shellapi.h: HRESULT SHGetPropertyStoreForWindow(HWND hwnd, REFIID riid, void **ppv);
// propsys.h (IPropertyStore: QueryInterface, AddRef, Release, GetCount, GetAt, GetValue, SetValue, Commit),
// propidlbase.h (PROPVARIANT), wtypes.h (PROPERTYKEY, VT_ values), propkey.h (the PKEY_ values),
// combaseapi.h / objbase.h (CoInitializeEx, CoUninitialize, CoTaskMemFree), winuser.h.
#[repr(C)]
struct PROPERTYKEY {
    fmtid: GUID,
    pid: u32,
}

/// PROPVARIANT (24 bytes on x64): the type, three reserved words, then the value.
#[repr(C)]
struct PROPVARIANT {
    vt: u16,
    r: [u16; 3],
    val: usize,
    val2: usize,
}

const VT_EMPTY: u16 = 0;
const VT_I4: u16 = 3;
const VT_BOOL: u16 = 11;
const VT_LPWSTR: u16 = 31;

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetPropertyStoreForWindow(hwnd: HANDLE, riid: *const GUID, ppv: *mut *mut c_void) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, model: u32) -> i32;
    fn CoUninitialize();
    fn CoTaskMemFree(p: *mut c_void);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateWindowExW(
        ex: u32,
        class: *const u16,
        name: *const u16,
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
    fn DestroyWindow(hwnd: HANDLE) -> BOOL;
    fn IsWindow(hwnd: HANDLE) -> BOOL;
}

const IID_IUnknown: GUID = guid(0x00000000, 0x0000, 0x0000, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IDispatch: GUID = guid(0x00020400, 0x0000, 0x0000, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPropertyStore: GUID = guid(0x886D8EEB, 0x8CF2, 0x4446, [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99]);
const IID_IPropertyStoreCapabilities: GUID =
    guid(0xC8E2D566, 0x186E, 0x4D49, [0xBF, 0x41, 0x69, 0x09, 0xEA, 0xD5, 0x6A, 0xCC]);
const IID_INamedPropertyStore: GUID =
    guid(0x71604B0F, 0x97B0, 0x4764, [0x85, 0x77, 0x2F, 0x13, 0xE9, 0x8A, 0x14, 0x22]);
/// The System.AppUserModel.* keys share this format id (propkey.h).
const APPMODEL: GUID = guid(0x9F4C2855, 0x9F79, 0x4B39, [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3]);
const PKEY_AppUserModel_RelaunchCommand: PROPERTYKEY = PROPERTYKEY { fmtid: APPMODEL, pid: 2 };
const PKEY_AppUserModel_RelaunchIconResource: PROPERTYKEY = PROPERTYKEY { fmtid: APPMODEL, pid: 3 };
const PKEY_AppUserModel_RelaunchDisplayNameResource: PROPERTYKEY = PROPERTYKEY { fmtid: APPMODEL, pid: 4 };
const PKEY_AppUserModel_ID: PROPERTYKEY = PROPERTYKEY { fmtid: APPMODEL, pid: 5 };
const PKEY_AppUserModel_PreventPinning: PROPERTYKEY = PROPERTYKEY { fmtid: APPMODEL, pid: 9 };
/// System.Title: a property that has nothing to do with windows.
const PKEY_Title: PROPERTYKEY =
    PROPERTYKEY { fmtid: guid(0xF29F85E0, 0x4FF9, 0x1068, [0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9]), pid: 2 };

/// An IPropertyStore pointer and its methods, called through the vtable.
#[derive(Clone, Copy)]
struct Store(*mut c_void);

impl Store {
    fn slot(self, i: usize) -> usize {
        // SAFETY: a COM object: its first pointer is the vtable, which has at least 8 entries for IPropertyStore.
        unsafe { *(*(self.0 as *const *const usize)).add(i) }
    }
    fn query(self, iid: &GUID) -> (u32, *mut c_void) {
        let mut out = 0x1111_2222usize as *mut c_void;
        // SAFETY: IUnknown::QueryInterface.
        let hr = unsafe {
            core::mem::transmute::<usize, extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32>(
                self.slot(0),
            )(self.0, iid, &mut out)
        };
        (hr as u32, out)
    }
    fn release(self) -> u32 {
        // SAFETY: IUnknown::Release.
        unsafe { core::mem::transmute::<usize, extern "system" fn(*mut c_void) -> u32>(self.slot(2))(self.0) }
    }
    fn count(self) -> (u32, u32) {
        let mut n = 0xAAAAu32;
        // SAFETY: IPropertyStore::GetCount.
        let hr = unsafe {
            core::mem::transmute::<usize, extern "system" fn(*mut c_void, *mut u32) -> i32>(self.slot(3))(
                self.0, &mut n,
            )
        };
        (hr as u32, n)
    }
    fn at(self, i: u32, key: &mut PROPERTYKEY) -> u32 {
        // SAFETY: IPropertyStore::GetAt.
        (unsafe {
            core::mem::transmute::<usize, extern "system" fn(*mut c_void, u32, *mut PROPERTYKEY) -> i32>(self.slot(4))(
                self.0, i, key,
            )
        }) as u32
    }
    fn get(self, key: &PROPERTYKEY, v: &mut PROPVARIANT) -> u32 {
        // SAFETY: IPropertyStore::GetValue.
        (unsafe {
            core::mem::transmute::<usize, extern "system" fn(*mut c_void, *const PROPERTYKEY, *mut PROPVARIANT) -> i32>(
                self.slot(5),
            )(self.0, key, v)
        }) as u32
    }
    fn set(self, key: &PROPERTYKEY, v: &PROPVARIANT) -> u32 {
        // SAFETY: IPropertyStore::SetValue.
        (unsafe {
            core::mem::transmute::<usize, extern "system" fn(*mut c_void, *const PROPERTYKEY, *const PROPVARIANT) -> i32>(
                self.slot(6),
            )(self.0, key, v)
        }) as u32
    }
    fn commit(self) -> u32 {
        // SAFETY: IPropertyStore::Commit.
        (unsafe { core::mem::transmute::<usize, extern "system" fn(*mut c_void) -> i32>(self.slot(7))(self.0) }) as u32
    }
}

/// SHGetPropertyStoreForWindow: (HRESULT, what is in the out pointer).
fn store_for(hwnd: HANDLE, iid: &GUID) -> (u32, *mut c_void) {
    let mut out = 0x1111_2222usize as *mut c_void;
    // SAFETY: live IID and out pointer; the window handle may be anything.
    let hr = unsafe { SHGetPropertyStoreForWindow(hwnd, iid, &mut out) };
    (hr as u32, out)
}

fn ptr_text(p: *mut c_void) -> &'static str {
    if p.is_null() {
        "NULL"
    } else if p as usize == 0x1111_2222 {
        "untouched"
    } else {
        "an object"
    }
}

/// A hidden top-level window of the predefined class "Static".
fn window(title: &str) -> HANDLE {
    let class = wide("Static");
    let name = wide(title);
    // SAFETY: NUL-terminated strings; no parent, menu, instance or parameter.
    unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            name.as_ptr(),
            0,
            10,
            10,
            200,
            100,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    }
}

/// One line: SetValue(key, a VT_LPWSTR of `text`).
fn set_text(t: &mut Test, s: Store, what: &str, key: &PROPERTYKEY, text: &str) {
    let w = wide(text);
    let v = PROPVARIANT { vt: VT_LPWSTR, r: [0; 3], val: w.as_ptr() as usize, val2: 0 };
    say!(t, "SetValue({what}, VT_LPWSTR \"{text}\") = {:#x}", s.set(key, &v));
}

/// One line: GetValue(key): the HRESULT, the type, and the text of a VT_LPWSTR (freed afterwards).
fn get_text(t: &mut Test, s: Store, what: &str, key: &PROPERTYKEY) {
    let mut v = PROPVARIANT { vt: 0xAAAA, r: [0; 3], val: 0, val2: 0 };
    let hr = s.get(key, &mut v);
    if v.vt == VT_LPWSTR {
        let p = v.val as *const u16;
        say!(t, "GetValue({what}) = {hr:#x} vt {} \"{}\"", v.vt, units_text(wstr_at(p, 200)).as_str());
        // SAFETY: the string GetValue allocated for the caller.
        unsafe { CoTaskMemFree(p as *mut c_void) };
    } else if v.vt == VT_BOOL || v.vt == VT_I4 {
        say!(t, "GetValue({what}) = {hr:#x} vt {} value {}", v.vt, v.val as u32 as i32);
    } else {
        say!(t, "GetValue({what}) = {hr:#x} vt {}", v.vt);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: COINIT_APARTMENTTHREADED (2), as on Chrome's UI thread.
    let hr = unsafe { CoInitializeEx(core::ptr::null_mut(), 2) };
    say!(t, "CoInitializeEx(apartment threaded) = {:#x}", hr as u32);
    let w = window("poly store");
    say!(t, "window created {}", opened(w));
    let (hr, p) = store_for(w, &IID_IPropertyStore);
    say!(t, "SHGetPropertyStoreForWindow(window, IID_IPropertyStore) = {hr:#x} out pointer {}", ptr_text(p));
    if hr != 0 || p.is_null() {
        return;
    }
    let s = Store(p);
    let (hr, n) = s.count();
    say!(t, "a new window's store: GetCount = {hr:#x} count {n}");
    get_text(t, s, "AppUserModel.ID", &PKEY_AppUserModel_ID);
    set_text(t, s, "AppUserModel.ID", &PKEY_AppUserModel_ID, "Poly.Spec.Window");
    get_text(t, s, "AppUserModel.ID", &PKEY_AppUserModel_ID);
    set_text(
        t,
        s,
        "AppUserModel.RelaunchCommand",
        &PKEY_AppUserModel_RelaunchCommand,
        "\"C:\\poly\\app.exe\" --profile=x",
    );
    set_text(
        t,
        s,
        "AppUserModel.RelaunchDisplayNameResource",
        &PKEY_AppUserModel_RelaunchDisplayNameResource,
        "Poly App",
    );
    set_text(t, s, "AppUserModel.RelaunchIconResource", &PKEY_AppUserModel_RelaunchIconResource, "C:\\poly\\app.exe,0");
    get_text(t, s, "AppUserModel.RelaunchCommand", &PKEY_AppUserModel_RelaunchCommand);
    get_text(t, s, "AppUserModel.RelaunchDisplayNameResource", &PKEY_AppUserModel_RelaunchDisplayNameResource);
    get_text(t, s, "AppUserModel.RelaunchIconResource", &PKEY_AppUserModel_RelaunchIconResource);
    let (hr, n) = s.count();
    say!(t, "after four values: GetCount = {hr:#x} count {n}");
    say!(t, "Commit = {:#x}", s.commit());
    get_text(t, s, "AppUserModel.ID after Commit", &PKEY_AppUserModel_ID);

    // A second store of the same window sees the values: they belong to the window.
    let (hr, p2) = store_for(w, &IID_IPropertyStore);
    say!(
        t,
        "a second call for the same window = {hr:#x} out pointer {}; the same object as the first {}",
        ptr_text(p2),
        u32::from(p2 == p)
    );
    if hr == 0 && !p2.is_null() {
        let s2 = Store(p2);
        get_text(t, s2, "AppUserModel.ID through the second store", &PKEY_AppUserModel_ID);
        // Changing the value through the second store, reading through the first.
        set_text(t, s2, "AppUserModel.ID through the second store", &PKEY_AppUserModel_ID, "Poly.Spec.Other");
        get_text(t, s, "AppUserModel.ID through the first store", &PKEY_AppUserModel_ID);
        s2.release();
    }
    // Another window has its own values.
    let w2 = window("poly store 2");
    let (hr, p3) = store_for(w2, &IID_IPropertyStore);
    if hr == 0 && !p3.is_null() {
        get_text(t, Store(p3), "AppUserModel.ID of another window", &PKEY_AppUserModel_ID);
        Store(p3).release();
    }
    s.release();
    // SAFETY: windows this test made.
    unsafe {
        DestroyWindow(w2);
        DestroyWindow(w);
        CoUninitialize();
    }
}
