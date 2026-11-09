//! IsWellKnownSid with SIDs that are not well-known, domain-relative ones, and bad types.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winuser.h, wingdi.h, windef.h, winbase.h and
// winerror.h (constants, layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PSID = *mut c_void;
type PVOID = *mut c_void;
type BOOL = i32;
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
        let mut b = Buf::<400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SECURITY_MAX_SID_SIZE: usize = 68;
const WinWorldSid: u32 = 1;
/// A `SID_IDENTIFIER_AUTHORITY` plus up to 15 sub-authorities in a buffer.
#[repr(C)]
#[derive(Clone, Copy)]
struct SidBuf {
    b: [u8; SECURITY_MAX_SID_SIZE + 4],
}

impl SidBuf {
    const fn new() -> SidBuf {
        SidBuf { b: [0; SECURITY_MAX_SID_SIZE + 4] }
    }
    fn as_psid(&mut self) -> PSID {
        self.b.as_mut_ptr().cast()
    }
    fn as_const(&self) -> *const c_void {
        self.b.as_ptr().cast()
    }
    /// The bytes of the SID (its length from the header).
    fn bytes(&self) -> &[u8] {
        let n = 8 + 4 * usize::from(self.b[1]);
        &self.b[..n.min(self.b.len())]
    }
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// A SID from its string form (up to 68 bytes), through ConvertStringSidToSidW.
fn sid_of(s: &str) -> SidBuf {
    let w = wide(s);
    let mut p: PSID = core::ptr::null_mut();
    let mut out = SidBuf::new();
    // SAFETY: `w` is NUL-terminated; `p` receives a LocalAlloc'ed SID, copied and freed below.
    unsafe {
        if ConvertStringSidToSidW(w.as_ptr(), &mut p) != 0 {
            let n = GetLengthSid(p) as usize;
            core::ptr::copy_nonoverlapping(p.cast::<u8>(), out.b.as_mut_ptr(), n.min(out.b.len()));
            LocalFree(p);
        }
    }
    out
}

/// `ConvertSidToStringSidW` of a well-known SID built by `CreateWellKnownSid`.
fn well_known(kind: u32) -> SidBuf {
    let mut sid = SidBuf::new();
    let mut len = SECURITY_MAX_SID_SIZE as u32;
    // SAFETY: `sid` has room for the largest SID; `len` is its size.
    unsafe { CreateWellKnownSid(kind, core::ptr::null(), sid.as_psid(), &mut len) };
    sid
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn CreateWellKnownSid(kind: u32, domain: *const c_void, sid: PSID, len: *mut u32) -> BOOL;
    fn IsWellKnownSid(sid: *const c_void, kind: u32) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let cases = [
        "S-1-5-21-111-222-333-500",
        "S-1-5-21-111-222-333-501",
        "S-1-5-21-111-222-333-512",
        "S-1-5-21-111-222-333-1001",
        "S-1-5-21-111-222-333",
        "S-1-5-80-1-2-3-4-5",
        "S-1-5-5-0-12345",
        "S-1-5-99",
        "S-1-2-3",
    ];
    for text in cases {
        let sid = sid_of(text);
        let mut b = Buf::<300>::new();
        for kind in 0..=125u32 {
            // SAFETY: a valid SID.
            if unsafe { IsWellKnownSid(sid.as_const(), kind) } != 0 {
                let _ = write!(b, "{kind} ");
            }
        }
        say!(t, "{text}: matches {}", b.as_str());
    }
    let world = well_known(WinWorldSid);
    for kind in [126u32, 200, 0xFFFF_FFFF] {
        // SAFETY: a valid SID.
        let r = unsafe { IsWellKnownSid(world.as_const(), kind) };
        say!(t, "type {kind}: {r}");
    }
}
