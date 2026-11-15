//! ToUnicodeEx with flag 4 and dead keys (Chrome calls it this way for every
//! key to find out whether a key is a dead key without disturbing the
//! thread's dead key state). Every virtual key from 0x20 to 0xFE is tried
//! with four states (no modifier, Shift, Ctrl+Alt as AltGr, Shift+Ctrl+Alt)
//! on the thread's layout, only with flag 4, so nothing is stored: whether
//! any combination gives a negative result (a dead key) is printed as
//! yes/no (it depends on the layout); no combination gives 2 or more (a
//! composite needs a stored dead key, which flag 4 never stores); with a
//! dead key found: the result and character of the first one, the same
//! result on a second and a third call, the buffer sizes 0, 1, 2, the next
//! key (a, then Space) right after it translates as if the dead key had not
//! been pressed (the same as before the dead key), and the result of a
//! plain letter is the same before and after the whole scan. Without a dead
//! key only the last line is printed.
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

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: plain query.
    let hkl = unsafe { GetKeyboardLayout(0) };
    let states = [
        ("no modifier", kstate(&[])),
        ("Shift", kstate(&[VK_SHIFT])),
        ("Ctrl+Alt", kstate(&[VK_CONTROL, VK_MENU])),
        ("Shift+Ctrl+Alt", kstate(&[VK_SHIFT, VK_CONTROL, VK_MENU])),
    ];
    let plain = kstate(&[]);
    let a_before = tue(0x41, &plain, 4, hkl, 4);
    let space_before = tue(0x20, &plain, 4, hkl, 4);

    let mut first: Option<(u32, usize, i32, u16)> = None;
    let mut composites = 0;
    for vk in 0x20u32..=0xFE {
        for (i, (_, st)) in states.iter().enumerate() {
            let r = tue(vk, st, 4, hkl, 4);
            if r.ret < 0 && first.is_none() {
                first = Some((vk, i, r.ret, r.c0));
            }
            if r.ret >= 2 {
                composites += 1;
            }
        }
    }
    say!(t, "a dead key was found: {}", first.is_some() as i32);
    say!(t, "results of 2 or more: {composites}");
    if let Some((vk, i, ret, c0)) = first {
        let (name, st) = &states[i];
        say!(t, "first dead key: with state \"{name}\": ret={ret} char={}", Ch(c0 as u32));
        let r2 = tue(vk, st, 4, hkl, 4);
        let r3 = tue(vk, st, 4, hkl, 4);
        say!(t, "second and third call: ret={} and {}, same character: {}", r2.ret, r3.ret, (r2.c0 == c0 && r3.c0 == c0) as i32);
        for size in [0, 1, 2] {
            let r = tue(vk, st, 4, hkl, size);
            say!(t, "buffer of {size}: ret={} first {}", r.ret, if r.c0 == 0x5555 { "untouched" } else { "written" });
        }
        let a = tue(0x41, &plain, 4, hkl, 4);
        say!(t, "a right after the dead key: ret={} {}, the same as before: {}", a.ret, Ch(a.c0 as u32), (a.ret == a_before.ret && a.c0 == a_before.c0) as i32);
        let sp = tue(0x20, &plain, 4, hkl, 4);
        say!(t, "Space right after the dead key: ret={} {}, the same as before: {}", sp.ret, Ch(sp.c0 as u32), (sp.ret == space_before.ret && sp.c0 == space_before.c0) as i32);
    }
    let a_after = tue(0x41, &plain, 4, hkl, 4);
    say!(t, "a after the whole scan: ret={} {}, the same as before: {}", a_after.ret, Ch(a_after.c0 as u32), (a_after.ret == a_before.ret && a_after.c0 == a_before.c0) as i32);
}
