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
// Same as sde_start, run with the filtered token of the administrator (standard).
// the desktop's own Shell.Application (IShellWindows -> the desktop view -> IShellDispatch2.ShellExecute: the child is started by Explorer).
// The started program is this executable with "--child <report file>"; it writes what it is (token, session, parent) into the report.

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetShellWindow() -> HANDLE;
    fn GetWindowThreadProcessId(h: HANDLE, pid: *mut u32) -> u32;
}
#[link(name = "oleaut32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SysAllocString(s: *const u16) -> *mut u16;
    fn SysFreeString(s: *mut u16);
}
#[link(name = "shell32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ShellExecuteExW(info: *mut u64) -> BOOL;
}
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn OpenProcessToken(p: HANDLE, access: u32, out: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(t: HANDLE, class: u32, buf: *mut u8, n: u32, ret: *mut u32) -> BOOL;
}
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetModuleFileNameW(m: HANDLE, buf: *mut u16, n: u32) -> u32;
    fn GetCommandLineW() -> *const u16;
    fn GetCurrentProcessId() -> u32;
    fn OpenProcess(access: u32, inherit: BOOL, pid: u32) -> HANDLE;
    fn QueryFullProcessImageNameW(h: HANDLE, flags: u32, name: *mut u16, n: *mut u32) -> BOOL;
    fn Sleep(ms: u32);
    fn GetTickCount() -> u32;
    fn WaitForSingleObject(h: HANDLE, ms: u32) -> u32;
    fn ProcessIdToSessionId(pid: u32, sid: *mut u32) -> BOOL;
}
#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryInformationProcess(h: HANDLE, class: u32, buf: *mut u8, n: u32, ret: *mut u32) -> i32;
}

const CLSID_Shell: GUID = guid(0x13709620, 0xC279, 0x11CE, [0xA4, 0x9E, 0x44, 0x45, 0x53, 0x54, 0x00, 0x00]);
const CLSID_ShellWindows: GUID = guid(0x9BA05972, 0xF6A8, 0x11CF, [0xA4, 0x42, 0x00, 0xA0, 0xC9, 0x0A, 0x8F, 0x39]);
const IID_IShellWindows: GUID = guid(0x85CB6900, 0x4D95, 0x11CF, [0x96, 0x0C, 0x00, 0x80, 0xC7, 0xF4, 0xEE, 0x85]);
const IID_IDispatch: GUID = guid(0x00020400, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IServiceProvider: GUID = guid(0x6D5140C1, 0x7436, 0x11CE, [0x80, 0x34, 0x00, 0xAA, 0x00, 0x60, 0x09, 0xFA]);
const SID_STopLevelBrowser: GUID = guid(0x4C96BE40, 0x915C, 0x11CF, [0x99, 0xD3, 0x00, 0xAA, 0x00, 0x4A, 0xE8, 0x37]);
const IID_IShellBrowser: GUID = guid(0x000214E2, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IShellFolderViewDual: GUID = guid(0xE7A1AF80, 0x4D96, 0x11CF, [0x96, 0x0C, 0x00, 0x80, 0xC7, 0xF4, 0xEE, 0x85]);
const GUID_NULL: GUID = guid(0, 0, 0, [0; 8]);

// ---- child ----

fn args_has_child() -> Option<[u16; 300]> {
    // SAFETY: the process's command line.
    let cl = unsafe { wstr_at(GetCommandLineW(), 2000) };
    let pat: [u16; 8] = [0x2d, 0x2d, 0x63, 0x68, 0x69, 0x6c, 0x64, 0x20]; // "--child "
    let pos = cl.windows(8).position(|w| w == pat)?;
    let mut out = [0u16; 300];
    let rest = &cl[pos + 8..];
    let rest = if rest.first() == Some(&0x22) { &rest[1..] } else { rest };
    let mut n = 0;
    for &c in rest {
        if c == 0x22 || n >= 299 {
            break;
        }
        out[n] = c;
        n += 1;
    }
    Some(out)
}

fn token_u32(class: u32) -> u32 {
    let mut tok: HANDLE = core::ptr::null_mut();
    let mut v = 0xFFFF_FFFFu32;
    // SAFETY: the current process's own token.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), 8, &mut tok) != 0 {
            let mut n = 0u32;
            GetTokenInformation(tok, class, &mut v as *mut u32 as *mut u8, 4, &mut n);
            CloseHandle(tok);
        }
    }
    v
}

fn token_integrity() -> u32 {
    let mut tok: HANDLE = core::ptr::null_mut();
    let mut buf = [0u8; 128];
    let mut rid = 0xFFFF_FFFFu32;
    // SAFETY: TOKEN_MANDATORY_LABEL: a pointer to the SID, then attributes.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), 8, &mut tok) != 0 {
            let mut n = 0u32;
            if GetTokenInformation(tok, 25, buf.as_mut_ptr(), 128, &mut n) != 0 {
                let sid = *(buf.as_ptr() as *const *const u8);
                let cnt = *sid.add(1) as usize;
                rid = *(sid.add(8 + 4 * (cnt - 1)) as *const u32);
            }
            CloseHandle(tok);
        }
    }
    rid
}

fn parent_pid() -> u32 {
    let mut pbi = [0u64; 6];
    let mut n = 0u32;
    // SAFETY: ProcessBasicInformation of the current process.
    unsafe { NtQueryInformationProcess(GetCurrentProcess(), 0, pbi.as_mut_ptr() as *mut u8, 48, &mut n) };
    pbi[5] as u32
}

/// The file name (lower case) of the image of process `pid`, or "?" when it cannot be asked.
fn image_name_of(pid: u32) -> Buf<64> {
    let mut b = Buf::<64>::new();
    // SAFETY: PROCESS_QUERY_LIMITED_INFORMATION.
    unsafe {
        let h = OpenProcess(0x1000, 0, pid);
        if h.is_null() {
            let _ = write!(b, "?");
            return b;
        }
        let mut name = [0u16; 300];
        let mut n = 300u32;
        if QueryFullProcessImageNameW(h, 0, name.as_mut_ptr(), &mut n) != 0 {
            let s = wstr_at(name.as_ptr(), 299);
            let start = s.iter().rposition(|&c| c == 0x5c).map_or(0, |i| i + 1);
            for &c in &s[start..] {
                let c = if (0x41..=0x5a).contains(&c) { c + 32 } else { c };
                let _ = write!(b, "{}", c as u8 as char);
            }
        } else {
            let _ = write!(b, "?");
        }
        CloseHandle(h);
    }
    b
}

fn child_main(report: &[u16], parent_of_test: u32) -> ! {
    let ppid = parent_pid();
    let mut sid = 0u32;
    // SAFETY: plain call.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut sid) };
    let mut b = Buf::<400>::new();
    let _ = write!(b, "elevation {}\n", token_u32(20));
    let _ = write!(b, "elevation type {}\n", token_u32(18));
    let _ = write!(b, "integrity {:#x}\n", token_integrity());
    let _ = write!(b, "session is 0: {}\n", sid == 0);
    let _ = write!(b, "parent image {}\n", image_name_of(ppid).as_str());
    let _ = write!(b, "parent is the test process: {}\n", ppid == parent_of_test);
    let mut z = [0u16; 301];
    z[..report.len().min(300)].copy_from_slice(&report[..report.len().min(300)]);
    write_all(&z, b.as_str().as_bytes());
    exit(0)
}

macro_rules! child_main_entry {
    ($test:path) => {
        #[unsafe(no_mangle)]
        pub extern "system" fn mainCRTStartup() -> ! {
            if let Some(a) = args_has_child() {
                // "<report path>|<pid of the test>"
                let n = wstr_at(a.as_ptr(), 299).len();
                let s = &a[..n];
                let bar = s.iter().position(|&c| c == 0x7c).unwrap_or(n);
                let mut pid = 0u32;
                for &c in &s[(bar + 1).min(n)..] {
                    pid = pid * 10 + (c as u32).wrapping_sub(0x30);
                }
                child_main(&s[..bar], pid);
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
child_main_entry!(test);

// ---- parent ----

fn my_image() -> [u16; 300] {
    let mut b = [0u16; 300];
    // SAFETY: valid buffer.
    unsafe { GetModuleFileNameW(core::ptr::null_mut(), b.as_mut_ptr(), 299) };
    b
}

/// The child's arguments: `--child <report>|<this pid>`.
fn child_args(report: &[u16]) -> [u16; 600] {
    let mut a = [0u16; 600];
    let mut n = 0;
    for c in "--child ".chars() {
        a[n] = c as u32 as u16;
        n += 1;
    }
    let r = wstr_at(report.as_ptr(), 299);
    for &c in r {
        a[n] = c;
        n += 1;
    }
    a[n] = 0x7c;
    n += 1;
    // SAFETY: plain call.
    let mut pid = unsafe { GetCurrentProcessId() };
    let mut digits = [0u16; 10];
    let mut k = 0;
    while pid > 0 {
        digits[k] = 0x30 + (pid % 10) as u16;
        pid /= 10;
        k += 1;
    }
    while k > 0 {
        k -= 1;
        a[n] = digits[k];
        n += 1;
    }
    a
}

/// Waits (up to 12 s) for the report and prints it line by line, prefixed with the label.
fn show_report(t: &mut Test, label: &str, report: &[u16]) {
    // SAFETY: plain calls.
    let t0 = unsafe { GetTickCount() };
    loop {
        if let Some((b, n)) = read_all::<400>(report) {
            if n > 0 && b[..n].ends_with(b"\n") {
                for line in b[..n].split(|&c| c == b'\n').filter(|l| !l.is_empty()) {
                    let mut s = Buf::<200>::new();
                    for &c in line {
                        let _ = write!(s, "{}", c as char);
                    }
                    say!(t, "{label}: child: {}", s.as_str());
                }
                return;
            }
        }
        // SAFETY: plain calls.
        if unsafe { GetTickCount().wrapping_sub(t0) } > 12000 {
            say!(t, "{label}: no report within 12 s");
            return;
        }
        // SAFETY: plain call.
        unsafe { Sleep(100) };
    }
}

fn bstr(s: &[u16]) -> *mut u16 {
    // SAFETY: a NUL-terminated string.
    unsafe { SysAllocString(s.as_ptr()) }
}

/// Late-bound `ShellExecute(file, args, dir, verb, show)` on an IDispatch of Shell.Application; returns the HRESULT of Invoke.
fn dispatch_shell_execute(disp: *mut c_void, file: &[u16], args: &[u16]) -> (i32, i32) {
    let name = wide("ShellExecute");
    let mut names = [name.as_ptr()];
    let mut id = 0i32;
    let hr = vc!(disp, 5, (*const GUID, *mut *const u16, u32, u32, *mut i32) -> i32, &GUID_NULL, names.as_mut_ptr(), 1, 0x409, &mut id);
    if hr < 0 {
        return (hr, 0);
    }
    let bf = bstr(file);
    let ba = bstr(args);
    // arguments in reverse order: show, verb, dir, args, file
    let mut v: [[u64; 3]; 5] = [[0; 3]; 5];
    v[0] = [3, 0, 0]; // VT_I4: SW_HIDE... shown as 0
    v[1] = [10, 0x8002_0004, 0]; // VT_ERROR, DISP_E_PARAMNOTFOUND
    v[2] = [10, 0x8002_0004, 0];
    v[3] = [8, ba as u64, 0]; // VT_BSTR
    v[4] = [8, bf as u64, 0];
    let mut dp: [u64; 3] = [v.as_mut_ptr() as u64, 0, 5];
    let mut result = [0u64; 3];
    let hr = vc!(disp, 6, (i32, *const GUID, u32, u16, *mut u64, *mut u64, *mut c_void, *mut u32) -> i32, id, &GUID_NULL, 0x409, 1, dp.as_mut_ptr(), result.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut());
    // SAFETY: the BSTRs were allocated above.
    unsafe {
        SysFreeString(bf);
        SysFreeString(ba);
    }
    (id, hr)
}

fn test(t: &mut Test) {
    say!(t, "caller: elevation {}, elevation type {}, integrity {:#x}", token_u32(20), token_u32(18), token_integrity());
    let mut sid = 0u32;
    // SAFETY: plain call.
    unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut sid) };
    say!(t, "caller: session is 0: {}", sid == 0);
    // SAFETY: plain calls.
    let (shell_wnd, mut shell_pid) = unsafe {
        let w = GetShellWindow();
        let mut p = 0u32;
        GetWindowThreadProcessId(w, &mut p);
        (w, p)
    };
    say!(t, "the desktop's shell window exists: {}; its process is {}", !shell_wnd.is_null(), image_name_of(shell_pid).as_str());
    shell_pid = 0;
    let _ = shell_pid;
    // SAFETY: plain call.
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    let image = my_image();

    // A: ShellExecuteExW from this process
    {
        let rep = scratch("sde_a.txt");
        let args = child_args(&rep);
        let mut info = [0u64; 14];
        info[0] = 112 | (0x40u64 << 32);
        info[3] = image.as_ptr() as u64;
        info[4] = args.as_ptr() as u64;
        // SAFETY: a SHELLEXECUTEINFOW of 112 bytes.
        let ok = unsafe { ShellExecuteExW(info.as_mut_ptr()) };
        say!(t, "A ShellExecuteExW: ok {}", ok != 0);
        if ok != 0 {
            show_report(t, "A", &rep);
            // SAFETY: the process handle.
            unsafe { CloseHandle(info[13] as HANDLE) };
        }
    }
    // B: Shell.Application created in this process
    {
        let rep = scratch("sde_b.txt");
        let args = child_args(&rep);
        let mut disp: *mut c_void = core::ptr::null_mut();
        // SAFETY: valid pointers.
        let hr = unsafe { CoCreateInstance(&CLSID_Shell, core::ptr::null_mut(), CLSCTX_INPROC_SERVER, &IID_IDispatch, &mut disp) };
        say!(t, "B Shell.Application: {:#x}", hr);
        if hr >= 0 {
            let (_, hr) = dispatch_shell_execute(disp, &image, &args);
            say!(t, "B ShellExecute through Invoke: {:#x}", hr);
            if hr >= 0 {
                show_report(t, "B", &rep);
            }
            release(disp);
        }
    }
    // C: the desktop's Shell.Application through IShellWindows
    {
        let rep = scratch("sde_c.txt");
        let args = child_args(&rep);
        let mut sw: *mut c_void = core::ptr::null_mut();
        // SAFETY: valid pointers; CLSCTX_LOCAL_SERVER (4): the object lives in Explorer.
        let hr = unsafe { CoCreateInstance(&CLSID_ShellWindows, core::ptr::null_mut(), 4, &IID_IShellWindows, &mut sw) };
        say!(t, "C ShellWindows: {:#x}", hr);
        if hr >= 0 {
            let mut loc = [3u64, 0, 0]; // VT_I4 CSIDL_DESKTOP (0)
            let mut root = [0u64; 3]; // VT_EMPTY
            let mut hwnd = 0i32;
            let mut disp: *mut c_void = core::ptr::null_mut();
            let hr = vc!(sw, 15, (*mut u64, *mut u64, i32, *mut i32, i32, *mut *mut c_void) -> i32, loc.as_mut_ptr(), root.as_mut_ptr(), 8, &mut hwnd, 1, &mut disp);
            say!(t, "C FindWindowSW(desktop): {:#x}, window found {}", hr, hwnd != 0);
            if hr >= 0 && !disp.is_null() {
                let (h1, sp) = qi(disp, &IID_IServiceProvider);
                let mut sb: *mut c_void = core::ptr::null_mut();
                let h2 = if sp.is_null() { h1 } else { vc!(sp, 3, (*const GUID, *const GUID, *mut *mut c_void) -> i32, &SID_STopLevelBrowser, &IID_IShellBrowser, &mut sb) };
                let mut sv: *mut c_void = core::ptr::null_mut();
                let h3 = if sb.is_null() { h2 } else { vc!(sb, 15, (*mut *mut c_void) -> i32, &mut sv) };
                let mut bg: *mut c_void = core::ptr::null_mut();
                let h4 = if sv.is_null() { h3 } else { vc!(sv, 15, (u32, *const GUID, *mut *mut c_void) -> i32, 0, &IID_IDispatch, &mut bg) };
                let (h5, fv) = if bg.is_null() { (h4, core::ptr::null_mut()) } else { qi(bg, &IID_IShellFolderViewDual) };
                let mut app: *mut c_void = core::ptr::null_mut();
                let h6 = if fv.is_null() { h5 } else { vc!(fv, 7, (*mut *mut c_void) -> i32, &mut app) };
                say!(t, "C chain: service provider {:#x}, top-level browser {:#x}, active view {:#x}, background object {:#x}, folder view {:#x}, Application {:#x}", h1, h2, h3, h4, h5, h6);
                if !app.is_null() {
                    let (_, hr) = dispatch_shell_execute(app, &image, &args);
                    say!(t, "C ShellExecute through Invoke: {:#x}", hr);
                    if hr >= 0 {
                        show_report(t, "C", &rep);
                    }
                }
                for p in [app, fv, bg, sv, sb, sp, disp] {
                    release(p);
                }
            }
            release(sw);
        }
    }
    // SAFETY: plain call.
    unsafe { WaitForSingleObject(GetCurrentProcess(), 0) };
}
