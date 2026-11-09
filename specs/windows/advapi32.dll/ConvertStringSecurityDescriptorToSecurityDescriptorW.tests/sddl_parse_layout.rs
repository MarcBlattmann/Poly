//! The self-relative descriptor that SDDL strings turn into: header, offsets, the order of the parts, total
//! size and bytes, for the owner, group, DACL and SACL in any order of appearance.
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

/// The bytes as lowercase hex, no separators.
struct Hex<'a>(&'a [u8]);

impl core::fmt::Display for Hex<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for b in self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

const SE_SELF_RELATIVE: u16 = 0x8000;
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

/// Names of the SECURITY_DESCRIPTOR_CONTROL bits that are set, joined with `|`.
fn sd_ctrl_names(c: u16) -> Buf<200> {
    let mut b = Buf::<200>::new();
    let names: [(u16, &str); 16] = [
        (0x0001, "OWNER_DEFAULTED"),
        (0x0002, "GROUP_DEFAULTED"),
        (0x0004, "DACL_PRESENT"),
        (0x0008, "DACL_DEFAULTED"),
        (0x0010, "SACL_PRESENT"),
        (0x0020, "SACL_DEFAULTED"),
        (0x0040, "DACL_UNTRUSTED"),
        (0x0080, "SERVER_SECURITY"),
        (0x0100, "DACL_AUTO_INHERIT_REQ"),
        (0x0200, "SACL_AUTO_INHERIT_REQ"),
        (0x0400, "DACL_AUTO_INHERITED"),
        (0x0800, "SACL_AUTO_INHERITED"),
        (0x1000, "DACL_PROTECTED"),
        (0x2000, "SACL_PROTECTED"),
        (0x4000, "RM_CONTROL_VALID"),
        (0x8000, "SELF_RELATIVE"),
    ];
    let mut first = true;
    for (bit, name) in names {
        if c & bit != 0 {
            let _ = write!(b, "{}{name}", if first { "" } else { "|" });
            first = false;
        }
    }
    if first {
        let _ = write!(b, "-");
    }
    b
}

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

/// Prints the header of a descriptor: absolute (control, padding, the four pointer slots as
/// null/set) or self-relative (control and the four offsets).
fn sd_head(t: &mut Test, label: &str, sd: *const c_void) {
    // SAFETY: the caller passes a descriptor of at least 20 bytes (self-relative) or 40 bytes (absolute).
    let first = unsafe { core::slice::from_raw_parts(sd as *const u8, 4) };
    let control = u16::from_le_bytes([first[2], first[3]]);
    if control & SE_SELF_RELATIVE != 0 {
        // SAFETY: a self-relative descriptor has a 20-byte header.
        let h = unsafe { core::slice::from_raw_parts(sd as *const u8, 20) };
        sdsay!(
            t,
            "{label}: self-relative rev={} sbz1={} control={control:#06x} [{}] owner={} group={} sacl={} dacl={}",
            h[0],
            h[1],
            sd_ctrl_names(control).as_str(),
            le32(h, 4),
            le32(h, 8),
            le32(h, 12),
            le32(h, 16)
        );
    } else {
        // SAFETY: an absolute descriptor is 40 bytes on x64.
        let h = unsafe { core::slice::from_raw_parts(sd as *const u8, 40) };
        let slot = |o: usize| {
            let mut a = [0u8; 8];
            a.copy_from_slice(&h[o..o + 8]);
            if u64::from_le_bytes(a) == 0 { "null" } else { "set" }
        };
        sdsay!(
            t,
            "{label}: absolute rev={} sbz1={} control={control:#06x} [{}] pad={} owner={} group={} sacl={} dacl={}",
            h[0],
            h[1],
            sd_ctrl_names(control).as_str(),
            Hex(&h[4..8]),
            slot(8),
            slot(16),
            slot(24),
            slot(32)
        );
    }
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
}

poly_wtest::main!(test);

fn show(t: &mut Test, s: &str) {
    let sd = sd_from(s);
    if sd.ok == 0 {
        sdsay!(t, "{s:?}: failed err={}", sd.err);
        return;
    }
    sdsay!(t, "{s:?}: size={}", sd.size);
    sd_head(t, "  header", sd.ptr());
    sdsay!(t, "  bytes: {}", Hex(sd.bytes()));
}

fn test(t: &mut Test) {
    for s in [
        "",
        "O:BA",
        "G:SY",
        "O:BAG:SY",
        "G:SYO:BA",
        "D:",
        "D:NO_ACCESS_CONTROL",
        "D:(A;;GA;;;WD)",
        "S:(AU;SAFA;GA;;;WD)",
        "S:",
        "O:BAG:SYD:(A;;GA;;;WD)",
        "D:(A;;GA;;;WD)O:BAG:SY",
        "O:BAD:(A;;GA;;;WD)G:SY",
        "S:(AU;SAFA;GA;;;WD)D:(A;;GA;;;WD)G:SYO:BA",
        "O:BAG:SYD:(A;;GA;;;WD)S:(AU;SAFA;GA;;;WD)",
        "O:BAG:SYD:(A;;GA;;;WD)(D;;GW;;;BU)S:(AU;SAFA;GA;;;WD)",
        "O:BAG:SYD:S:",
        "D:S:",
        "D:(A;;GA;;;WD)S:",
        "S:(ML;;NW;;;LW)",
        "S:(AU;SAFA;GA;;;WD)(ML;;NW;;;LW)",
        "O:S-1-5-21-1-2-3-500G:S-1-5-21-1-2-3-513",
    ] {
        show(t, s);
    }
}
