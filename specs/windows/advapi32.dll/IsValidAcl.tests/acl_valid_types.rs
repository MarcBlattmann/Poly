//! IsValidAcl: which ACE types are accepted in ACLs of revision 2, 3 and 4 (a one-ACE ACL for each
//! type, with the ACE laid out as an access ACE, an object ACE with a 4-byte flags field and two
//! GUIDs, or a compound ACE with two SIDs).
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
type BOOL = i32;
type PACL = *mut c_void;
const FALSE: BOOL = 0;
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

/// The last error after a call, if it failed (`ok` is 0); -1 after a success,
/// where the last error is not specified.
fn gle_if(ok: i32) -> i64 {
    if ok == 0 { i64::from(gle()) } else { -1 }
}

/// `say!` with a longer line (hex dumps of whole ACLs).
macro_rules! asay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2400>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

/// Fills `buf` with 0xCC, then InitializeAcl(buf, size, rev); returns its result.
fn acl_init_fill<const N: usize>(buf: &mut Raw<N>, size: u32, rev: u32) -> i32 {
    buf.b.fill(0xCC);
    clear_error();
    // SAFETY: `buf` is at least as large as the ACL size given in every use.
    unsafe { InitializeAcl(buf.p(), size, rev) }
}

/// The ACL header, and all of the ACL's bytes (its size field, at most the buffer) as hex.
fn acl_show<const N: usize>(t: &mut Test, label: &str, buf: &Raw<N>) {
    let size = usize::from(buf.u16_at(2)).min(N);
    asay!(
        t,
        "{label}: rev={} sbz1={} size={} count={} sbz2={} bytes={}",
        buf.b[0],
        buf.b[1],
        buf.u16_at(2),
        buf.u16_at(4),
        buf.u16_at(6),
        Hex(&buf.b[..size])
    );
}

/// A short name for an error value printed after a failure: the number, or "-" after success.
fn acl_err(ok: i32) -> i64 {
    gle_if(ok)
}

/// IsValidAcl on the buffer: (result, last error after FALSE or -1).
fn acl_valid<const N: usize>(b: &Raw<N>) -> (i32, i64) {
    clear_error();
    // SAFETY: the buffer holds the ACL bytes; IsValidAcl reads the header and the ACEs by their sizes.
    let ok = unsafe { IsValidAcl(b.b.as_ptr().cast()) };
    (ok, acl_err(ok))
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn SetLastError(code: u32);
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn IsValidAcl(acl: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn build(rev: u32, ty: u8, shape: u32) -> Raw<1100> {
    let sid = sid_of("S-1-1-0");
    let sb = sid.bytes();
    let mut b = Raw::<1100>::new();
    let ace_size: usize = match shape {
        0 => 8 + sb.len(),
        1 => 12 + 32 + sb.len(),
        _ => 12 + 2 * sb.len(),
    };
    acl_init_fill(&mut b, 200, rev);
    b.b[8..200].fill(0);
    b.b[4] = 1;
    b.b[8] = ty;
    b.b[10..12].copy_from_slice(&(ace_size as u16).to_le_bytes());
    b.b[12..16].copy_from_slice(&0x1F01FFu32.to_le_bytes());
    match shape {
        0 => b.b[16..16 + sb.len()].copy_from_slice(sb),
        1 => b.b[8 + 12 + 32..8 + 12 + 32 + sb.len()].copy_from_slice(sb),
        _ => {
            b.b[8 + 12..8 + 12 + sb.len()].copy_from_slice(sb);
            b.b[8 + 12 + sb.len()..8 + 12 + 2 * sb.len()].copy_from_slice(sb);
        }
    }
    b
}

fn test(t: &mut Test) {
    let types: [u8; 48] = [
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
        30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 0x55, 0x7F, 0x80, 0xC0, 0xFE, 0xFF,
    ];
    for shape in 0..3u32 {
        for rev in [2u32, 3, 4] {
            let mut list = Buf::<400>::new();
            for &ty in &types {
                let b = build(rev, ty, shape);
                if acl_valid(&b).0 != 0 {
                    let _ = write!(list, " {ty:#x}");
                }
            }
            asay!(t, "shape {shape} revision {rev}: valid types:{}", list.as_str());
        }
    }
    let b = build(4, 0, 0);
    acl_show(t, "access ACE reference", &b);
}
