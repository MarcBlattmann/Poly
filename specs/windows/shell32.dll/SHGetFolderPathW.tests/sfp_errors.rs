//! SHGetFolderPathW: values that are not folders, the flags
//! (CSIDL_FLAG_CREATE, CSIDL_FLAG_DONT_VERIFY, CSIDL_FLAG_NO_ALIAS ...),
//! dwFlags other than 0 and 1, a made-up window handle, a made-up token,
//! what the buffer holds after a failure.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HWND = *mut c_void;
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

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

type HRESULT = i32;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, n: u32) -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetFolderPathW(hwnd: HWND, csidl: i32, token: HANDLE, flags: u32, path: *mut u16) -> HRESULT;
}

// ---- shell folder helpers of these tests ----

fn w16(s: &str) -> [u16; 300] {
    let mut a = [0u16; 300];
    for (i, u) in s.encode_utf16().take(299).enumerate() {
        a[i] = u;
    }
    a
}

fn wlen(p: &[u16]) -> usize {
    p.iter().position(|&u| u == 0).unwrap_or(p.len())
}

fn env(name: &str) -> [u16; 300] {
    let n = w16(name);
    let mut b = [0u16; 300];
    // SAFETY: b has 300 units.
    unsafe { GetEnvironmentVariableW(n.as_ptr(), b.as_mut_ptr(), 300) };
    b
}

fn starts_ci(path: &[u16], root: &[u16]) -> bool {
    let r = wlen(root);
    if r == 0 || wlen(path) < r {
        return false;
    }
    for i in 0..r {
        let a = path[i];
        let b = root[i];
        let la = if (0x41..=0x5A).contains(&a) { a + 32 } else { a };
        let lb = if (0x41..=0x5A).contains(&b) { b + 32 } else { b };
        if la != lb {
            return false;
        }
    }
    true
}

/// A path as a text that holds no machine name: the first known root
/// (%USERPROFILE%, %ProgramData%, %SystemRoot%, %ProgramFiles(x86)%,
/// %ProgramFiles%) replaced by its name, the rest as it is; other paths
/// are named "outside the known roots" with their length class.
fn classify(path: &[u16]) -> Buf<300> {
    let mut out = Buf::<300>::new();
    let n = wlen(path);
    if n == 0 {
        let _ = write!(out, "<empty>");
        return out;
    }
    for (label, var) in [("%PUBLIC%", "PUBLIC"), ("%USERPROFILE%", "USERPROFILE"), ("%ProgramData%", "ProgramData"), ("%SystemRoot%", "SystemRoot"), ("%ProgramFiles(x86)%", "ProgramFiles(x86)"), ("%ProgramFiles%", "ProgramFiles")] {
        let root = env(var);
        if starts_ci(path, &root) {
            let _ = write!(out, "{label}");
            for &u in &path[wlen(&root)..n] {
                let _ = out.write_char(if u < 0x80 { u as u8 as char } else { '?' });
            }
            return out;
        }
    }
    let _ = write!(out, "<outside the known roots, {} characters, drive letter {}>", n, if n > 1 && path[1] == 0x3A { "yes" } else { "no" });
    out
}

fn classify_a(path: &[u8]) -> Buf<300> {
    let mut w = [0u16; 300];
    for (i, &b) in path.iter().take_while(|&&b| b != 0).take(299).enumerate() {
        w[i] = b as u16;
    }
    classify(&w)
}

/// (csidl, name) of the folders that are asked.
const CSIDLS: [(i32, &str); 52] = [
    (0x00, "DESKTOP"),
    (0x02, "PROGRAMS"),
    (0x03, "CONTROLS"),
    (0x04, "PRINTERS"),
    (0x05, "PERSONAL"),
    (0x06, "FAVORITES"),
    (0x07, "STARTUP"),
    (0x08, "RECENT"),
    (0x09, "SENDTO"),
    (0x0a, "BITBUCKET"),
    (0x0b, "STARTMENU"),
    (0x0d, "MYMUSIC"),
    (0x0e, "MYVIDEO"),
    (0x10, "DESKTOPDIRECTORY"),
    (0x11, "DRIVES"),
    (0x12, "NETWORK"),
    (0x13, "NETHOOD"),
    (0x14, "FONTS"),
    (0x15, "TEMPLATES"),
    (0x16, "COMMON_STARTMENU"),
    (0x17, "COMMON_PROGRAMS"),
    (0x18, "COMMON_STARTUP"),
    (0x19, "COMMON_DESKTOPDIRECTORY"),
    (0x1a, "APPDATA"),
    (0x1b, "PRINTHOOD"),
    (0x1c, "LOCAL_APPDATA"),
    (0x1d, "ALTSTARTUP"),
    (0x1e, "COMMON_ALTSTARTUP"),
    (0x1f, "COMMON_FAVORITES"),
    (0x20, "INTERNET_CACHE"),
    (0x21, "COOKIES"),
    (0x22, "HISTORY"),
    (0x23, "COMMON_APPDATA"),
    (0x24, "WINDOWS"),
    (0x25, "SYSTEM"),
    (0x26, "PROGRAM_FILES"),
    (0x27, "MYPICTURES"),
    (0x28, "PROFILE"),
    (0x29, "SYSTEMX86"),
    (0x2a, "PROGRAM_FILESX86"),
    (0x2b, "PROGRAM_FILES_COMMON"),
    (0x2c, "PROGRAM_FILES_COMMONX86"),
    (0x2d, "COMMON_TEMPLATES"),
    (0x2e, "COMMON_DOCUMENTS"),
    (0x2f, "COMMON_ADMINTOOLS"),
    (0x30, "ADMINTOOLS"),
    (0x31, "CONNECTIONS"),
    (0x35, "COMMON_MUSIC"),
    (0x36, "COMMON_PICTURES"),
    (0x37, "COMMON_VIDEO"),
    (0x38, "RESOURCES"),
    (0x3b, "CDBURN_AREA"),
];

fn hr(v: i32) -> &'static str {
    match v as u32 {
        0 => "S_OK",
        1 => "S_FALSE",
        0x8000_4005 => "E_FAIL",
        0x8007_0057 => "E_INVALIDARG",
        0x8007_0002 => "HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)",
        0x8007_0003 => "HRESULT_FROM_WIN32(ERROR_PATH_NOT_FOUND)",
        0x8000_4001 => "E_NOTIMPL",
        0x8007_0006 => "E_HANDLE",
        0x8000_4003 => "E_POINTER",
        0x8007_007B => "HRESULT_FROM_WIN32(ERROR_INVALID_NAME)",
        _ => "another HRESULT",
    }
}

poly_wtest::main!(test);

fn call(t: &mut Test, label: &str, csidl: i32, token: HANDLE, flags: u32) {
    let mut p = [0xAAAAu16; 300];
    set_error(0xDEAD);
    // SAFETY: p has 300 units.
    let r = unsafe { SHGetFolderPathW(core::ptr::null_mut(), csidl, token, flags, p.as_mut_ptr()) };
    let e = last_error();
    let path = if r >= 0 && p[0] != 0xAAAA { classify(&p) } else { let mut b = Buf::<300>::new(); let _ = write!(b, "-"); b };
    say!(t, "{label}: {} ({:#x}), buffer {}, path {}, last error {}", hr(r), r as u32, if p[0] == 0xAAAA { "untouched" } else if p[0] == 0 { "empty string" } else { "written" }, path.as_str(), if r >= 0 { -2 } else if e == 0xDEAD { -1 } else { e as i64 });
}

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    for (label, id) in [("0x0001", 0x1), ("0x000c", 0xc), ("0x000f", 0xf), ("0x0032", 0x32), ("0x003f", 0x3f), ("0x0040", 0x40), ("0x00ff", 0xff), ("0x7fff", 0x7fff), ("0xffff", 0xffff), ("-1", -1), ("0x10000", 0x10000)] {
        call(t, label, id, core::ptr::null_mut(), 0);
    }
    for (label, id) in [("APPDATA | CSIDL_FLAG_CREATE", 0x1a | 0x8000), ("APPDATA | CSIDL_FLAG_DONT_VERIFY", 0x1a | 0x4000), ("DESKTOP | CSIDL_FLAG_DONT_VERIFY", 0x4000), ("PERSONAL | CSIDL_FLAG_NO_ALIAS", 0x5 | 0x1000), ("PERSONAL | CSIDL_FLAG_DONT_UNEXPAND", 0x5 | 0x2000), ("PERSONAL | CSIDL_FLAG_PER_USER_INIT", 0x5 | 0x800), ("COMMON_OEM_LINKS 0x3a", 0x3a), ("COMMON_OEM_LINKS | CSIDL_FLAG_DONT_VERIFY", 0x3a | 0x4000)] {
        call(t, label, id, core::ptr::null_mut(), 0);
    }
    for f in [0u32, 1, 2, 3, 0x10, 0xFFFF_FFFF] {
        let mut l = Buf::<60>::new();
        let _ = write!(l, "APPDATA with dwFlags {f:#x}");
        call(t, l.as_str(), 0x1a, core::ptr::null_mut(), f);
    }
    call(t, "APPDATA with a made-up token", 0x1a, 0x1234 as HANDLE, 0);
    call(t, "WINDOWS with a made-up token", 0x24, 0x1234 as HANDLE, 0);
    // a made-up window handle and a small buffer cannot be used; the handle is checked
    let mut p = [0xAAAAu16; 300];
    // SAFETY: p has 300 units.
    let r = unsafe { SHGetFolderPathW(0x1234 as HWND, 0x1a, core::ptr::null_mut(), 0, p.as_mut_ptr()) };
    say!(t, "made-up window handle: {} ({:#x}), path {}", hr(r), r as u32, classify(&p).as_str());
    // the path is not written past MAX_PATH: the unit after 260 stays
    let mut q = [0xAAAAu16; 300];
    // SAFETY: q has 300 units.
    let r = unsafe { SHGetFolderPathW(core::ptr::null_mut(), 0x1a, core::ptr::null_mut(), 0, q.as_mut_ptr()) };
    say!(t, "units after the path: {}; units from 260 on untouched: {}", if r >= 0 { wlen(&q) } else { 0 } < 260, q[260..].iter().all(|&u| u == 0xAAAA));
}
