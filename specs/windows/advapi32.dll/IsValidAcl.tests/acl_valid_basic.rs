//! IsValidAcl: fresh ACLs of all revisions and sizes, ACLs with ACEs, a changed revision byte,
//! changed size field, nonzero reserved bytes.
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

const SECURITY_MAX_SID_SIZE: usize = 68;
const ACL_REVISION: u32 = 2;
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
    fn AddAccessAllowedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAce(acl: PACL, revision: u32, mask: u32, sid: *const c_void) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    for rev in [2u32, 3, 4] {
        for size in [8u32, 12, 16, 64, 1000] {
            let mut b = Raw::<1100>::new();
            acl_init_fill(&mut b, size, rev);
            let (ok, err) = acl_valid(&b);
            asay!(t, "empty rev {rev} size {size}: valid={ok} err={err}");
        }
    }
    let mut good = Raw::<70000>::new();
    acl_three(&mut good, 128);
    let (ok, err) = acl_valid(&good);
    asay!(t, "three ACEs: valid={ok} err={err}");
    for rev in [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 0x10, 0x80, 0xFF] {
        let mut b = good;
        b.b[0] = rev;
        let (ok, err) = acl_valid(&b);
        asay!(t, "revision byte {rev}: valid={ok} err={err}");
    }
    for rev in [0u8, 1, 2, 3, 4, 5] {
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 16, 2);
        b.b[0] = rev;
        let (ok, err) = acl_valid(&b);
        asay!(t, "empty ACL, revision byte {rev}: valid={ok} err={err}");
    }
    for (label, off, val) in
        [("sbz1 = 1", 1usize, 1u8), ("sbz1 = 0xFF", 1, 0xFF), ("sbz2 = 1", 6, 1), ("sbz2 = 0x80", 7, 0x80)]
    {
        let mut b = good;
        b.b[off] = val;
        let (ok, err) = acl_valid(&b);
        asay!(t, "{label}: valid={ok} err={err}");
    }
    for size in [
        0u16, 1, 4, 7, 8, 9, 12, 27, 28, 71, 72, 73, 74, 75, 76, 127, 128, 129, 130, 131, 132, 1000, 1100, 2000, 0xFFFF,
    ] {
        let mut b = good;
        b.b[2..4].copy_from_slice(&size.to_le_bytes());
        let (ok, err) = acl_valid(&b);
        asay!(t, "three ACEs (72 bytes used), size field {size}: valid={ok} err={err}");
    }
}
