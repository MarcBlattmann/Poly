//! ToUnicodeEx (4108 calls in a Chrome run) with the thread's current layout
//! (GetKeyboardLayout(0)) and flag 4 (the keyboard state is not changed),
//! each call with a keyboard state array of the test's own (all keys up
//! except the ones named): the return value (0 no translation, 1 one
//! character, negative a dead key, 2 or more a composite) and the first
//! character as printed ASCII (control characters as U+XXXX, anything else as
//! "non-ASCII"). The keys p o l y A unshifted, with Shift (VK_SHIFT, and
//! VK_LSHIFT alone), with CapsLock toggled (VK_CAPITAL low bit) alone and
//! together with Shift, with Ctrl (control characters), with Alt alone; the
//! digits unshifted; the keys Space, Return, Tab, Back, Escape, Delete; keys
//! that give no character (Shift, Control, F1, Left, CapsLock, the left Windows
//! key); the numpad (with and without NumLock toggled); flags 0, 1 and 4
//! on a plain letter (no dead key is involved, so the state of the thread is
//! not changed by any of them); buffer sizes 0, 1, 2, 4, 8 (what is written
//! to a buffer that is too small); a scan code with the key-up bit; virtual
//! keys 0, 0xFF, 0x100 and 0xFFFFFFFF; a NULL layout and a made-up layout.
//! The NULL state and NULL buffer cases are in tue_null.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

const VK_SHIFT: usize = 0x10;
const VK_MENU: usize = 0x12;
const VK_CONTROL: usize = 0x11;
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

type HKL = *mut c_void;
const MAPVK_VK_TO_VSC: u32 = 0;
const VK_CAPITAL: i32 = 0x14;
const VK_NUMLOCK: i32 = 0x90;
const VK_LSHIFT: i32 = 0xA0;
/// A character as the tests print it: printable ASCII in quotes, control
/// characters as U+XXXX, anything else (layout dependent) as "non-ASCII".
struct Ch(u32);

impl core::fmt::Display for Ch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self.0 {
            0x20..=0x7E => write!(f, "'{}'", self.0 as u8 as char),
            0..=0x7F => write!(f, "U+{:04X}", self.0),
            _ => f.write_str("non-ASCII"),
        }
    }
}

/// A keyboard state array with the given virtual keys down (0x80).
fn kstate(down: &[usize]) -> [u8; 256] {
    let mut s = [0u8; 256];
    for &k in down {
        s[k] = 0x80;
    }
    s
}

/// The result of a ToUnicodeEx call: the return value and the first two
/// characters of the buffer (0x5555 is what is left of the pre-filled
/// buffer when the call writes nothing).
struct Tu {
    ret: i32,
    c0: u16,
    c1: u16,
}

/// ToUnicodeEx of a virtual key (its scan code from MapVirtualKeyW) with a
/// state, flags and layout, into a buffer of `n` characters (at most 8).
fn tue(vk: u32, state: &[u8; 256], flags: u32, hkl: HKL, n: i32) -> Tu {
    let mut buf = [0x5555u16; 8];
    // SAFETY: state has 256 bytes; buf has 8 characters and n is at most 8.
    let ret = unsafe {
        let scan = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
        ToUnicodeEx(vk, scan, state.as_ptr(), buf.as_mut_ptr(), n, flags, hkl)
    };
    Tu { ret, c0: buf[0], c1: buf[1] }
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
    fn GetKeyboardLayout(thread: u32) -> HKL;
    fn ToUnicodeEx(vk: u32, scan: u32, state: *const u8, buf: *mut u16, n: i32, flags: u32, layout: HKL) -> i32;
}

fn rep(t: &mut Test, label: &str, r: &Tu) {
    if r.ret == 0 {
        let c = if r.c0 == 0x5555 { "untouched" } else if r.c0 == 0 { "NUL" } else { "written" };
        say!(t, "  {label}: ret=0 buffer[0] {c}");
    } else if r.ret == 1 {
        say!(t, "  {label}: ret=1 {}", Ch(r.c0 as u32));
    } else {
        say!(t, "  {label}: ret={} first={} second={}", r.ret, Ch(r.c0 as u32), Ch(r.c1 as u32));
    }
}

fn raw(vk: u32, scan: u32, state: *const u8, n: i32, flags: u32, hkl: HKL) -> Tu {
    let mut buf = [0x5555u16; 8];
    // SAFETY: state is 256 bytes; buf has 8 characters and n is at most 8.
    let ret = unsafe { ToUnicodeEx(vk, scan, state, buf.as_mut_ptr(), n, flags, hkl) };
    Tu { ret, c0: buf[0], c1: buf[1] }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain query.
    let hkl = unsafe { GetKeyboardLayout(0) };
    let none = kstate(&[]);
    let letters: [(&str, u32); 5] = [("p", 0x50), ("o", 0x4F), ("l", 0x4C), ("y", 0x59), ("A", 0x41)];

    say!(t, "== letters, no modifier");
    for (n, vk) in letters {
        rep(t, n, &tue(vk, &none, 4, hkl, 4));
    }
    say!(t, "== letters with Shift (VK_SHIFT 0x80)");
    let shift = kstate(&[VK_SHIFT]);
    for (n, vk) in letters {
        rep(t, n, &tue(vk, &shift, 4, hkl, 4));
    }
    say!(t, "== letters with only VK_LSHIFT 0x80");
    let lshift = kstate(&[VK_LSHIFT as usize]);
    for (n, vk) in letters {
        rep(t, n, &tue(vk, &lshift, 4, hkl, 4));
    }
    say!(t, "== letters with CapsLock toggled (VK_CAPITAL = 1), and with Shift too");
    let mut caps = kstate(&[]);
    caps[VK_CAPITAL as usize] = 1;
    let mut caps_shift = kstate(&[VK_SHIFT]);
    caps_shift[VK_CAPITAL as usize] = 1;
    for (n, vk) in letters {
        rep(t, n, &tue(vk, &caps, 4, hkl, 4));
        rep(t, "  with Shift", &tue(vk, &caps_shift, 4, hkl, 4));
        let _ = n;
    }
    say!(t, "== letters with Ctrl (VK_CONTROL 0x80), with Alt (VK_MENU 0x80)");
    let ctrl = kstate(&[VK_CONTROL]);
    let alt = kstate(&[VK_MENU]);
    for (n, vk) in letters {
        rep(t, n, &tue(vk, &ctrl, 4, hkl, 4));
        rep(t, "  with Alt", &tue(vk, &alt, 4, hkl, 4));
    }

    say!(t, "== digits unshifted, and with Shift as a class");
    for (n, vk) in [("0", 0x30u32), ("1", 0x31), ("5", 0x35), ("9", 0x39)] {
        rep(t, n, &tue(vk, &none, 4, hkl, 4));
        let r = tue(vk, &shift, 4, hkl, 4);
        say!(t, "    with Shift: ret={} ASCII={}", r.ret, (r.c0 < 0x80) as i32);
    }

    say!(t, "== keys with a character of their own");
    for (n, vk) in [("Space", 0x20u32), ("Return", 0x0D), ("Tab", 0x09), ("Back", 0x08), ("Escape", 0x1B), ("Delete", 0x2E)] {
        rep(t, n, &tue(vk, &none, 4, hkl, 4));
    }
    say!(t, "== keys without a character");
    for (n, vk) in [("Shift", 0x10u32), ("Control", 0x11), ("F1", 0x70), ("Left", 0x25), ("CapsLock", 0x14), ("LWin", 0x5B)] {
        rep(t, n, &tue(vk, &none, 4, hkl, 4));
    }
    say!(t, "== numpad: Numpad5, Add, Multiply, Divide; Numpad5 with NumLock toggled");
    for (n, vk) in [("Numpad5", 0x65u32), ("Add", 0x6B), ("Multiply", 0x6A), ("Divide", 0x6F)] {
        rep(t, n, &tue(vk, &none, 4, hkl, 4));
    }
    let mut num = kstate(&[]);
    num[VK_NUMLOCK as usize] = 1;
    rep(t, "Numpad5 with NumLock", &tue(0x65, &num, 4, hkl, 4));
    rep(t, "Left with NumLock", &tue(0x25, &num, 4, hkl, 4));

    say!(t, "== flags 0, 1, 4 on p and on Return");
    for (n, vk) in [("p", 0x50u32), ("Return", 0x0D)] {
        for flags in [0u32, 1, 4] {
            let mut b = Buf::<64>::new();
            let _ = write!(b, "{n} flags {flags}");
            rep(t, b.as_str(), &tue(vk, &none, flags, hkl, 4));
        }
    }

    say!(t, "== buffer sizes for p and Return");
    for (n, vk) in [("p", 0x50u32), ("Return", 0x0D)] {
        for size in [0, 1, 2, 4, 8] {
            let mut b = Buf::<64>::new();
            let _ = write!(b, "{n} size {size}");
            rep(t, b.as_str(), &tue(vk, &none, 4, hkl, size));
        }
    }

    say!(t, "== scan codes for p");
    // SAFETY: plain query.
    let scan = unsafe { MapVirtualKeyW(0x50, MAPVK_VK_TO_VSC) };
    rep(t, "scan code of p", &raw(0x50, scan, none.as_ptr(), 4, 4, hkl));
    rep(t, "scan code 0", &raw(0x50, 0, none.as_ptr(), 4, 4, hkl));
    rep(t, "scan code with bit 7 (up)", &raw(0x50, scan | 0x80, none.as_ptr(), 4, 4, hkl));
    rep(t, "scan code with bit 15 (up)", &raw(0x50, scan | 0x8000, none.as_ptr(), 4, 4, hkl));

    say!(t, "== virtual keys out of range");
    for (n, vk) in [("0", 0u32), ("0xFF", 0xFF), ("0x100", 0x100), ("0xFFFFFFFF", 0xFFFF_FFFF)] {
        rep(t, n, &raw(vk, 0, none.as_ptr(), 4, 4, hkl));
    }

    say!(t, "== layouts");
    rep(t, "NULL layout", &raw(0x50, scan, none.as_ptr(), 4, 4, core::ptr::null_mut()));
    rep(t, "made-up layout 0x12345678", &raw(0x50, scan, none.as_ptr(), 4, 4, 0x1234_5678usize as HKL));
    rep(t, "the thread's layout again", &raw(0x50, scan, none.as_ptr(), 4, 4, hkl));
}
