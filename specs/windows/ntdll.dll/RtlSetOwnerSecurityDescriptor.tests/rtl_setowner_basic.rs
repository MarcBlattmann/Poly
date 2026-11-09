//! RtlSetOwnerSecurityDescriptor stores the pointer to the caller's SID in the descriptor (no copy), sets or
//! clears SE_OWNER_DEFAULTED from the last argument and returns a status.
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

type PSID = *mut c_void;
type PVOID = *mut c_void;
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

const SECURITY_MAX_SID_SIZE: usize = 68;
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

/// An 8-aligned scratch buffer for token information and other results.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct Raw<const N: usize> {
    b: [u8; N],
}

impl<const N: usize> Raw<N> {
    const fn new() -> Raw<N> {
        Raw { b: [0; N] }
    }
    fn p(&mut self) -> PVOID {
        self.b.as_mut_ptr().cast()
    }
    fn u32_at(&self, off: usize) -> u32 {
        u32::from_le_bytes([self.b[off], self.b[off + 1], self.b[off + 2], self.b[off + 3]])
    }
    fn u16_at(&self, off: usize) -> u16 {
        u16::from_le_bytes([self.b[off], self.b[off + 1]])
    }
    fn u64_at(&self, off: usize) -> u64 {
        let mut a = [0u8; 8];
        a.copy_from_slice(&self.b[off..off + 8]);
        u64::from_le_bytes(a)
    }
}

/// A SID with authority {0,0,0,0,0,auth} and the given sub-authorities, built byte by byte.
fn rtl_sid(auth: u8, subs: &[u32]) -> SidBuf {
    let mut s = SidBuf::new();
    s.b[0] = 1;
    s.b[1] = subs.len().min(15) as u8;
    s.b[7] = auth;
    for (i, v) in subs.iter().enumerate().take(15) {
        s.b[8 + 4 * i..12 + 4 * i].copy_from_slice(&v.to_le_bytes());
    }
    s
}

/// Where a pointer points: "null", "name+offset" inside one of the named buffers, or "other".
fn rtl_where(p: *const c_void, bases: &[(&str, *const u8, usize)]) -> Buf<48> {
    let mut b = Buf::<48>::new();
    if p.is_null() {
        let _ = write!(b, "null");
        return b;
    }
    for (name, base, len) in bases {
        let d = (p as usize).wrapping_sub(*base as usize);
        if d < *len {
            let _ = write!(b, "{name}+{d}");
            return b;
        }
    }
    let _ = write!(b, "other");
    b
}

/// A new absolute security descriptor (RtlCreateSecurityDescriptor, revision 1) in a zeroed buffer.
fn rtl_new_sd() -> Raw<256> {
    let mut sd = Raw::<256>::new();
    // SAFETY: the buffer has room for the 40-byte descriptor.
    unsafe { RtlCreateSecurityDescriptor(sd.p(), 1) };
    sd
}

/// The pointer stored at `off` of an absolute descriptor.
fn rtl_sd_ptr(sd: &Raw<256>, off: usize) -> *const c_void {
    sd.u64_at(off) as usize as *const c_void
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCreateSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, revision: u32) -> i32;
    fn RtlSetOwnerSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, owner: *const c_void, defaulted: u8) -> i32;
}

poly_wtest::main!(test);

fn show(t: &mut Test, label: &str, st: i32, sd: &Raw<256>, bases: &[(&str, *const u8, usize)]) {
    say!(
        t,
        "{label}: status={:#010x} control={:#06x} owner={}",
        st as u32,
        sd.u16_at(2),
        rtl_where(rtl_sd_ptr(sd, 8), bases).as_str()
    );
}

fn test(t: &mut Test) {
    let first = rtl_sid(5, &[32, 544]);
    let second = rtl_sid(5, &[18]);
    let mut bad = rtl_sid(5, &[1]);
    bad.b[0] = 0;
    let bases = [
        ("first", first.b.as_ptr(), first.b.len()),
        ("second", second.b.as_ptr(), second.b.len()),
        ("bad", bad.b.as_ptr(), bad.b.len()),
    ];
    let null: *const c_void = core::ptr::null();
    let mut sd = rtl_new_sd();
    // SAFETY: valid descriptor and SIDs.
    unsafe {
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), first.as_const(), 0);
        show(t, "first, not defaulted", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), first.as_const(), 1);
        show(t, "first, defaulted", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), second.as_const(), 0);
        show(t, "second, not defaulted (replaces, clears the bit)", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), null, 1);
        show(t, "NULL, defaulted", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), null, 0);
        show(t, "NULL, not defaulted", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), first.as_const(), 2);
        show(t, "first, defaulted argument 2", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), first.as_const(), 0xFF);
        show(t, "first, defaulted argument 0xFF", st, &sd, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd.p(), bad.as_const(), 0);
        show(t, "SID with revision 0 (not validated?)", st, &sd, &bases);
        let mut sd2 = rtl_new_sd();
        sd2.b[2..4].copy_from_slice(&0x7FF0u16.to_le_bytes());
        let st = RtlSetOwnerSecurityDescriptor(sd2.p(), first.as_const(), 1);
        show(t, "other control bits 0x7ff0 kept?", st, &sd2, &bases);
        let st = RtlSetOwnerSecurityDescriptor(sd2.p(), first.as_const(), 0);
        show(t, "other control bits 0x7ff0, defaulted cleared", st, &sd2, &bases);
    }
}
