//! NtQuerySecurityObject on an unnamed event (which has no security descriptor of its own), a named event and an
//! unnamed job object: the self-relative descriptor returned for each SECURITY_INFORMATION combination (control
//! bits, offsets of the parts, owner, group, DACL entries), including the empty mask, undefined and label bits.
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
const TRUE: BOOL = 1;
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

const STANDARD_RIGHTS_REQUIRED: u32 = 0x000F_0000;
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const GROUP_SECURITY_INFORMATION: u32 = 0x2;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const SACL_SECURITY_INFORMATION: u32 = 0x8;
const LABEL_SECURITY_INFORMATION: u32 = 0x10;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const UNPROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x2000_0000;
const JOB_OBJECT_ALL_ACCESS: u32 = STANDARD_RIGHTS_REQUIRED | SYNCHRONIZE | 0x3F;
/// winbase.h: `SECURITY_ATTRIBUTES`.
#[repr(C)]
struct SECURITY_ATTRIBUTES {
    length: u32,
    descriptor: PVOID,
    inherit: BOOL,
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

/// ntdef.h: `OBJECT_ATTRIBUTES` (48 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy)]
struct NT_OA {
    length: u32,
    root: HANDLE,
    name: *const UnicodeString,
    attributes: u32,
    descriptor: *const c_void,
    qos: *const c_void,
}

/// The handle's type name (NtQueryObject, ObjectTypeInformation, whose first field is the name).
fn nt_type_name(h: HANDLE) -> Buf<64> {
    let mut raw = Raw::<512>::new();
    let mut ret = 0u32;
    let mut out = Buf::<64>::new();
    // SAFETY: 512 writable bytes, as the class needs.
    let st = unsafe { NtQueryObject(h, 2, raw.p(), 512, &mut ret) };
    if st < 0 {
        let _ = write!(out, "<error {:#010x}>", st);
        return out;
    }
    let len = usize::from(raw.u16_at(0)) / 2;
    let p = raw.u64_at(8) as usize as *const u16;
    for i in 0..len.min(30) {
        // SAFETY: the name lies inside `raw` (len units from the pointer in the structure).
        let c = unsafe { *p.add(i) };
        let _ = write!(out, "{}", if c < 0x80 { c as u8 as char } else { '?' });
    }
    out
}

/// An unnamed job object with all access, or null.
fn nt_job() -> HANDLE {
    let mut h: HANDLE = core::ptr::null_mut();
    // SAFETY: valid out pointer; no object attributes.
    unsafe { NtCreateJobObject(&mut h, JOB_OBJECT_ALL_ACCESS, core::ptr::null()) };
    h
}

/// Print the ACL at `off` inside the self-relative descriptor at `base`: revision, size, count and each ACE
/// (type, flags, mask, masked SID).
fn nt_print_acl(t: &mut Test, label: &str, base: *const u8, off: usize) {
    if off == 0 {
        say!(t, "{label}: none");
        return;
    }
    // SAFETY: `off` lies inside the 4096-byte result buffer the caller read the descriptor into.
    let acl = unsafe { base.add(off) };
    // SAFETY: the ACL header is 8 bytes inside the buffer.
    let (rev, size, count) = unsafe {
        (*acl, u16::from_le_bytes([*acl.add(2), *acl.add(3)]), u16::from_le_bytes([*acl.add(4), *acl.add(5)]))
    };
    say!(t, "{label}: revision={rev} size={size} count={count}");
    let mut pos = 8usize;
    for i in 0..usize::from(count).min(16) {
        // SAFETY: the ACEs follow the header inside the ACL (bounded by the loop count and the buffer).
        let (ty, fl, sz, mask) = unsafe {
            let a = acl.add(pos);
            (
                *a,
                *a.add(1),
                u16::from_le_bytes([*a.add(2), *a.add(3)]),
                u32::from_le_bytes([*a.add(4), *a.add(5), *a.add(6), *a.add(7)]),
            )
        };
        // SAFETY: the SID of an access ACE starts at byte 8 of the ACE.
        let sid = unsafe { acl.add(pos + 8) } as *const c_void;
        say!(t, "{label}[{i}]: type={ty} flags={fl:#x} size={sz} mask={mask:#x} sid={}", mask_sid(sid).as_str());
        pos += usize::from(sz).max(8);
    }
}

/// NtQuerySecurityObject(info) of a handle printed: status, length, the self-relative layout (control, offsets),
/// the masked owner and group SIDs and the ACLs.
fn nt_print_sd(t: &mut Test, label: &str, h: HANDLE, info: u32) {
    let mut raw = Raw::<4096>::new();
    let mut ret = 0xAAAA_AAAAu32;
    // SAFETY: 4096 writable bytes.
    let st = unsafe { NtQuerySecurityObject(h, info, raw.p(), 4096, &mut ret) };
    if st < 0 {
        say!(t, "{label}: status={st:#010x} needed={}", if ret == 0xAAAA_AAAA { -1 } else { i64::from(ret) });
        return;
    }
    let base = raw.b.as_ptr();
    let (o, g, s, d) =
        (raw.u32_at(4) as usize, raw.u32_at(8) as usize, raw.u32_at(12) as usize, raw.u32_at(16) as usize);
    say!(
        t,
        "{label}: status={st:#010x} length={ret} revision={} control={:#06x} offsets owner={o} group={g} sacl={s} dacl={d}",
        raw.b[0],
        raw.u16_at(2)
    );
    if o != 0 {
        // SAFETY: the owner SID lies inside the buffer.
        say!(t, "{label}: owner={}", mask_sid(unsafe { base.add(o) } as *const c_void).as_str());
    }
    if g != 0 {
        // SAFETY: the group SID lies inside the buffer.
        say!(t, "{label}: group={}", mask_sid(unsafe { base.add(g) } as *const c_void).as_str());
    }
    if info & (SACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION) != 0 {
        nt_print_acl(t, "  sacl", base, s);
    }
    if info & DACL_SECURITY_INFORMATION != 0 {
        nt_print_acl(t, "  dacl", base, d);
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn CreateEventW(attrs: *const SECURITY_ATTRIBUTES, manual: BOOL, initial: BOOL, name: *const u16) -> HANDLE;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
}

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtQueryObject(h: HANDLE, class: u32, info: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtQuerySecurityObject(h: HANDLE, info: u32, sd: PVOID, len: u32, ret_len: *mut u32) -> i32;
    fn NtCreateJobObject(job: *mut HANDLE, access: u32, attrs: *const NT_OA) -> i32;
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    // SAFETY: an unnamed event, then a named one in the local namespace.
    let ev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, core::ptr::null()) };
    let w = wide("PolySecNtEvtSd1");
    // SAFETY: a named event, closed below.
    let nev = unsafe { CreateEventW(core::ptr::null(), TRUE, FALSE, w.as_ptr()) };
    say!(t, "event type={}", nt_type_name(ev).as_str());
    let infos: [(&str, u32); 17] = [
        ("OWNER", OWNER_SECURITY_INFORMATION),
        ("GROUP", GROUP_SECURITY_INFORMATION),
        ("DACL", DACL_SECURITY_INFORMATION),
        ("SACL", SACL_SECURITY_INFORMATION),
        ("LABEL", LABEL_SECURITY_INFORMATION),
        ("OWNER|GROUP|DACL", OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION),
        ("GROUP|DACL", GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION),
        ("0", 0),
        ("ATTRIBUTE 0x20", 0x20),
        ("SCOPE 0x40", 0x40),
        ("PROCESS_TRUST_LABEL 0x80", 0x80),
        ("ACCESS_FILTER 0x100", 0x100),
        ("BACKUP 0x10000", 0x10000),
        ("PROTECTED_DACL 0x80000000", PROTECTED_DACL_SECURITY_INFORMATION),
        ("UNPROTECTED_DACL 0x20000000", UNPROTECTED_DACL_SECURITY_INFORMATION),
        ("DACL|PROTECTED_DACL", DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION),
        ("0xFFFFFFFF", 0xFFFF_FFFF),
    ];
    for (name, info) in infos {
        let mut l = Buf::<100>::new();
        let _ = write!(l, "unnamed event {name}");
        nt_print_sd(t, l.as_str(), ev, info);
    }
    for (name, info) in infos {
        let mut l = Buf::<100>::new();
        let _ = write!(l, "named event {name}");
        nt_print_sd(t, l.as_str(), nev, info);
    }
    // SAFETY: closing the events.
    unsafe {
        CloseHandle(ev);
        CloseHandle(nev);
    }
    let job = nt_job();
    say!(t, "job type={}", nt_type_name(job).as_str());
    for (name, info) in [
        ("job OWNER|GROUP|DACL", OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION),
        ("job LABEL", LABEL_SECURITY_INFORMATION),
        ("job SACL", SACL_SECURITY_INFORMATION),
    ] {
        nt_print_sd(t, name, job, info);
    }
    // SAFETY: closing the job.
    unsafe { NtClose(job) };
}
