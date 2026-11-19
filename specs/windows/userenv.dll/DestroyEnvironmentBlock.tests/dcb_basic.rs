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

// ---- token / child-process helpers for CreateProcessWithTokenW and CreateEnvironmentBlock tests ----
// Declarations from Microsoft Learn (prototypes) and winnt.h, winbase.h, processthreadsapi.h, userenv.h constants.

type PVOID = *mut c_void;

#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    reserved: *mut u16,
    desktop: *mut u16,
    title: *mut u16,
    x: u32,
    y: u32,
    xsize: u32,
    ysize: u32,
    xcount: u32,
    ycount: u32,
    fill: u32,
    flags: u32,
    show: u16,
    cb_reserved2: u16,
    reserved2: *mut u8,
    std_in: HANDLE,
    std_out: HANDLE,
    std_err: HANDLE,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    process: HANDLE,
    thread: HANDLE,
    pid: u32,
    tid: u32,
}

#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
}

fn new_si() -> STARTUPINFOW {
    STARTUPINFOW {
        cb: core::mem::size_of::<STARTUPINFOW>() as u32,
        reserved: core::ptr::null_mut(),
        desktop: core::ptr::null_mut(),
        title: core::ptr::null_mut(),
        x: 0,
        y: 0,
        xsize: 0,
        ysize: 0,
        xcount: 0,
        ycount: 0,
        fill: 0,
        flags: 0,
        show: 0,
        cb_reserved2: 0,
        reserved2: core::ptr::null_mut(),
        std_in: core::ptr::null_mut(),
        std_out: core::ptr::null_mut(),
        std_err: core::ptr::null_mut(),
    }
}

fn new_pi() -> PROCESS_INFORMATION {
    PROCESS_INFORMATION { process: core::ptr::null_mut(), thread: core::ptr::null_mut(), pid: 0, tid: 0 }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn GetExitCodeProcess(h: HANDLE, code: *mut u32) -> BOOL;
    fn GetCurrentProcessId() -> u32;
    fn ProcessIdToSessionId(pid: u32, sid: *mut u32) -> BOOL;
    fn GetCurrentDirectoryW(n: u32, b: *mut u16) -> u32;
    fn SetCurrentDirectoryW(b: *const u16) -> BOOL;
    fn SetEnvironmentVariableW(n: *const u16, v: *const u16) -> BOOL;
    fn GetEnvironmentVariableW(n: *const u16, b: *mut u16, len: u32) -> u32;
    fn GetHandleInformation(h: HANDLE, f: *mut u32) -> BOOL;
    fn GetStartupInfoW(si: *mut STARTUPINFOW);
    fn ResumeThread(h: HANDLE) -> u32;
    fn TerminateProcess(h: HANDLE, code: u32) -> BOOL;
    fn CreateEventW(sa: *mut SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
    fn GetWindowsDirectoryW(b: *mut u16, n: u32) -> u32;
    fn GetStdHandle(n: u32) -> HANDLE;
    fn ExitProcess(code: u32) -> !;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(p: HANDLE, access: u32, out: *mut HANDLE) -> BOOL;
    fn DuplicateTokenEx(t: HANDLE, access: u32, sa: *mut SECURITY_ATTRIBUTES, imp: u32, ttype: u32, out: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(t: HANDLE, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> BOOL;
    fn CreateProcessWithTokenW(
        token: HANDLE,
        logon_flags: u32,
        app: *const u16,
        cmd: *mut u16,
        flags: u32,
        env: *mut c_void,
        cwd: *const u16,
        si: *mut STARTUPINFOW,
        pi: *mut PROCESS_INFORMATION,
    ) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetShellWindow() -> *mut c_void;
    fn GetWindowThreadProcessId(w: *mut c_void, pid: *mut u32) -> u32;
}

#[link(name = "userenv", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateEnvironmentBlock(env: *mut *mut c_void, token: HANDLE, inherit: BOOL) -> BOOL;
    fn DestroyEnvironmentBlock(env: *mut c_void) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationProcess(p: HANDLE, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
}

const TOKEN_ASSIGN_PRIMARY: u32 = 1;
const TOKEN_DUPLICATE: u32 = 2;
const TOKEN_QUERY: u32 = 8;
const TOKEN_ALL_ACCESS: u32 = 0xF01FF;
const PROCESS_QUERY_INFORMATION: u32 = 0x400;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const CREATE_SUSPENDED: u32 = 4;
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x400;
const CREATE_NEW_CONSOLE: u32 = 0x10;
const DETACHED_PROCESS: u32 = 8;
const LOGON_WITH_PROFILE: u32 = 1;
const LOGON_NETCREDENTIALS_ONLY: u32 = 2;
const SECURITY_IMPERSONATION: u32 = 2;
const TOKEN_PRIMARY: u32 = 1;
const TOKEN_IMPERSONATION: u32 = 2;

/// An 8-aligned scratch buffer.
#[repr(C, align(8))]
struct Raw<const N: usize> {
    b: [u8; N],
}
impl<const N: usize> Raw<N> {
    const fn new() -> Raw<N> {
        Raw { b: [0; N] }
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[off..off + 8]);
        u64::from_le_bytes(a)
    }
}

/// A fixed-size token information value (u32 classes need the exact length 4).
fn tok_u32(tok: HANDLE, class: u32) -> Option<u32> {
    let mut v = 0u32;
    let mut n = 0u32;
    // SAFETY: valid buffer of 4 bytes.
    let ok = unsafe { GetTokenInformation(tok, class, (&mut v as *mut u32).cast(), 4, &mut n) };
    if ok != 0 { Some(v) } else { None }
}

/// The SID of TokenUser (class 1) or the integrity label (class 25) of a token: its bytes (up to 68).
fn tok_sid(tok: HANDLE, class: u32) -> Option<([u8; 68], usize)> {
    let mut r = Raw::<512>::new();
    let mut n = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetTokenInformation(tok, class, r.b.as_mut_ptr(), 512, &mut n) };
    if ok == 0 {
        return None;
    }
    let base = r.b.as_ptr() as usize;
    let p = r.u64_at(0) as usize;
    let off = p.wrapping_sub(base);
    if off >= 480 {
        return None;
    }
    let len = 8 + 4 * usize::from(r.b[off + 1]);
    let mut o = [0u8; 68];
    for i in 0..len.min(68) {
        o[i] = r.b[off + i];
    }
    Some((o, len.min(68)))
}

fn priv_count(tok: HANDLE) -> u32 {
    let mut r = Raw::<2048>::new();
    let mut n = 0u32;
    // SAFETY: valid buffer.
    let ok = unsafe { GetTokenInformation(tok, 3, r.b.as_mut_ptr(), 2048, &mut n) };
    if ok != 0 { r.u32_at(0) } else { 0xFFFF }
}

/// The user SID bytes of a token (up to 32), as hex.
fn user_hex(tok: HANDLE) -> Buf<80> {
    let mut b = Buf::<80>::new();
    if let Some((s, n)) = tok_sid(tok, 1) {
        for i in 0..n.min(32) {
            let _ = write!(b, "{:02x}", s[i]);
        }
    }
    b
}

/// A line of the token facts a test may print (no ids): type, elevation type, elevated, integrity RID, whether the session
/// is 0, privilege count, has-restrictions.
fn token_facts(tok: HANDLE) -> Buf<200> {
    let mut b = Buf::<200>::new();
    let ty = tok_u32(tok, 8).unwrap_or(0xFFFF);
    let et = tok_u32(tok, 18).unwrap_or(0xFFFF);
    let el = tok_u32(tok, 20).unwrap_or(0xFFFF);
    let sess = tok_u32(tok, 12).unwrap_or(0xFFFF);
    let hr = tok_u32(tok, 21).unwrap_or(0xFFFF);
    let pc = priv_count(tok);
    let il = tok_sid(tok, 25).map_or(0xEEEE, |(s, n)| u32::from_le_bytes([s[n - 4], s[n - 3], s[n - 2], s[n - 1]]));
    let _ = write!(b, "type={ty} elevation_type={et} elevated={el} integrity={il:#x} session_is_0={} privileges={pc} restricted={hr}", sess == 0);
    b
}

fn own_token(access: u32) -> HANDLE {
    let mut t: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer.
    unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut t) };
    t
}

fn dup_primary(src: HANDLE, access: u32) -> HANDLE {
    let mut t: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer.
    unsafe { DuplicateTokenEx(src, access, core::ptr::null_mut(), SECURITY_IMPERSONATION, TOKEN_PRIMARY, &mut t) };
    t
}

/// The token of the shell's process (Explorer) as a primary duplicate, as Chrome's setup gets it: GetShellWindow,
/// GetWindowThreadProcessId, OpenProcess(QUERY_INFORMATION), OpenProcessToken(DUPLICATE), DuplicateTokenEx.
/// Returns (shell_window_found, process_opened, token_opened, duplicate).
fn shell_token(access: u32) -> (bool, bool, bool, HANDLE) {
    // SAFETY: plain calls with valid out pointers.
    unsafe {
        let w = GetShellWindow();
        if w.is_null() {
            return (false, false, false, core::ptr::null_mut());
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, &mut pid);
        let p = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if p.is_null() {
            return (true, false, false, core::ptr::null_mut());
        }
        let mut t: HANDLE = core::ptr::null_mut();
        if OpenProcessToken(p, TOKEN_DUPLICATE, &mut t) == 0 {
            CloseHandle(p);
            return (true, true, false, core::ptr::null_mut());
        }
        let d = dup_primary(t, access);
        CloseHandle(t);
        CloseHandle(p);
        (true, true, true, d)
    }
}

/// The ASCII text after `--child ` on the command line, if any.
fn child_args() -> Option<([u8; 160], usize)> {
    // SAFETY: the command line is NUL-terminated.
    let cl = wstr_at(unsafe { GetCommandLineW() }, 600);
    let key = b"--child ";
    if cl.len() < key.len() {
        return None;
    }
    let mut start = None;
    for i in 0..=(cl.len() - key.len()) {
        if cl[i..i + key.len()].iter().zip(key.iter()).all(|(&c, &k)| c == u16::from(k)) {
            start = Some(i + key.len());
            break;
        }
    }
    let s = start?;
    let mut out = [0u8; 160];
    let mut n = 0;
    for &c in &cl[s..] {
        if n < 160 && c < 0x80 {
            out[n] = c as u8;
            n += 1;
        }
    }
    Some((out, n))
}

fn hexval(s: &[u8]) -> u64 {
    let mut v = 0u64;
    for &c in s {
        v = v * 16 + u64::from(match c { b'0'..=b'9' => c - b'0', b'a'..=b'f' => c - b'a' + 10, _ => 0 });
    }
    v
}

fn ends_with_nocase(a: &[u16], suffix: &str) -> bool {
    let s: &[u8] = suffix.as_bytes();
    if a.len() < s.len() {
        return false;
    }
    let l = |c: u16| if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
    a[a.len() - s.len()..].iter().zip(s).all(|(&x, &y)| l(x) == l(u16::from(y)))
}

fn env_present(name: &str) -> bool {
    let w = wide(name);
    let mut b = [0u16; 8];
    // SAFETY: valid buffers.
    unsafe { GetEnvironmentVariableW(w.as_ptr(), b.as_mut_ptr(), 8) != 0 }
}

/// Child mode `facts <file> <handle hex> <parent pid hex>`: writes what the child sees to <file> beside the image.
fn child_facts(a: &[u8]) -> ! {
    let mut parts = a.split(|&c| c == b' ');
    let _mode = parts.next();
    let fname = parts.next().unwrap_or(b"x.txt");
    let hv = hexval(parts.next().unwrap_or(b"0"));
    let hv2 = hexval(parts.next().unwrap_or(b"0"));
    let ppid_exp = hexval(parts.next().unwrap_or(b"0")) as u32;
    let mut o = Buf::<1500>::new();
    let tok = own_token(TOKEN_QUERY);
    let _ = writeln!(o, "token: {}", token_facts(tok).as_str());
    let _ = writeln!(o, "user_hex={}", user_hex(tok).as_str());
    let mut pbi = Raw::<64>::new();
    let mut rl = 0u32;
    // SAFETY: valid buffer.
    unsafe { NtQueryInformationProcess(GetCurrentProcess(), 0, pbi.b.as_mut_ptr(), 48, &mut rl) };
    let ppid = pbi.u64_at(40) as u32;
    let _ = writeln!(o, "parent_is_caller={}", ppid == ppid_exp);
    let mut f = 0u32;
    // SAFETY: valid out pointer.
    let hok = unsafe { GetHandleInformation(hv as usize as HANDLE, &mut f) };
    let mut f2 = 0u32;
    // SAFETY: valid out pointer.
    let hok2 = unsafe { GetHandleInformation(hv2 as usize as HANDLE, &mut f2) };
    let _ = writeln!(o, "inheritable_handle_valid={} plain_handle_valid={}", hok != 0, hok2 != 0);
    let mut cwd = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetCurrentDirectoryW(300, cwd.as_mut_ptr()) } as usize;
    let cw = &cwd[..n];
    let mut img_dir = poly_wtest::image_path();
    let last = img_dir.iter().rposition(|&c| c == 0x5c).unwrap_or(0);
    img_dir = &img_dir[..last];
    let mut win = [0u16; 300];
    // SAFETY: valid buffer.
    let wn = unsafe { GetWindowsDirectoryW(win.as_mut_ptr(), 300) } as usize;
    let _ = writeln!(
        o,
        "cwd_is_image_dir={} cwd_is_windows={} cwd_is_system32={}",
        eq_nocase(cw, img_dir),
        eq_nocase(cw, &win[..wn]),
        ends_with_nocase(cw, "\\system32")
    );
    let _ = writeln!(
        o,
        "env: marker={} USERNAME={} USERPROFILE={} APPDATA={} LOCALAPPDATA={} TEMP={} SystemRoot={} Path={}",
        env_present("POLYSPECB4MARK"),
        env_present("USERNAME"),
        env_present("USERPROFILE"),
        env_present("APPDATA"),
        env_present("LOCALAPPDATA"),
        env_present("TEMP"),
        env_present("SystemRoot"),
        env_present("Path")
    );
    let mut si = new_si();
    // SAFETY: valid struct.
    unsafe { GetStartupInfoW(&mut si) };
    let title = wstr_at(si.title, 100);
    let desk = wstr_at(si.desktop, 100);
    let _ = writeln!(
        o,
        "startup: title_is_polyspec={} desktop_is_winsta0_default={} desktop_empty={} flags={:#x} show={}",
        eq_nocase(title, &wide("polyspecb4")[..10]),
        eq_nocase(desk, &wide("winsta0\\default")[..15]),
        desk.is_empty(),
        si.flags,
        si.show
    );
    let mut sess = 0u32;
    // SAFETY: valid out pointer.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut sess) };
    let _ = writeln!(o, "session_is_0={}", sess == 0);
    let w = scratch_str(fname);
    write_all(&w, o.as_str().as_bytes());
    // SAFETY: plain call.
    unsafe { ExitProcess(23) }
}

/// A scratch-directory path from ASCII bytes.
fn scratch_str(name: &[u8]) -> [u16; 300] {
    let s = core::str::from_utf8(name).unwrap_or("x.txt");
    scratch(s)
}

/// The dispatcher run first in every test program of this group (before the observation file is opened).
fn child_dispatch() {
    let Some((a, n)) = child_args() else { return };
    let a = &a[..n];
    if a.starts_with(b"facts ") {
        child_facts(a);
    }
    if a.starts_with(b"exit7") {
        // SAFETY: plain call.
        unsafe { ExitProcess(7) }
    }
}

macro_rules! cmain {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            child_dispatch();
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

/// The test's own executable path (for lpApplicationName), NUL-terminated.
fn image_nt() -> [u16; 300] {
    let img = poly_wtest::image_path();
    let mut o = [0u16; 300];
    for (i, &c) in img.iter().enumerate().take(299) {
        o[i] = c;
    }
    o
}

/// "\"<image>\" --child <rest>" as a mutable command line.
fn child_cmd(rest: &str) -> [u16; 600] {
    let img = poly_wtest::image_path();
    let mut o = [0u16; 600];
    let mut n = 0;
    o[n] = b'"' as u16;
    n += 1;
    for &c in img.iter().take(300) {
        o[n] = c;
        n += 1;
    }
    o[n] = b'"' as u16;
    n += 1;
    for c in " --child ".chars().chain(rest.chars()) {
        if n < 599 {
            o[n] = c as u32 as u16;
            n += 1;
        }
    }
    o
}

/// Outcome of one CreateProcessWithTokenW attempt.
struct Attempt {
    ok: bool,
    err: i64,
    pi: PROCESS_INFORMATION,
}

#[allow(clippy::too_many_arguments)]
fn cpwt(tok: HANDLE, logon: u32, app: *const u16, cmd: *mut u16, flags: u32, env: *mut c_void, cwd: *const u16, si: &mut STARTUPINFOW) -> Attempt {
    let mut pi = new_pi();
    clear_error();
    // SAFETY: pointers are valid or NULL as chosen by the caller.
    let ok = unsafe { CreateProcessWithTokenW(tok, logon, app, cmd, flags, env, cwd, si, &mut pi) };
    let err = gle_if(ok);
    Attempt { ok: ok != 0, err, pi }
}

/// Waits for the child (10 s) and returns its exit code, or -1.
fn wait_child(h: HANDLE) -> i64 {
    // SAFETY: a process handle.
    unsafe {
        if WaitForSingleObject(h, 10000) != 0 {
            TerminateProcess(h, 99);
            return -1;
        }
        let mut c = 0u32;
        GetExitCodeProcess(h, &mut c);
        i64::from(c)
    }
}

/// Prints the lines of a child's report file (up to 20), after the prefix.
fn show_report(t: &mut Test, file: &str, prefix: &str) {
    let p = scratch(file);
    match read_all::<2000>(&p) {
        Some((b, n)) => {
            for line in b[..n].split(|&c| c == b'\n') {
                if line.is_empty() || line.starts_with(b"user_hex=") {
                    continue;
                }
                say!(t, "{prefix}{}", core::str::from_utf8(line).unwrap_or("?"));
            }
        }
        None => say!(t, "{prefix}no report file"),
    }
    // SAFETY: valid path.
    unsafe { DeleteFileW(p.as_ptr()) };
}

/// Whether the child's user SID equals `tok`'s (reads `user_hex=` of the report, without deleting it).
fn report_user_same(file: &str, tok: HANDLE) -> Option<bool> {
    let p = scratch(file);
    let (b, n) = read_all::<2000>(&p)?;
    let want = user_hex(tok);
    for line in b[..n].split(|&c| c == b'\n') {
        if let Some(rest) = line.strip_prefix(b"user_hex=") {
            return Some(rest == want.as_str().as_bytes());
        }
    }
    None
}
// DestroyEnvironmentBlock: releasing blocks of CreateEnvironmentBlock, repeated create/destroy, independent blocks, NULL.

poly_wtest::main!(test);

fn make(tok: HANDLE, inherit: i32) -> *mut c_void {
    let mut env: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid out pointer.
    unsafe { CreateEnvironmentBlock(&mut env, tok, inherit) };
    env
}

fn first_entry_len(env: *mut c_void) -> usize {
    wstr_at(env as *const u16, 400).len()
}

fn test(t: &mut Test) {
    let tok = own_token(TOKEN_QUERY | TOKEN_DUPLICATE);
    let e = make(tok, 0);
    // SAFETY: a block from CreateEnvironmentBlock.
    clear_error();
    let d = unsafe { DestroyEnvironmentBlock(e) };
    say!(t, "destroy a block: result {d}");
    // 50 rounds
    let mut ok = 0;
    for _ in 0..50 {
        let e = make(tok, 1);
        // SAFETY: a block from CreateEnvironmentBlock.
        if !e.is_null() && unsafe { DestroyEnvironmentBlock(e) } != 0 {
            ok += 1;
        }
    }
    say!(t, "50 create/destroy rounds, all succeeded: {}", ok == 50);
    // two blocks at the same time are separate allocations with the same content
    let a = make(tok, 0);
    let b = make(tok, 0);
    say!(t, "two blocks are distinct pointers: {}", a != b);
    let la = first_entry_len(a);
    let lb = first_entry_len(b);
    say!(t, "first entries have the same length: {}", la == lb && la > 0);
    // SAFETY: the block is writable memory of ours; the first unit is changed on `a` only.
    unsafe { *(a as *mut u16) = 0x5a };
    say!(t, "changing block a leaves block b alone: {}", first_entry_len(b) == lb && unsafe { *(b as *const u16) } != 0x5a);
    // SAFETY: destroying in the opposite order of creation.
    let (d1, d2) = unsafe { (DestroyEnvironmentBlock(b), DestroyEnvironmentBlock(a)) };
    say!(t, "destroy b then a: {d1} {d2}");
    // a block of the inherit=TRUE kind and of the token-less kind
    let n = make(core::ptr::null_mut(), 0);
    say!(t, "block for a NULL token: {}", !n.is_null());
    // SAFETY: from CreateEnvironmentBlock.
    say!(t, "destroy it: {}", unsafe { DestroyEnvironmentBlock(n) });
    clear_error();
    // SAFETY: NULL is the case under test.
    let d = unsafe { DestroyEnvironmentBlock(core::ptr::null_mut()) };
    say!(t, "destroy NULL: result {d}, last error {}", gle_if(d));
}
