//! SHGetSpecialFolderLocation: the item identifier list of each CSIDL folder
//! 7-Zip's file manager and other programs ask for (desktop, documents,
//! drives, network, recycle bin, application data ...): the result code, a
//! non-NULL list of a size above the 2-byte terminator, the path
//! SHGetPathFromIDListW gives back (a file system folder: the path, as
//! %USERPROFILE%, %SystemRoot% ... plus the rest; a virtual folder: FALSE),
//! and ILFree on every list. Machine names do not appear in the output.
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

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<300>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

type HRESULT = i32;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, n: u32) -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetSpecialFolderLocation(hwnd: HWND, csidl: i32, pidl: *mut *mut c_void) -> HRESULT;
    fn SHGetPathFromIDListW(pidl: *const c_void, path: *mut u16) -> BOOL;
    fn ILFree(pidl: *mut c_void);
    fn ILGetSize(pidl: *const c_void) -> u32;
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

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    for (id, name) in CSIDLS {
        let mut pidl: *mut c_void = 0x5A5A as *mut c_void;
        // SAFETY: pidl is writable.
        let r = unsafe { SHGetSpecialFolderLocation(core::ptr::null_mut(), id, &mut pidl) };
        if r < 0 {
            say!(t, "{name} ({id:#x}): {} ({:#x}), list pointer {}", hr(r), r as u32, if pidl as usize == 0x5A5A { "untouched" } else if pidl.is_null() { "NULL" } else { "set" });
            continue;
        }
        // SAFETY: a list the shell gave us.
        unsafe {
            let size = ILGetSize(pidl);
            let mut p = [0xAAAAu16; 300];
            let ok = SHGetPathFromIDListW(pidl, p.as_mut_ptr());
            let path = if ok != 0 { classify(&p) } else { let mut b = Buf::<300>::new(); let _ = write!(b, "-"); b };
            say!(t, "{name} ({id:#x}): {} ({:#x}); list non-NULL {}, larger than a terminator {}; SHGetPathFromIDListW = {ok}, path {}{}", hr(r), r as u32, !pidl.is_null(), size > 2, path.as_str(), if ok == 0 { if p[0] == 0xAAAA { " (buffer untouched)" } else { " (buffer written)" } } else { "" });
            ILFree(pidl);
        }
    }
}
