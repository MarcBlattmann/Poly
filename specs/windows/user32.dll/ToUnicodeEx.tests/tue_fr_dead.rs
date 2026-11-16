//! The dead keys of the French (KLID 0000040C) layout combined with base keys, ToUnicodeEx flags 0.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

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

/// The pixel at (x, y) of a top-down 32-bit DIB of width w.
fn px(bits: *mut u32, w: i32, x: i32, y: i32) -> u32 {
    // SAFETY: callers stay inside the DIB.
    unsafe { *bits.add((y * w + x) as usize) }
}

/// The rows of a small top-down 32-bit DIB as text: one character per
/// pixel from `key` (pixel value, char), '?' for others.
fn rows(t: &mut Test, bits: *mut u32, w: i32, h: i32, key: &[(u32, char)]) {
    for y in 0..h {
        let mut b = Buf::<300>::new();
        let _ = write!(b, "  |");
        for x in 0..w {
            let v = px(bits, w, x, y);
            let c = key.iter().find(|k| k.0 == v).map(|k| k.1).unwrap_or('?');
            let _ = b.write_char(c);
        }
        let _ = b.write_str("|");
        t.line(b.as_str());
    }
}

type HKL = *mut c_void;

use core::fmt::Write as _;
use poly_wtest::*;

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadKeyboardLayoutW(klid: *const u16, flags: u32) -> HKL;
    fn UnloadKeyboardLayout(hkl: HKL) -> i32;
    fn ToUnicodeEx(vk: u32, scan: u32, state: *const u8, buf: *mut u16, n: i32, flags: u32, hkl: HKL) -> i32;
    fn MapVirtualKeyExW(code: u32, map: u32, hkl: HKL) -> u32;
    fn VkKeyScanExW(ch: u16, hkl: HKL) -> i16;
}

const KLID: &str = "0000040C";
const KLF_NOTELLSHELL: u32 = 0x80;

/// The layout, loaded for this process (not activated, shell not told).
fn load_layout() -> HKL {
    let mut k = [0u16; 9];
    for (i, b) in KLID.bytes().enumerate() {
        k[i] = b as u16;
    }
    // SAFETY: a NUL-terminated KLID string.
    unsafe { LoadKeyboardLayoutW(k.as_ptr(), KLF_NOTELLSHELL) }
}

/// The 256-byte key state of a modifier set: bit 0 Shift, 1 Ctrl, 2 Alt, 3 Caps Lock (toggled).
fn state_of(m: u32) -> [u8; 256] {
    let mut ks = [0u8; 256];
    if m & 1 != 0 {
        ks[0x10] = 0x80;
    }
    if m & 2 != 0 {
        ks[0x11] = 0x80;
    }
    if m & 4 != 0 {
        ks[0x12] = 0x80;
    }
    if m & 8 != 0 {
        ks[0x14] = 0x01;
    }
    ks
}

/// One ToUnicodeEx call, the result as text: "-" no character, "xxxx" one unit, "~xxxx" a dead
/// key's spacing character, "xxxx+yyyy" two units.
fn cell(vk: u32, sc: u32, m: u32, flags: u32, hkl: HKL, out: &mut Buf<300>) {
    let ks = state_of(m);
    let mut u = [0u16; 8];
    // SAFETY: a 256-byte state array and an 8-unit buffer.
    let r = unsafe { ToUnicodeEx(vk, sc, ks.as_ptr(), u.as_mut_ptr(), 8, flags, hkl) };
    if r == 0 {
        let _ = out.write_str("-");
    } else if r < 0 {
        let _ = write!(out, "~{:04x}", u[0]);
    } else {
        for i in 0..(r as usize).min(4) {
            if i > 0 {
                let _ = out.write_str("+");
            }
            let _ = write!(out, "{:04x}", u[i]);
        }
    }
}

/// The rows of every table: digits, letters, the OEM keys, Space and the control keys, the numeric pad.
const KEYS: [u32; 74] = [
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F, 0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0xBA, 0xBB,
    0xBC, 0xBD, 0xBE, 0xBF, 0xC0, 0xDB, 0xDC, 0xDD, 0xDE, 0xDF, 0xE2, 0x20, 0x08, 0x09, 0x0D, 0x1B, 0x60, 0x61, 0x62,
    0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x25, 0x2E, 0x7F, 0x03,
];

/// The modifier sets of a row, with their labels.
const MODS: [(&str, u32); 10] = [
    ("N", 0),
    ("S", 1),
    ("C", 8),
    ("SC", 9),
    ("Ct", 2),
    ("CtS", 3),
    ("AG", 6),
    ("AGS", 7),
    ("Al", 4),
    ("AGC", 14),
];

poly_wtest::main!(test);

/// A dead key, then a base key, with flags 0 (the pending dead key is consumed by the next key).
fn combo(hkl: HKL, dvk: u32, dm: u32, bvk: u32, bm: u32, out: &mut Buf<300>) {
    let ds = state_of(dm);
    let bs = state_of(bm);
    let mut u = [0u16; 8];
    // SAFETY: plain queries; a 256-byte state and an 8-unit buffer.
    unsafe {
        let dsc = MapVirtualKeyExW(dvk, 0, hkl);
        ToUnicodeEx(dvk, dsc, ds.as_ptr(), u.as_mut_ptr(), 8, 0, hkl);
        let bsc = MapVirtualKeyExW(bvk, 0, hkl);
        let r = ToUnicodeEx(bvk, bsc, bs.as_ptr(), u.as_mut_ptr(), 8, 0, hkl);
        if r == 0 {
            let _ = out.write_str(" -");
        } else if r < 0 {
            let _ = write!(out, " ~{:04x}", u[0]);
        } else {
            let _ = out.write_str(" ");
            for i in 0..(r as usize).min(4) {
                if i > 0 {
                    let _ = out.write_str("+");
                }
                let _ = write!(out, "{:04x}", u[i]);
            }
        }
    }
}

fn test(t: &mut Test) {
    let hkl = load_layout();
    say!(t, "load {KLID}: {:#010x}", hkl as usize);
    if hkl.is_null() {
        return;
    }
    // The bases: (vk, modifiers).
    let bases: [(u32, u32); 22] = [
        (0x41, 0), (0x45, 0), (0x49, 0), (0x4F, 0), (0x55, 0), (0x4E, 0), (0x59, 0),
        (0x41, 1), (0x45, 1), (0x49, 1), (0x4F, 1), (0x55, 1), (0x4E, 1), (0x59, 1),
        (0x20, 0), (0x58, 0), (0x31, 0), (0x08, 0), (0x0D, 0), (0x20, 2),
        (0x41, 2), (0x41, 6),
    ];
    say!(t, "bases: a e i o u n y A E I O U N Y Space x 1 Back Enter Ctrl+Space Ctrl+a AltGr+a");
    let mut found = 0;
    for dvk in KEYS {
        // SAFETY: plain query.
        let dsc = unsafe { MapVirtualKeyExW(dvk, 0, hkl) };
        for (name, dm) in MODS {
            let mut c = Buf::<300>::new();
            cell(dvk, dsc, dm, 4, hkl, &mut c);
            if !c.as_str().starts_with('~') {
                continue;
            }
            found += 1;
            let mut line = Buf::<300>::new();
            let _ = write!(line, "dead {dvk:02x} {name} {}:", c.as_str());
            for (bvk, bm) in bases {
                combo(hkl, dvk, dm, bvk, bm, &mut line);
            }
            t.line(line.as_str());
            let mut l2 = Buf::<300>::new();
            let _ = write!(l2, "dead {dvk:02x} {name} twice:");
            combo(hkl, dvk, dm, dvk, dm, &mut l2);
            t.line(l2.as_str());
        }
    }
    say!(t, "dead keys found: {found}");
    // SAFETY: the layout loaded above.
    let r = unsafe { UnloadKeyboardLayout(hkl) };
    say!(t, "unload: {r}");
}
