//! Which names are one key: a key made at one of the paths below HKLM\SOFTWARE\Classes, HKLM\SOFTWARE\Classes\Wow6432Node and
//! HKLM\SOFTWARE\Wow6432Node (some made with an explicit view, some in the process's own view) and the places each view finds it.
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
// ---- creating and removing the test's own marker keys (this kit's own code) ----
const KEY_ALL_ACCESS: u32 = 0x000F_003F;
const REG_OPTION_NON_VOLATILE: u32 = 0;
const REG_OPTION_OPEN_LINK: u32 = 8;

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: HKEY,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        sam: u32,
        security: *const c_void,
        result: *mut HKEY,
        disposition: *mut u32,
    ) -> LONG;
    fn RegDeleteKeyExW(key: HKEY, sub: *const u16, sam: u32, reserved: u32) -> LONG;
    fn RegSetValueExW(key: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> LONG;
}

/// Creates (or opens) `path` below `root` with the view bit `sam`; the status and the handle.
fn create(root: HKEY, path: &str, sam: u32) -> (LONG, HKEY) {
    let p = w(path);
    let mut h: HKEY = 0;
    let mut d = 0u32;
    // SAFETY: a call into advapi32 with NUL-ended text and output slots of this test.
    let r = unsafe {
        RegCreateKeyExW(
            root,
            p.b.as_ptr(),
            0,
            core::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS | sam,
            core::ptr::null(),
            &mut h,
            &mut d,
        )
    };
    (r, h)
}

/// Creates `path` and closes it at once; the status.
fn make(root: HKEY, path: &str, sam: u32) -> LONG {
    let (r, h) = create(root, path, sam);
    if r == 0 {
        close(h);
    }
    r
}

/// Deletes the key `path` (without children) below `root` in the view `sam`; the status.
fn delete(root: HKEY, path: &str, sam: u32) -> LONG {
    let p = w(path);
    // SAFETY: a call into advapi32 with NUL-ended text.
    unsafe { RegDeleteKeyExW(root, p.b.as_ptr(), sam, 0) }
}

/// Sets the text value `name` of `key`.
fn set_text(key: HKEY, name: &str, s: &str) -> LONG {
    let n = w(name);
    let v = w(s);
    let len = (s.len() as u32 + 1) * 2;
    // SAFETY: NUL-ended text and a data buffer of `len` bytes.
    unsafe { RegSetValueExW(key, n.b.as_ptr(), 0, REG_SZ, v.b.as_ptr() as *const u8, len) }
}

poly_wtest::main!(test);

const HL: HKEY = HKEY_LOCAL_MACHINE;

/// A marker: where it is made, with which view bit, its name, and the places (below HKLM and below HKCR) that are asked for it.
struct Mark {
    path: &'static str,
    sam: u32,
    name: &'static str,
    hklm: &'static [&'static str],
    hkcr: &'static [&'static str],
}

const CLS: &[&str] = &["SOFTWARE\\Classes\\", "SOFTWARE\\Classes\\Wow6432Node\\", "SOFTWARE\\Wow6432Node\\Classes\\"];
const CLSID: &[&str] = &[
    "SOFTWARE\\Classes\\CLSID\\",
    "SOFTWARE\\Classes\\Wow6432Node\\CLSID\\",
    "SOFTWARE\\Wow6432Node\\Classes\\CLSID\\",
];
const IFACE: &[&str] = &[
    "SOFTWARE\\Classes\\Interface\\",
    "SOFTWARE\\Classes\\Wow6432Node\\Interface\\",
    "SOFTWARE\\Wow6432Node\\Classes\\Interface\\",
];
const TLIB: &[&str] = &[
    "SOFTWARE\\Classes\\TypeLib\\",
    "SOFTWARE\\Classes\\Wow6432Node\\TypeLib\\",
    "SOFTWARE\\Wow6432Node\\Classes\\TypeLib\\",
];
const APPID: &[&str] = &[
    "SOFTWARE\\Classes\\AppID\\",
    "SOFTWARE\\Classes\\Wow6432Node\\AppID\\",
    "SOFTWARE\\Wow6432Node\\Classes\\AppID\\",
];
const SW: &[&str] = &["SOFTWARE\\", "SOFTWARE\\Wow6432Node\\"];

const MARKS: [Mark; 17] = [
    Mark {
        path: "SOFTWARE\\Classes\\WdAliasA",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasA",
        hklm: CLS,
        hkcr: &["", "Wow6432Node\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Wow6432Node\\WdAliasB",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasB",
        hklm: CLS,
        hkcr: &["", "Wow6432Node\\"],
    },
    Mark {
        path: "SOFTWARE\\Wow6432Node\\Classes\\WdAliasC",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasC",
        hklm: CLS,
        hkcr: &["", "Wow6432Node\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\TypeLib\\WdAliasD",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasD",
        hklm: TLIB,
        hkcr: &["TypeLib\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Wow6432Node\\TypeLib\\WdAliasE",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasE",
        hklm: TLIB,
        hkcr: &["TypeLib\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\AppID\\WdAliasF",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasF",
        hklm: APPID,
        hkcr: &["AppID\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Wow6432Node\\AppID\\WdAliasG",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasG",
        hklm: APPID,
        hkcr: &["AppID\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\CLSID\\WdAliasH",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasH",
        hklm: CLSID,
        hkcr: &["CLSID\\", "Wow6432Node\\CLSID\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Wow6432Node\\CLSID\\WdAliasI",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasI",
        hklm: CLSID,
        hkcr: &["CLSID\\", "Wow6432Node\\CLSID\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Interface\\WdAliasJ",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasJ",
        hklm: IFACE,
        hkcr: &["Interface\\", "Wow6432Node\\Interface\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Wow6432Node\\Interface\\WdAliasK",
        sam: KEY_WOW64_64KEY,
        name: "WdAliasK",
        hklm: IFACE,
        hkcr: &["Interface\\", "Wow6432Node\\Interface\\"],
    },
    Mark { path: "SOFTWARE\\Wow6432Node\\WdAliasL", sam: KEY_WOW64_64KEY, name: "WdAliasL", hklm: SW, hkcr: &[] },
    Mark { path: "SOFTWARE\\WdAliasM", sam: KEY_WOW64_64KEY, name: "WdAliasM", hklm: SW, hkcr: &[] },
    // made in the process's own view (no view bit): a 64-bit process makes the 64-bit key, a 32-bit process the 32-bit one
    Mark {
        path: "SOFTWARE\\Classes\\CLSID\\WdAliasN",
        sam: 0,
        name: "WdAliasN",
        hklm: CLSID,
        hkcr: &["CLSID\\", "Wow6432Node\\CLSID\\"],
    },
    Mark {
        path: "SOFTWARE\\Classes\\Interface\\WdAliasO",
        sam: 0,
        name: "WdAliasO",
        hklm: IFACE,
        hkcr: &["Interface\\", "Wow6432Node\\Interface\\"],
    },
    Mark { path: "SOFTWARE\\Classes\\TypeLib\\WdAliasP", sam: 0, name: "WdAliasP", hklm: TLIB, hkcr: &["TypeLib\\"] },
    Mark { path: "SOFTWARE\\WdAliasQ", sam: 0, name: "WdAliasQ", hklm: SW, hkcr: &[] },
];

/// Every physical place a marker can be at, to remove it (the 64-bit view is the physical registry).
const PHYSICAL: [&str; 21] = [
    "SOFTWARE\\Classes\\WdAliasA",
    "SOFTWARE\\Classes\\Wow6432Node\\WdAliasB",
    "SOFTWARE\\Classes\\Wow6432Node\\WdAliasC",
    "SOFTWARE\\Wow6432Node\\Classes\\WdAliasC",
    "SOFTWARE\\Classes\\TypeLib\\WdAliasD",
    "SOFTWARE\\Classes\\Wow6432Node\\TypeLib\\WdAliasE",
    "SOFTWARE\\Classes\\AppID\\WdAliasF",
    "SOFTWARE\\Classes\\Wow6432Node\\AppID\\WdAliasG",
    "SOFTWARE\\Classes\\CLSID\\WdAliasH",
    "SOFTWARE\\Classes\\Wow6432Node\\CLSID\\WdAliasI",
    "SOFTWARE\\Classes\\Interface\\WdAliasJ",
    "SOFTWARE\\Classes\\Wow6432Node\\Interface\\WdAliasK",
    "SOFTWARE\\Wow6432Node\\WdAliasL",
    "SOFTWARE\\WdAliasM",
    "SOFTWARE\\Classes\\CLSID\\WdAliasN",
    "SOFTWARE\\Classes\\Wow6432Node\\CLSID\\WdAliasN",
    "SOFTWARE\\Classes\\Interface\\WdAliasO",
    "SOFTWARE\\Classes\\Wow6432Node\\Interface\\WdAliasO",
    "SOFTWARE\\Classes\\TypeLib\\WdAliasP",
    "SOFTWARE\\WdAliasQ",
    "SOFTWARE\\Wow6432Node\\WdAliasQ",
];

fn clean() {
    for p in PHYSICAL {
        delete(HL, p, KEY_WOW64_64KEY);
    }
}

fn test(t: &mut Test) {
    clean();
    for m in &MARKS {
        t.check("the marker is made", make(HL, m.path, m.sam) == 0, 0);
    }
    for m in &MARKS {
        say!(t, "marker {} made at {} with view bit {:#x}", m.name, m.path, m.sam);
        for &(vn, sam) in VIEWS.iter() {
            let mut line = Buf::<590>::new();
            let _ = write!(line, "  {}:", vn);
            for p in m.hklm {
                let mut full = Buf::<300>::new();
                let _ = write!(full, "{}{}", p, m.name);
                let _ = write!(line, " {}={}", p, if exists(HL, full.as_str(), sam) { "yes" } else { "no" });
            }
            for p in m.hkcr {
                let mut full = Buf::<300>::new();
                let _ = write!(full, "{}{}", p, m.name);
                let _ = write!(
                    line,
                    " HKCR\\{}={}",
                    p,
                    if exists(HKEY_CLASSES_ROOT, full.as_str(), sam) { "yes" } else { "no" }
                );
            }
            t.line(line.as_str());
        }
    }
    clean();
    let mut left = 0;
    for p in PHYSICAL {
        if exists(HL, p, KEY_WOW64_64KEY) {
            left += 1;
        }
    }
    t.expect("no marker is left", left, 0);
}
