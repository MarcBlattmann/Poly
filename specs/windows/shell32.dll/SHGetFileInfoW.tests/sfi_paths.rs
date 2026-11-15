//! SHGetFileInfoW on paths that exist (a directory and files the test
//! makes in the temp directory): display name (a file with an extension the
//! system does not know keeps it), type name, attributes, SHGFI_ICONLOCATION
//! and SHGFI_EXETYPE (of the test program itself), what is returned for a
//! path that does not exist (with and without SHGFI_USEFILEATTRIBUTES), a
//! wrong structure size, and a path with a trailing backslash.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HINSTANCE = *mut c_void;
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

/// An all-zero value of a C struct.
fn zeroed<T>() -> T {
    // SAFETY: only used for the C structs above, for which zero is valid.
    unsafe { core::mem::zeroed() }
}

/// The UTF-16 text in `buf` up to its first NUL, as ASCII (others as '?').
fn ascii_of(buf: &[u16], out: &mut Buf<300>) {
    for &c in buf {
        if c == 0 {
            break;
        }
        let _ = out.write_char(if (0x20..0x7f).contains(&c) { c as u8 as char } else { '?' });
    }
}

type HRESULT = i32;
/// shellapi.h SHFILEINFOW (x64)
#[repr(C)]
#[derive(Clone, Copy)]
struct SHFILEINFOW {
    icon: *mut c_void,
    icon_index: i32,
    attributes: u32,
    display_name: [u16; 260],
    type_name: [u16; 80],
}

const SHGFI_DISPLAYNAME: u32 = 0x200;
const SHGFI_TYPENAME: u32 = 0x400;
const SHGFI_ATTRIBUTES: u32 = 0x800;
const SHGFI_ICONLOCATION: u32 = 0x1000;
const SHGFI_EXETYPE: u32 = 0x2000;
const SHGFI_SYSICONINDEX: u32 = 0x4000;
const SHGFI_USEFILEATTRIBUTES: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL5: u32 = 0x80;
const GENERIC_WRITE5: u32 = 0x4000_0000;
const CREATE_ALWAYS5: u32 = 2;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn CloseHandle(handle: HANDLE) -> BOOL;
    fn GetModuleFileNameW(module: HINSTANCE, buf: *mut u16, max: u32) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, n: u32) -> u32;
    fn GetTempPathW(n: u32, buf: *mut u16) -> u32;
    fn CreateDirectoryW(path: *const u16, sa: *mut c_void) -> BOOL;
    fn RemoveDirectoryW(path: *const u16) -> BOOL;
    fn CreateFileW(path: *const u16, access: u32, share: u32, sa: *mut c_void, disposition: u32, flags: u32, template: HANDLE) -> HANDLE;
    fn DeleteFileW(path: *const u16) -> BOOL;
    fn WriteFile(h: HANDLE, buf: *const u8, n: u32, written: *mut u32, ov: *mut c_void) -> BOOL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
}

#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetFileInfoW(path: *const u16, attrs: u32, info: *mut SHFILEINFOW, cb: u32, flags: u32) -> usize;
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

fn info() -> SHFILEINFOW {
    let mut i: SHFILEINFOW = zeroed();
    i.icon_index = -77;
    i.attributes = 0x5A5A_5A5A;
    i.display_name = [0x7777; 260];
    i.type_name = [0x7777; 80];
    i
}

fn text(buf: &[u16]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    if buf[0] == 0x7777 {
        let _ = write!(b, "<untouched>");
        return b;
    }
    ascii_of(buf, &mut b);
    b
}

fn join(dir: &[u16; 300], name: &str) -> [u16; 300] {
    let mut p = *dir;
    let n = wlen(&p);
    let mut i = n;
    for u in name.encode_utf16() {
        p[i] = u;
        i += 1;
    }
    p[i] = 0;
    p
}

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    // a directory of ours in the temp directory
    let mut tmp = [0u16; 300];
    // SAFETY: tmp has 300 units.
    unsafe { GetTempPathW(250, tmp.as_mut_ptr()) };
    let mut dir = tmp;
    let pid = unsafe { GetCurrentProcessId() };
    let mut name = Buf::<60>::new();
    let _ = write!(name, "polytest5-{pid}\\");
    let dirw = join(&dir, name.as_str());
    dir = dirw;
    // SAFETY: NUL-terminated paths.
    unsafe { CreateDirectoryW(dir.as_ptr(), core::ptr::null_mut()) };
    let f1 = join(&dir, "report.polytestxyz");
    let f2 = join(&dir, "notes.txt");
    let sub = join(&dir, "sub.polytestdir");
    // SAFETY: NUL-terminated paths; handles closed at once.
    unsafe {
        for f in [&f1, &f2] {
            let h = CreateFileW(f.as_ptr(), GENERIC_WRITE5, 0, core::ptr::null_mut(), CREATE_ALWAYS5, 0x80, core::ptr::null_mut());
            let mut n = 0;
            WriteFile(h, b"hello".as_ptr(), 5, &mut n, core::ptr::null_mut());
            CloseHandle(h);
        }
        CreateDirectoryW(sub.as_ptr(), core::ptr::null_mut());
    }
    let size = core::mem::size_of::<SHFILEINFOW>() as u32;
    for (label, p) in [("file with an unknown extension", &f1), ("file notes.txt", &f2), ("directory", &sub), ("temp directory of the test", &dir)] {
        let mut a = info();
        // SAFETY: a is an SHFILEINFOW.
        let r = unsafe { SHGetFileInfoW(p.as_ptr(), 0, &mut a, size, SHGFI_DISPLAYNAME | SHGFI_TYPENAME | SHGFI_ATTRIBUTES | SHGFI_SYSICONINDEX) };
        let nm = text(&a.display_name);
        let shown = if label.starts_with("temp directory") { "<the directory's name>" } else { nm.as_str() };
        say!(t, "{label}: ret non-zero {}; display name '{}'; type '{}'; attributes {:#x}; icon index set {}", r != 0, shown, text(&a.type_name).as_str(), a.attributes, a.icon_index >= 0);
    }
    // a name that does not exist
    let ghost = join(&dir, "ghost.polytestxyz");
    for (label, flags, attrs) in [("TYPENAME", SHGFI_TYPENAME, 0), ("DISPLAYNAME", SHGFI_DISPLAYNAME, 0), ("ATTRIBUTES", SHGFI_ATTRIBUTES, 0), ("SYSICONINDEX", SHGFI_SYSICONINDEX, 0), ("TYPENAME | USEFILEATTRIBUTES", SHGFI_TYPENAME | SHGFI_USEFILEATTRIBUTES, FILE_ATTRIBUTE_NORMAL5), ("DISPLAYNAME | USEFILEATTRIBUTES", SHGFI_DISPLAYNAME | SHGFI_USEFILEATTRIBUTES, FILE_ATTRIBUTE_NORMAL5)] {
        let mut a = info();
        // SAFETY: a is an SHFILEINFOW.
        let r = unsafe { SHGetFileInfoW(ghost.as_ptr(), attrs, &mut a, size, flags) };
        say!(t, "a file that does not exist, {label}: ret {}, type '{}', display '{}'", r as i64 as i32, text(&a.type_name).as_str(), text(&a.display_name).as_str());
    }
    // trailing backslash on a directory
    let mut sl = sub;
    let n = wlen(&sl);
    sl[n] = 0x5C;
    sl[n + 1] = 0;
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(sl.as_ptr(), 0, &mut a, size, SHGFI_TYPENAME | SHGFI_DISPLAYNAME) };
    say!(t, "directory with a trailing backslash: ret non-zero {}, type '{}', display name '{}'", r != 0, text(&a.type_name).as_str(), text(&a.display_name).as_str());
    // the test program: EXETYPE and ICONLOCATION
    let mut me = [0u16; 300];
    // SAFETY: me has 300 units.
    unsafe { GetModuleFileNameW(core::ptr::null_mut(), me.as_mut_ptr(), 299) };
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(me.as_ptr(), 0, &mut a, size, SHGFI_EXETYPE) };
    say!(t, "SHGFI_EXETYPE of this console program: {r:#x} (low word 'PE' is 0x4550)");
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(f2.as_ptr(), 0, &mut a, size, SHGFI_EXETYPE) };
    say!(t, "SHGFI_EXETYPE of a text file: {r:#x}");
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(me.as_ptr(), 0, &mut a, size, SHGFI_ICONLOCATION) };
    say!(t, "SHGFI_ICONLOCATION of this program: ret {}, a location text was written {}, icon index {}", r as i64 as i32, a.display_name[0] != 0x7777, a.icon_index);
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(f2.as_ptr(), 0, &mut a, size, SHGFI_ICONLOCATION) };
    say!(t, "SHGFI_ICONLOCATION of notes.txt: ret {}, a location text was written {}", r as i64 as i32, a.display_name[0] != 0x7777);
    // a wrong structure size
    for cb in [0u32, 4, size - 1, size + 4] {
        let mut a = info();
        // SAFETY: a is large enough for every size tried (the structure size is checked by the function).
        let r = unsafe { SHGetFileInfoW(f2.as_ptr(), 0, &mut a, cb, SHGFI_TYPENAME) };
        say!(t, "cbFileInfo {}: ret non-zero {}, type '{}'", if cb == size { "right" } else if cb == size - 1 { "one less" } else if cb == size + 4 { "four more" } else if cb == 0 { "0" } else { "4" }, r != 0, text(&a.type_name).as_str());
    }
    // empty path and NULL path
    let e = w16("");
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(e.as_ptr(), 0, &mut a, size, SHGFI_TYPENAME) };
    say!(t, "empty path: ret {}", r as i64 as i32);
    let mut a = info();
    // SAFETY: a is an SHFILEINFOW.
    let r = unsafe { SHGetFileInfoW(core::ptr::null(), 0, &mut a, size, SHGFI_TYPENAME) };
    say!(t, "NULL path: ret {}", r as i64 as i32);
    // clean up
    // SAFETY: our files and directories.
    unsafe {
        DeleteFileW(f1.as_ptr());
        DeleteFileW(f2.as_ptr());
        RemoveDirectoryW(sub.as_ptr());
        RemoveDirectoryW(dir.as_ptr());
    }
}
