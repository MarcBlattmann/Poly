//! SHGetFileInfoW icons: SHGFI_ICON with SHGFI_SMALLICON and the default
//! (large) size gives an icon handle of 16 x 16 and 32 x 32 pixels, the
//! handle can be destroyed, SHGFI_SYSICONINDEX returns the system image
//! list (a handle whose icon size is 16 x 16 with SHGFI_SMALLICON and
//! 32 x 32 without), the icon index lies inside it, a directory with
//! SHGFI_OPENICON has another index than a closed one, and the overlay and
//! selected flags. Pixels are not looked at.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type HGDIOBJ = *mut c_void;
type HBITMAP = *mut c_void;
type BOOL = i32;
/// wingdi.h
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BITMAP {
    kind: i32,
    width: i32,
    height: i32,
    width_bytes: i32,
    planes: u16,
    bits_pixel: u16,
    bits: usize,
}

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

type HICON = *mut c_void;
type HIMAGELIST = *mut c_void;
/// winuser.h
#[repr(C)]
#[derive(Clone, Copy)]
struct ICONINFO {
    f_icon: BOOL,
    x_hotspot: u32,
    y_hotspot: u32,
    hbm_mask: HBITMAP,
    hbm_color: HBITMAP,
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

const SHGFI_ICON: u32 = 0x100;
const SHGFI_SYSICONINDEX: u32 = 0x4000;
const SHGFI_LINKOVERLAY: u32 = 0x8000;
const SHGFI_SELECTED: u32 = 0x10000;
const SHGFI_SMALLICON: u32 = 0x1;
const SHGFI_OPENICON: u32 = 0x2;
const SHGFI_SHELLICONSIZE: u32 = 0x4;
const SHGFI_USEFILEATTRIBUTES: u32 = 0x10;
const FILE_ATTRIBUTE_NORMAL5: u32 = 0x80;
const FILE_ATTRIBUTE_DIRECTORY5: u32 = 0x10;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, n: u32) -> u32;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetSystemMetrics(index: i32) -> i32;
    fn GetIconInfo(icon: HICON, info: *mut ICONINFO) -> BOOL;
    fn DestroyIcon(icon: HICON) -> BOOL;
}

#[link(name = "gdi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn DeleteObject(obj: HGDIOBJ) -> BOOL;
    fn GetObjectW(obj: HGDIOBJ, size: i32, out: *mut c_void) -> i32;
}

#[link(name = "comctl32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImageList_GetImageCount(il: HIMAGELIST) -> i32;
    fn ImageList_GetIconSize(il: HIMAGELIST, cx: *mut i32, cy: *mut i32) -> BOOL;
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
    i
}

fn icon_size(ic: *mut c_void) -> (i32, i32) {
    let mut ii: ICONINFO = zeroed();
    // SAFETY: ii is an ICONINFO; the bitmaps it gives are ours to delete.
    unsafe {
        if GetIconInfo(ic, &mut ii) == 0 {
            return (-1, -1);
        }
        let mut bm: BITMAP = zeroed();
        let src = if ii.hbm_color.is_null() { ii.hbm_mask } else { ii.hbm_color };
        GetObjectW(src, core::mem::size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as *mut c_void);
        if !ii.hbm_color.is_null() {
            DeleteObject(ii.hbm_color);
        }
        if !ii.hbm_mask.is_null() {
            DeleteObject(ii.hbm_mask);
        }
        (bm.width, bm.height)
    }
}

fn test(t: &mut Test) {
    // SAFETY: plain calls.
    unsafe { GetSystemMetrics(0) };
    let size = core::mem::size_of::<SHFILEINFOW>() as u32;
    let file = w16("c:\\polytest\\file.txt");
    let dir = w16("c:\\polytest\\folder");
    for (label, flags) in [("SHGFI_ICON (large)", SHGFI_ICON), ("SHGFI_ICON | SHGFI_SMALLICON", SHGFI_ICON | SHGFI_SMALLICON), ("SHGFI_ICON | SHGFI_SHELLICONSIZE", SHGFI_ICON | SHGFI_SHELLICONSIZE), ("SHGFI_ICON | SHGFI_SMALLICON | SHGFI_OPENICON", SHGFI_ICON | SHGFI_SMALLICON | SHGFI_OPENICON), ("SHGFI_ICON | SHGFI_LINKOVERLAY", SHGFI_ICON | SHGFI_LINKOVERLAY), ("SHGFI_ICON | SHGFI_SELECTED", SHGFI_ICON | SHGFI_SELECTED), ("SHGFI_ICON | SHGFI_SYSICONINDEX", SHGFI_ICON | SHGFI_SYSICONINDEX)] {
        for (what, path, attrs) in [("file", &file, FILE_ATTRIBUTE_NORMAL5), ("directory", &dir, FILE_ATTRIBUTE_DIRECTORY5)] {
            let mut a = info();
            // SAFETY: a is an SHFILEINFOW.
            let r = unsafe { SHGetFileInfoW(path.as_ptr(), attrs, &mut a, size, flags | SHGFI_USEFILEATTRIBUTES) };
            let (w, h) = if a.icon.is_null() { (-1, -1) } else { icon_size(a.icon) };
            say!(t, "{label} for a {what}: ret non-zero {}, icon handle {}, size {w} x {h}, index set {}", r != 0, if a.icon.is_null() { "NULL" } else { "set" }, a.icon_index >= 0);
            if !a.icon.is_null() {
                // SAFETY: an icon we own.
                say!(t, "  DestroyIcon = {}", unsafe { DestroyIcon(a.icon) });
            }
        }
    }
    // the system image list
    for (label, flags) in [("large", SHGFI_SYSICONINDEX), ("small", SHGFI_SYSICONINDEX | SHGFI_SMALLICON), ("shell size", SHGFI_SYSICONINDEX | SHGFI_SHELLICONSIZE)] {
        let mut a = info();
        // SAFETY: a is an SHFILEINFOW.
        let il = unsafe { SHGetFileInfoW(file.as_ptr(), FILE_ATTRIBUTE_NORMAL5, &mut a, size, flags | SHGFI_USEFILEATTRIBUTES) };
        if il == 0 {
            say!(t, "image list {label}: NULL");
            continue;
        }
        let (mut cx, mut cy) = (0, 0);
        // SAFETY: the system image list the shell gave us; it is not destroyed.
        let (n, ok) = unsafe { (ImageList_GetImageCount(il as HIMAGELIST), ImageList_GetIconSize(il as HIMAGELIST, &mut cx, &mut cy)) };
        say!(t, "image list {label}: icon size {cx} x {cy} (call {ok}), the index lies inside the list {}", a.icon_index >= 0 && a.icon_index < n);
    }
    // closed and open folder
    let mut closed = info();
    let mut open = info();
    // SAFETY: SHFILEINFOW structures.
    unsafe {
        SHGetFileInfoW(dir.as_ptr(), FILE_ATTRIBUTE_DIRECTORY5, &mut closed, size, SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES);
        SHGetFileInfoW(dir.as_ptr(), FILE_ATTRIBUTE_DIRECTORY5, &mut open, size, SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_OPENICON | SHGFI_USEFILEATTRIBUTES);
    }
    say!(t, "a directory: the open icon has another index than the closed one: {}", closed.icon_index != open.icon_index);
    // the same index for the same file type, small and large
    let mut a = info();
    let mut b = info();
    // SAFETY: SHFILEINFOW structures.
    unsafe {
        SHGetFileInfoW(file.as_ptr(), FILE_ATTRIBUTE_NORMAL5, &mut a, size, SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES);
        SHGetFileInfoW(file.as_ptr(), FILE_ATTRIBUTE_NORMAL5, &mut b, size, SHGFI_SYSICONINDEX | SHGFI_USEFILEATTRIBUTES);
    }
    say!(t, "the small and the large list use the same index for a file: {}", a.icon_index == b.icon_index);
}
