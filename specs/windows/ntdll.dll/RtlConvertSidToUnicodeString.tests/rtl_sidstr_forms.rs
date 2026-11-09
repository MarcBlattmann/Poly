//! RtlConvertSidToUnicodeString: the text forms (authority in decimal or 0x hex), and invalid SIDs
//! with both settings of AllocateDestinationString.
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

/// ntdef.h: `UNICODE_STRING` (writable buffer).
#[repr(C)]
#[derive(Clone, Copy)]
struct RTL_UNICODE_STRING {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

/// The text of a counted UTF-16 string (ASCII only; others as `?`), by its Length field.
fn rtl_us_text(us: &RTL_UNICODE_STRING) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if us.buffer.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    for i in 0..usize::from(us.length / 2) {
        // SAFETY: the caller's buffer holds at least Length bytes.
        let c = unsafe { *us.buffer.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn RtlInitializeSid(sid: PSID, authority: *const [u8; 6], count: u8) -> i32;
    fn RtlConvertSidToUnicodeString(dst: *mut RTL_UNICODE_STRING, sid: *const c_void, allocate: u8) -> i32;
    fn RtlFreeUnicodeString(s: *mut RTL_UNICODE_STRING);
}

poly_wtest::main!(test);

fn convert(t: &mut Test, label: &str, sid: *const c_void) {
    let mut us = RTL_UNICODE_STRING { length: 0x7777, maximum_length: 0x7777, buffer: core::ptr::null_mut() };
    // SAFETY: the SID buffers are large enough for their count bytes; a returned buffer is freed.
    unsafe {
        let st = RtlConvertSidToUnicodeString(&mut us, sid, 1);
        if st >= 0 {
            say!(t, "{label}: status={:#010x} text={}", st as u32, rtl_us_text(&us).as_str());
            RtlFreeUnicodeString(&mut us);
        } else {
            say!(
                t,
                "{label}: status={:#010x} length={} maximum={} buffer_null={}",
                st as u32,
                us.length,
                us.maximum_length,
                us.buffer.is_null()
            );
        }
        let mut buf = [0u16; 128];
        let mut us2 = RTL_UNICODE_STRING { length: 0, maximum_length: 256, buffer: buf.as_mut_ptr() };
        let st2 = RtlConvertSidToUnicodeString(&mut us2, sid, 0);
        say!(t, "{label}: no allocation: status={:#010x} length={}", st2 as u32, us2.length);
    }
}

fn test(t: &mut Test) {
    let auths: [[u8; 6]; 7] = [
        [0, 0, 0, 0, 0, 5],
        [0; 6],
        [0, 0, 0xFF, 0xFF, 0xFF, 0xFF],
        [0, 0, 1, 0, 0, 0],
        [0, 1, 0, 0, 0, 0],
        [1, 2, 3, 4, 5, 6],
        [0xFF; 6],
    ];
    for auth in auths {
        let mut sid = SidBuf::new();
        // SAFETY: `sid` can hold one sub-authority.
        unsafe { RtlInitializeSid(sid.as_psid(), &auth, 1) };
        sid.b[8] = 7;
        let mut label = Buf::<40>::new();
        let _ = write!(label, "authority {}", Hex(&auth));
        convert(t, label.as_str(), sid.as_const());
    }
    let mut big = Raw::<1100>::new();
    for (rev, count) in [(0u8, 2u8), (2, 2), (255, 1), (1, 16), (1, 17), (1, 255)] {
        big.b.fill(0);
        big.b[0] = rev;
        big.b[1] = count;
        big.b[7] = 5;
        let mut label = Buf::<40>::new();
        let _ = write!(label, "revision {rev} count {count}");
        convert(t, label.as_str(), big.p());
    }
}
