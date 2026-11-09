//! SDDL access-right strings in an ACE ("D:(A;;<rights>;;;WD)"): the mask each two-letter right and hex form
//! produces, concatenations, and the strings that are rejected.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, sddl.h, aclapi.h, jobapi2.h,
// processthreadsapi.h, winternl.h, ntstatus.h and winerror.h (constants,
// layouts); the helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type PVOID = *mut c_void;
type BOOL = i32;
type PACL = *mut c_void;
type PSECURITY_DESCRIPTOR = *mut c_void;
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

const SDDL_REVISION_1: u32 = 1;
/// The last-error code after a failed call, as a plain number.
fn gle() -> u32 {
    // SAFETY: reads the calling thread's last error.
    unsafe { GetLastError() }
}

fn clear_error() {
    // SAFETY: sets the calling thread's last error.
    unsafe { SetLastError(0xDEAD) };
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
/// A self-relative descriptor made by ConvertStringSecurityDescriptorToSecurityDescriptorW.
struct SdLocal {
    p: PSECURITY_DESCRIPTOR,
    size: u32,
    ok: i32,
    err: i64,
}

impl SdLocal {
    fn ptr(&self) -> *const c_void {
        self.p as *const c_void
    }
    fn bytes(&self) -> &[u8] {
        if self.p.is_null() {
            return &[];
        }
        // SAFETY: the allocation is `size` bytes as reported by the conversion.
        unsafe { core::slice::from_raw_parts(self.p as *const u8, self.size as usize) }
    }
}

impl Drop for SdLocal {
    fn drop(&mut self) {
        if !self.p.is_null() {
            // SAFETY: allocated with LocalAlloc by the conversion.
            unsafe { LocalFree(self.p) };
        }
    }
}

fn sd_from_rev(s: &str, rev: u32) -> SdLocal {
    let w = wide(s);
    let mut p: PSECURITY_DESCRIPTOR = core::ptr::null_mut();
    let mut size = 0xAAAA_AAAAu32;
    clear_error();
    // SAFETY: `w` is NUL-terminated; both out pointers are valid.
    let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), rev, &mut p, &mut size) };
    let err = gle_if(ok);
    if ok == 0 {
        p = core::ptr::null_mut();
    }
    SdLocal { p, size, ok, err }
}

fn sd_from(s: &str) -> SdLocal {
    sd_from_rev(s, SDDL_REVISION_1)
}

/// GetSecurityDescriptorDacl / Sacl.
struct SdGetAcl {
    ok: i32,
    present: BOOL,
    acl: *mut c_void,
    defaulted: BOOL,
    err: i64,
}

fn sd_get_dacl(sd: *const c_void) -> SdGetAcl {
    let mut acl = SD_SENTINEL as *mut c_void;
    let mut present: BOOL = 0x66;
    let mut d: BOOL = 0x77;
    clear_error();
    // SAFETY: `sd` is a descriptor; the out pointers are valid.
    let ok = unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut acl, &mut d) };
    SdGetAcl { ok, present, acl, defaulted: d, err: gle_if(ok) }
}

/// The (type, flags, mask) of the first ACE of an ACL, or None for NULL or an empty ACL.
fn sd_first_ace(acl: *const c_void) -> Option<(u8, u8, u32)> {
    if acl.is_null() || acl as usize == SD_SENTINEL {
        return None;
    }
    // SAFETY: the caller passes a pointer to an ACL header; the first ACE (if any) is within 16 bytes.
    let h = unsafe { core::slice::from_raw_parts(acl as *const u8, 16) };
    if u16::from_le_bytes([h[4], h[5]]) == 0 {
        return None;
    }
    Some((h[8], h[9], le32(h, 12)))
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        revision: u32,
        sd: *mut PSECURITY_DESCRIPTOR,
        size: *mut u32,
    ) -> BOOL;
    fn GetSecurityDescriptorDacl(sd: *const c_void, present: *mut BOOL, dacl: *mut PACL, defaulted: *mut BOOL) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for r in [
        "GA",
        "GR",
        "GW",
        "GX",
        "FA",
        "FR",
        "FW",
        "FX",
        "KA",
        "KR",
        "KW",
        "KX",
        "RC",
        "SD",
        "WD",
        "WO",
        "CC",
        "DC",
        "LC",
        "SW",
        "RP",
        "WP",
        "DT",
        "LO",
        "CR",
        "GAGR",
        "GRGWGX",
        "FAFR",
        "RCSDWDWO",
        "CCDCLCSWRPWPDTLOCR",
        "GRRC",
        "KRKW",
        "FRFW",
        "FXCC",
        "SDSD",
        "0x1",
        "0x12345",
        "0X10",
        "0xFFFFFFFF",
        "0x0",
        "0x00000001",
        "0x1F01FF",
        "0xabc",
        "0xABC",
        "123",
        "0",
        "01",
        "0x100000000",
        "0xFFFFFFFFF",
        "0x",
        "0xZ",
        "0x1G",
        "0x1GA",
        "GA0x1",
        "GAZZ",
        "ZZ",
        "ga",
        "G",
        "GAG",
        " GA",
        "GA ",
        "",
        "R",
        "RW",
        "AX",
        "-1",
        "0x-1",
        "0x 1",
    ] {
        let mut text = Buf::<120>::new();
        let _ = write!(text, "D:(A;;{r};;;WD)");
        let sd = sd_from(text.as_str());
        if sd.ok == 0 {
            sdsay!(t, "{r:?}: failed err={}", sd.err);
            continue;
        }
        let d = sd_get_dacl(sd.ptr());
        match sd_first_ace(d.acl) {
            Some((ty, flags, mask)) => sdsay!(t, "{r:?}: type={ty} flags={flags:#x} mask={mask:#010x}"),
            None => sdsay!(t, "{r:?}: no ACE"),
        }
    }
}
