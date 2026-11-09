//! RtlSetGroupSecurityDescriptor errors: the descriptor's revision, a buffer that was never initialized, the
//! self-relative form (nothing can be set in it).
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
const SE_SELF_RELATIVE: u16 = 0x8000;
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
    fn RtlSetGroupSecurityDescriptor(sd: PSECURITY_DESCRIPTOR, group: *const c_void, defaulted: u8) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let sid = rtl_sid(5, &[32, 544]);
    for rev in [0u8, 1, 2, 3, 255] {
        let mut sd = rtl_new_sd();
        sd.b[0] = rev;
        // SAFETY: valid descriptor and SID.
        let st = unsafe { RtlSetGroupSecurityDescriptor(sd.p(), sid.as_const(), 1) };
        say!(
            t,
            "descriptor revision {rev}: status={:#010x} control={:#06x} pointer_set={}",
            st as u32,
            sd.u16_at(2),
            !rtl_sd_ptr(&sd, 16).is_null()
        );
    }
    let mut z = Raw::<256>::new();
    // SAFETY: a zeroed buffer.
    let st = unsafe { RtlSetGroupSecurityDescriptor(z.p(), sid.as_const(), 1) };
    say!(
        t,
        "zeroed buffer: status={:#010x} control={:#06x} pointer_set={}",
        st as u32,
        z.u16_at(2),
        !rtl_sd_ptr(&z, 16).is_null()
    );
    let mut sd = rtl_new_sd();
    sd.b[2..4].copy_from_slice(&SE_SELF_RELATIVE.to_le_bytes());
    // SAFETY: a descriptor with the self-relative bit and nothing else.
    let st = unsafe { RtlSetGroupSecurityDescriptor(sd.p(), sid.as_const(), 1) };
    let changed = sd.b.iter().enumerate().filter(|&(i, &b)| b != 0 && i != 3).count();
    say!(
        t,
        "self-relative bit set: status={:#010x} control={:#06x} other_bytes_changed={changed}",
        st as u32,
        sd.u16_at(2)
    );
}
