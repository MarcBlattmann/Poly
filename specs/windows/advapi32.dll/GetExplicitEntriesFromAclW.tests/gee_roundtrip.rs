//! GetExplicitEntriesFromAclW: the entries of ACLs built with AddAccessAllowedAceEx,
//! AddAccessDeniedAceEx and AddAuditAccessAceEx: count, access mode, inheritance, trustee form
//! and type, SIDs; an empty ACL; a null ACL.
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

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 255 units).
fn wide(s: &str) -> [u16; 256] {
    let mut w = [0u16; 256];
    for (i, c) in s.bytes().enumerate().take(255) {
        w[i] = u16::from(c);
    }
    w
}

/// The units of a NUL-terminated UTF-16 string.
fn wide_len(p: *const u16) -> usize {
    let mut n = 0;
    // SAFETY: the caller passes a NUL-terminated string.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
    }
    n
}

/// A UTF-16 string pointer as text (ASCII only; others as `?`).
fn wide_text<const N: usize>(p: *const u16) -> Buf<N> {
    let mut b = Buf::<N>::new();
    if p.is_null() {
        let _ = write!(b, "(null)");
        return b;
    }
    let n = wide_len(p);
    for i in 0..n {
        // SAFETY: i < n, inside the string.
        let c = unsafe { *p.add(i) };
        let _ = write!(b, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    b
}

/// The SID in the text form of `ConvertSidToStringSidW`, or an error marker.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a valid SID or the call fails; `out` is a valid out pointer.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut out) };
    if ok == 0 {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let b = wide_text::<200>(out);
    // SAFETY: the string was allocated by the call with LocalAlloc.
    unsafe { LocalFree(out.cast()) };
    b
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

/// The text of a SID with the machine-dependent parts masked: S-1-5-21-a-b-c-RID prints as
/// S-1-5-21-<machine>-RID and a logon-session SID S-1-5-5-x-y as S-1-5-5-<logon>.
fn mask_sid(sid: *const c_void) -> Buf<200> {
    let full = sid_text(sid);
    let s = full.as_str();
    let mut b = Buf::<200>::new();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let rid = s.rsplit('-').next().unwrap_or("?");
        if rid.parse::<u32>().map_or(false, |r| r >= 1000) {
            let _ = write!(b, "S-1-5-21-<machine>-<account>");
        } else {
            let _ = write!(b, "S-1-5-21-<machine>-{rid}");
        }
    } else if s.starts_with("S-1-5-5-") {
        let _ = write!(b, "S-1-5-5-<logon>");
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_NAME: u32 = 1;
/// accctrl.h `TRUSTEE_W` (32 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct TRUSTEE_W {
    multiple: *mut c_void,
    multiple_op: u32,
    form: u32,
    kind: u32,
    name: *mut u16,
}

/// accctrl.h `EXPLICIT_ACCESS_W` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct EXPLICIT_ACCESS_W {
    access: u32,
    mode: u32,
    inherit: u32,
    trustee: TRUSTEE_W,
}

impl EXPLICIT_ACCESS_W {
    fn filled(byte: u8) -> EXPLICIT_ACCESS_W {
        let mut e = EXPLICIT_ACCESS_W {
            access: 0,
            mode: 0,
            inherit: 0,
            trustee: TRUSTEE_W {
                multiple: core::ptr::null_mut(),
                multiple_op: 0,
                form: 0,
                kind: 0,
                name: core::ptr::null_mut(),
            },
        };
        // SAFETY: a plain-old-data structure; every byte is overwritten.
        unsafe {
            core::ptr::write_bytes(
                &mut e as *mut EXPLICIT_ACCESS_W as *mut u8,
                byte,
                core::mem::size_of::<EXPLICIT_ACCESS_W>(),
            )
        };
        e
    }
    /// An entry for a SID trustee, built by hand (no API call): form SID, type `kind`.
    fn for_sid(sid: PSID, access: u32, mode: u32, inherit: u32, kind: u32) -> EXPLICIT_ACCESS_W {
        EXPLICIT_ACCESS_W {
            access,
            mode,
            inherit,
            trustee: TRUSTEE_W {
                multiple: core::ptr::null_mut(),
                multiple_op: 0,
                form: TRUSTEE_IS_SID,
                kind,
                name: sid.cast(),
            },
        }
    }
}

/// A SID text with the machine- and user-specific parts masked (see `mask_sid`), or "(null)".
fn c2_sid(p: *const c_void) -> Buf<200> {
    if p.is_null() {
        let mut b = Buf::<200>::new();
        let _ = write!(b, "(null)");
        return b;
    }
    mask_sid(p)
}

/// One `EXPLICIT_ACCESS_W` as text; the trustee name/SID pointer as text when the form says so.
fn c2_ea(t: &mut Test, label: &str, e: &EXPLICIT_ACCESS_W) {
    let name: Buf<120> = if e.trustee.name.is_null() {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "(null)");
        b
    } else if e.trustee.form == TRUSTEE_IS_SID {
        let s = c2_sid(e.trustee.name.cast());
        let mut b = Buf::<120>::new();
        let _ = write!(b, "sid {}", s.as_str());
        b
    } else if e.trustee.form == TRUSTEE_IS_NAME {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "name {}", wide_text::<100>(e.trustee.name).as_str());
        b
    } else {
        let mut b = Buf::<120>::new();
        let _ = write!(b, "ptr");
        b
    };
    say!(
        t,
        "{label}: access={:#x} mode={} inherit={:#x} trustee: multiple_null={} op={} form={} type={} {}",
        e.access,
        e.mode,
        e.inherit,
        e.trustee.multiple.is_null(),
        e.trustee.multiple_op,
        e.trustee.form,
        e.trustee.kind,
        name.as_str()
    );
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LocalFree(p: PVOID) -> PVOID;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAccessDeniedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn AddAuditAccessAceEx(
        acl: PACL,
        revision: u32,
        flags: u32,
        mask: u32,
        sid: *const c_void,
        success: BOOL,
        failure: BOOL,
    ) -> BOOL;
    fn GetExplicitEntriesFromAclW(acl: PACL, count: *mut u32, entries: *mut *mut EXPLICIT_ACCESS_W) -> u32;
}

poly_wtest::main!(test);

fn show(t: &mut Test, label: &str, acl: PACL) {
    let mut n = 0xDEADu32;
    let mut entries: *mut EXPLICIT_ACCESS_W = core::ptr::null_mut();
    // SAFETY: valid ACL (or null) and out pointers.
    let r = unsafe { GetExplicitEntriesFromAclW(acl, &mut n, &mut entries) };
    say!(t, "{label}: result {r} count {n} entries_null={}", entries.is_null());
    if r == 0 && !entries.is_null() {
        for i in 0..n.min(10) as usize {
            let mut b = Buf::<40>::new();
            let _ = write!(b, "{label} entry {i}");
            // SAFETY: the call returned `n` entries.
            let e = unsafe { *entries.add(i) };
            c2_ea(t, b.as_str(), &e);
        }
        // SAFETY: the array was allocated by the call with LocalAlloc.
        unsafe { LocalFree(entries.cast()) };
    }
}

fn test(t: &mut Test) {
    let everyone = sid_of("S-1-1-0");
    let system = sid_of("S-1-5-18");
    let admins = sid_of("S-1-5-32-544");
    let mut buf = AclBuf::new();
    // SAFETY: a 1024-byte buffer and valid SIDs.
    unsafe {
        InitializeAcl(buf.p(), 1024, 4);
        AddAccessAllowedAceEx(buf.p(), 4, 0, 0x120089, everyone.as_const());
        AddAccessAllowedAceEx(buf.p(), 4, 3, 0x1F01FF, system.as_const());
        AddAccessDeniedAceEx(buf.p(), 4, 0x1, 0x2, admins.as_const());
        AddAuditAccessAceEx(buf.p(), 4, 0x40 | 0x80, 0x1, everyone.as_const(), 1, 1);
    }
    show(t, "mixed ACL", buf.p());
    let mut empty = AclBuf::new();
    // SAFETY: a 1024-byte buffer.
    unsafe { InitializeAcl(empty.p(), 1024, 2) };
    show(t, "empty ACL", empty.p());
    show(t, "null ACL", core::ptr::null_mut());
}
