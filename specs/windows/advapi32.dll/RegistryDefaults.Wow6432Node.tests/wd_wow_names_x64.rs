//! The Wow6432Node keys a Windows machine has before any program is installed: the children of
//! HKLM\SOFTWARE, of HKLM\SOFTWARE\Wow6432Node and of HKLM\SOFTWARE\Classes\Wow6432Node, as the 64-bit view and the 32-bit view show them.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, static_mut_refs, clippy::all)]

// ---- shared helpers of the default-registry tests (this kit's own code): names and values only ----
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type LONG = i32;
type HKEY = usize;

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
    fn push(&mut self, c: u8) {
        if self.n < N {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            self.push(c);
        }
        Ok(())
    }
}

/// `t.line` with formatting.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<590>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const HKEY_CLASSES_ROOT: HKEY = 0x8000_0000;
const HKEY_CURRENT_USER: HKEY = 0x8000_0001;
const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002;
const KEY_READ: u32 = 0x0002_0019;
const KEY_WOW64_64KEY: u32 = 0x0100;
const KEY_WOW64_32KEY: u32 = 0x0200;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_BINARY: u32 = 3;
const REG_DWORD: u32 = 4;
const REG_MULTI_SZ: u32 = 7;
const REG_QWORD: u32 = 11;
const ERROR_NO_MORE_ITEMS: LONG = 259;

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegOpenKeyExW(key: HKEY, sub: *const u16, options: u32, sam: u32, result: *mut HKEY) -> LONG;
    fn RegCloseKey(key: HKEY) -> LONG;
    fn RegEnumKeyExW(
        key: HKEY,
        index: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        class: *mut u16,
        class_len: *mut u32,
        last_write: *mut u64,
    ) -> LONG;
    fn RegEnumValueW(
        key: HKEY,
        index: u32,
        name: *mut u16,
        name_len: *mut u32,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        data_len: *mut u32,
    ) -> LONG;
    fn RegQueryInfoKeyW(
        key: HKEY,
        class: *mut u16,
        class_len: *mut u32,
        reserved: *mut u32,
        subkeys: *mut u32,
        max_subkey: *mut u32,
        max_class: *mut u32,
        values: *mut u32,
        max_value_name: *mut u32,
        max_value: *mut u32,
        security: *mut u32,
        last_write: *mut u64,
    ) -> LONG;
    fn RegQueryValueExW(
        key: HKEY,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> LONG;
}

/// A NUL-ended UTF-16 copy of an ASCII text.
struct W {
    b: [u16; 300],
}

fn w(s: &str) -> W {
    let mut x = W { b: [0; 300] };
    for (i, c) in s.bytes().enumerate() {
        x.b[i] = u16::from(c);
    }
    x
}

/// The views a process can ask for: its own default and the two explicit ones. `sam` is the extra
/// access bit; the labels name what the bit selects.
#[cfg(target_pointer_width = "64")]
const VIEWS: [(&str, u32); 2] = [("64-bit view", KEY_WOW64_64KEY), ("32-bit view", KEY_WOW64_32KEY)];
#[cfg(not(target_pointer_width = "64"))]
const VIEWS: [(&str, u32); 3] =
    [("default view", 0), ("32-bit view", KEY_WOW64_32KEY), ("64-bit view", KEY_WOW64_64KEY)];

fn open(root: HKEY, path: &str, sam: u32) -> Option<HKEY> {
    let p = w(path);
    let mut h: HKEY = 0;
    // SAFETY: a call into advapi32 with NUL-ended text and an output slot of this test.
    let r = unsafe { RegOpenKeyExW(root, p.b.as_ptr(), 0, KEY_READ | sam, &mut h) };
    if r == 0 { Some(h) } else { None }
}

/// The status of opening `path` (0 when it exists).
fn open_status(root: HKEY, path: &str, sam: u32) -> LONG {
    let p = w(path);
    let mut h: HKEY = 0;
    // SAFETY: a call into advapi32 with NUL-ended text and an output slot of this test.
    let r = unsafe { RegOpenKeyExW(root, p.b.as_ptr(), 0, KEY_READ | sam, &mut h) };
    if r == 0 {
        close(h);
    }
    r
}

fn close(h: HKEY) {
    // SAFETY: closes a key handle of this test.
    unsafe { RegCloseKey(h) };
}

fn exists(root: HKEY, path: &str, sam: u32) -> bool {
    open_status(root, path, sam) == 0
}

/// The number of subkeys and values of an open key.
fn counts(h: HKEY) -> (u32, u32) {
    let (mut s, mut v) = (0u32, 0u32);
    // SAFETY: output slots of this test; the other outputs are not asked for.
    unsafe {
        RegQueryInfoKeyW(
            h,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut s,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut v,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        );
    }
    (s, v)
}

fn put_ascii<const N: usize>(out: &mut Buf<N>, units: &[u16]) {
    for &u in units {
        out.push(if (0x20..0x7f).contains(&u) { u as u8 } else { b'?' });
    }
}

/// The name of the `i`th subkey of `h` (ASCII, other characters as `?`); false past the last.
fn sub_name(h: HKEY, i: u32, out: &mut Buf<200>) -> bool {
    let mut name = [0u16; 260];
    let mut len = 260u32;
    // SAFETY: a name buffer of 260 units and its length.
    let r = unsafe {
        RegEnumKeyExW(
            h,
            i,
            name.as_mut_ptr(),
            &mut len,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
        )
    };
    if r != 0 {
        return false;
    }
    put_ascii(out, &name[..len as usize]);
    true
}

/// Whether `h` has a subkey of exactly this name (compared without case).
fn has_sub(h: HKEY, want: &str) -> bool {
    let mut i = 0;
    loop {
        let mut b = Buf::<200>::new();
        if !sub_name(h, i, &mut b) {
            return false;
        }
        if b.as_str().eq_ignore_ascii_case(want) {
            return true;
        }
        i += 1;
    }
}

/// `value <name> = <kind> <data>` of the `i`th value; false past the last.
fn value_line<const N: usize>(h: HKEY, i: u32, out: &mut Buf<N>) -> bool {
    let mut name = [0u16; 260];
    let mut nlen = 260u32;
    let mut data = [0u8; 400];
    let mut dlen = 400u32;
    let mut kind = 0u32;
    // SAFETY: a name buffer of 260 units and a data buffer of 400 bytes with their lengths.
    let r = unsafe {
        RegEnumValueW(
            h,
            i,
            name.as_mut_ptr(),
            &mut nlen,
            core::ptr::null_mut(),
            &mut kind,
            data.as_mut_ptr(),
            &mut dlen,
        )
    };
    if r != 0 {
        return false;
    }
    let _ = write!(out, "value \"");
    put_ascii(out, &name[..nlen as usize]);
    let _ = write!(out, "\" = ");
    format_data(out, kind, &data[..dlen as usize]);
    true
}

fn format_data<const N: usize>(out: &mut Buf<N>, kind: u32, d: &[u8]) {
    let units = |d: &[u8]| -> [u16; 200] {
        let mut u = [0u16; 200];
        for (k, c) in d.chunks_exact(2).take(200).enumerate() {
            u[k] = u16::from_le_bytes([c[0], c[1]]);
        }
        u
    };
    match kind {
        REG_SZ | REG_EXPAND_SZ => {
            let _ = write!(out, "{} \"", if kind == REG_SZ { "sz" } else { "expand_sz" });
            let u = units(d);
            let n = u.iter().position(|&c| c == 0).unwrap_or(u.len());
            put_ascii(out, &u[..n]);
            let _ = write!(out, "\"");
        }
        REG_MULTI_SZ => {
            let _ = write!(out, "multi_sz \"");
            let u = units(d);
            let mut n = d.len() / 2;
            while n > 0 && u[n.min(199) - 1] == 0 && n > 1 {
                n -= 1;
            }
            let n = n.min(200);
            for &c in &u[..n] {
                if c == 0 {
                    out.push(b'|');
                } else {
                    put_ascii(out, &[c]);
                }
            }
            let _ = write!(out, "\"");
        }
        REG_DWORD if d.len() == 4 => {
            let _ = write!(out, "dword 0x{:08x}", u32::from_le_bytes([d[0], d[1], d[2], d[3]]));
        }
        REG_QWORD if d.len() == 8 => {
            let mut b = [0u8; 8];
            b.copy_from_slice(d);
            let _ = write!(out, "qword 0x{:016x}", u64::from_le_bytes(b));
        }
        _ => {
            let _ = write!(out, "type {} {} bytes", kind, d.len());
            if kind == REG_BINARY {
                let _ = write!(out, " hex");
                for b in d.iter().take(16) {
                    let _ = write!(out, " {:02x}", b);
                }
            }
        }
    }
}

/// The data of the value `name` (empty text is the default value) as a formatted line part, or None.
fn value_text<const N: usize>(h: HKEY, name: &str, out: &mut Buf<N>) -> bool {
    let n = w(name);
    let mut data = [0u8; 400];
    let mut len = 400u32;
    let mut kind = 0u32;
    // SAFETY: NUL-ended text and a data buffer of 400 bytes.
    let r = unsafe { RegQueryValueExW(h, n.b.as_ptr(), core::ptr::null_mut(), &mut kind, data.as_mut_ptr(), &mut len) };
    if r != 0 {
        return false;
    }
    format_data(out, kind, &data[..len as usize]);
    true
}

/// One line per value of `h`, prefixed by `label`.
fn dump_values(t: &mut Test, label: &str, h: HKEY) {
    let mut i = 0;
    loop {
        let mut b = Buf::<590>::new();
        let _ = write!(b, "{}: ", label);
        if !value_line(h, i, &mut b) {
            break;
        }
        t.line(b.as_str());
        i += 1;
    }
}

/// The names of the subkeys of `h`, up to `max`, in one or several lines of about 500 characters.
fn dump_subkeys(t: &mut Test, label: &str, h: HKEY, max: u32) {
    let (n, _) = counts(h);
    say!(t, "{}: {} subkeys", label, n);
    let mut line = Buf::<590>::new();
    let _ = write!(line, "{}: names", label);
    let mut i = 0;
    while i < n && i < max {
        let mut b = Buf::<200>::new();
        if !sub_name(h, i, &mut b) {
            break;
        }
        if line.n + b.n + 3 > 500 {
            t.line(line.as_str());
            line = Buf::<590>::new();
            let _ = write!(line, "{}: names", label);
        }
        let _ = write!(line, " [{}]", b.as_str());
        i += 1;
    }
    t.line(line.as_str());
}

/// Subkey and value counts of `root\path` in the view `sam`, and whether it exists.
fn show_counts(t: &mut Test, tag: &str, root: HKEY, path: &str, sam: u32) {
    match open(root, path, sam) {
        Some(h) => {
            let (s, v) = counts(h);
            say!(t, "{} {}: exists, {} subkeys, {} values", tag, path, s, v);
            close(h);
        }
        None => say!(t, "{} {}: missing {}", tag, path, open_status(root, path, sam)),
    }
}

/// The text of the default value or a named value as `Some("kind \"text\"")`.
fn text_of(root: HKEY, path: &str, sam: u32, name: &str) -> Option<Buf<300>> {
    let h = open(root, path, sam)?;
    let mut b = Buf::<300>::new();
    let ok = value_text(h, name, &mut b);
    close(h);
    if ok { Some(b) } else { None }
}

poly_wtest::main!(test);

const HL: HKEY = HKEY_LOCAL_MACHINE;

fn children(t: &mut Test, label: &str, path: &str, sam: u32, names: &[&str]) {
    match open(HL, path, sam) {
        Some(h) => {
            let mut line = Buf::<590>::new();
            let _ = write!(line, "{} has:", label);
            for n in names {
                let _ = write!(line, " {}={}", n, if has_sub(h, n) { "yes" } else { "no" });
            }
            t.line(line.as_str());
            close(h);
        }
        None => say!(t, "{} is missing ({})", label, open_status(HL, path, sam)),
    }
}

fn test(t: &mut Test) {
    for &(vn, sam) in VIEWS.iter() {
        say!(t, "=== {}", vn);
        // what a fresh machine has at the top of the software hive
        children(
            t,
            "SOFTWARE",
            "SOFTWARE",
            sam,
            &["Classes", "Clients", "Microsoft", "Policies", "RegisteredApplications", "Wow6432Node"],
        );
        children(
            t,
            "SOFTWARE\\Wow6432Node",
            "SOFTWARE\\Wow6432Node",
            sam,
            &["Classes", "Clients", "Microsoft", "Policies", "RegisteredApplications", "Wow6432Node"],
        );
        children(
            t,
            "SOFTWARE\\Classes\\Wow6432Node",
            "SOFTWARE\\Classes\\Wow6432Node",
            sam,
            &["CLSID", "Interface", "TypeLib", "AppID", "DirectShow", "Media Type", "MediaFoundation"],
        );
        children(
            t,
            "SOFTWARE\\Wow6432Node\\Classes",
            "SOFTWARE\\Wow6432Node\\Classes",
            sam,
            &["CLSID", "Interface", "TypeLib", "AppID", "DirectShow", "Media Type", "MediaFoundation", "Wow6432Node"],
        );
        // keys that exist
        for p in [
            "SOFTWARE\\Wow6432Node",
            "SOFTWARE\\Wow6432Node\\Wow6432Node",
            "SOFTWARE\\Wow6432Node\\Microsoft",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows NT\\CurrentVersion",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\App Paths",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Run",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\RunOnce",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Ole",
            "SOFTWARE\\Wow6432Node\\Microsoft\\COM3",
            "SOFTWARE\\Wow6432Node\\Microsoft\\Rpc",
            "SOFTWARE\\Wow6432Node\\Classes",
            "SOFTWARE\\Wow6432Node\\Classes\\CLSID",
            "SOFTWARE\\Wow6432Node\\Classes\\Interface",
            "SOFTWARE\\Wow6432Node\\Classes\\TypeLib",
            "SOFTWARE\\Wow6432Node\\Classes\\AppID",
            "SOFTWARE\\Classes\\Wow6432Node",
            "SOFTWARE\\Classes\\Wow6432Node\\CLSID",
            "SOFTWARE\\Classes\\Wow6432Node\\Interface",
            "SOFTWARE\\Classes\\Wow6432Node\\TypeLib",
            "SOFTWARE\\Classes\\Wow6432Node\\AppID",
            "SOFTWARE\\Classes\\Wow6432Node\\Wow6432Node",
            "SOFTWARE\\Classes\\CLSID",
            "SOFTWARE\\Classes\\Interface",
            "SOFTWARE\\Classes\\TypeLib",
            "SOFTWARE\\Classes\\AppID",
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion",
            "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion",
            "SOFTWARE\\Microsoft\\Ole",
            "SOFTWARE\\Microsoft\\COM3",
        ] {
            say!(t, "{}: {}", p, if exists(HL, p, sam) { "exists" } else { "missing" });
        }
        for p in [
            "Wow6432Node",
            "Wow6432Node\\CLSID",
            "Wow6432Node\\Interface",
            "Wow6432Node\\TypeLib",
            "Wow6432Node\\AppID",
            "CLSID",
            "Interface",
            "TypeLib",
            "AppID",
        ] {
            say!(
                t,
                "HKEY_CLASSES_ROOT\\{}: {}",
                p,
                if exists(HKEY_CLASSES_ROOT, p, sam) { "exists" } else { "missing" }
            );
        }
    }
}
