//! MapVirtualKeyW's map types beyond the five documented ones (5, 6, 7, 8,
//! 0x100, 0xFFFFFFFF) for 'A' and scan 0x1E: the result and last error;
//! out-of-range codes (vk 0x100, 0x141 and 0xFFFF for VK_TO_VSC and
//! VK_TO_CHAR, scan 0x100 and 0x1E1E for VSC_TO_VK); round trips: VK_TO_VSC
//! then VSC_TO_VK and VSC_TO_VK_EX for the left/right modifiers and the
//! arrows; MapVirtualKeyW agrees with GetKeyboardLayout's layout
//! (MapVirtualKeyExW with it gives the same).
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

/// SetLastError.
fn set_error(code: u32) {
    // SAFETY: sets this thread's last-error value.
    unsafe { SetLastError(code) }
}

/// GetLastError.
fn last_error() -> u32 {
    // SAFETY: reads this thread's last-error value.
    unsafe { GetLastError() }
}

type HKL = *mut c_void;
const MAPVK_VK_TO_VSC: u32 = 0;
const MAPVK_VSC_TO_VK: u32 = 1;
const MAPVK_VK_TO_CHAR: u32 = 2;
const MAPVK_VSC_TO_VK_EX: u32 = 3;
const MAPVK_VK_TO_VSC_EX: u32 = 4;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MapVirtualKeyW(code: u32, map: u32) -> u32;
    fn GetKeyboardLayout(thread: u32) -> HKL;
}

#[link(name = "user32", kind = "raw-dylib")]
unsafe extern "system" {
    fn MapVirtualKeyExW(code: u32, map: u32, layout: HKL) -> u32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for map in [5u32, 6, 7, 8, 0x100, 0xFFFF_FFFF] {
        set_error(0);
        // SAFETY: plain query.
        let a = unsafe { MapVirtualKeyW(0x41, map) };
        let e1 = last_error();
        set_error(0);
        // SAFETY: plain query.
        let b = unsafe { MapVirtualKeyW(0x1E, map) };
        say!(t, "map {map:#x}: (0x41) {a:#x} error {e1}; (0x1e) {b:#x} error {}", last_error());
    }
    for vk in [0x100u32, 0x141, 0xFFFF] {
        set_error(0);
        // SAFETY: plain queries.
        let (s, c) = unsafe { (MapVirtualKeyW(vk, MAPVK_VK_TO_VSC), MapVirtualKeyW(vk, MAPVK_VK_TO_CHAR)) };
        say!(t, "vk {vk:#x}: vsc={s:#x} char={c:#x} error {}", last_error());
    }
    for sc in [0x100u32, 0x1E1E] {
        // SAFETY: plain query.
        let v = unsafe { MapVirtualKeyW(sc, MAPVK_VSC_TO_VK) };
        say!(t, "scan {sc:#x}: vk={v:#x}");
    }
    for vk in [0xA0u32, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x25, 0x26, 0x2E] {
        // SAFETY: plain queries.
        let (s, sx) = unsafe { (MapVirtualKeyW(vk, MAPVK_VK_TO_VSC), MapVirtualKeyW(vk, MAPVK_VK_TO_VSC_EX)) };
        // SAFETY: plain queries.
        let (v, vx, vxx) = unsafe {
            (
                MapVirtualKeyW(s, MAPVK_VSC_TO_VK),
                MapVirtualKeyW(s, MAPVK_VSC_TO_VK_EX),
                MapVirtualKeyW(sx, MAPVK_VSC_TO_VK_EX),
            )
        };
        say!(t, "vk {vk:#04x} -> vsc {s:#x} -> vk {v:#04x} / vk_ex {vx:#04x}; vsc_ex {sx:#x} -> vk_ex {vxx:#04x}");
    }
    // SAFETY: plain query.
    let hkl = unsafe { GetKeyboardLayout(0) };
    let mut same = true;
    for code in 0u32..256 {
        for map in 0u32..5 {
            // SAFETY: plain queries.
            if unsafe { MapVirtualKeyW(code, map) != MapVirtualKeyExW(code, map, hkl) } {
                same = false;
            }
        }
    }
    t.check("MapVirtualKeyExW with the thread's layout agrees", same, 0);
    say!(t, "layout HKL low 32 bits {:#010x}", hkl as usize as u32);
}
