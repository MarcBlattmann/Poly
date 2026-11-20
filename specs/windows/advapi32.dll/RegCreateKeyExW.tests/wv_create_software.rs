//! RegCreateKeyExW in a 32-bit process: keys made one level at a time from `HKEY_LOCAL_MACHINE`
//! itself, from `HKEY_LOCAL_MACHINE\Software` opened with each view choice (the class key
//! `Classes\CLSID\{GUID}` and a vendor key `<vendor>\Sub` of the program's own), and where each
//! lands, native or in the 32-bit view.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, static_mut_refs, clippy::all)]

// ---- shared helpers of the 32-bit registry view tests (the programs' own code) ----
// poly-arch: x86
//
// These programs are 32-bit: on 64-bit Windows they run under WOW64 and see the 32-bit view of
// the registry. They make keys only under a GUID and a vendor name of their own, with the
// physical (64-bit view) paths below, and remove them again. The declarations are from Microsoft
// Learn (prototypes) and the public SDK headers winreg.h, winnt.h and winerror.h (constants).
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
        let mut b = Buf::<700>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

// winreg.h predefined keys, winnt.h access rights.
const HKEY_CLASSES_ROOT: HKEY = 0x8000_0000;
const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002;
const KEY_READ: u32 = 0x0002_0019;
const KEY_WRITE: u32 = 0x0002_0006;
const KEY_ALL_ACCESS: u32 = 0x000F_003F;
const DELETE: u32 = 0x0001_0000;
const KEY_WOW64_64KEY: u32 = 0x0100;
const KEY_WOW64_32KEY: u32 = 0x0200;
const REG_SZ: u32 = 1;

#[link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated")]
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
    fn RegOpenKeyExW(key: HKEY, sub: *const u16, options: u32, sam: u32, result: *mut HKEY) -> LONG;
    fn RegCloseKey(key: HKEY) -> LONG;
    fn RegDeleteKeyExW(key: HKEY, sub: *const u16, sam: u32, reserved: u32) -> LONG;
    fn RegDeleteKeyW(key: HKEY, sub: *const u16) -> LONG;
    fn RegSetValueExW(key: HKEY, name: *const u16, reserved: u32, kind: u32, data: *const u8, len: u32) -> LONG;
    fn RegQueryValueExW(
        key: HKEY,
        name: *const u16,
        reserved: *mut u32,
        kind: *mut u32,
        data: *mut u8,
        len: *mut u32,
    ) -> LONG;
}

/// The test's own GUID and vendor name: everything these programs create lives under them.
const G: &str = "{A3F0C9E1-5B7D-4C28-8E16-D0B4F2A97C35}";
const VENDOR: &str = "PolyWow64Spec_A3F0";

/// The physical keys (paths below HKEY_LOCAL_MACHINE, as the 64-bit view shows them, which does
/// no redirection) a program's keys can be: the native and the 32-bit-view place of the class key
/// `CLSID\{G}` and of the vendor key.
const CLSID_N: &str = "Software\\Classes\\CLSID\\{A3F0C9E1-5B7D-4C28-8E16-D0B4F2A97C35}";
const CLSID_X: &str = "Software\\Classes\\Wow6432Node\\CLSID\\{A3F0C9E1-5B7D-4C28-8E16-D0B4F2A97C35}";
const VENDOR_N: &str = "Software\\PolyWow64Spec_A3F0";
const VENDOR_X: &str = "Software\\Wow6432Node\\PolyWow64Spec_A3F0";

/// A NUL-ended UTF-16 copy of an ASCII text.
struct W {
    b: [u16; 200],
}

fn w(s: &str) -> W {
    let mut x = W { b: [0; 200] };
    for (i, c) in s.bytes().enumerate() {
        x.b[i] = u16::from(c);
    }
    x
}

fn create(parent: HKEY, path: &str, sam: u32) -> (LONG, HKEY, u32) {
    let p = w(path);
    let mut h: HKEY = 0;
    let mut d = 0u32;
    // SAFETY: a call into advapi32 with NUL-ended text and output slots of this test.
    let r = unsafe {
        RegCreateKeyExW(parent, p.b.as_ptr(), 0, core::ptr::null(), 0, sam, core::ptr::null(), &mut h, &mut d)
    };
    (r, h, d)
}

fn open(parent: HKEY, path: &str, sam: u32) -> (LONG, HKEY) {
    let p = w(path);
    let mut h: HKEY = 0;
    // SAFETY: a call into advapi32 with NUL-ended text and an output slot of this test.
    let r = unsafe { RegOpenKeyExW(parent, p.b.as_ptr(), 0, sam, &mut h) };
    (r, h)
}

fn close(h: HKEY) {
    // SAFETY: closes a key handle of this test.
    unsafe { RegCloseKey(h) };
}

/// Opens `path` below `parent` with `sam` and closes it at once; the status.
fn probe(parent: HKEY, path: &str, sam: u32) -> LONG {
    let (r, h) = open(parent, path, sam);
    if r == 0 {
        close(h);
    }
    r
}

/// Sets the value `name` of `key` to the text `s`.
fn set_text(key: HKEY, name: &str, s: &str) -> LONG {
    let n = w(name);
    let v = w(s);
    let len = (s.len() as u32 + 1) * 2;
    // SAFETY: NUL-ended text and a data buffer of `len` bytes.
    unsafe { RegSetValueExW(key, n.b.as_ptr(), 0, REG_SZ, v.b.as_ptr() as *const u8, len) }
}

/// The text value `name` of `key` (up to 60 characters) appended to `out`; the status.
fn get_text(key: HKEY, name: &str, out: &mut Buf<64>) -> LONG {
    let n = w(name);
    let mut data = [0u16; 64];
    let mut len = 128u32;
    let mut kind = 0u32;
    // SAFETY: NUL-ended text and a data buffer of 128 bytes.
    let r = unsafe {
        RegQueryValueExW(key, n.b.as_ptr(), core::ptr::null_mut(), &mut kind, data.as_mut_ptr() as *mut u8, &mut len)
    };
    if r == 0 {
        for &c in data.iter().take_while(|&&c| c != 0) {
            let _ = out.write_char(char::from_u32(u32::from(c)).unwrap_or('?'));
        }
    }
    r
}

/// Whether `path` (from HKEY_LOCAL_MACHINE) exists in the 64-bit view, which is the physical
/// registry: no redirection happens there.
fn phys(path: &str) -> bool {
    probe(HKEY_LOCAL_MACHINE, path, KEY_READ | KEY_WOW64_64KEY) == 0
}

/// Which of two physical places hold a key: `native`, `x86`, `both` or `none`.
fn side(native: &str, x86: &str) -> &'static str {
    match (phys(native), phys(x86)) {
        (true, true) => "both",
        (true, false) => "native",
        (false, true) => "x86",
        (false, false) => "none",
    }
}

/// Deletes the physical key `path` (it has no subkeys) in the 64-bit view; the status.
fn kill(path: &str) -> LONG {
    let p = w(path);
    // SAFETY: a call into advapi32 with NUL-ended text.
    unsafe { RegDeleteKeyExW(HKEY_LOCAL_MACHINE, p.b.as_ptr(), KEY_WOW64_64KEY, 0) }
}

/// The parents of the vendor key the programs make keys below (paths below HKEY_LOCAL_MACHINE).
const PARENTS: [&str; 17] = [
    "Software",
    "Software\\Microsoft",
    "Software\\Microsoft\\Windows\\CurrentVersion",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Run",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    "Software\\Microsoft\\Windows\\CurrentVersion\\App Paths",
    "Software\\Microsoft\\Windows\\CurrentVersion\\Policies",
    "Software\\Policies",
    "Software\\RegisteredApplications",
    "Software\\Classes",
    "Software\\Classes\\CLSID",
    "Software\\Classes\\Interface",
    "Software\\Classes\\TypeLib",
    "Software\\Classes\\AppID",
    "Software\\Classes\\DirectShow",
    "Software\\Classes\\Media Type",
    "Software\\Classes\\MediaFoundation",
];

/// Removes every key a program of this file can have made (children before parents), in both
/// physical places, so that a run leaves nothing behind and starts from nothing.
fn clean() {
    for name in ["CLSID", "Interface", "TypeLib", "AppID", "DirectShow", "MediaFoundation"] {
        for node in ["", "Wow6432Node\\"] {
            let mut b = Buf::<200>::new();
            let _ = write!(b, "Software\\Classes\\{}{}\\{}\\Child", node, name, G);
            kill(b.as_str());
            let mut b = Buf::<200>::new();
            let _ = write!(b, "Software\\Classes\\{}{}\\{}", node, name, G);
            kill(b.as_str());
        }
    }
    for parent in PARENTS {
        let mut x86 = Buf::<240>::new();
        if let Some(rest) = parent.strip_prefix("Software\\Classes\\") {
            let _ = write!(x86, "Software\\Classes\\Wow6432Node\\{}", rest);
        } else if let Some(rest) = parent.strip_prefix("Software\\") {
            let _ = write!(x86, "Software\\Wow6432Node\\{}", rest);
        } else {
            let _ = write!(x86, "Software\\Wow6432Node");
        }
        for place in [parent, x86.as_str()] {
            for sub in ["\\Sub", ""] {
                let mut b = Buf::<260>::new();
                let _ = write!(b, "{}\\{}{}", place, VENDOR, sub);
                kill(b.as_str());
            }
        }
    }
}

fn bit(b: u32) -> &'static str {
    match b {
        0 => "none",
        KEY_WOW64_32KEY => "32",
        KEY_WOW64_64KEY => "64",
        _ => "?",
    }
}

/// Makes the two class keys `CLSID\{G}` of the native and the 32-bit place, each with the value
/// `view` naming it (`native` / `x86`), and the same for the vendor key, through the 64-bit view.
fn make_pairs() {
    for (path, mark) in [(CLSID_N, "native"), (CLSID_X, "x86"), (VENDOR_N, "native"), (VENDOR_X, "x86")] {
        let (r, h, _) = create(HKEY_LOCAL_MACHINE, path, KEY_ALL_ACCESS | KEY_WOW64_64KEY);
        if r == 0 {
            set_text(h, "view", mark);
            close(h);
        }
    }
}

/// What `view` value the key `path` below `parent`, opened with `sam`, has (the `make_pairs` marker),
/// as `status marker`.
fn marker(parent: HKEY, path: &str, sam: u32, out: &mut Buf<64>) -> LONG {
    let (r, h) = open(parent, path, KEY_READ | sam);
    if r == 0 {
        let q = get_text(h, "view", out);
        if q != 0 {
            let _ = write!(out, "unmarked");
        }
        close(h);
    }
    r
}
// ---- case runners of the 32-bit registry view tests (the programs' own code) ----

/// Creates the components `rel` one at a time (each relative to the key made before), the first
/// with the view bit `first` and the others with `later`, starting at `start` (a predefined key),
/// or at `start\start_path` opened with `ob`; then prints the statuses and dispositions and where
/// the last key is (`native` / `x86` / `both` / `none` of the two physical places given).
fn steps(
    t: &mut Test,
    tag: &str,
    start: HKEY,
    start_path: &str,
    ob: u32,
    rel: &[&str],
    first: u32,
    later: u32,
    native: &str,
    x86: &str,
) {
    clean();
    let mut cur = start;
    let mut mine = false;
    let mut st = Buf::<160>::new();
    if !start_path.is_empty() {
        let (r, h) = open(start, start_path, KEY_READ | KEY_WRITE | ob);
        let _ = write!(st, "open {} ", r);
        if r != 0 {
            say!(t, "{} [handle {}] first {} later {}: {}", tag, bit(ob), bit(first), bit(later), st.as_str());
            return;
        }
        cur = h;
        mine = true;
    }
    for (i, c) in rel.iter().enumerate() {
        let (r, h, d) = create(cur, c, KEY_ALL_ACCESS | if i == 0 { first } else { later });
        let _ = write!(st, "create {}/{} ", r, d);
        if r != 0 {
            break;
        }
        if mine {
            close(cur);
        }
        cur = h;
        mine = true;
    }
    if mine {
        close(cur);
    }
    say!(
        t,
        "{} [handle {}] first {} later {}: {}-> {}",
        tag,
        bit(ob),
        bit(first),
        bit(later),
        st.as_str(),
        side(native, x86)
    );
    clean();
}

/// Creates the whole of `rel` in one call below `start\start_path` opened with `ob`.
fn joined(
    t: &mut Test,
    tag: &str,
    start: HKEY,
    start_path: &str,
    ob: u32,
    rel: &str,
    first: u32,
    native: &str,
    x86: &str,
) {
    clean();
    let mut cur = start;
    let mut mine = false;
    let mut st = Buf::<160>::new();
    if !start_path.is_empty() {
        let (r, h) = open(start, start_path, KEY_READ | KEY_WRITE | ob);
        let _ = write!(st, "open {} ", r);
        if r != 0 {
            say!(t, "{} [handle {}] one call {}: {}", tag, bit(ob), bit(first), st.as_str());
            return;
        }
        cur = h;
        mine = true;
    }
    let (r, h, d) = create(cur, rel, KEY_ALL_ACCESS | first);
    let _ = write!(st, "create {}/{} ", r, d);
    if r == 0 {
        close(h);
    }
    if mine {
        close(cur);
    }
    say!(t, "{} [handle {}] one call {}: {}-> {}", tag, bit(ob), bit(first), st.as_str(), side(native, x86));
    clean();
}

/// Whether the key `path` below `parent` opens with each of the three view choices
/// (none, 32, 64): `ok` or the status.
fn reopen(t: &mut Test, tag: &str, parent: HKEY, path: &str) {
    let f = |b: u32| -> Buf<12> {
        let r = probe(parent, path, KEY_READ | b);
        let mut s = Buf::<12>::new();
        if r == 0 {
            let _ = write!(s, "found");
        } else {
            let _ = write!(s, "{}", r);
        }
        s
    };
    let (a, b, c) = (f(0), f(KEY_WOW64_32KEY), f(KEY_WOW64_64KEY));
    say!(t, "{} reopen: none {}, 32 {}, 64 {}", tag, a.as_str(), b.as_str(), c.as_str());
}

/// `steps` followed by the reopening of the class key from HKEY_LOCAL_MACHINE and HKEY_CLASSES_ROOT
/// (both done before the cleanup).
fn steps_reopen(t: &mut Test, tag: &str, start: HKEY, start_path: &str, ob: u32, rel: &[&str], first: u32, later: u32) {
    clean();
    let mut cur = start;
    let mut mine = false;
    let mut st = Buf::<160>::new();
    if !start_path.is_empty() {
        let (r, h) = open(start, start_path, KEY_READ | KEY_WRITE | ob);
        let _ = write!(st, "open {} ", r);
        if r != 0 {
            say!(t, "{} [handle {}] first {} later {}: {}", tag, bit(ob), bit(first), bit(later), st.as_str());
            return;
        }
        cur = h;
        mine = true;
    }
    for (i, c) in rel.iter().enumerate() {
        let (r, h, d) = create(cur, c, KEY_ALL_ACCESS | if i == 0 { first } else { later });
        let _ = write!(st, "create {}/{} ", r, d);
        if r != 0 {
            break;
        }
        if mine {
            close(cur);
        }
        cur = h;
        mine = true;
    }
    if mine {
        close(cur);
    }
    say!(
        t,
        "{} [handle {}] first {} later {}: {}-> {}",
        tag,
        bit(ob),
        bit(first),
        bit(later),
        st.as_str(),
        side(CLSID_N, CLSID_X)
    );
    reopen(t, "  from HKLM", HKEY_LOCAL_MACHINE, CLSID_N);
    reopen(t, "  from HKCR", HKEY_CLASSES_ROOT, "CLSID\\{A3F0C9E1-5B7D-4C28-8E16-D0B4F2A97C35}");
    clean();
}

/// Opens `rel` below `start\start_path` (opened with `ob`) with `cb` and prints the marker of the
/// key found (`make_pairs` made the keys).
fn open_case(t: &mut Test, tag: &str, start: HKEY, start_path: &str, ob: u32, rel: &str, cb: u32) {
    let mut cur = start;
    let mut mine = false;
    if !start_path.is_empty() {
        let (r, h) = open(start, start_path, KEY_READ | ob);
        if r != 0 {
            say!(t, "{} [handle {}] open {}: parent {}", tag, bit(ob), bit(cb), r);
            return;
        }
        cur = h;
        mine = true;
    }
    let mut m = Buf::<64>::new();
    let r = marker(cur, rel, cb, &mut m);
    say!(t, "{} [handle {}] open {}: {} {}", tag, bit(ob), bit(cb), r, m.as_str());
    if mine {
        close(cur);
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let bs = [0, KEY_WOW64_32KEY, KEY_WOW64_64KEY];
    clean();
    // from the Software key
    for &ob in &bs {
        steps(
            t,
            "software class key",
            HKEY_LOCAL_MACHINE,
            "Software",
            ob,
            &["Classes", "CLSID", G],
            0,
            0,
            CLSID_N,
            CLSID_X,
        );
    }
    for &ob in &bs {
        for &fb in &bs {
            steps(
                t,
                "software vendor key",
                HKEY_LOCAL_MACHINE,
                "Software",
                ob,
                &[VENDOR, "Sub"],
                fb,
                0,
                VENDOR_N,
                VENDOR_X,
            );
        }
    }
    for &ob in &bs {
        joined(
            t,
            "software vendor key",
            HKEY_LOCAL_MACHINE,
            "Software",
            ob,
            "PolyWow64Spec_A3F0\\Sub",
            0,
            VENDOR_N,
            VENDOR_X,
        );
    }
    // from HKEY_LOCAL_MACHINE itself
    for &fb in &bs {
        steps(
            t,
            "machine class key",
            HKEY_LOCAL_MACHINE,
            "",
            0,
            &["Software", "Classes", "CLSID", G],
            fb,
            0,
            CLSID_N,
            CLSID_X,
        );
    }
    for &fb in &bs {
        steps(
            t,
            "machine vendor key",
            HKEY_LOCAL_MACHINE,
            "",
            0,
            &["Software", VENDOR, "Sub"],
            fb,
            0,
            VENDOR_N,
            VENDOR_X,
        );
    }
    for &fb in &bs {
        joined(
            t,
            "machine vendor key",
            HKEY_LOCAL_MACHINE,
            "",
            0,
            "Software\\PolyWow64Spec_A3F0\\Sub",
            fb,
            VENDOR_N,
            VENDOR_X,
        );
    }
    // a key named Wow6432Node made by name is the physical place of the 32-bit view
    for &ob in &bs {
        steps(
            t,
            "software node",
            HKEY_LOCAL_MACHINE,
            "Software\\Classes",
            ob,
            &["Wow6432Node", "CLSID", G],
            0,
            0,
            CLSID_N,
            CLSID_X,
        );
    }
    clean();
}
