//! GetAclInformation: unknown information classes, and ACLs that are damaged (revision, count,
//! ACE size, size field).
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
const ACL_REVISION: u32 = 2;
const AclRevisionInformation: u32 = 1;
const AclSizeInformation: u32 = 2;
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

/// A short name for an error value printed after a failure: the number, or "-" after success.
fn acl_err(ok: i32) -> i64 {
    gle_if(ok)
}

/// A revision-2 ACL of `size` bytes with three ACEs: allowed S-1-1-0 mask 1 (20 bytes), denied
/// S-1-5-18 mask 2 (20 bytes), allowed S-1-5-32-544 mask 4 (24 bytes); 72 bytes are in use.
fn acl_three<const N: usize>(buf: &mut Raw<N>, size: u32) {
    acl_init_fill(buf, size, ACL_REVISION);
    let a = sid_of("S-1-1-0");
    let s = sid_of("S-1-5-18");
    let adm = sid_of("S-1-5-32-544");
    // SAFETY: `buf` holds an initialised ACL with room for the three ACEs; the SIDs are valid.
    unsafe {
        AddAccessAllowedAce(buf.p(), 2, 1, a.as_const());
        AddAccessDeniedAce(buf.p(), 2, 2, s.as_const());
        AddAccessAllowedAce(buf.p(), 2, 4, adm.as_const());
    }
}

/// GetAclInformation: (result, last error) with the buffer `out` of `len` bytes.
fn acl_get_info<const N: usize, const M: usize>(b: &Raw<N>, out: &mut Raw<M>, len: u32, class: u32) -> (i32, i64) {
    clear_error();
    // SAFETY: `out` has at least `len` bytes in every use; `b` holds the ACL.
    let ok = unsafe { GetAclInformation(b.b.as_ptr().cast(), out.p(), len, class) };
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
    fn GetAclInformation(acl: *const c_void, info: *mut c_void, len: u32, class: u32) -> BOOL;
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let mut good = Raw::<1100>::new();
    acl_three(&mut good, 128);
    for class in [0u32, 1, 2, 3, 4, 5, 6, 7, 16, 0xFF, 0x100, 0xFFFF_FFFF] {
        let mut out = Raw::<64>::new();
        out.b.fill(0xAA);
        let (ok, err) = acl_get_info(&good, &mut out, 16, class);
        let changed = out.b.iter().filter(|&&x| x != 0xAA).count();
        asay!(t, "class {class:#x}, length 16: ok={ok} err={err} bytes written={changed}");
    }
    let cases: [(&str, fn(&mut Raw<1100>)); 14] = [
        ("revision 0", |b| b.b[0] = 0),
        ("revision 1", |b| b.b[0] = 1),
        ("revision 5", |b| b.b[0] = 5),
        ("revision 255", |b| b.b[0] = 255),
        ("count 5 (only three ACEs)", |b| b.b[4] = 5),
        ("count 100", |b| b.b[4] = 100),
        ("count 1", |b| b.b[4] = 1),
        ("count 0", |b| b.b[4] = 0),
        ("first ACE size 0", |b| b.b[10] = 0),
        ("first ACE size 4", |b| b.b[10] = 4),
        ("first ACE size 400", |b| {
            b.b[10] = 0x90;
            b.b[11] = 0x01;
        }),
        ("acl size 40", |b| b.b[2] = 40),
        ("acl size 72", |b| b.b[2] = 72),
        ("acl size 1100", |b| {
            b.b[2] = 0x4C;
            b.b[3] = 0x04;
        }),
    ];
    for (label, f) in cases {
        let mut b = good;
        f(&mut b);
        for class in [AclRevisionInformation, AclSizeInformation] {
            let mut out = Raw::<64>::new();
            out.b.fill(0xAA);
            let (ok, err) = acl_get_info(&b, &mut out, 12, class);
            let changed = out.b.iter().filter(|&&x| x != 0xAA).count();
            asay!(
                t,
                "{label}, class {class}: ok={ok} err={err} values={},{},{} bytes written={changed}",
                out.u32_at(0),
                out.u32_at(4),
                out.u32_at(8)
            );
        }
    }
}
