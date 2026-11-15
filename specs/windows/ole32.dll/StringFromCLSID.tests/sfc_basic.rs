//! StringFromCLSID(&clsid, &string): the text of a GUID in registry format,
//! '{' eight hex digits '-' four '-' four '-' four '-' twelve '}', in capital
//! letters, in memory from CoTaskMemAlloc (the caller frees it with
//! CoTaskMemFree), 38 characters and a NUL; for the all-zero GUID, ones with
//! every digit kind, and the well-known ids; a NULL clsid pointer is
//! accepted like GUID_NULL? (the result is printed); a NULL output pointer
//! fails (E_INVALIDARG or E_POINTER, printed); no COM initialization is needed;
//! it equals StringFromGUID2's text; Chrome uses it for ids of the system
//! it talks to (S3).
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

/// guiddef.h
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct GUID {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

const GUID_DEVINTERFACE_USB_DEVICE: GUID = GUID { d1: 0xA5DCBF10, d2: 0x6530, d3: 0x11D2, d4: [0x90, 0x1F, 0x00, 0xC0, 0x4F, 0xB9, 0x51, 0xED] };
const GUID_DEVINTERFACE_MONITOR: GUID = GUID { d1: 0xE6F07B5F, d2: 0xEE97, d3: 0x4A90, d4: [0xB0, 0x76, 0x33, 0xF5, 0x7B, 0xF4, 0xEA, 0xA7] };
type HRESULT = i32;
const IID_IUNKNOWN: GUID = GUID { d1: 0, d2: 0, d3: 0, d4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
fn hr(h: i32) -> u32 {
    h as u32
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetLastError(code: u32);
}

#[link(name = "ole32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CoTaskMemFree(p: *mut c_void);
    fn StringFromCLSID(clsid: *const GUID, out: *mut *mut u16) -> HRESULT;
    fn StringFromGUID2(guid: *const GUID, buf: *mut u16, max: i32) -> i32;
}

poly_wtest::main!(test);

fn text(p: *const u16, out: &mut Buf<80>) -> usize {
    let mut n = 0;
    // SAFETY: a NUL-terminated string from the allocator.
    unsafe {
        while *p.add(n) != 0 && n < 70 {
            let c = *p.add(n);
            let _ = write!(out, "{}", if (32..127).contains(&c) { c as u8 as char } else { '?' });
            n += 1;
        }
    }
    n
}

fn test(t: &mut Test) {
    // SAFETY: local outputs; the strings are freed with CoTaskMemFree.
    unsafe {
        let guids = [
            ("GUID_NULL", GUID { d1: 0, d2: 0, d3: 0, d4: [0; 8] }),
            ("IID_IUnknown", IID_IUNKNOWN),
            ("a GUID with every kind of digit", GUID { d1: 0xDEADBEEF, d2: 0x1A2B, d3: 0x3C4D, d4: [0x5E, 0x6F, 0x70, 0x81, 0x92, 0xA3, 0xB4, 0xC5] }),
            ("all ones", GUID { d1: 0xFFFF_FFFF, d2: 0xFFFF, d3: 0xFFFF, d4: [0xFF; 8] }),
            ("the monitor interface class", GUID_DEVINTERFACE_MONITOR),
        ];
        for (name, g) in guids {
            let mut s: *mut u16 = 5usize as *mut u16;
            set_error(0xDEAD);
            let r = StringFromCLSID(&g, &mut s);
            let mut b = Buf::<80>::new();
            let n = if r == 0 { text(s, &mut b) } else { 0 };
            say!(t, "{name}: returns {:#x}, {n} characters, text {}, 16-byte aligned block {}", hr(r), b.as_str(), s as usize % 16 == 0 || r != 0);
            if r == 0 {
                CoTaskMemFree(s as *mut c_void);
            }
        }
        // the same text as StringFromGUID2
        let g = GUID_DEVINTERFACE_USB_DEVICE;
        let mut s: *mut u16 = core::ptr::null_mut();
        StringFromCLSID(&g, &mut s);
        let mut buf = [0u16; 64];
        let n = StringFromGUID2(&g, buf.as_mut_ptr(), 64);
        let mut same = n == 39;
        for i in 0..38 {
            same &= *s.add(i) == buf[i];
        }
        say!(t, "StringFromGUID2 returns {n} (characters with the NUL) and the same text: {same}");
        CoTaskMemFree(s as *mut c_void);
        // arguments
        let mut s: *mut u16 = 5usize as *mut u16;
        let r = StringFromCLSID(core::ptr::null(), &mut s);
        say!(t, "a NULL clsid pointer: returns {:#x}, string {}", hr(r), if r == 0 { "made" } else { "none" });
        if r == 0 {
            let mut b = Buf::<80>::new();
            text(s, &mut b);
            say!(t, "  its text {}", b.as_str());
            CoTaskMemFree(s as *mut c_void);
        }
        let r = StringFromCLSID(&g, core::ptr::null_mut());
        say!(t, "a NULL output pointer: returns {:#x}", hr(r));
        // 500 calls: each block is freed
        let mut ok = true;
        for _ in 0..500 {
            let mut s: *mut u16 = core::ptr::null_mut();
            ok &= StringFromCLSID(&g, &mut s) == 0 && !s.is_null();
            CoTaskMemFree(s as *mut c_void);
        }
        say!(t, "five hundred calls and frees: {ok}");
    }
}
