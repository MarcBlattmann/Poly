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

// ---- shell link helpers (GUIDs and slot numbers from the public headers shobjidl_core.h, objidl.h, propsys.h) ----

const CLSID_ShellLink: GUID = guid(0x0002_1401, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IShellLinkA: GUID = guid(0x0002_14EE, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IShellLinkW: GUID = guid(0x0002_14F9, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPersistFile: GUID = guid(0x0000_010B, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPersistStream: GUID = guid(0x0000_0109, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPersist: GUID = guid(0x0000_010C, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const IID_IPropertyStore: GUID = guid(0x886D_8EEB, 0x8CF2, 0x4446, [0x8D, 0x02, 0xCD, 0xBA, 0x1D, 0xBD, 0xCF, 0x99]);
const IID_IShellLinkDataList: GUID = guid(0x45E2_B4AE, 0xB1C3, 0x11D0, [0xB9, 0x2F, 0x00, 0xA0, 0xC9, 0x03, 0x12, 0xE1]);

// IShellLinkW / IShellLinkA vtable slots (after IUnknown's 0..2)
const SL_GETPATH: usize = 3;
const SL_GETIDLIST: usize = 4;
const SL_SETIDLIST: usize = 5;
const SL_GETDESC: usize = 6;
const SL_SETDESC: usize = 7;
const SL_GETWORKDIR: usize = 8;
const SL_SETWORKDIR: usize = 9;
const SL_GETARGS: usize = 10;
const SL_SETARGS: usize = 11;
const SL_GETHOTKEY: usize = 12;
const SL_SETHOTKEY: usize = 13;
const SL_GETSHOWCMD: usize = 14;
const SL_SETSHOWCMD: usize = 15;
const SL_GETICON: usize = 16;
const SL_SETICON: usize = 17;
const SL_SETRELPATH: usize = 18;
const SL_RESOLVE: usize = 19;
const SL_SETPATH: usize = 20;

// IPersistFile slots
const PF_GETCLASSID: usize = 3;
const PF_ISDIRTY: usize = 4;
const PF_LOAD: usize = 5;
const PF_SAVE: usize = 6;
const PF_SAVECOMPLETED: usize = 7;
const PF_GETCURFILE: usize = 8;

fn hr_text(hr: i32) -> Buf<16> {
    let mut b = Buf::<16>::new();
    let _ = write!(b, "{:#x}", hr as u32);
    b
}

fn sl_new() -> *mut c_void {
    let (_hr, p) = create(&CLSID_ShellLink, &IID_IShellLinkW);
    p
}

fn sl_set(sl: *mut c_void, slot: usize, s: &str) -> i32 {
    let w = wide(s);
    vc!(sl, slot, (*const u16) -> i32, w.as_ptr())
}

fn sl_set_w(sl: *mut c_void, slot: usize, w: &[u16]) -> i32 {
    vc!(sl, slot, (*const u16) -> i32, w.as_ptr())
}

/// A NUL-terminated string of `n` copies of `c` in `buf` (n < buf.len()).
fn fill_units(buf: &mut [u16], c: u16, n: usize) {
    for (i, u) in buf.iter_mut().enumerate() {
        *u = if i < n { c } else { 0 };
    }
}

struct Got {
    hr: i32,
    len: usize,
    terminated: bool,
    past_end_untouched: bool,
    head: Buf<200>,
}

const SENT: u16 = 0x7777;

fn finish_got(hr: i32, buf: &[u16], cch: usize) -> Got {
    let mut len = 0;
    while len < cch && buf[len] != 0 {
        len += 1;
    }
    let terminated = len < cch;
    let head = units_text(&buf[..len.min(90)]);
    let mut h = Buf::<200>::new();
    let _ = write!(h, "{}", head.as_str());
    Got { hr, len, terminated, past_end_untouched: buf[cch] == SENT, head: h }
}

/// Calls a Get(buf, cch) string getter (description, working directory, arguments).
fn sl_get(sl: *mut c_void, slot: usize, cch: usize) -> Got {
    let mut buf = [SENT; 4400];
    let hr = vc!(sl, slot, (*mut u16, i32) -> i32, buf.as_mut_ptr(), cch as i32);
    finish_got(hr, &buf, cch)
}

/// GetPath(buf, cch, fd, flags); also returns whether the find data was written (attributes) and its attribute word.
fn sl_getpath(sl: *mut c_void, cch: usize, flags: u32, want_fd: bool) -> (Got, u32, u32) {
    let mut buf = [SENT; 4400];
    let mut fd = [0u8; 600];
    for b in fd.iter_mut() {
        *b = 0xEE;
    }
    let fdp: *mut u8 = if want_fd { fd.as_mut_ptr() } else { core::ptr::null_mut() };
    let hr = vc!(sl, SL_GETPATH, (*mut u16, i32, *mut u8, u32) -> i32, buf.as_mut_ptr(), cch as i32, fdp, flags);
    let attrs = u32::from_le_bytes([fd[0], fd[1], fd[2], fd[3]]);
    let size_lo = u32::from_le_bytes([fd[32], fd[33], fd[34], fd[35]]);
    (finish_got(hr, &buf, cch), attrs, size_lo)
}

fn sl_geticon(sl: *mut c_void, cch: usize) -> (Got, i32) {
    let mut buf = [SENT; 4400];
    let mut idx = 0x5555i32;
    let hr = vc!(sl, SL_GETICON, (*mut u16, i32, *mut i32) -> i32, buf.as_mut_ptr(), cch as i32, &mut idx as *mut i32);
    (finish_got(hr, &buf, cch), idx)
}

fn show_got(t: &mut Test, what: &str, g: &Got) {
    say!(t, "{what}: hr {} len {} terminated {} text [{}]", hr_text(g.hr).as_str(), g.len, g.terminated, g.head.as_str());
}

fn pf_of(sl: *mut c_void) -> *mut c_void {
    let (_hr, p) = qi(sl, &IID_IPersistFile);
    p
}

fn pf_save(pf: *mut c_void, path: &[u16], remember: i32) -> i32 {
    vc!(pf, PF_SAVE, (*const u16, i32) -> i32, path.as_ptr(), remember)
}

fn pf_load(pf: *mut c_void, path: &[u16], mode: u32) -> i32 {
    vc!(pf, PF_LOAD, (*const u16, u32) -> i32, path.as_ptr(), mode)
}

/// GetCurFile: (hr, text of the returned string or "<null>"), frees the string.
fn pf_curfile(pf: *mut c_void) -> (i32, Buf<400>) {
    let mut p: *mut u16 = 0x1234usize as *mut u16;
    let hr = vc!(pf, PF_GETCURFILE, (*mut *mut u16) -> i32, &mut p as *mut *mut u16);
    let mut b = Buf::<400>::new();
    if p.is_null() {
        let _ = write!(b, "<null>");
    } else if p as usize == 0x1234 {
        let _ = write!(b, "<untouched>");
    } else {
        let u = wstr_at(p, 300);
        let _ = write!(b, "{}", units_text(u).as_str());
        // SAFETY: GetCurFile returns a CoTaskMemAlloc'd string.
        unsafe { CoTaskMemFree(p as *mut c_void) };
    }
    (hr, b)
}

// ---- a reader for the file Save writes (the format is MS-SHLLINK, a public Microsoft format specification) ----

fn rd16(d: &[u8], o: usize) -> u32 {
    if o + 2 > d.len() { return 0; }
    u32::from(u16::from_le_bytes([d[o], d[o + 1]]))
}
fn rd32(d: &[u8], o: usize) -> u32 {
    if o + 4 > d.len() { return 0; }
    u32::from_le_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

const FLAG_NAMES: [&str; 28] = [
    "HasLinkTargetIDList", "HasLinkInfo", "HasName", "HasRelativePath", "HasWorkingDir", "HasArguments", "HasIconLocation",
    "IsUnicode", "ForceNoLinkInfo", "HasExpString", "RunInSeparateProcess", "Unused1", "HasDarwinID", "RunAsUser",
    "HasExpIcon", "NoPidlAlias", "Unused2", "RunWithShimLayer", "ForceNoLinkTrack", "EnableTargetMetadata",
    "DisableLinkPathTracking", "DisableKnownFolderTracking", "DisableKnownFolderAlias", "AllowLinkToLink",
    "UnaliasOnSave", "PreferEnvironmentPath", "KeepLocalIDListForUNCTarget", "b27",
];

/// What a string-data entry holds (UTF-16 with a 16-bit count, or ANSI if IsUnicode is clear).
fn lnk_string(d: &[u8], o: usize, unicode: bool) -> (Buf<300>, usize) {
    let n = rd16(d, o) as usize;
    let mut b = Buf::<300>::new();
    if unicode {
        let mut u = [0u16; 150];
        let m = n.min(150);
        for i in 0..m {
            u[i] = rd16(d, o + 2 + 2 * i) as u16;
        }
        let _ = write!(b, "{}", units_text(&u[..m]).as_str());
        (b, 2 + 2 * n)
    } else {
        for i in 0..n.min(150) {
            let c = d.get(o + 2 + i).copied().unwrap_or(b'?');
            let _ = write!(b, "{}", if (0x20..0x7f).contains(&c) { c as char } else { '.' });
        }
        (b, 2 + n)
    }
}

/// Prints the structure of a .lnk: header fields, flags, IDList size and item count, LinkInfo summary, string data and the extra data blocks.
fn lnk_dump(t: &mut Test, d: &[u8]) {
    say!(t, "file size {} header size {:#x}", d.len(), rd32(d, 0));
    let clsid_ok = d.len() > 0x14 && d[4..20] == [0x01, 0x14, 0x02, 0, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    say!(t, "link CLSID ok {clsid_ok}");
    let flags = rd32(d, 0x14);
    let mut names = Buf::<600>::new();
    for (i, n) in FLAG_NAMES.iter().enumerate() {
        if flags & (1 << i) != 0 {
            let _ = write!(names, " {n}");
        }
    }
    say!(t, "LinkFlags {:#010x}:{}", flags, names.as_str());
    say!(t, "FileAttributes {:#x} FileSize nonzero {} IconIndex {} ShowCommand {} HotKey {:#x}", rd32(d, 0x18), rd32(d, 0x34) != 0, rd32(d, 0x38) as i32, rd32(d, 0x3C), rd16(d, 0x40));
    say!(t, "reserved {:#x} {:#x} {:#x}", rd16(d, 0x42), rd32(d, 0x44), rd32(d, 0x48));
    let mut o = 0x4C;
    let unicode = flags & 0x80 != 0;
    if flags & 1 != 0 {
        let sz = rd16(d, o) as usize;
        // count items
        let mut items = 0;
        let mut p = o + 2;
        let mut sizes = Buf::<500>::new();
        while p + 2 <= o + 2 + sz {
            let s = rd16(d, p) as usize;
            if s == 0 {
                break;
            }
            items += 1;
            let ty = d.get(p + 2).copied().unwrap_or(0);
            let _ = write!(sizes, " {s}/{ty:#04x}");
            if ty & 0x70 == 0x30 {
                // file-entry item: size(2) type(1) pad(1) file size(4) date(2) time(2) attributes(2) primary name (ANSI, NUL ended)
                let mut nm = Buf::<60>::new();
                let mut i = p + 14;
                while i < p + s && d[i] != 0 && nm.n < 50 {
                    let _ = write!(nm, "{}", d[i] as char);
                    i += 1;
                }
                let ext = d[p..p + s].windows(4).any(|w| w == [0x04, 0x00, 0xEF, 0xBE]);
                let _ = write!(sizes, "[{}{}]", nm.as_str(), if ext { ",ext04" } else { "" });
            }
            p += s;
        }
        say!(t, "IDList: size {sz} items {items} (items size/type:{})", sizes.as_str());
        o += 2 + sz;
    }
    if flags & 2 != 0 {
        let sz = rd32(d, o) as usize;
        let hsz = rd32(d, o + 4);
        let lf = rd32(d, o + 8);
        say!(t, "LinkInfo: size {sz} header {hsz} flags {lf:#x}");
        let vid = rd32(d, o + 12) as usize;
        let lbp = rd32(d, o + 16) as usize;
        let cnr = rd32(d, o + 20) as usize;
        let cps = rd32(d, o + 24) as usize;
        say!(t, "LinkInfo: VolumeIDOffset {vid} LocalBasePathOffset {lbp} CommonNetworkRelativeLinkOffset {cnr} CommonPathSuffixOffset {cps}");
        if lf & 1 != 0 && lbp != 0 {
            let mut b = Buf::<300>::new();
            let mut i = o + lbp;
            while i < d.len() && d[i] != 0 && b.n < 250 {
                let _ = write!(b, "{}", d[i] as char);
                i += 1;
            }
            say!(t, "LinkInfo local base path [{}]", b.as_str());
        }
        if lf & 1 != 0 && vid != 0 {
            say!(t, "LinkInfo volume: drive type {} serial-present {}", rd32(d, o + vid + 4), rd32(d, o + vid + 8) != 0);
        }
        if cps != 0 {
            let mut b = Buf::<300>::new();
            let mut i = o + cps;
            while i < d.len() && d[i] != 0 && b.n < 250 {
                let _ = write!(b, "{}", d[i] as char);
                i += 1;
            }
            say!(t, "LinkInfo common path suffix [{}]", b.as_str());
        }
        o += sz;
    }
    for (bit, name) in [(2u32, "NAME_STRING"), (3, "RELATIVE_PATH"), (4, "WORKING_DIR"), (5, "COMMAND_LINE_ARGUMENTS"), (6, "ICON_LOCATION")] {
        if flags & (1 << bit) != 0 {
            let (s, used) = lnk_string(d, o, unicode);
            if bit == 3 {
                // the number of leading "..\\" depends on where the file is: report only whether there are any
                let mut rest = s.as_str();
                let mut n = 0;
                while rest.starts_with("..\\") {
                    rest = &rest[3..];
                    n += 1;
                }
                say!(t, "{name}: leading up-levels {} tail [{rest}]", n > 0);
            } else {
                say!(t, "{name}: [{}]", s.as_str());
            }
            o += used;
        }
    }
    // extra data
    loop {
        if o + 4 > d.len() {
            say!(t, "(file ends at {} without a terminal block)", o);
            break;
        }
        let sz = rd32(d, o) as usize;
        if sz < 4 {
            say!(t, "TerminalBlock at offset-from-extra-start; file ends {} bytes after it: {}", sz, d.len() - o - 4);
            break;
        }
        let sig = rd32(d, o + 4);
        if sig == 0xA000_0009 {
            // its size depends on the user's SID and on the volume: not printed
            say!(t, "ExtraData block signature {sig:#010x}");
        } else {
            say!(t, "ExtraData block signature {sig:#010x} size {sz}");
        }
        o += sz;
    }
}

// ---- property store helpers (propsys.h, wtypes.h: PROPERTYKEY = GUID + DWORD, PROPVARIANT is 24 bytes on x64) ----

#[repr(C)]
#[derive(Clone, Copy)]
struct PKEY {
    fmtid: GUID,
    pid: u32,
}

const FMTID_AppUserModel: GUID = guid(0x9F4C_2855, 0x9F79, 0x4B39, [0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3]);
const PKEY_AppUserModel_ID: PKEY = PKEY { fmtid: FMTID_AppUserModel, pid: 5 };
const PKEY_AppUserModel_ToastActivatorCLSID: PKEY = PKEY { fmtid: FMTID_AppUserModel, pid: 26 };
const PKEY_AppUserModel_RelaunchCommand: PKEY = PKEY { fmtid: FMTID_AppUserModel, pid: 2 };
const PKEY_AppUserModel_PreventPinning: PKEY = PKEY { fmtid: FMTID_AppUserModel, pid: 9 };
const FMTID_Link: GUID = guid(0x0DEF_AD77, 0xA74A, 0x4C7C, [0x80, 0xD6, 0x17, 0x6F, 0x7B, 0xE4, 0xC5, 0xE1]);
const FMTID_LinkTarget: GUID = guid(0xB9B4_B3FC, 0x2B51, 0x4A42, [0xB5, 0xD8, 0x32, 0x41, 0x46, 0xAF, 0xCF, 0x25]);
const PKEY_Link_TargetParsingPath: PKEY = PKEY { fmtid: FMTID_LinkTarget, pid: 2 };
const FMTID_Link2: GUID = guid(0x0B1B_E4B4, 0x0CFA, 0x4E23, [0x84, 0xF5, 0xE3, 0xA1, 0x4D, 0x8F, 0xC1, 0x64]);

const VT_EMPTY: u16 = 0;
const VT_LPWSTR: u16 = 31;
const VT_CLSID: u16 = 72;
const VT_UI4: u16 = 19;
const VT_BOOL: u16 = 11;

const PS_GETCOUNT: usize = 3;
const PS_GETAT: usize = 4;
const PS_GETVALUE: usize = 5;
const PS_SETVALUE: usize = 6;
const PS_COMMIT: usize = 7;

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn PropVariantClear(pv: *mut u8) -> i32;
}

impl Buf<300> {
    fn new_text(s: &str) -> Buf<300> {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "{s}");
        b
    }
}

/// Whether the value of a store entry is printed: only keys whose values do not depend on the machine.
fn show_value(k: &PKEY) -> bool {
    k.fmtid == FMTID_AppUserModel || k.fmtid == FMTID_LinkTarget || k.fmtid == guid(0xF29F_85E0, 0x4FF9, 0x1068, [0xAB, 0x91, 0x08, 0x00, 0x2B, 0x27, 0xB3, 0xD9]) || k.fmtid == guid(0x5E1B_0001, 0x0001, 0x0002, [1, 2, 3, 4, 5, 6, 7, 8])
}

fn pv_empty() -> [u8; 24] {
    [0u8; 24]
}

/// A VT_LPWSTR PROPVARIANT whose string is CoTaskMemAlloc'd (owned by the variant).
fn pv_lpwstr(s: &str) -> [u8; 24] {
    let w = wide(s);
    let n = wstr_at(w.as_ptr(), 300).len() + 1;
    // SAFETY: allocation of n*2 bytes, filled from `w`.
    let p = unsafe { CoTaskMemAlloc(n * 2) } as *mut u16;
    for i in 0..n {
        // SAFETY: within the allocation.
        unsafe { *p.add(i) = w[i] };
    }
    let mut v = [0u8; 24];
    v[0..2].copy_from_slice(&VT_LPWSTR.to_le_bytes());
    v[8..16].copy_from_slice(&(p as usize as u64).to_le_bytes());
    v
}

/// A VT_CLSID PROPVARIANT (pointer to a CoTaskMemAlloc'd GUID).
fn pv_clsid(g: &GUID) -> [u8; 24] {
    // SAFETY: 16 bytes.
    let p = unsafe { CoTaskMemAlloc(16) } as *mut GUID;
    // SAFETY: just allocated.
    unsafe { *p = *g };
    let mut v = [0u8; 24];
    v[0..2].copy_from_slice(&VT_CLSID.to_le_bytes());
    v[8..16].copy_from_slice(&(p as usize as u64).to_le_bytes());
    v
}

fn pv_vt(v: &[u8; 24]) -> u16 {
    u16::from_le_bytes([v[0], v[1]])
}

fn pv_ptr(v: &[u8; 24]) -> usize {
    u64::from_le_bytes([v[8], v[9], v[10], v[11], v[12], v[13], v[14], v[15]]) as usize
}

fn pv_text(v: &[u8; 24]) -> Buf<300> {
    let mut b = Buf::<300>::new();
    let vt = pv_vt(v);
    match vt {
        VT_LPWSTR => {
            let u = wstr_at(pv_ptr(v) as *const u16, 250);
            let _ = write!(b, "VT_LPWSTR [{}]", units_text(u).as_str());
        }
        VT_CLSID => {
            let p = pv_ptr(v) as *const GUID;
            if p.is_null() {
                let _ = write!(b, "VT_CLSID null");
            } else {
                // SAFETY: a GUID pointer in a VT_CLSID variant.
                let g = unsafe { *p };
                let _ = write!(b, "VT_CLSID {}", guid_text(&g).as_str());
            }
        }
        2 | 3 | 11 | 16..=23 => {
            let _ = write!(b, "vt {vt} value {:#x}", pv_ptr(v) as u32);
        }
        VT_EMPTY => {
            let _ = write!(b, "VT_EMPTY");
        }
        _ => {
            let _ = write!(b, "vt {vt} (pointer or other)");
        }
    }
    b
}

fn pv_clear(v: &mut [u8; 24]) -> i32 {
    // SAFETY: a PROPVARIANT we built or the store filled.
    unsafe { PropVariantClear(v.as_mut_ptr()) }
}

fn ps_set(ps: *mut c_void, key: &PKEY, v: &[u8; 24]) -> i32 {
    vc!(ps, PS_SETVALUE, (*const PKEY, *const u8) -> i32, key as *const PKEY, v.as_ptr())
}

fn ps_get(ps: *mut c_void, key: &PKEY) -> (i32, [u8; 24]) {
    let mut v = [0xAAu8; 24];
    let hr = vc!(ps, PS_GETVALUE, (*const PKEY, *mut u8) -> i32, key as *const PKEY, v.as_mut_ptr());
    (hr, v)
}

fn ps_of(sl: *mut c_void) -> *mut c_void {
    let (_hr, p) = qi(sl, &IID_IPropertyStore);
    p
}

fn ps_count(ps: *mut c_void) -> (i32, u32) {
    let mut n = 0xFFFFu32;
    let hr = vc!(ps, PS_GETCOUNT, (*mut u32) -> i32, &mut n as *mut u32);
    (hr, n)
}

fn key_text(k: &PKEY) -> Buf<100> {
    let mut b = Buf::<100>::new();
    let _ = write!(b, "{} pid {}", guid_text(&k.fmtid).as_str(), k.pid);
    b
}

/// Lists the keys of a store (index, key, value type).
fn ps_list(t: &mut Test, label: &str, ps: *mut c_void) {
    let (hr, n) = ps_count(ps);
    say!(t, "{label}: GetCount hr {} count {n}", hr_text(hr).as_str());
    for i in 0..n.min(40) {
        let mut k = PKEY { fmtid: guid(0, 0, 0, [0; 8]), pid: 0 };
        let hr = vc!(ps, PS_GETAT, (u32, *mut PKEY) -> i32, i, &mut k as *mut PKEY);
        let (hr2, mut v) = ps_get(ps, &k);
        say!(t, "{label}: [{i}] GetAt hr {} key {} GetValue hr {} {}", hr_text(hr).as_str(), key_text(&k).as_str(), hr_text(hr2).as_str(), if show_value(&k) { pv_text(&v) } else { Buf::<300>::new_text("(value not printed)") }.as_str());
        pv_clear(&mut v);
    }
}

// propsys.dll InitPropVariantFromCLSID: the variant it builds. (propsys is loaded at run time: it is not an import of the test.)

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(m: *mut c_void, name: *const u8) -> *mut c_void;
}

poly_wtest::main!(test);

type InitFn = unsafe extern "system" fn(*const GUID, *mut u8) -> i32;

fn test(t: &mut Test) {
    // A first-line comment tells this test apart from pv_clsid: the variant goes into a ShellLink's property store.
    let name = wide("propsys.dll");
    // SAFETY: valid strings.
    let m = unsafe { LoadLibraryW(name.as_ptr()) };
    // SAFETY: valid NUL-terminated name.
    let f = unsafe { GetProcAddress(m, b"InitPropVariantFromCLSID ".as_ptr()) };
    if f.is_null() {
        say!(t, "not exported");
        return;
    }
    // SAFETY: the export has this signature (propvarutil.h).
    let init: InitFn = unsafe { core::mem::transmute(f) };
    let g = guid(0xA6B5_E3B5, 0x1122, 0x4A0B, [0x80, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66]);
    // SAFETY: plain call.
    unsafe { CoInitializeEx(core::ptr::null_mut(), COINIT_APARTMENTTHREADED) };
    let (_h, sl2) = create(&CLSID_ShellLink, &IID_IShellLinkW);
    if !sl2.is_null() {
        let ps = ps_of(sl2);
        let mut v = [0u8; 24];
        // SAFETY: valid pointers.
        unsafe { init(&g, v.as_mut_ptr()) };
        let hr = ps_set(ps, &PKEY_AppUserModel_ToastActivatorCLSID, &v);
        pv_clear(&mut v);
        let (hr2, mut got) = ps_get(ps, &PKEY_AppUserModel_ToastActivatorCLSID);
        say!(t, "stored as ToastActivatorCLSID: SetValue hr {} GetValue hr {} {}", hr_text(hr).as_str(), hr_text(hr2).as_str(), pv_text(&got).as_str());
        pv_clear(&mut got);
        release(ps);
        release(sl2);
    }
    // SAFETY: balanced.
    unsafe { CoUninitialize() };
}
