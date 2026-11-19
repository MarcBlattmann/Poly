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

// ---- COM helpers (declarations from Learn's prototypes and the public headers named per use) ----

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> GUID {
    GUID { d1, d2, d3, d4 }
}

/// "{xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx}" text of a GUID.
fn guid_text(g: &GUID) -> Buf<64> {
    let mut b = Buf::<64>::new();
    let _ = write!(
        b,
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        g.d1, g.d2, g.d3, g.d4[0], g.d4[1], g.d4[2], g.d4[3], g.d4[4], g.d4[5], g.d4[6], g.d4[7]
    );
    b
}

const IID_IUnknown: GUID = guid(0, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const S_OK: i32 = 0;
const S_FALSE: i32 = 1;
const E_NOINTERFACE: i32 = 0x8000_4002u32 as i32;

const CLSCTX_INPROC_SERVER: u32 = 1;
const CLSCTX_ALL: u32 = 0x17;
const COINIT_APARTMENTTHREADED: u32 = 2;
const COINIT_MULTITHREADED: u32 = 0;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoInitializeEx(reserved: *mut c_void, coinit: u32) -> i32;
    fn CoUninitialize();
    fn CoCreateInstance(clsid: *const GUID, outer: *mut c_void, ctx: u32, iid: *const GUID, out: *mut *mut c_void) -> i32;
    fn CoTaskMemAlloc(n: usize) -> *mut c_void;
    fn CoTaskMemFree(p: *mut c_void);
}

/// The function pointer in slot `idx` of the vtable of COM object `obj`.
unsafe fn slot(obj: *mut c_void, idx: usize) -> usize {
    // SAFETY (caller): `obj` is a live COM interface pointer with at least idx+1 slots.
    unsafe { *(*(obj as *mut *mut usize)).add(idx) }
}

/// Calls vtable slot `idx` of `obj`: `vc!(obj, idx, (T1, T2) -> R, a1, a2)` (the types are the method's after `this`).
macro_rules! vc {
    ($obj:expr, $idx:expr, ($($t:ty),*) -> $r:ty $(, $a:expr)*) => {{
        let o: *mut c_void = $obj;
        // SAFETY: `o` is a live interface pointer; the slot has the signature given by the types.
        unsafe {
            let f: unsafe extern "system" fn(*mut c_void $(, $t)*) -> $r = core::mem::transmute(slot(o, $idx));
            f(o $(, $a)*)
        }
    }};
}

fn qi(obj: *mut c_void, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: IUnknown::QueryInterface is slot 0.
    let hr = unsafe {
        let f: unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> i32 = core::mem::transmute(slot(obj, 0));
        f(obj, iid, &mut out)
    };
    (hr, out)
}
fn add_ref(obj: *mut c_void) -> u32 {
    // SAFETY: slot 1 is AddRef.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(obj, 1));
        f(obj)
    }
}
fn release(obj: *mut c_void) -> u32 {
    if obj.is_null() {
        return 0;
    }
    // SAFETY: slot 2 is Release.
    unsafe {
        let f: unsafe extern "system" fn(*mut c_void) -> u32 = core::mem::transmute(slot(obj, 2));
        f(obj)
    }
}
fn create(clsid: &GUID, iid: &GUID) -> (i32, *mut c_void) {
    let mut out: *mut c_void = core::ptr::null_mut();
    // SAFETY: valid pointers.
    let hr = unsafe { CoCreateInstance(clsid, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, iid, &mut out) };
    (hr, out)
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

// ---- known-folder helpers (shell32; prototypes from Learn, ids from knownfolders.h) ----
#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SHGetKnownFolderPath(rfid: *const GUID, flags: u32, token: HANDLE, out: *mut *mut u16) -> i32;
    fn SHGetFolderPathW(hwnd: HANDLE, csidl: i32, token: HANDLE, flags: u32, path: *mut u16) -> i32;
}

const FID_PublicDesktop: GUID = guid(0xC4AA340D, 0xF20F, 0x4863, [0xAF, 0xEF, 0xF8, 0x7E, 0xF2, 0xE6, 0xBA, 0x25]);
const FID_CommonPrograms: GUID = guid(0x0139D44E, 0x6AFE, 0x49F2, [0x86, 0x90, 0x3D, 0xAF, 0xCA, 0xE6, 0xFF, 0xB8]);
const FID_CommonStartMenu: GUID = guid(0xA4115719, 0xD62E, 0x491D, [0xAA, 0x7C, 0xE7, 0x4B, 0x8B, 0xE3, 0xB0, 0x67]);
const FID_CommonStartup: GUID = guid(0x82A5EA35, 0xD9CD, 0x47C5, [0x96, 0x29, 0xE1, 0x5D, 0x2F, 0x71, 0x4E, 0x6E]);
const FID_Programs: GUID = guid(0xA77F5D77, 0x2E2B, 0x44C3, [0xA6, 0xA2, 0xAB, 0xA6, 0x01, 0x05, 0x4A, 0x51]);
const FID_StartMenu: GUID = guid(0x625B53C3, 0xAB48, 0x4EC1, [0xBA, 0x1F, 0xA1, 0xEF, 0x41, 0x46, 0xFC, 0x19]);
const FID_Startup: GUID = guid(0xB97D20BB, 0xF46A, 0x4C97, [0xBA, 0x10, 0x5E, 0x36, 0x08, 0x43, 0x08, 0x54]);
const FID_QuickLaunch: GUID = guid(0x52A4F021, 0x7B75, 0x48A9, [0x9F, 0x6B, 0x4B, 0x87, 0xA2, 0x10, 0xBC, 0x8F]);
const FID_ProgramData: GUID = guid(0x62AB5D82, 0xFDC1, 0x4DC3, [0xA9, 0xDD, 0x07, 0x0D, 0x1D, 0x49, 0x5D, 0x97]);
const FID_Public: GUID = guid(0xDFDF76A2, 0xC82A, 0x4D63, [0x90, 0x6A, 0x56, 0x44, 0xAC, 0x45, 0x73, 0x85]);
const FID_Profile: GUID = guid(0x5E6C858F, 0x0E22, 0x4760, [0x9A, 0xFE, 0xEA, 0x33, 0x17, 0xB6, 0x71, 0x73]);
const FID_UserPinned: GUID = guid(0x9E3995AB, 0x1F9C, 0x4F13, [0xB8, 0x27, 0x48, 0xB2, 0x4B, 0x6C, 0x71, 0x74]);
const FID_RoamingAppData: GUID = guid(0x3EB685DB, 0x65F9, 0x4CF6, [0xA0, 0x3A, 0xE3, 0xEF, 0x65, 0x72, 0x9F, 0x3D]);
const FID_ProgramFiles: GUID = guid(0x905E63B6, 0xC1BF, 0x494E, [0xB2, 0x9C, 0x65, 0xB7, 0x32, 0xD3, 0xD2, 0x1A]);
const FID_ProgramFilesX86: GUID = guid(0x7C5A40EF, 0xA0FB, 0x4BFC, [0x87, 0x4A, 0xC0, 0xF2, 0xE0, 0xB9, 0xFA, 0x8E]);

/// SHGetKnownFolderPath: (hr, path units without NUL; empty on failure).
fn kf(id: &GUID, flags: u32, token: isize) -> (i32, [u16; 300], usize) {
    let mut p: *mut u16 = 1 as *mut u16;
    // SAFETY: valid out pointer; the token is NULL, -1 or a handle the caller owns.
    let hr = unsafe { SHGetKnownFolderPath(id, flags, token as HANDLE, &mut p) };
    let mut out = [0u16; 300];
    let mut n = 0;
    if hr >= 0 && !p.is_null() {
        let s = wstr_at(p, 299);
        n = s.len();
        out[..n].copy_from_slice(s);
        // SAFETY: the path is CoTaskMem memory.
        unsafe { CoTaskMemFree(p as *mut c_void) };
    } else {
        say_null(p);
    }
    (hr, out, n)
}
fn say_null(_p: *mut u16) {}

/// SHGetFolderPathW: (hr, path, len).
fn csidl(id: i32, flags: u32, dw: u32) -> (i32, [u16; 300], usize) {
    let mut b = [0u16; 300];
    // SAFETY: a MAX_PATH+ buffer.
    let hr = unsafe { SHGetFolderPathW(core::ptr::null_mut(), id | flags as i32, core::ptr::null_mut(), dw, b.as_mut_ptr()) };
    let n = wstr_at(b.as_ptr(), 299).len();
    (hr, b, n)
}

fn is_dir(p: &[u16]) -> bool {
    // SAFETY: NUL-terminated.
    let a = unsafe { GetFileAttributesW(p.as_ptr()) };
    a != INVALID_FILE_ATTRIBUTES && a & 0x10 != 0
}

fn starts_nocase(a: &[u16], pre: &[u16]) -> bool {
    a.len() >= pre.len() && !pre.is_empty() && eq_nocase(&a[..pre.len()], pre)
}

/// Path shown relative to the first of the roots that is a prefix: <Public>, <ProgramData>, <Profile>, <ProgramFiles>, <drive>.
fn rel(p: &[u16]) -> Buf<700> {
    let roots: [(&GUID, &str); 4] = [(&FID_Public, "<Public>"), (&FID_ProgramData, "<ProgramData>"), (&FID_Profile, "<Profile>"), (&FID_ProgramFiles, "<ProgramFiles>")];
    let mut b = Buf::<700>::new();
    for (g, name) in roots {
        let (hr, r, n) = kf(g, 0, 0);
        if hr == 0 && starts_nocase(p, &r[..n]) && (p.len() == n || p[n] == 0x5c) {
            let _ = write!(b, "{}{}", name, units_text(&p[n..]).as_str());
            return b;
        }
    }
    let _ = write!(b, "{}", units_text(p).as_str());
    b
}

/// One line "<name>: hr 0x.. path <rel> (dir: 1)".
fn show_kf(t: &mut Test, name: &str, id: &GUID, flags: u32, token: isize) -> (i32, [u16; 300], usize) {
    let r = kf(id, flags, token);
    if r.0 >= 0 {
        let mut z = [0u16; 301];
        z[..r.2].copy_from_slice(&r.1[..r.2]);
        say!(t, "{name}: hr {:#x} path {} (an existing directory: {})", r.0, rel(&r.1[..r.2]).as_str(), is_dir(&z));
    } else {
        say!(t, "{name}: hr {:#x} (no path)", r.0);
    }
    r
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetWindowsDirectoryW(buf: *mut u16, len: u32) -> u32;
}

/// Text of `u` with every occurrence of the Windows directory (any case) shown as <Windows>.
fn norm(u: &[u16]) -> Buf<700> {
    let mut wd = [0u16; 300];
    // SAFETY: valid buffer.
    let n = unsafe { GetWindowsDirectoryW(wd.as_mut_ptr(), 300) } as usize;
    let mut b = Buf::<700>::new();
    let mut i = 0;
    while i < u.len() {
        if n > 0 && i + n <= u.len() && eq_nocase(&u[i..i + n], &wd[..n]) {
            let _ = write!(b, "<Windows>");
            i += n;
        } else {
            let c = u[i];
            if (0x20..0x7f).contains(&c) {
                let _ = write!(b, "{}", c as u8 as char);
            } else {
                let _ = write!(b, "<{c:04x}>");
            }
            i += 1;
        }
    }
    b
}

// ---- registry helpers (advapi32; Learn's prototypes) ----
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(root: isize, sub: *const u16, res: u32, class: *const u16, opts: u32, sam: u32, sa: *mut c_void, out: *mut isize, disp: *mut u32) -> i32;
    fn RegOpenKeyExW(root: isize, sub: *const u16, opts: u32, sam: u32, out: *mut isize) -> i32;
    fn RegSetValueExW(key: isize, name: *const u16, res: u32, ty: u32, data: *const u8, n: u32) -> i32;
    fn RegCloseKey(key: isize) -> i32;
    fn RegDeleteTreeW(root: isize, sub: *const u16) -> i32;
    fn RegDeleteKeyExW(root: isize, sub: *const u16, sam: u32, res: u32) -> i32;
}

const HKCU: isize = 0x8000_0001u32 as i32 as isize;
const HKLM: isize = 0x8000_0002u32 as i32 as isize;
const KEY_ALL_ACCESS: u32 = 0xF003F;
const KEY_WOW64_32KEY: u32 = 0x200;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;

/// Creates (or opens) the key, sets the value (empty name = default) as REG_SZ. Returns the registry error (0 = ok).
fn reg_sz(root: isize, sub: &str, name: &str, val: &str) -> i32 {
    reg_sz_ex(root, sub, name, val, REG_SZ, 0)
}
fn reg_sz_ex(root: isize, sub: &str, name: &str, val: &str, ty: u32, extra_sam: u32) -> i32 {
    let s = wide(sub);
    let n = wide(name);
    let v = wide(val);
    let vn = wstr_at(v.as_ptr(), 299).len();
    let mut k: isize = 0;
    let mut d = 0u32;
    // SAFETY: valid NUL-terminated strings and out pointers.
    unsafe {
        let r = RegCreateKeyExW(root, s.as_ptr(), 0, core::ptr::null(), 0, KEY_ALL_ACCESS | extra_sam, core::ptr::null_mut(), &mut k, &mut d);
        if r != 0 {
            return r;
        }
        let r = RegSetValueExW(k, n.as_ptr(), 0, ty, v.as_ptr() as *const u8, ((vn + 1) * 2) as u32);
        RegCloseKey(k);
        r
    }
}
/// Deletes a key tree; 0 or 2 (not found) are both fine for cleanup.
fn reg_del(root: isize, sub: &str) -> i32 {
    let s = wide(sub);
    // SAFETY: valid NUL-terminated string.
    unsafe { RegDeleteTreeW(root, s.as_ptr()) }
}
// Who may create files in the shortcut folders (elevated token): create, write, delete a unique file in each.

poly_wtest::main!(test);

fn join(dir: &[u16], name: &str) -> [u16; 300] {
    let mut o = [0u16; 300];
    o[..dir.len()].copy_from_slice(dir);
    let mut n = dir.len();
    o[n] = 0x5c;
    n += 1;
    for c in name.chars() {
        o[n] = c as u32 as u16;
        n += 1;
    }
    o
}

fn try_create(t: &mut Test, label: &str, dir: &[u16]) {
    let f = join(dir, "PolySpecB4_kfc_write.tmp");
    // SAFETY: valid NUL-terminated path; CREATE_NEW 1.
    unsafe {
        DeleteFileW(f.as_ptr());
        clear_error();
        let h = CreateFileW(f.as_ptr(), GENERIC_WRITE, 0, core::ptr::null_mut(), 1, FILE_ATTRIBUTE_NORMAL, core::ptr::null_mut());
        if h as isize == -1 {
            say!(t, "{label}: create fails with {}", gle());
            return;
        }
        let mut got = 0u32;
        let w = WriteFile(h, b"abc".as_ptr(), 3, &mut got, core::ptr::null_mut());
        CloseHandle(h);
        let ex = exists(&f);
        clear_error();
        let d = DeleteFileW(f.as_ptr());
        say!(t, "{label}: create ok, write {} ({got} bytes), exists {ex}, delete {} then exists {}", w != 0, d != 0, exists(&f));
    }
}

fn test(t: &mut Test) {
    // SAFETY: plain call.
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    for (n, id) in [
        ("PublicDesktop", &FID_PublicDesktop),
        ("CommonStartMenu", &FID_CommonStartMenu),
        ("CommonPrograms", &FID_CommonPrograms),
        ("CommonStartup", &FID_CommonStartup),
        ("ProgramData", &FID_ProgramData),
        ("Programs", &FID_Programs),
        ("QuickLaunch", &FID_QuickLaunch),
    ] {
        let r = kf(id, 0, 0);
        if r.0 < 0 {
            say!(t, "{n}: no path {:#x}", r.0);
            continue;
        }
        try_create(t, n, &r.1[..r.2]);
    }
}
