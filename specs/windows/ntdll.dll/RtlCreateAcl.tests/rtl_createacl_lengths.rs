//! RtlCreateAcl: the smallest ACL length it accepts (the 8-byte header), what it writes (the header
//! only, or more), and the largest length (a 16-bit size field).
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
type PACL = *mut c_void;
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

/// An ACL in an 8-aligned buffer.
#[repr(C, align(8))]
#[derive(Clone, Copy)]
struct AclBuf {
    b: [u8; 1024],
}

impl AclBuf {
    const fn new() -> AclBuf {
        AclBuf { b: [0; 1024] }
    }
    fn p(&mut self) -> PACL {
        self.b.as_mut_ptr().cast()
    }
    /// The ACL's bytes by its own size field.
    fn bytes(&self) -> &[u8] {
        let n = usize::from(u16::from_le_bytes([self.b[2], self.b[3]]));
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

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlCreateAcl(acl: PACL, len: u32, revision: u32) -> i32;
    fn RtlValidAcl(acl: *const c_void) -> u8;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for len in [0u32, 1, 4, 7, 8, 9, 10, 11, 12, 15, 16, 17, 64, 65, 66, 67, 1020, 1021, 1024] {
        let mut a = AclBuf::new();
        a.b.fill(0xCC);
        // SAFETY: the buffer has 1024 bytes and `len` never exceeds that.
        let st = unsafe { RtlCreateAcl(a.p(), len, 2) };
        let changed = a.b.iter().filter(|&&b| b != 0xCC).count();
        // SAFETY: the ACL was initialized when the call succeeded; otherwise it is read as is (bytes are 0xCC or a header).
        let valid = unsafe { RtlValidAcl(a.p()) };
        say!(
            t,
            "length {len}: status={:#010x} header={} bytes_changed={changed} valid={valid}",
            st as u32,
            Hex(&a.b[..8])
        );
    }
    let mut big = Raw::<70000>::new();
    for len in [0xFFF8u32, 0xFFFC, 0xFFFD, 0xFFFF, 0x10000, 0x10001, 0x10004] {
        big.b.fill(0xCC);
        // SAFETY: the buffer has 70000 bytes, more than `len`.
        let st = unsafe { RtlCreateAcl(big.p(), len, 2) };
        let changed = big.b.iter().filter(|&&b| b != 0xCC).count();
        say!(t, "length {len:#x}: status={:#010x} header={} bytes_changed={changed}", st as u32, Hex(&big.b[..8]));
    }
}
