//! SHGetKnownFolderPath: the flags of KNOWN_FOLDER_FLAG (all but KF_FLAG_INIT, which may write
//! a desktop.ini) on a folder that exists (Downloads,
//! LocalAppData, Windows), flags that are not defined, and the token argument (NULL, -1 for the
//! default user, the process's own token, a value that is not a handle).
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

// shlobj_core.h: HRESULT SHGetKnownFolderPath(REFKNOWNFOLDERID rfid, DWORD dwFlags, HANDLE hToken, PWSTR *ppszPath);
// knownfolders.h (the FOLDERID_ values), shlobj_core.h (KNOWN_FOLDER_FLAG), combaseapi.h (CoTaskMemFree),
// sysinfoapi.h, fileapi.h.
#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(rfid: *const GUID, flags: u32, token: HANDLE, path: *mut *mut u16) -> i32;
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemFree(p: *mut c_void);
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn GetWindowsDirectoryW(buf: *mut u16, n: u32) -> u32;
    fn GetSystemDirectoryW(buf: *mut u16, n: u32) -> u32;
}

const FOLDERID_Downloads: GUID = guid(0x374DE290, 0x123F, 0x4565, [0x91, 0x64, 0x39, 0xC4, 0x92, 0x5E, 0x46, 0x7B]);
const FOLDERID_Profile: GUID = guid(0x5E6C858F, 0x0E22, 0x4760, [0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73]);
const FOLDERID_LocalAppData: GUID = guid(0xF1B32785, 0x6FBA, 0x4FCF, [0x9D, 0x55, 0x7B, 0x8E, 0x7F, 0x15, 0x70, 0x91]);
const FOLDERID_RoamingAppData: GUID =
    guid(0x3EB685DB, 0x65F9, 0x4CF6, [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D]);
const FOLDERID_LocalAppDataLow: GUID =
    guid(0xA520A1A4, 0x1780, 0x4FF6, [0xBD, 0x18, 0x16, 0x73, 0x43, 0xC5, 0xAF, 0x16]);
const FOLDERID_Desktop: GUID = guid(0xB4BFCC3A, 0xDB2C, 0x424C, [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41]);
const FOLDERID_Documents: GUID = guid(0xFDD39AD0, 0x238F, 0x46AF, [0xAD, 0xB4, 0x6C, 0x85, 0x48, 0x03, 0x69, 0xC7]);
const FOLDERID_Programs: GUID = guid(0xA77F5D77, 0x2E2B, 0x44C3, [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51]);
const FOLDERID_Windows: GUID = guid(0xF38BF404, 0x1D43, 0x42F2, [0x93, 0x05, 0x67, 0xDE, 0x0B, 0x28, 0xFC, 0x23]);
const FOLDERID_System: GUID = guid(0x1AC14E77, 0x02E7, 0x4E5D, [0xB7, 0x44, 0x2E, 0xB1, 0xAE, 0x51, 0x98, 0xB7]);
const FOLDERID_ProgramFiles: GUID = guid(0x905E63B6, 0xC1BF, 0x494E, [0xB2, 0x9C, 0x65, 0xB7, 0x32, 0xD3, 0xD2, 0x1A]);
const FOLDERID_ProgramFilesX86: GUID =
    guid(0x7C5A40EF, 0xA0FB, 0x4BFC, [0x87, 0x4A, 0xC0, 0xF2, 0xE0, 0xB9, 0xFA, 0x8E]);
const FOLDERID_ProgramData: GUID = guid(0x62AB5D82, 0xFDC1, 0x4DC3, [0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97]);
const FOLDERID_Public: GUID = guid(0xDFDF76A2, 0xC82A, 0x4D63, [0x90, 0x6A, 0x56, 0x44, 0xAC, 0x45, 0x73, 0x85]);
const FOLDERID_UserProfiles: GUID = guid(0x0762D272, 0xC50A, 0x4BB0, [0xA3, 0x82, 0x69, 0x7D, 0xCD, 0x72, 0x9B, 0x80]);
const FOLDERID_Fonts: GUID = guid(0xFD228CB7, 0xAE11, 0x4AE3, [0x86, 0x4C, 0x16, 0xF3, 0x91, 0x0A, 0xB8, 0xFE]);
const FOLDERID_ComputerFolder: GUID =
    guid(0x0AC0837C, 0xBBF8, 0x452A, [0x85, 0x0D, 0x79, 0xD0, 0x8E, 0x66, 0x7C, 0xA7]);
const FOLDERID_ControlPanelFolder: GUID =
    guid(0x82A74AEB, 0xAEB4, 0x465C, [0xA0, 0x14, 0xD0, 0x97, 0xEE, 0x34, 0x6D, 0x63]);
const FOLDERID_Playlists: GUID = guid(0xDE92C1C7, 0x837F, 0x4F69, [0xA3, 0xBB, 0x86, 0xE6, 0x31, 0x20, 0x4A, 0x23]);
const FOLDERID_PhotoAlbums: GUID = guid(0x69D2CF90, 0xFC33, 0x4FB7, [0x9A, 0x0C, 0xEB, 0xB0, 0xF0, 0xFC, 0xB4, 0x3C]);
/// Not a known folder.
const NOT_A_FOLDER: GUID = guid(0x504F4C59, 0x0005, 0x4005, [0x80, 0x05, 0x50, 0x4F, 0x4C, 0x59, 0x00, 0x05]);

const KF_FLAG_DEFAULT: u32 = 0;
const KF_FLAG_SIMPLE_IDLIST: u32 = 0x0000_0100;
const KF_FLAG_NOT_PARENT_RELATIVE: u32 = 0x0000_0200;
const KF_FLAG_DEFAULT_PATH: u32 = 0x0000_0400;
const KF_FLAG_INIT: u32 = 0x0000_0800;
const KF_FLAG_NO_ALIAS: u32 = 0x0000_1000;
const KF_FLAG_DONT_UNEXPAND: u32 = 0x0000_2000;
const KF_FLAG_DONT_VERIFY: u32 = 0x0000_4000;
const KF_FLAG_CREATE: u32 = 0x0000_8000;
const KF_FLAG_ALIAS_ONLY: u32 = 0x8000_0000;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// A path the call handed out, copied (the system's memory is freed at once).
struct Path {
    u: [u16; 400],
    n: usize,
}

impl Path {
    fn units(&self) -> &[u16] {
        &self.u[..self.n]
    }
}

/// SHGetKnownFolderPath: the HRESULT, whether the out pointer is NULL afterwards, and the path.
fn known(id: &GUID, flags: u32, token: HANDLE) -> (u32, bool, Path) {
    let mut p = 0x1111_2222usize as *mut u16;
    // SAFETY: a live GUID and out pointer.
    let hr = unsafe { SHGetKnownFolderPath(id, flags, token, &mut p) } as u32;
    let mut out = Path { u: [0; 400], n: 0 };
    let null = p.is_null();
    if hr == 0 && !null {
        let s = wstr_at(p, 399);
        out.u[..s.len()].copy_from_slice(s);
        out.n = s.len();
        // SAFETY: memory SHGetKnownFolderPath allocated, freed once.
        unsafe { CoTaskMemFree(p.cast()) };
    }
    (hr, null, out)
}

/// The places paths are shown relative to, so that no line holds a drive letter, a user name or the
/// Windows directory's own name.
struct Roots {
    profile: Path,
    profiles: Path,
    windows: Path,
}

fn roots() -> Roots {
    let mut w = Path { u: [0; 400], n: 0 };
    // SAFETY: the buffer holds 400 units.
    w.n = unsafe { GetWindowsDirectoryW(w.u.as_mut_ptr(), 399) } as usize;
    Roots {
        profile: known(&FOLDERID_Profile, 0, core::ptr::null_mut()).2,
        profiles: known(&FOLDERID_UserProfiles, 0, core::ptr::null_mut()).2,
        windows: w,
    }
}

fn starts_with(p: &[u16], root: &[u16]) -> bool {
    !root.is_empty()
        && p.len() >= root.len()
        && eq_nocase(&p[..root.len()], root)
        && (p.len() == root.len() || p[root.len()] == u16::from(b'\\'))
}

impl Roots {
    /// `<Profile>\...`, `<UserProfiles>\...`, `<Windows>\...` or `<drive>\...`.
    fn shown(&self, p: &[u16]) -> Buf<600> {
        let mut b = Buf::<600>::new();
        let (name, rest) = if starts_with(p, self.profile.units()) {
            ("<Profile>", &p[self.profile.n..])
        } else if starts_with(p, self.profiles.units()) {
            ("<UserProfiles>", &p[self.profiles.n..])
        } else if starts_with(p, self.windows.units()) {
            ("<Windows>", &p[self.windows.n..])
        } else if p.len() >= 2 && p[1] == u16::from(b':') {
            ("<drive>", &p[2..])
        } else {
            ("", p)
        };
        let _ = write!(b, "{name}{}", units_text(rest).as_str());
        b
    }
}

/// 1 when `p` names an existing directory.
fn is_dir(p: &[u16]) -> u32 {
    let mut z = [0u16; 401];
    z[..p.len()].copy_from_slice(p);
    // SAFETY: NUL-terminated.
    let a = unsafe { GetFileAttributesW(z.as_ptr()) };
    u32::from(a != u32::MAX && a & FILE_ATTRIBUTE_DIRECTORY != 0)
}

/// One line for a call.
fn say_known(t: &mut Test, r: &Roots, what: &str, id: &GUID, flags: u32, token: HANDLE) {
    let (hr, null, p) = known(id, flags, token);
    if hr == 0 {
        say!(
            t,
            "{what}: hr {hr:#x} path {} (an existing directory: {})",
            r.shown(p.units()).as_str(),
            is_dir(p.units())
        );
    } else {
        say!(t, "{what}: hr {hr:#x} out pointer {}", if null { "NULL" } else { "not NULL" });
    }
}

poly_wtest::main!(test);

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
}

fn test(t: &mut Test) {
    let r = roots();
    let none: HANDLE = core::ptr::null_mut();
    let flags: [(&str, u32); 12] = [
        ("KF_FLAG_DEFAULT", KF_FLAG_DEFAULT),
        ("KF_FLAG_DONT_VERIFY", KF_FLAG_DONT_VERIFY),
        ("KF_FLAG_CREATE", KF_FLAG_CREATE),
        ("KF_FLAG_DONT_UNEXPAND", KF_FLAG_DONT_UNEXPAND),
        ("KF_FLAG_NO_ALIAS", KF_FLAG_NO_ALIAS),
        ("KF_FLAG_DEFAULT_PATH", KF_FLAG_DEFAULT_PATH),
        ("KF_FLAG_DEFAULT_PATH | KF_FLAG_NOT_PARENT_RELATIVE", KF_FLAG_DEFAULT_PATH | KF_FLAG_NOT_PARENT_RELATIVE),
        ("KF_FLAG_NOT_PARENT_RELATIVE", KF_FLAG_NOT_PARENT_RELATIVE),
        ("KF_FLAG_SIMPLE_IDLIST", KF_FLAG_SIMPLE_IDLIST),
        ("KF_FLAG_ALIAS_ONLY", KF_FLAG_ALIAS_ONLY),
        ("flag 0x1", 1),
        ("flags 0xff", 0xFF),
    ];
    for (name, f) in flags {
        let mut what = Buf::<100>::new();
        let _ = write!(what, "Downloads, {name}");
        say_known(t, &r, what.as_str(), &FOLDERID_Downloads, f, none);
    }
    for (name, f) in [
        ("KF_FLAG_DONT_VERIFY", KF_FLAG_DONT_VERIFY),
        ("KF_FLAG_CREATE", KF_FLAG_CREATE),
        ("KF_FLAG_DEFAULT_PATH", KF_FLAG_DEFAULT_PATH),
        ("KF_FLAG_DONT_UNEXPAND", KF_FLAG_DONT_UNEXPAND),
    ] {
        let mut what = Buf::<100>::new();
        let _ = write!(what, "LocalAppData, {name}");
        say_known(t, &r, what.as_str(), &FOLDERID_LocalAppData, f, none);
        let mut what = Buf::<100>::new();
        let _ = write!(what, "Windows, {name}");
        say_known(t, &r, what.as_str(), &FOLDERID_Windows, f, none);
    }

    // The token: -1 is the default user's profile.
    let default_user = -1isize as HANDLE;
    say_known(t, &r, "Profile, token -1", &FOLDERID_Profile, 0, default_user);
    say_known(t, &r, "LocalAppData, token -1", &FOLDERID_LocalAppData, 0, default_user);
    say_known(t, &r, "Downloads, token -1", &FOLDERID_Downloads, 0, default_user);
    say_known(t, &r, "Windows, token -1", &FOLDERID_Windows, 0, default_user);
    // The process's own token, opened with TOKEN_QUERY | TOKEN_IMPERSONATE, and with TOKEN_QUERY only.
    for (name, access) in
        [("TOKEN_QUERY | TOKEN_IMPERSONATE", 0x8 | 0x4u32), ("TOKEN_QUERY", 0x8), ("TOKEN_DUPLICATE", 0x2)]
    {
        let mut tok: HANDLE = core::ptr::null_mut();
        // SAFETY: live out pointer.
        let ok = unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut tok) };
        let (hr, _, p) = known(&FOLDERID_Downloads, 0, tok);
        say!(
            t,
            "Downloads, own token opened with {name} ({ok}): hr {hr:#x} the same path as with NULL {}",
            u32::from(hr == 0 && p.units() == known(&FOLDERID_Downloads, 0, none).2.units())
        );
        close(tok);
    }
    // A value that is not a handle, and a handle that is not a token.
    say_known(t, &r, "Downloads, token 0x7ffffff0 (not a handle)", &FOLDERID_Downloads, 0, 0x7FFF_FFF0usize as HANDLE);
    // SAFETY: an unnamed event.
    let ev = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    say_known(t, &r, "Downloads, token = an event handle", &FOLDERID_Downloads, 0, ev);
    say_known(t, &r, "Windows, token = an event handle", &FOLDERID_Windows, 0, ev);
    close(ev);
}
