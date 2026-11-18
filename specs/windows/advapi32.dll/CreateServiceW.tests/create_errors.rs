//! The errors of CreateServiceW: a name that exists (also in another case), a display name equal to another
//! service's name, names with a backslash or slash or empty, NULL name or path, an empty path, bad type, start and
//! error control values, own and share process together, a dependency on itself, a bad account, and the length
//! limits of names (256 units) and display names (256 units).
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations in this file are from Microsoft Learn (prototypes) and the public SDK headers named
// beside them (winsvc.h, winnt.h, winreg.h, winerror.h: constants and layouts); the helpers are this
// test's own. The program plays two roles: the test, and, started by the service control manager with
// `--child <mode>` on its command line, a service process that writes what it sees to a report file
// beside its image (`<image>.rep`), which the test reads back after the service has stopped.

use core::ffi::c_void;
use core::fmt::Write as _;
use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use poly_wtest::*;

type BOOL = i32;
type SH = *mut c_void;

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

impl<const N: usize> core::fmt::Display for Buf<N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<1000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 299 units).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.bytes().enumerate().take(299) {
        w[i] = u16::from(c);
    }
    w
}

/// A string of `n` copies of `c` (n up to 599), NUL-terminated UTF-16.
fn wide_rep(c: u8, n: usize) -> [u16; 600] {
    let mut w = [0u16; 600];
    for x in w.iter_mut().take(n.min(599)) {
        *x = u16::from(c);
    }
    w
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

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn utext<const N: usize>(u: &[u16]) -> Buf<N> {
    let mut b = Buf::<N>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

/// Like `utext`, but the path of this test's own image is written as `<image>` (the path differs from run to run).
fn utext_scrub<const N: usize>(u: &[u16]) -> Buf<N> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let low = |c: u16| if (0x41..=0x5a).contains(&c) { c + 0x20 } else { c };
    let mut b = Buf::<N>::new();
    let mut i = 0;
    while i < u.len() {
        if n > 0 && i + n <= u.len() && (0..n).all(|j| low(u[i + j]) == low(exe[j])) {
            let _ = write!(b, "<image>");
            i += n;
            continue;
        }
        let c = u[i];
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
        i += 1;
    }
    b
}

/// The text of a NUL-terminated UTF-16 string at `p`, or `NULL`.
fn ptext(p: *const u16) -> Buf<400> {
    if p.is_null() {
        let mut b = Buf::<400>::new();
        let _ = write!(b, "NULL");
        return b;
    }
    utext_scrub::<400>(wstr_at(p, 390))
}

// ------------------------------------------------------------------ constants (winsvc.h, winnt.h)

const SC_MANAGER_CONNECT: u32 = 0x1;
const SC_MANAGER_CREATE_SERVICE: u32 = 0x2;
const SC_MANAGER_ENUMERATE_SERVICE: u32 = 0x4;
const SC_MANAGER_LOCK: u32 = 0x8;
const SC_MANAGER_QUERY_LOCK_STATUS: u32 = 0x10;
const SC_MANAGER_MODIFY_BOOT_CONFIG: u32 = 0x20;
const SC_MANAGER_ALL_ACCESS: u32 = 0xF003F;
const SERVICE_QUERY_CONFIG: u32 = 0x1;
const SERVICE_CHANGE_CONFIG: u32 = 0x2;
const SERVICE_QUERY_STATUS: u32 = 0x4;
const SERVICE_ENUMERATE_DEPENDENTS: u32 = 0x8;
const SERVICE_START: u32 = 0x10;
const SERVICE_STOP: u32 = 0x20;
const SERVICE_PAUSE_CONTINUE: u32 = 0x40;
const SERVICE_INTERROGATE: u32 = 0x80;
const SERVICE_USER_DEFINED_CONTROL: u32 = 0x100;
const SERVICE_ALL_ACCESS: u32 = 0xF01FF;
const DELETE: u32 = 0x10000;
const READ_CONTROL: u32 = 0x20000;
const WRITE_DAC: u32 = 0x40000;
const WRITE_OWNER: u32 = 0x80000;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const SERVICE_KERNEL_DRIVER: u32 = 0x1;
const SERVICE_WIN32_OWN_PROCESS: u32 = 0x10;
const SERVICE_WIN32_SHARE_PROCESS: u32 = 0x20;
const SERVICE_WIN32: u32 = 0x30;
const SERVICE_INTERACTIVE_PROCESS: u32 = 0x100;
const SERVICE_BOOT_START: u32 = 0;
const SERVICE_SYSTEM_START: u32 = 1;
const SERVICE_AUTO_START: u32 = 2;
const SERVICE_DEMAND_START: u32 = 3;
const SERVICE_DISABLED: u32 = 4;
const SERVICE_ERROR_IGNORE: u32 = 0;
const SERVICE_ERROR_NORMAL: u32 = 1;
const SERVICE_ERROR_SEVERE: u32 = 2;
const SERVICE_ERROR_CRITICAL: u32 = 3;
const SERVICE_NO_CHANGE: u32 = 0xFFFF_FFFF;
const SERVICE_STOPPED: u32 = 1;
const SERVICE_START_PENDING: u32 = 2;
const SERVICE_STOP_PENDING: u32 = 3;
const SERVICE_RUNNING: u32 = 4;
const SERVICE_CONTINUE_PENDING: u32 = 5;
const SERVICE_PAUSE_PENDING: u32 = 6;
const SERVICE_PAUSED: u32 = 7;
const SERVICE_ACCEPT_STOP: u32 = 0x1;
const SERVICE_ACCEPT_PAUSE_CONTINUE: u32 = 0x2;
const SERVICE_ACCEPT_SHUTDOWN: u32 = 0x4;
const SERVICE_CONTROL_STOP: u32 = 1;
const SERVICE_CONTROL_PAUSE: u32 = 2;
const SERVICE_CONTROL_CONTINUE: u32 = 3;
const SERVICE_CONTROL_INTERROGATE: u32 = 4;
const SERVICE_CONTROL_SHUTDOWN: u32 = 5;
const SERVICE_ACTIVE: u32 = 1;
const SERVICE_INACTIVE: u32 = 2;
const SERVICE_STATE_ALL: u32 = 3;
const SC_STATUS_PROCESS_INFO: u32 = 0;
const SC_ENUM_PROCESS_INFO: u32 = 0;
const SERVICE_CONFIG_DESCRIPTION: u32 = 1;
const SERVICE_CONFIG_FAILURE_ACTIONS: u32 = 2;
const SERVICE_CONFIG_DELAYED_AUTO_START_INFO: u32 = 3;
const SERVICE_CONFIG_FAILURE_ACTIONS_FLAG: u32 = 4;
const SERVICE_CONFIG_SERVICE_SID_INFO: u32 = 5;
const SERVICE_CONFIG_REQUIRED_PRIVILEGES_INFO: u32 = 6;
const SERVICE_CONFIG_PRESHUTDOWN_INFO: u32 = 7;
const SC_ACTION_NONE: u32 = 0;
const SC_ACTION_RESTART: u32 = 1;
const SC_ACTION_REBOOT: u32 = 2;
const SC_ACTION_RUN_COMMAND: u32 = 3;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_MORE_DATA: u32 = 234;
const INFINITE: u32 = 0xFFFF_FFFF;
const DACL_SECURITY_INFORMATION: u32 = 4;
const OWNER_SECURITY_INFORMATION: u32 = 1;
const GROUP_SECURITY_INFORMATION: u32 = 2;
const KEY_READ: u32 = 0x20019;
const HKEY_LOCAL_MACHINE: usize = 0x8000_0002;
const INVALID_HANDLE_VALUE: SH = -1isize as SH;
/// A value no call sets as last error: "unchanged" when it is still there afterwards.
const MARK: u32 = 0xDEAD;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SERVICE_STATUS {
    ty: u32,
    state: u32,
    accepted: u32,
    win32: u32,
    specific: u32,
    check: u32,
    hint: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SERVICE_STATUS_PROCESS {
    ty: u32,
    state: u32,
    accepted: u32,
    win32: u32,
    specific: u32,
    check: u32,
    hint: u32,
    pid: u32,
    flags: u32,
}

#[repr(C)]
struct ENUM_SERVICE_STATUS_PROCESSW {
    name: *const u16,
    display: *const u16,
    status: SERVICE_STATUS_PROCESS,
}

#[repr(C)]
struct SERVICE_TABLE_ENTRYW {
    name: *const u16,
    proc: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
}

#[repr(C)]
struct SC_ACTION {
    ty: u32,
    delay: u32,
}

#[repr(C)]
struct SERVICE_FAILURE_ACTIONSW {
    reset_period: u32,
    reboot_msg: *mut u16,
    command: *mut u16,
    actions: u32,
    sc_actions: *mut SC_ACTION,
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    security_descriptor: *mut c_void,
    inherit_handle: BOOL,
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenSCManagerW(machine: *const u16, database: *const u16, access: u32) -> SH;
    fn OpenServiceW(scm: SH, name: *const u16, access: u32) -> SH;
    fn CreateServiceW(
        scm: SH,
        name: *const u16,
        display: *const u16,
        access: u32,
        ty: u32,
        start: u32,
        error: u32,
        path: *const u16,
        group: *const u16,
        tag: *mut u32,
        deps: *const u16,
        account: *const u16,
        password: *const u16,
    ) -> SH;
    fn DeleteService(h: SH) -> BOOL;
    fn CloseServiceHandle(h: SH) -> BOOL;
    fn ChangeServiceConfigW(
        h: SH,
        ty: u32,
        start: u32,
        error: u32,
        path: *const u16,
        group: *const u16,
        tag: *mut u32,
        deps: *const u16,
        account: *const u16,
        password: *const u16,
        display: *const u16,
    ) -> BOOL;
    fn ChangeServiceConfig2W(h: SH, level: u32, info: *const c_void) -> BOOL;
    fn QueryServiceConfigW(h: SH, buf: *mut u8, n: u32, needed: *mut u32) -> BOOL;
    fn QueryServiceConfig2W(h: SH, level: u32, buf: *mut u8, n: u32, needed: *mut u32) -> BOOL;
    fn QueryServiceStatusEx(h: SH, level: u32, buf: *mut u8, n: u32, needed: *mut u32) -> BOOL;
    fn QueryServiceStatus(h: SH, st: *mut SERVICE_STATUS) -> BOOL;
    fn StartServiceW(h: SH, argc: u32, argv: *const *const u16) -> BOOL;
    fn ControlService(h: SH, control: u32, st: *mut SERVICE_STATUS) -> BOOL;
    fn EnumServicesStatusExW(
        scm: SH,
        level: u32,
        ty: u32,
        state: u32,
        buf: *mut u8,
        n: u32,
        needed: *mut u32,
        count: *mut u32,
        resume: *mut u32,
        group: *const u16,
    ) -> BOOL;
    fn StartServiceCtrlDispatcherW(table: *const SERVICE_TABLE_ENTRYW) -> BOOL;
    fn RegisterServiceCtrlHandlerW(name: *const u16, handler: Option<unsafe extern "system" fn(u32)>) -> SH;
    fn RegisterServiceCtrlHandlerExW(
        name: *const u16,
        handler: Option<unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32>,
        ctx: *mut c_void,
    ) -> SH;
    fn SetServiceStatus(h: SH, st: *const SERVICE_STATUS) -> BOOL;
    fn QueryServiceObjectSecurity(h: SH, info: u32, sd: *mut u8, n: u32, needed: *mut u32) -> BOOL;
    fn SetServiceObjectSecurity(h: SH, info: u32, sd: *const u8) -> BOOL;
    fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
        sd: *const u8,
        rev: u32,
        info: u32,
        out: *mut *mut u16,
        len: *mut u32,
    ) -> BOOL;
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        rev: u32,
        sd: *mut *mut u8,
        len: *mut u32,
    ) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut *mut c_void) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn RegOpenKeyExW(key: usize, sub: *const u16, opts: u32, access: u32, out: *mut usize) -> i32;
    fn RegEnumValueW(
        key: usize,
        idx: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        ty: *mut u32,
        data: *mut u8,
        data_len: *mut u32,
    ) -> i32;
    fn RegEnumKeyExW(
        key: usize,
        idx: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_len: *mut u32,
        time: *mut u64,
    ) -> i32;
    fn RegCloseKey(key: usize) -> i32;
    fn OpenProcessToken(process: SH, access: u32, token: *mut SH) -> BOOL;
    fn GetTokenInformation(token: SH, class: u32, buf: *mut u8, n: u32, needed: *mut u32) -> BOOL;
    fn CreateRestrictedToken(
        token: SH,
        flags: u32,
        n_disable: u32,
        disable: *const [*mut c_void; 2],
        n_del: u32,
        del: *const c_void,
        n_restrict: u32,
        restrict: *const c_void,
        out: *mut SH,
    ) -> BOOL;
    fn DuplicateTokenEx(
        token: SH,
        access: u32,
        sa: *const SECURITY_ATTRIBUTES,
        level: u32,
        ty: u32,
        out: *mut SH,
    ) -> BOOL;
    fn SetThreadToken(thread: *const SH, token: SH) -> BOOL;
    fn RevertToSelf() -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: SH) -> BOOL;
    fn GetCurrentProcess() -> SH;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcessId() -> u32;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn Sleep(ms: u32);
    fn GetTickCount64() -> u64;
    fn CreateEventW(sa: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> SH;
    fn SetEvent(h: SH) -> BOOL;
    fn WaitForSingleObject(h: SH, ms: u32) -> u32;
    fn GetCommandLineW() -> *const u16;
    fn GetModuleFileNameW(module: SH, buf: *mut u16, n: u32) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: *const SECURITY_ATTRIBUTES,
        disp: u32,
        flags: u32,
        template: SH,
    ) -> SH;
    fn WriteFile(h: SH, buf: *const u8, n: u32, put: *mut u32, ov: *mut c_void) -> BOOL;
    fn ReadFile(h: SH, buf: *mut u8, n: u32, got: *mut u32, ov: *mut c_void) -> BOOL;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> SH;
    fn TerminateProcess(h: SH, code: u32) -> BOOL;
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn GetComputerNameW(buf: *mut u16, n: *mut u32) -> BOOL;
    fn ExitProcess(code: u32) -> !;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetProcessWindowStation() -> SH;
    fn GetThreadDesktop(tid: u32) -> SH;
    fn GetUserObjectInformationW(h: SH, index: i32, info: *mut c_void, len: u32, needed: *mut u32) -> BOOL;
}

// ------------------------------------------------------------------ small helpers

fn last_error() -> u32 {
    // SAFETY: no arguments.
    unsafe { GetLastError() }
}

fn mark_error() {
    // SAFETY: sets this thread's last error.
    unsafe { SetLastError(MARK) };
}

fn sleep(ms: u32) {
    // SAFETY: plain call.
    unsafe { Sleep(ms) };
}

fn now_ms() -> u64 {
    // SAFETY: no arguments.
    unsafe { GetTickCount64() }
}

fn close_h(h: SH) {
    if !h.is_null() && h != INVALID_HANDLE_VALUE {
        // SAFETY: a handle this test opened, closed once.
        unsafe { CloseHandle(h) };
    }
}

/// `ok` or `err <n>`: the outcome of one call (the last error is read only after a failure; after a
/// success it is not defined).
fn res(ok: bool, e: u32) -> Buf<24> {
    let mut b = Buf::<24>::new();
    if ok {
        let _ = write!(b, "ok");
    } else {
        let _ = write!(b, "err {e}");
    }
    b
}

fn res_h(h: SH, e: u32) -> Buf<24> {
    res(!h.is_null(), e)
}

/// 1 for a usable handle, else 0.
fn opened(h: SH) -> u32 {
    u32::from(!h.is_null() && h != INVALID_HANDLE_VALUE)
}

fn p(w: &[u16]) -> *const u16 {
    w.as_ptr()
}

// ------------------------------------------------------------------ the service control manager

fn scm_open(access: u32) -> (SH, u32) {
    mark_error();
    // SAFETY: NULL machine and database mean the local active database.
    let h = unsafe { OpenSCManagerW(core::ptr::null(), core::ptr::null(), access) };
    (h, last_error())
}

fn scm_open_named(machine: Option<&str>, db: Option<&str>, access: u32) -> (SH, u32) {
    let m = machine.map(wide);
    let d = db.map(wide);
    mark_error();
    // SAFETY: the buffers are NUL-terminated and live for the call.
    let h = unsafe {
        OpenSCManagerW(
            m.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            d.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            access,
        )
    };
    (h, last_error())
}

fn svc_open(scm: SH, name: &str, access: u32) -> (SH, u32) {
    let n = wide(name);
    mark_error();
    // SAFETY: NUL-terminated name.
    let h = unsafe { OpenServiceW(scm, n.as_ptr(), access) };
    (h, last_error())
}

fn svc_open_raw(scm: SH, name: *const u16, access: u32) -> (SH, u32) {
    mark_error();
    // SAFETY: the caller passes NULL or a NUL-terminated name.
    let h = unsafe { OpenServiceW(scm, name, access) };
    (h, last_error())
}

/// What `CreateServiceW` is given (None = NULL).
struct Spec<'a> {
    name: Option<&'a str>,
    display: Option<&'a str>,
    access: u32,
    ty: u32,
    start: u32,
    error: u32,
    path: Option<&'a str>,
    deps: Option<&'a [u16]>,
    account: Option<&'a str>,
    password: Option<&'a str>,
}

impl<'a> Spec<'a> {
    fn new(name: &'a str, path: &'a str) -> Spec<'a> {
        Spec {
            name: Some(name),
            display: None,
            access: SERVICE_ALL_ACCESS,
            ty: SERVICE_WIN32_OWN_PROCESS,
            start: SERVICE_DEMAND_START,
            error: SERVICE_ERROR_NORMAL,
            path: Some(path),
            deps: None,
            account: None,
            password: None,
        }
    }
}

fn svc_create(scm: SH, s: &Spec) -> (SH, u32) {
    let n = s.name.map(wide);
    let d = s.display.map(wide);
    let pa = s.path.map(wide);
    let ac = s.account.map(wide);
    let pw = s.password.map(wide);
    mark_error();
    // SAFETY: every pointer is NULL or a NUL-terminated buffer that lives for the call.
    let h = unsafe {
        CreateServiceW(
            scm,
            n.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            d.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            s.access,
            s.ty,
            s.start,
            s.error,
            pa.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            core::ptr::null(),
            core::ptr::null_mut(),
            s.deps.map_or(core::ptr::null(), |x| x.as_ptr()),
            ac.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            pw.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
        )
    };
    (h, last_error())
}

/// Create an own-process service of this image, started with `--child <mode>`.
fn svc_create_self(scm: SH, name: &str, mode: &str, start: u32) -> (SH, u32) {
    let cmd = self_cmd(mode);
    let mut s = Spec::new(name, cmd.as_str());
    s.start = start;
    svc_create(scm, &s)
}

/// `"<image path>" --child <mode>`, as ASCII text (the service's ImagePath; a test's image path is ASCII).
fn self_cmd(mode: &str) -> Buf<700> {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 599) } as usize;
    let mut cmd = Buf::<700>::new();
    let _ = write!(cmd, "\"{}\" --child {mode}", utext::<650>(&exe[..n]).as_str());
    cmd
}

fn svc_delete(h: SH) -> (bool, u32) {
    mark_error();
    // SAFETY: a service handle this test opened.
    let r = unsafe { DeleteService(h) };
    (r != 0, last_error())
}

fn svc_close(h: SH) -> (bool, u32) {
    mark_error();
    // SAFETY: closing a handle (or garbage on purpose: the call validates its argument).
    let r = unsafe { CloseServiceHandle(h) };
    (r != 0, last_error())
}

fn svc_start(h: SH, args: &[&str]) -> (bool, u32) {
    let w: [[u16; 100]; 4] = core::array::from_fn(|i| {
        let mut x = [0u16; 100];
        if let Some(s) = args.get(i) {
            for (j, c) in s.bytes().enumerate().take(99) {
                x[j] = u16::from(c);
            }
        }
        x
    });
    let ptrs: [*const u16; 4] = core::array::from_fn(|i| w[i].as_ptr());
    mark_error();
    // SAFETY: `ptrs` holds `args.len()` pointers to NUL-terminated strings that live for the call.
    let r = unsafe { StartServiceW(h, args.len() as u32, if args.is_empty() { core::ptr::null() } else { ptrs.as_ptr() }) };
    (r != 0, last_error())
}

fn svc_control(h: SH, code: u32) -> (bool, u32, SERVICE_STATUS) {
    let mut st = SERVICE_STATUS::default();
    mark_error();
    // SAFETY: `st` is a live out structure.
    let r = unsafe { ControlService(h, code, &mut st) };
    (r != 0, last_error(), st)
}

fn status_ex(h: SH) -> Result<SERVICE_STATUS_PROCESS, u32> {
    let mut st = SERVICE_STATUS_PROCESS::default();
    let mut need = 0u32;
    mark_error();
    // SAFETY: `st` is 36 bytes.
    let r = unsafe { QueryServiceStatusEx(h, SC_STATUS_PROCESS_INFO, &mut st as *mut _ as *mut u8, 36, &mut need) };
    if r != 0 {
        Ok(st)
    } else {
        Err(last_error())
    }
}

fn status_plain(h: SH) -> Result<SERVICE_STATUS, u32> {
    let mut st = SERVICE_STATUS::default();
    mark_error();
    // SAFETY: `st` is a live out structure.
    let r = unsafe { QueryServiceStatus(h, &mut st) };
    if r != 0 {
        Ok(st)
    } else {
        Err(last_error())
    }
}

/// Poll until the service is in `state` (50 ms steps, at most `ms`).
fn wait_state(h: SH, state: u32, ms: u32) -> bool {
    let t0 = now_ms();
    loop {
        if let Ok(s) = status_ex(h) {
            if s.state == state {
                return true;
            }
        }
        if now_ms() - t0 >= u64::from(ms) {
            return false;
        }
        sleep(50);
    }
}

fn state_name(s: u32) -> &'static str {
    match s {
        1 => "STOPPED",
        2 => "START_PENDING",
        3 => "STOP_PENDING",
        4 => "RUNNING",
        5 => "CONTINUE_PENDING",
        6 => "PAUSE_PENDING",
        7 => "PAUSED",
        _ => "?",
    }
}

/// One status line: state, accepted controls, exit codes, checkpoint and wait hint (never the process id:
/// only whether there is one).
fn status_line(prefix: &str, s: &SERVICE_STATUS_PROCESS) -> Buf<300> {
    let mut b = Buf::<300>::new();
    let _ = write!(
        b,
        "{prefix}type={:#x} state={} accepted={:#x} win32={} specific={} checkpoint={} hint={} has_pid={} flags={}",
        s.ty,
        state_name(s.state),
        s.accepted,
        s.win32,
        s.specific,
        s.check,
        s.hint,
        u32::from(s.pid != 0),
        s.flags
    );
    b
}

/// End a test service whatever state it is in: stop it, end its process if it will not stop, delete it,
/// and wait (2 s) for the registry key to go.
fn nuke(scm: SH, name: &str) {
    let (h, _) = svc_open(scm, name, SERVICE_ALL_ACCESS);
    if h.is_null() {
        return;
    }
    if let Ok(s) = status_ex(h) {
        if s.state != SERVICE_STOPPED {
            let _ = svc_control(h, SERVICE_CONTROL_STOP);
            if !wait_state(h, SERVICE_STOPPED, 1500) {
                if let Ok(s2) = status_ex(h) {
                    if s2.pid != 0 {
                        // SAFETY: PROCESS_TERMINATE on the service's own process, closed after use.
                        let ph = unsafe { OpenProcess(0x1, 0, s2.pid) };
                        if !ph.is_null() {
                            // SAFETY: a process handle this test opened.
                            unsafe { TerminateProcess(ph, 0xDEAD) };
                            close_h(ph);
                        }
                        let _ = wait_state(h, SERVICE_STOPPED, 3000);
                    }
                }
            }
        }
    }
    let _ = svc_delete(h);
    let _ = svc_close(h);
    let t0 = now_ms();
    while key_exists(name) && now_ms() - t0 < 2000 {
        sleep(50);
    }
}

// ------------------------------------------------------------------ the registry under Services

fn svc_key_path(name: &str) -> [u16; 300] {
    let mut s = Buf::<280>::new();
    let _ = write!(s, "SYSTEM\\CurrentControlSet\\Services\\{name}");
    wide(s.as_str())
}

fn key_exists(name: &str) -> bool {
    let k = svc_key_path(name);
    let mut h = 0usize;
    // SAFETY: NUL-terminated key path, a live out parameter.
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, k.as_ptr(), 0, KEY_READ, &mut h) };
    if r == 0 {
        // SAFETY: the key just opened.
        unsafe { RegCloseKey(h) };
        true
    } else {
        false
    }
}

/// The values of `HKLM\SYSTEM\CurrentControlSet\Services\<name>` as sorted lines `name type data`
/// (a DWORD as a number, a string as text, a multi-string as `a|b`, binary as hex), then its subkeys.
fn dump_key(t: &mut Test, label: &str, name: &str) {
    let k = svc_key_path(name);
    let mut h = 0usize;
    // SAFETY: NUL-terminated key path, a live out parameter.
    let r = unsafe { RegOpenKeyExW(HKEY_LOCAL_MACHINE, k.as_ptr(), 0, KEY_READ, &mut h) };
    if r != 0 {
        say!(t, "{label}: key not present (RegOpenKeyExW {r})");
        return;
    }
    let mut lines: [Buf<300>; 24] = core::array::from_fn(|_| Buf::<300>::new());
    let mut n = 0;
    let mut i = 0u32;
    loop {
        let mut nm = [0u16; 128];
        let mut nl = 128u32;
        let mut ty = 0u32;
        let mut data = [0u8; 400];
        let mut dl = 400u32;
        // SAFETY: buffers of the lengths passed.
        let r = unsafe {
            RegEnumValueW(h, i, nm.as_mut_ptr(), &mut nl, core::ptr::null_mut(), &mut ty, data.as_mut_ptr(), &mut dl)
        };
        if r != 0 {
            break;
        }
        i += 1;
        if n >= 24 {
            continue;
        }
        let b = &mut lines[n];
        n += 1;
        let _ = write!(b, "{} type={} ", utext::<100>(&nm[..nl as usize]).as_str(), ty);
        match ty {
            4 if dl >= 4 => {
                let _ = write!(b, "{}", u32::from_le_bytes([data[0], data[1], data[2], data[3]]));
            }
            1 | 2 => {
                let units = (dl as usize / 2).min(200);
                let mut tmp = [0u16; 200];
                for j in 0..units {
                    tmp[j] = u16::from_le_bytes([data[j * 2], data[j * 2 + 1]]);
                }
                let mut end = units;
                while end > 0 && tmp[end - 1] == 0 {
                    end -= 1;
                }
                let _ = write!(b, "{}", utext_scrub::<250>(&tmp[..end]).as_str());
            }
            7 => {
                let units = (dl as usize / 2).min(200);
                let mut tmp = [0u16; 200];
                for j in 0..units {
                    tmp[j] = u16::from_le_bytes([data[j * 2], data[j * 2 + 1]]);
                }
                let mut end = units;
                while end > 0 && tmp[end - 1] == 0 {
                    end -= 1;
                }
                for u in &mut tmp[..end] {
                    if *u == 0 {
                        *u = u16::from(b'|');
                    }
                }
                let _ = write!(b, "{}", utext_scrub::<250>(&tmp[..end]).as_str());
            }
            _ => {
                for j in 0..(dl as usize).min(100) {
                    let _ = write!(b, "{:02x}", data[j]);
                }
            }
        }
    }
    // insertion sort of the lines, so the order of the registry's enumeration does not matter
    for a in 1..n {
        let mut j = a;
        while j > 0 && lines[j - 1].as_str() > lines[j].as_str() {
            lines.swap(j - 1, j);
            j -= 1;
        }
    }
    for l in lines.iter().take(n) {
        say!(t, "{label}: value {}", l.as_str());
    }
    let mut subs = Buf::<200>::new();
    let mut i = 0u32;
    loop {
        let mut nm = [0u16; 128];
        let mut nl = 128u32;
        // SAFETY: buffers of the lengths passed; NULL for the unused out parameters.
        let r = unsafe {
            RegEnumKeyExW(
                h,
                i,
                nm.as_mut_ptr(),
                &mut nl,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        };
        if r != 0 {
            break;
        }
        i += 1;
        let _ = write!(subs, "{} ", utext::<100>(&nm[..nl as usize]).as_str());
    }
    say!(t, "{label}: subkeys [{}]", subs.as_str());
    // SAFETY: the key opened above.
    unsafe { RegCloseKey(h) };
}

// ------------------------------------------------------------------ QueryServiceConfig

struct Config {
    ty: u32,
    start: u32,
    error: u32,
    path: Buf<400>,
    group: Buf<100>,
    tag: u32,
    deps: Buf<200>,
    account: Buf<100>,
    display: Buf<400>,
}

fn multisz(p: *const u16) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if p.is_null() {
        let _ = write!(b, "NULL");
        return b;
    }
    let mut off = 0usize;
    // SAFETY: a double-NUL-terminated list the system returned; the scan is bounded.
    unsafe {
        while off < 190 && *p.add(off) != 0 {
            let s = wstr_at(p.add(off), 150);
            let _ = write!(b, "{}|", utext_scrub::<150>(s).as_str());
            off += s.len() + 1;
        }
    }
    b
}

/// `QueryServiceConfigW` in two calls (size, then data): the size the first call reports, the result.
fn query_config(h: SH) -> (Result<Config, u32>, u32, u32) {
    let mut need = 0u32;
    mark_error();
    // SAFETY: a zero-length buffer asks for the size.
    let r0 = unsafe { QueryServiceConfigW(h, core::ptr::null_mut(), 0, &mut need) };
    let e0 = if r0 != 0 { 0 } else { last_error() };
    if e0 != ERROR_INSUFFICIENT_BUFFER {
        return (Err(e0), e0, need);
    }
    let mut buf = [0u64; 400]; // 3200 bytes, 8-aligned
    let mut need2 = 0u32;
    mark_error();
    // SAFETY: `buf` holds 3200 bytes; the call is told the size it may use.
    let r = unsafe { QueryServiceConfigW(h, buf.as_mut_ptr() as *mut u8, need.min(3200), &mut need2) };
    if r == 0 {
        return (Err(last_error()), e0, need);
    }
    // SAFETY: QUERY_SERVICE_CONFIGW (winsvc.h): three DWORDs, then pointers into the same buffer; the layout
    // is read field by field at its x64 offsets.
    let c = unsafe {
        let b = buf.as_ptr() as *const u8;
        let rd32 = |o: usize| (b.add(o) as *const u32).read_unaligned();
        let rdp = |o: usize| (b.add(o) as *const *const u16).read_unaligned();
        Config {
            ty: rd32(0),
            start: rd32(4),
            error: rd32(8),
            path: ptext(rdp(16)),
            group: {
                let mut x = Buf::<100>::new();
                let _ = write!(x, "{}", utext_scrub::<90>(wstr_at(rdp(24), 90)).as_str());
                x
            },
            tag: rd32(32),
            deps: multisz(rdp(40)),
            account: {
                let mut x = Buf::<100>::new();
                let _ = write!(x, "{}", utext::<90>(wstr_at(rdp(48), 90)).as_str());
                x
            },
            display: ptext(rdp(56)),
        }
    };
    (Ok(c), e0, need)
}

fn config_line(prefix: &str, c: &Config) -> Buf<900> {
    let mut b = Buf::<900>::new();
    let _ = write!(
        b,
        "{prefix}type={:#x} start={} error={} path=[{}] group=[{}] tag={} deps=[{}] account=[{}] display=[{}]",
        c.ty,
        c.start,
        c.error,
        c.path.as_str(),
        c.group.as_str(),
        c.tag,
        c.deps.as_str(),
        c.account.as_str(),
        c.display.as_str()
    );
    b
}

/// `ChangeServiceConfigW` with only the named fields (SERVICE_NO_CHANGE / NULL for the others).
struct Change<'a> {
    ty: u32,
    start: u32,
    error: u32,
    path: Option<&'a str>,
    deps: Option<&'a [u16]>,
    account: Option<&'a str>,
    password: Option<&'a str>,
    display: Option<&'a str>,
}

impl<'a> Change<'a> {
    fn none() -> Change<'a> {
        Change {
            ty: SERVICE_NO_CHANGE,
            start: SERVICE_NO_CHANGE,
            error: SERVICE_NO_CHANGE,
            path: None,
            deps: None,
            account: None,
            password: None,
            display: None,
        }
    }
}

fn svc_change(h: SH, c: &Change) -> (bool, u32) {
    let pa = c.path.map(wide);
    let ac = c.account.map(wide);
    let pw = c.password.map(wide);
    let dn = c.display.map(wide);
    mark_error();
    // SAFETY: every pointer is NULL or a NUL-terminated buffer that lives for the call.
    let r = unsafe {
        ChangeServiceConfigW(
            h,
            c.ty,
            c.start,
            c.error,
            pa.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            core::ptr::null(),
            core::ptr::null_mut(),
            c.deps.map_or(core::ptr::null(), |x| x.as_ptr()),
            ac.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            pw.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
            dn.as_ref().map_or(core::ptr::null(), |x| x.as_ptr()),
        )
    };
    (r != 0, last_error())
}

fn change2(h: SH, level: u32, info: *const c_void) -> (bool, u32) {
    mark_error();
    // SAFETY: the caller passes the structure of that level (or NULL).
    let r = unsafe { ChangeServiceConfig2W(h, level, info) };
    (r != 0, last_error())
}

fn change2_u32(h: SH, level: u32, v: u32) -> (bool, u32) {
    let x = v;
    change2(h, level, &x as *const u32 as *const c_void)
}

fn change2_desc(h: SH, text: Option<&str>) -> (bool, u32) {
    let mut w = wide(text.unwrap_or(""));
    let ptr: *mut u16 = if text.is_some() { w.as_mut_ptr() } else { core::ptr::null_mut() };
    change2(h, SERVICE_CONFIG_DESCRIPTION, &ptr as *const *mut u16 as *const c_void)
}

fn change2_failure(h: SH, reset: u32, acts: &mut [SC_ACTION]) -> (bool, u32) {
    let f = SERVICE_FAILURE_ACTIONSW {
        reset_period: reset,
        reboot_msg: core::ptr::null_mut(),
        command: core::ptr::null_mut(),
        actions: acts.len() as u32,
        sc_actions: if acts.is_empty() { core::ptr::null_mut() } else { acts.as_mut_ptr() },
    };
    change2(h, SERVICE_CONFIG_FAILURE_ACTIONS, &f as *const _ as *const c_void)
}

/// `QueryServiceConfig2W` in two calls: (first call's error and size, second call's result, the text of the data).
fn query_config2(h: SH, level: u32) -> (u32, u32, Result<Buf<300>, u32>) {
    let mut need = 0u32;
    mark_error();
    // SAFETY: a zero-length buffer asks for the size.
    let r0 = unsafe { QueryServiceConfig2W(h, level, core::ptr::null_mut(), 0, &mut need) };
    let e0 = if r0 != 0 { 0 } else { last_error() };
    if e0 != ERROR_INSUFFICIENT_BUFFER {
        return (e0, need, Err(e0));
    }
    let mut buf = [0u64; 512];
    let mut need2 = 0u32;
    mark_error();
    // SAFETY: `buf` holds 4096 bytes; the call is told the size it may use.
    let r = unsafe { QueryServiceConfig2W(h, level, buf.as_mut_ptr() as *mut u8, need.min(4096), &mut need2) };
    if r == 0 {
        return (e0, need, Err(last_error()));
    }
    let b = buf.as_ptr() as *const u8;
    let mut o = Buf::<300>::new();
    // SAFETY: the structures of winsvc.h at their x64 offsets, read from the buffer filled above.
    unsafe {
        let rd32 = |off: usize| (b.add(off) as *const u32).read_unaligned();
        let rdp = |off: usize| (b.add(off) as *const *const u16).read_unaligned();
        match level {
            1 => {
                let _ = write!(o, "description=[{}]", ptext(rdp(0)).as_str());
            }
            2 => {
                let _ = write!(
                    o,
                    "reset={} reboot_msg=[{}] command=[{}] n={} [",
                    rd32(0),
                    ptext(rdp(8)).as_str(),
                    ptext(rdp(16)).as_str(),
                    rd32(24)
                );
                let arr = (b.add(32) as *const *const u32).read_unaligned();
                for i in 0..rd32(24).min(8) as usize {
                    let _ = write!(o, "{}:{} ", *arr.add(i * 2), *arr.add(i * 2 + 1));
                }
                let _ = write!(o, "]");
            }
            3 | 4 | 5 | 7 => {
                let _ = write!(o, "value={}", rd32(0));
            }
            6 => {
                let _ = write!(o, "privileges=[{}]", multisz(rdp(0)).as_str());
            }
            _ => {
                let _ = write!(o, "(size {need2})");
            }
        }
    }
    (e0, need, Ok(o))
}

// ------------------------------------------------------------------ the service role

static HSTATUS: AtomicUsize = AtomicUsize::new(0);
static STOP_EV: AtomicUsize = AtomicUsize::new(0);
static MODE: AtomicU32 = AtomicU32::new(0);
static LAST_CTRL: AtomicU32 = AtomicU32::new(0);
/// The service's current state, as it last reported it.
static CUR_STATE: AtomicU32 = AtomicU32::new(0);
/// The accepted-controls mask it last reported.
static CUR_ACC: AtomicU32 = AtomicU32::new(0);

const M_RUN: u32 = 0;
const M_RUNEX: u32 = 1;
const M_SLOW: u32 = 2;
const M_PEND: u32 = 3;
const M_CRASH: u32 = 4;
const M_EXITCODE: u32 = 5;
const M_NOACCEPT: u32 = 6;
const M_WHO: u32 = 7;
const M_STOPSLOW: u32 = 8;
const M_PAUSE: u32 = 9;
const M_FAILEXIT: u32 = 10;
const M_CKPT: u32 = 11;
const M_BADSTATE: u32 = 12;
const M_BADNAME: u32 = 13;
const M_NULLNAME: u32 = 14;
const M_EXRET: u32 = 15;

fn mode_of(s: &str) -> Option<u32> {
    Some(match s {
        "run" => M_RUN,
        "runex" => M_RUNEX,
        "slow" => M_SLOW,
        "pend" => M_PEND,
        "crash" => M_CRASH,
        "exitcode" => M_EXITCODE,
        "noaccept" => M_NOACCEPT,
        "who" => M_WHO,
        "stopslow" => M_STOPSLOW,
        "pause" => M_PAUSE,
        "failexit" => M_FAILEXIT,
        "ckpt" => M_CKPT,
        "badstate" => M_BADSTATE,
        "badname" => M_BADNAME,
        "nullname" => M_NULLNAME,
        "exret" => M_EXRET,
        _ => return None,
    })
}

fn report_path() -> [u16; 700] {
    let mut exe = [0u16; 600];
    // SAFETY: the buffer holds 600 units; a NULL module is the executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), exe.as_mut_ptr(), 590) } as usize;
    let mut out = [0u16; 700];
    out[..n].copy_from_slice(&exe[..n]);
    for (i, c) in ".rep".bytes().enumerate() {
        out[n + i] = u16::from(c);
    }
    out
}

/// Append one line to the report file (any process may; a service writes it, the test reads it).
fn log_line(s: &str) {
    let path = report_path();
    // SAFETY: NUL-terminated path; FILE_APPEND_DATA, shared for reading and writing, OPEN_ALWAYS.
    let h = unsafe { CreateFileW(path.as_ptr(), 0x4, 0x3, core::ptr::null(), 4, 0x80, core::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return;
    }
    let mut put = 0u32;
    // SAFETY: writes the bytes of `s` and a newline to the handle just opened.
    unsafe {
        WriteFile(h, s.as_ptr(), s.len() as u32, &mut put, core::ptr::null_mut());
        WriteFile(h, b"\n".as_ptr(), 1, &mut put, core::ptr::null_mut());
    }
    close_h(h);
}

macro_rules! logf {
    ($($a:tt)*) => {{
        let mut b = Buf::<500>::new();
        let _ = write!(b, $($a)*);
        log_line(b.as_str());
    }};
}

fn report_delete() {
    let path = report_path();
    // SAFETY: NUL-terminated path.
    unsafe { DeleteFileW(path.as_ptr()) };
}

/// The lines of the report file, written one per observation as `svc: <line>`.
fn report_show(t: &mut Test) {
    let path = report_path();
    // SAFETY: NUL-terminated path; GENERIC_READ, shared, OPEN_EXISTING.
    let h = unsafe { CreateFileW(path.as_ptr(), 0x8000_0000, 0x3, core::ptr::null(), 3, 0x80, core::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        say!(t, "svc: (no report file)");
        return;
    }
    let mut buf = [0u8; 6000];
    let mut got = 0u32;
    // SAFETY: reads at most 6000 bytes into `buf`.
    unsafe { ReadFile(h, buf.as_mut_ptr(), 6000, &mut got, core::ptr::null_mut()) };
    close_h(h);
    for line in buf[..got as usize].split(|&c| c == b'\n') {
        if line.is_empty() {
            continue;
        }
        let s = core::str::from_utf8(line).unwrap_or("?");
        say!(t, "svc: {s}");
    }
}

fn sid_text(sid: *const c_void) -> Buf<120> {
    let mut o: *mut u16 = core::ptr::null_mut();
    let mut b = Buf::<120>::new();
    // SAFETY: a SID inside a token-information buffer; the string is freed with LocalFree.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut o) != 0 {
            let _ = write!(b, "{}", utext::<110>(wstr_at(o, 110)).as_str());
            LocalFree(o as *mut c_void);
        }
    }
    b
}

/// What the service process is: its token's user, integrity level, session, a few groups, the window
/// station and desktop it runs on.
fn log_identity() {
    let mut tok: SH = core::ptr::null_mut();
    // SAFETY: TOKEN_QUERY of this process.
    if unsafe { OpenProcessToken(GetCurrentProcess(), 0x8, &mut tok) } == 0 {
        logf!("identity: OpenProcessToken failed {}", last_error());
        return;
    }
    let mut buf = [0u64; 256];
    let mut need = 0u32;
    // SAFETY: TokenUser (1) into a 2048-byte buffer.
    if unsafe { GetTokenInformation(tok, 1, buf.as_mut_ptr() as *mut u8, 2048, &mut need) } != 0 {
        // SAFETY: TOKEN_USER starts with a pointer to the SID.
        let sid = unsafe { (buf.as_ptr() as *const *const c_void).read() };
        logf!("identity: user={}", sid_text(sid).as_str());
    }
    let mut sess = 99u32;
    // SAFETY: TokenSessionId (12) is a DWORD.
    unsafe { GetTokenInformation(tok, 12, &mut sess as *mut u32 as *mut u8, 4, &mut need) };
    logf!("identity: session={sess}");
    // SAFETY: TokenIntegrityLevel (25): a SID_AND_ATTRIBUTES.
    if unsafe { GetTokenInformation(tok, 25, buf.as_mut_ptr() as *mut u8, 2048, &mut need) } != 0 {
        // SAFETY: the structure starts with a pointer to the SID.
        let sid = unsafe { (buf.as_ptr() as *const *const c_void).read() };
        logf!("identity: integrity={}", sid_text(sid).as_str());
    }
    // SAFETY: TokenGroups (2): a count, then SID_AND_ATTRIBUTES of 16 bytes each.
    if unsafe { GetTokenInformation(tok, 2, buf.as_mut_ptr() as *mut u8, 2048, &mut need) } != 0 {
        // SAFETY: reads the count and the group entries inside the buffer.
        unsafe {
            let n = (buf.as_ptr() as *const u32).read();
            let base = (buf.as_ptr() as *const u8).add(8);
            let mut has_service = 0;
            let mut has_everyone = 0;
            let mut has_auth = 0;
            for i in 0..n.min(60) as usize {
                let sid = (base.add(i * 16) as *const *const c_void).read();
                let attrs = (base.add(i * 16 + 8) as *const u32).read();
                let s = sid_text(sid);
                if s.as_str() == "S-1-5-6" {
                    has_service = 1;
                }
                if s.as_str() == "S-1-1-0" {
                    has_everyone = 1;
                }
                if s.as_str() == "S-1-5-11" {
                    has_auth = 1;
                }
                if s.as_str().starts_with("S-1-5-80-") {
                    logf!("identity: service-sid-group={} attrs={:#x}", s.as_str(), attrs);
                }
            }
            logf!("identity: groups SERVICE={has_service} Everyone={has_everyone} AuthenticatedUsers={has_auth}");
        }
    }
    close_h(tok);
    // SAFETY: the window station and desktop of this thread; names read into 128-unit buffers.
    unsafe {
        let ws = GetProcessWindowStation();
        let mut nm = [0u16; 128];
        let mut n = 0u32;
        if GetUserObjectInformationW(ws, 2, nm.as_mut_ptr() as *mut c_void, 256, &mut n) != 0 {
            logf!("identity: window-station={}", utext::<200>(wstr_at(nm.as_ptr(), 128)).as_str());
        }
        let mut fl = [0u32; 3];
        if GetUserObjectInformationW(ws, 1, fl.as_mut_ptr() as *mut c_void, 12, &mut n) != 0 {
            logf!("identity: window-station-visible={}", fl[2] & 1);
        }
        let dk = GetThreadDesktop(GetCurrentThreadId());
        let mut nm2 = [0u16; 128];
        if GetUserObjectInformationW(dk, 2, nm2.as_mut_ptr() as *mut c_void, 256, &mut n) != 0 {
            logf!("identity: desktop={}", utext::<200>(wstr_at(nm2.as_ptr(), 128)).as_str());
        }
    }
}

fn set_status(state: u32, accepted: u32, win32: u32, specific: u32, check: u32, hint: u32) -> (bool, u32) {
    let st = SERVICE_STATUS { ty: SERVICE_WIN32_OWN_PROCESS, state, accepted, win32, specific, check, hint };
    mark_error();
    // SAFETY: the handle the dispatcher gave this service, a live structure.
    let r = unsafe { SetServiceStatus(HSTATUS.load(Ordering::SeqCst) as SH, &st) };
    let e = last_error();
    CUR_STATE.store(state, Ordering::SeqCst);
    CUR_ACC.store(accepted, Ordering::SeqCst);
    (r != 0, e)
}

fn stop_event() -> SH {
    STOP_EV.load(Ordering::SeqCst) as SH
}

fn handle_control(ctrl: u32, ex: bool, ev: u32, ctx: usize) -> u32 {
    LAST_CTRL.store(ctrl, Ordering::SeqCst);
    if ex {
        logf!("control {ctrl} eventtype={ev} context-ok={}", u32::from(ctx == 0x5eed));
    } else {
        logf!("control {ctrl}");
    }
    let mode = MODE.load(Ordering::SeqCst);
    match ctrl {
        SERVICE_CONTROL_STOP => {
            if mode == M_STOPSLOW {
                let _ = set_status(SERVICE_STOP_PENDING, 0, 0, 0, 1, 3000);
                sleep(1200);
            } else {
                let _ = set_status(SERVICE_STOP_PENDING, 0, 0, 0, 1, 3000);
            }
            // SAFETY: the event created by the service's main function.
            unsafe { SetEvent(stop_event()) };
        }
        SERVICE_CONTROL_PAUSE if mode == M_PAUSE => {
            let _ = set_status(SERVICE_PAUSED, SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PAUSE_CONTINUE, 0, 0, 0, 0);
        }
        SERVICE_CONTROL_CONTINUE if mode == M_PAUSE => {
            let _ = set_status(SERVICE_RUNNING, SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_PAUSE_CONTINUE, 0, 0, 0, 0);
        }
        SERVICE_CONTROL_INTERROGATE => {
            let cur = CUR_STATE.load(Ordering::SeqCst);
            let _ = set_status(cur, CUR_ACC.load(Ordering::SeqCst), 0, 0, 0, 0);
        }
        _ => {}
    }
    0
}

unsafe extern "system" fn handler(ctrl: u32) {
    handle_control(ctrl, false, 0, 0);
}

unsafe extern "system" fn handler_ex(ctrl: u32, ev: u32, _data: *mut c_void, ctx: *mut c_void) -> u32 {
    handle_control(ctrl, true, ev, ctx as usize);
    if MODE.load(Ordering::SeqCst) == M_EXRET && ctrl >= 128 {
        return 120;
    }
    0
}

/// The service's main function (the dispatcher starts it on a thread of its own).
unsafe extern "system" fn svc_main(argc: u32, argv: *mut *mut u16) {
    let mode = MODE.load(Ordering::SeqCst);
    // SAFETY: argv has `argc` NUL-terminated strings.
    unsafe {
        let mut b = Buf::<400>::new();
        for i in 0..argc.min(6) as usize {
            let _ = write!(b, "[{}]", utext::<80>(wstr_at(*argv.add(i), 80)).as_str());
        }
        logf!("main: argc={argc} argv={}", b.as_str());
    }
    // SAFETY: the name is the first argument; own-process services ignore it, so any text works.
    let name = unsafe { *argv };
    let other = wide("PolySpecSomeOtherName");
    let h = if mode == M_BADNAME {
        // SAFETY: a NUL-terminated name and a handler function of this image.
        unsafe { RegisterServiceCtrlHandlerW(other.as_ptr(), Some(handler)) }
    } else if mode == M_NULLNAME {
        // SAFETY: a NULL name is passed on purpose.
        unsafe { RegisterServiceCtrlHandlerW(core::ptr::null(), Some(handler)) }
    } else if mode == M_RUNEX || mode == M_EXRET {
        // SAFETY: the handler is a function of this image; the context is a plain number.
        unsafe { RegisterServiceCtrlHandlerExW(name, Some(handler_ex), 0x5eed as *mut c_void) }
    } else {
        // SAFETY: the handler is a function of this image.
        unsafe { RegisterServiceCtrlHandlerW(name, Some(handler)) }
    };
    logf!("main: handler registered={}", opened(h));
    HSTATUS.store(h as usize, Ordering::SeqCst);
    // SAFETY: a manual-reset event for the stop request.
    let ev = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    STOP_EV.store(ev as usize, Ordering::SeqCst);
    let acc = if mode == M_NOACCEPT { 0 } else { SERVICE_ACCEPT_STOP };
    match mode {
        M_SLOW => {
            let (r, _) = set_status(SERVICE_START_PENDING, 0, 0, 0, 1, 5000);
            logf!("main: START_PENDING reported={}", u32::from(r));
            sleep(1500);
            let (r, _) = set_status(SERVICE_START_PENDING, 0, 0, 0, 2, 5000);
            logf!("main: START_PENDING 2 reported={}", u32::from(r));
            sleep(500);
        }
        M_PEND => {
            let (r, _) = set_status(SERVICE_START_PENDING, SERVICE_ACCEPT_STOP, 0, 0, 1, 20000);
            logf!("main: START_PENDING reported={}", u32::from(r));
            // SAFETY: waits on the stop event for at most 6 s.
            unsafe { WaitForSingleObject(stop_event(), 6000) };
            logf!("main: reporting STOPPED");
            let _ = set_status(SERVICE_STOPPED, 0, 0, 0, 0, 0);
            return;
        }
        M_EXITCODE => {
            logf!("main: reporting STOPPED with a service-specific error");
            let _ = set_status(SERVICE_STOPPED, 0, 1066, 42, 0, 0);
            return;
        }
        M_FAILEXIT => {
            let (r, _) = set_status(SERVICE_RUNNING, acc, 0, 0, 0, 0);
            logf!("main: RUNNING reported={}", u32::from(r));
            sleep(300);
            logf!("main: reporting STOPPED with exit code 5");
            let _ = set_status(SERVICE_STOPPED, 0, 5, 0, 0, 0);
            return;
        }
        M_BADSTATE => {
            let (r, e) = set_status(99, 0, 0, 0, 0, 0);
            logf!("main: state 99 reported={} err={}", u32::from(r), if r { 0 } else { e });
            let (r, e) = set_status(SERVICE_RUNNING, 0xFFFF_FFFF, 0, 0, 0, 0);
            logf!("main: accepted-all reported={} err={}", u32::from(r), if r { 0 } else { e });
        }
        M_CKPT => {
            for i in 1..=3u32 {
                let (r, _) = set_status(SERVICE_START_PENDING, 0, 0, 0, i, 7000 + i);
                logf!("main: START_PENDING checkpoint {i} reported={}", u32::from(r));
                sleep(300);
            }
        }
        _ => {}
    }
    let acc2 = if mode == M_PAUSE { acc | SERVICE_ACCEPT_PAUSE_CONTINUE } else { acc };
    let (r, _) = set_status(SERVICE_RUNNING, acc2, 0, 0, 0, 0);
    logf!("main: RUNNING reported={}", u32::from(r));
    if mode == M_WHO {
        log_identity();
    }
    if mode == M_CRASH {
        sleep(400);
        logf!("main: crashing now");
        // SAFETY: ends the process without reporting STOPPED, as a crash would.
        unsafe { ExitProcess(3) };
    }
    // SAFETY: waits on the stop event for at most 20 s (a test that forgot to stop the service).
    unsafe { WaitForSingleObject(stop_event(), 20000) };
    logf!("main: stop requested, last control {}", LAST_CTRL.load(Ordering::SeqCst));
    logf!("main: reporting STOPPED");
    let _ = set_status(SERVICE_STOPPED, 0, 0, 0, 0, 0);
}

/// The child role: `--child <mode>` is a service process started by the service control manager.
fn child(args: &str) -> u32 {
    let word = args.split(' ').next().unwrap_or("");
    match word {
        "exit_early" => {
            logf!("child: exiting before the dispatcher");
            return 7;
        }
        "no_dispatch" => {
            logf!("child: not calling the dispatcher, waiting");
            sleep(15000);
            return 0;
        }
        _ => {}
    }
    let Some(m) = mode_of(word) else {
        return 9;
    };
    MODE.store(m, Ordering::SeqCst);
    let name = wide("PolySpecIgnored");
    let table2 = [
        SERVICE_TABLE_ENTRYW { name: name.as_ptr(), proc: Some(svc_main) },
        SERVICE_TABLE_ENTRYW { name: core::ptr::null(), proc: None },
    ];
    // SAFETY: the table is a live array ended by a NULL entry, as winsvc.h describes.
    let ok = unsafe { StartServiceCtrlDispatcherW(table2.as_ptr()) };
    let _ = ok;
    0
}

// ------------------------------------------------------------------ child arguments, entry point

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

#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    if let Some(a) = child_args() {
        exit(child(a.as_str()))
    }
    if open_output().is_err() {
        exit(EXIT_NO_OUTPUT);
    }
    let mut t = Test::new();
    test(&mut t);
    let code = t.finish();
    exit(code)
}

/// The caller's token as a deny-only-Administrators, privilege-less impersonation token on this thread,
/// for `f`; then back to the process token. False when the token could not be made.
fn as_non_admin(f: &mut dyn FnMut()) -> bool {
    let mut tok: SH = core::ptr::null_mut();
    // SAFETY: TOKEN_DUPLICATE | TOKEN_QUERY of this process.
    if unsafe { OpenProcessToken(GetCurrentProcess(), 0x2 | 0x8, &mut tok) } == 0 {
        return false;
    }
    let sid_s = wide("S-1-5-32-544");
    let mut sid: *mut c_void = core::ptr::null_mut();
    // SAFETY: a NUL-terminated SID string; the SID is freed below.
    if unsafe { ConvertStringSidToSidW(sid_s.as_ptr(), &mut sid) } == 0 {
        close_h(tok);
        return false;
    }
    let disable: [*mut c_void; 2] = [sid, core::ptr::null_mut()];
    let mut restricted: SH = core::ptr::null_mut();
    // SAFETY: SID_AND_ATTRIBUTES array of one (SID, attributes 0); DISABLE_MAX_PRIVILEGE (1).
    let ok = unsafe {
        CreateRestrictedToken(tok, 0x1, 1, &disable, 0, core::ptr::null(), 0, core::ptr::null(), &mut restricted)
    };
    // SAFETY: the SID allocated above.
    unsafe { LocalFree(sid) };
    close_h(tok);
    if ok == 0 {
        return false;
    }
    let mut imp: SH = core::ptr::null_mut();
    // SAFETY: a TokenImpersonation (2) copy at SecurityImpersonation (2), full access.
    let ok = unsafe { DuplicateTokenEx(restricted, 0xF01FF, core::ptr::null(), 2, 2, &mut imp) };
    close_h(restricted);
    if ok == 0 {
        return false;
    }
    // SAFETY: the current thread (NULL) takes the impersonation token.
    let set = unsafe { SetThreadToken(core::ptr::null(), imp) };
    if set != 0 {
        f();
        // SAFETY: back to the process token.
        unsafe { RevertToSelf() };
    }
    close_h(imp);
    set != 0
}

fn computer_name() -> [u16; 300] {
    let mut w = [0u16; 300];
    let mut n = 299u32;
    // SAFETY: a 300-unit buffer and its length.
    unsafe { GetComputerNameW(w.as_mut_ptr(), &mut n) };
    w
}

// ------------------------------------------------------------------ shared steps of the tests

fn show_status(t: &mut Test, label: &str, h: SH) {
    match status_ex(h) {
        Ok(s) => say!(t, "{}", status_line(label, &s)),
        Err(e) => say!(t, "{label}QueryServiceStatusEx err {e}"),
    }
}

fn show_config(t: &mut Test, label: &str, h: SH) {
    let (r, _, _) = query_config(h);
    match r {
        Ok(c) => say!(t, "{}", config_line(label, &c)),
        Err(e) => say!(t, "{label}QueryServiceConfigW err {e}"),
    }
}

fn show_config2(t: &mut Test, label: &str, h: SH, level: u32) {
    let (e0, need, r) = query_config2(h, level);
    match r {
        Ok(s) => say!(t, "{label}level {level}: first call err {e0}, size {need}; {}", s),
        Err(e) => say!(t, "{label}level {level}: first call err {e0}, size {need}; err {e}"),
    }
}

/// Start the service and wait (5 s) until it is RUNNING.
fn start_wait(t: &mut Test, h: SH, args: &[&str]) -> bool {
    let (r, e) = svc_start(h, args);
    say!(t, "StartServiceW: {}", res(r, e));
    let up = wait_state(h, SERVICE_RUNNING, 5000);
    say!(t, "RUNNING within 5 s: {}", u32::from(up));
    up
}

/// Stop the service and wait (5 s) until it is STOPPED.
fn stop_wait(t: &mut Test, h: SH) -> bool {
    let (r, e, _) = svc_control(h, SERVICE_CONTROL_STOP);
    say!(t, "ControlService(STOP): {}", res(r, e));
    let down = wait_state(h, SERVICE_STOPPED, 5000);
    say!(t, "STOPPED within 5 s: {}", u32::from(down));
    down
}

/// A service of this image in `mode`, created with the default settings (handle, error).
fn mk(scm: SH, name: &str, mode: &str) -> SH {
    svc_create_self(scm, name, mode, SERVICE_DEMAND_START).0
}

/// The same, as an observation line.
fn mk_say(t: &mut Test, scm: SH, name: &str, mode: &str) -> SH {
    let (h, e) = svc_create_self(scm, name, mode, SERVICE_DEMAND_START);
    say!(t, "CreateServiceW({name}): {}", res_h(h, e));
    h
}

fn end(h: SH) {
    let _ = svc_delete(h);
    let _ = svc_close(h);
}

/// How many lines of the report file contain `needle`.
fn report_count(needle: &str) -> u32 {
    let path = report_path();
    // SAFETY: NUL-terminated path; GENERIC_READ, shared, OPEN_EXISTING.
    let h = unsafe { CreateFileW(path.as_ptr(), 0x8000_0000, 0x3, core::ptr::null(), 3, 0x80, core::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        return 0;
    }
    let mut buf = [0u8; 6000];
    let mut got = 0u32;
    // SAFETY: reads at most 6000 bytes into `buf`.
    unsafe { ReadFile(h, buf.as_mut_ptr(), 6000, &mut got, core::ptr::null_mut()) };
    close_h(h);
    let mut n = 0;
    for line in buf[..got as usize].split(|&c| c == b'\n') {
        if let Ok(s) = core::str::from_utf8(line) {
            if s.contains(needle) {
                n += 1;
            }
        }
    }
    n
}


fn test(t: &mut Test) {
    let (scm, e) = scm_open(SC_MANAGER_ALL_ACCESS);
    if scm.is_null() {
        say!(t, "OpenSCManagerW(SC_MANAGER_ALL_ACCESS): err {e}");
        return;
    }
    nuke(scm, "PolySpecSvcErrA");
    nuke(scm, "PolySpecSvcErrB");
    report_delete();
    body(t, scm);
    nuke(scm, "PolySpecSvcErrA");
    nuke(scm, "PolySpecSvcErrB");
    report_delete();
    let _ = svc_close(scm);
}

fn body(t: &mut Test, scm: SH) {
    const P: &str = "C:\\poly-spec-win-services\\none.exe";
    let a = mk_say(t, scm, "PolySpecSvcErrA", "run");
    let mut one = |t: &mut Test, label: &str, s: &Spec| {
        let (h, e) = svc_create(scm, s);
        say!(t, "CreateServiceW({label}): {}", res_h(h, e));
        if !h.is_null() {
            end(h);
            // the key goes when the last handle is closed, a moment later
            let t0 = now_ms();
            while key_exists(s.name.unwrap_or("")) && now_ms() - t0 < 2000 {
                sleep(20);
            }
        }
    };
    let mut s = Spec::new("PolySpecSvcErrA", P);
    one(t, "name exists", &s);
    s.name = Some("polyspecsvcerra");
    one(t, "name exists in other case", &s);
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.display = Some("PolySpecSvcErrA");
    one(t, "display name = the name of another service", &s);
    s.display = Some("polyspecsvcerra");
    one(t, "display name = that name in other case", &s);
    s.display = Some("Some display name");
    one(t, "good", &s);
    for (label, name) in [("name with backslash", "PolySpec\\Bad"), ("name with slash", "PolySpec/Bad"), ("empty name", "")] {
        let mut s = Spec::new(name, P);
        s.name = Some(name);
        one(t, label, &s);
    }
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.name = None;
    one(t, "NULL name", &s);
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.path = None;
    one(t, "NULL path", &s);
    s.path = Some("");
    one(t, "empty path", &s);
    for (label, ty, start, err) in [
        ("type 0x99", 0x99, SERVICE_DEMAND_START, 1),
        ("type 0", 0, SERVICE_DEMAND_START, 1),
        ("type own|share", 0x30, SERVICE_DEMAND_START, 1),
        ("start type 5", SERVICE_WIN32_OWN_PROCESS, 5, 1),
        ("start type 0xFFFFFFFF", SERVICE_WIN32_OWN_PROCESS, 0xFFFF_FFFF, 1),
        ("start type boot for an own-process service", SERVICE_WIN32_OWN_PROCESS, SERVICE_BOOT_START, 1),
        ("start type system for an own-process service", SERVICE_WIN32_OWN_PROCESS, SERVICE_SYSTEM_START, 1),
        ("start type auto", SERVICE_WIN32_OWN_PROCESS, SERVICE_AUTO_START, 1),
        ("start type disabled", SERVICE_WIN32_OWN_PROCESS, SERVICE_DISABLED, 1),
        ("error control 4", SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START, 4),
        ("error control 0", SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START, 0),
        ("error control severe", SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START, 2),
        ("error control critical", SERVICE_WIN32_OWN_PROCESS, SERVICE_DEMAND_START, 3),
        ("share process", SERVICE_WIN32_SHARE_PROCESS, SERVICE_DEMAND_START, 1),
        ("own process interactive", 0x110, SERVICE_DEMAND_START, 1),
        ("kernel driver, boot start", SERVICE_KERNEL_DRIVER, SERVICE_BOOT_START, 1),
    ] {
        let mut s = Spec::new("PolySpecSvcErrB", P);
        s.ty = ty;
        s.start = start;
        s.error = err;
        one(t, label, &s);
    }
    let mut dep_self = [0u16; 40];
    for (i, c) in "PolySpecSvcErrB".bytes().enumerate() {
        dep_self[i] = u16::from(c);
    }
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.deps = Some(&dep_self);
    one(t, "depends on itself", &s);
    let dep_missing = [b'P' as u16, b'S' as u16, b'x' as u16, b'x' as u16, 0, 0];
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.deps = Some(&dep_missing);
    one(t, "depends on a service that does not exist", &s);
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.account = Some("NoSuchAccountPolySpec");
    s.password = Some("x");
    one(t, "account that does not exist", &s);
    let mut s = Spec::new("PolySpecSvcErrB", P);
    s.account = Some("LocalSystem");
    one(t, "account LocalSystem", &s);
    s.account = Some("NT AUTHORITY\\LocalService");
    one(t, "account NT AUTHORITY\\LocalService", &s);
    s.account = Some("NT AUTHORITY\\NetworkService");
    one(t, "account NT AUTHORITY\\NetworkService", &s);
    // lengths
    let n256 = wide_rep(b'N', 256);
    let n257 = wide_rep(b'N', 257);
    let mut tx = Buf::<300>::new();
    let _ = write!(tx, "{}", utext::<290>(&n256[..256]).as_str());
    let mut tx257 = Buf::<300>::new();
    let _ = write!(tx257, "{}", utext::<290>(&n257[..257]).as_str());
    let mut s = Spec::new(tx.as_str(), P);
    one(t, "name of 256 units", &s);
    let s2 = Spec::new(tx257.as_str(), P);
    one(t, "name of 257 units", &s2);
    s.name = Some("PolySpecSvcErrB");
    s.display = Some(tx.as_str());
    one(t, "display name of 256 units", &s);
    s.display = Some(tx257.as_str());
    one(t, "display name of 257 units", &s);
    // a binary path that is not a file
    let mut s = Spec::new("PolySpecSvcErrB", "C:\\no such directory\\x.exe /a /b");
    one(t, "path of a file that does not exist", &s);
    s.path = Some("not a path at all");
    one(t, "path that is not a path", &s);
    end(a);
}
