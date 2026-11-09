//! AddAuditAccessAceEx: invalid SIDs, an ACL with too little room, damaged ACL headers.
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
const TRUE: BOOL = 1;
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

/// A SID with the given revision byte and sub-authority count, authority 5 and sub-authorities
/// 1, 2, 3, ...; for invalid SIDs (the buffer is large enough for any count).
fn acl_sid_raw(rev: u8, count: u8) -> Raw<1100> {
    let mut r = Raw::<1100>::new();
    r.b[0] = rev;
    r.b[1] = count;
    r.b[7] = 5;
    for i in 0..usize::from(count).min(250) {
        r.b[8 + 4 * i] = (i + 1) as u8;
    }
    r
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

/// One of the AddAccess*Ace functions as `(acl, revision, flags, mask, sid) -> BOOL`.
type AclAddFn = fn(PACL, u32, u32, u32, *const c_void) -> i32;

/// AddAuditAccessAceEx with success and failure auditing both on.
fn acl_add_audit_ex(acl: PACL, rev: u32, flags: u32, mask: u32, sid: *const c_void) -> i32 {
    // SAFETY: the pointers are an ACL buffer and a SID buffer of the caller.
    unsafe { AddAuditAccessAceEx(acl, rev, flags, mask, sid, TRUE, TRUE) }
}

/// Scenario: invalid SIDs, a full ACL, a damaged ACL header.
fn acl_scn_errors(t: &mut Test, add: AclAddFn, flags: u32, sid_text_: &str, mask: u32) {
    let sid = sid_of(sid_text_);
    // Invalid SIDs (revision byte, sub-authority count).
    for (rev, count) in
        [(1u8, 0u8), (1, 1), (1, 15), (0, 1), (2, 1), (3, 1), (0xFF, 1), (1, 16), (1, 17), (1, 100), (1, 255), (0, 0)]
    {
        let mut s = acl_sid_raw(rev, count);
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 1000, ACL_REVISION);
        clear_error();
        let ok = add(b.p(), ACL_REVISION, flags, mask, s.p());
        asay!(
            t,
            "sid revision {rev} count {count}: ok={ok} err={} acl count={} first bytes={}",
            acl_err(ok),
            b.u16_at(4),
            Hex(&b.b[8..16])
        );
    }
    // Space: the ACE is 8 + the SID's length bytes; sizes around it.
    let ace = 8 + sid.bytes().len();
    asay!(t, "ace size for {sid_text_}: {ace}");
    for size in [0u32, 4, 7, 8, 9, 12, 16, 19, 20, 27, 28, 29, 30, 31, 32, 35, 36, 47, 48, 49, 55, 56] {
        let mut b = Raw::<1100>::new();
        let init = acl_init_fill(&mut b, size, ACL_REVISION);
        if init == 0 {
            asay!(t, "acl size {size}: InitializeAcl failed, err={}", acl_err(init));
            continue;
        }
        let size_field = b.u16_at(2);
        clear_error();
        let ok1 = add(b.p(), ACL_REVISION, flags, mask, sid.as_const());
        let e1 = acl_err(ok1);
        let count1 = b.u16_at(4);
        clear_error();
        let ok2 = add(b.p(), ACL_REVISION, flags, mask, sid.as_const());
        let e2 = acl_err(ok2);
        let used = 8 + ace * usize::from(b.u16_at(4));
        let tail_untouched = b.b[usize::from(size_field).min(1100)..].iter().all(|&x| x == 0xCC);
        asay!(
            t,
            "acl size {size} (size field {size_field}): first ok={ok1} err={e1} count={count1}; second ok={ok2} err={e2} count={}; used={used}; tail untouched={tail_untouched}",
            b.u16_at(4)
        );
    }
    // A damaged ACL header.
    let cases: [(&str, fn(&mut Raw<1100>)); 14] = [
        ("revision 0", |b| b.b[0] = 0),
        ("revision 1", |b| b.b[0] = 1),
        ("revision 5", |b| b.b[0] = 5),
        ("revision 255", |b| b.b[0] = 255),
        ("sbz1 = 1", |b| b.b[1] = 1),
        ("sbz2 = 1", |b| b.b[6] = 1),
        ("size 0", |b| b.b[2] = 0),
        ("size 4", |b| b.b[2] = 4),
        ("size 7", |b| b.b[2] = 7),
        ("size 8", |b| b.b[2] = 8),
        ("size 1000", |b| {
            b.b[2] = 0xE8;
            b.b[3] = 0x03;
        }),
        ("count 1 without an ACE", |b| b.b[4] = 1),
        ("count 200 without ACEs", |b| b.b[4] = 200),
        ("count 0xFFFF without ACEs", |b| {
            b.b[4] = 0xFF;
            b.b[5] = 0xFF;
        }),
    ];
    for (label, f) in cases {
        let mut b = Raw::<1100>::new();
        acl_init_fill(&mut b, 64, ACL_REVISION);
        b.b[8..].fill(0);
        f(&mut b);
        clear_error();
        let ok = add(b.p(), ACL_REVISION, flags, mask, sid.as_const());
        asay!(
            t,
            "header {label}: ok={ok} err={} count after={} first bytes={}",
            acl_err(ok),
            b.u16_at(4),
            Hex(&b.b[8..16])
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
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAuditAccessAceEx(
        acl: PACL,
        revision: u32,
        flags: u32,
        mask: u32,
        sid: *const c_void,
        success: BOOL,
        failure: BOOL,
    ) -> BOOL;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    acl_scn_errors(t, acl_add_audit_ex, 0xC0, "S-1-1-0", 0x1);
}
