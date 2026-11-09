//! GetNamedSecurityInfoW on a file and a directory made by the test with a known protected DACL:
//! the owner, group, DACL and SACL pointers it returns (inside one allocation or not), the ACL, the
//! security descriptor's allocation, which information bits are returned, and errors for bad names.
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
const OWNER_SECURITY_INFORMATION: u32 = 0x1;
const GROUP_SECURITY_INFORMATION: u32 = 0x2;
const DACL_SECURITY_INFORMATION: u32 = 0x4;
const SACL_SECURITY_INFORMATION: u32 = 0x8;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
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

/// The bytes of the little-endian u32 at `off`.
fn le32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
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

const SE_FILE_OBJECT: u32 = 1;
const SE_REGISTRY_KEY: u32 = 4;
/// The Win32 path of a file or folder named `leaf` next to this executable (the test's scratch
/// directory on the oracle), NUL-terminated.
fn c2_beside(leaf: &str) -> [u16; 520] {
    let mut p = [0u16; 520];
    // SAFETY: a buffer of 520 units for the module path of this executable.
    let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), p.as_mut_ptr(), 500) } as usize;
    let mut cut = n;
    while cut > 0 && p[cut - 1] != u16::from(b'\\') {
        cut -= 1;
    }
    for (i, b) in leaf.bytes().enumerate() {
        p[cut + i] = u16::from(b);
    }
    p[cut + leaf.len()] = 0;
    p
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

/// The ACE list of an ACL as lines `<label>: ace i type flags mask sid` (via `sd_acl_show`).
fn c2_acl(t: &mut Test, label: &str, acl: *const c_void) {
    sd_acl_show(t, label, acl);
}

/// `say!` with a longer line (SDDL texts, hex dumps).
macro_rules! sdsay {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<2000>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const SD_SENTINEL: usize = 0x1234_5678_9ABC;
/// An ACL's header and ACEs from a pointer: "rev size count" then per ACE "type flags size mask sid".
fn sd_acl_show(t: &mut Test, label: &str, acl: *const c_void) {
    if acl.is_null() {
        sdsay!(t, "{label}: (null)");
        return;
    }
    if acl as usize == SD_SENTINEL {
        sdsay!(t, "{label}: (not returned)");
        return;
    }
    // SAFETY: the caller passes a pointer to an ACL header (8 bytes at least).
    let head = unsafe { core::slice::from_raw_parts(acl as *const u8, 8) };
    let size = usize::from(u16::from_le_bytes([head[2], head[3]])).min(1024);
    let count = usize::from(u16::from_le_bytes([head[4], head[5]]));
    sdsay!(
        t,
        "{label}: acl rev={} sbz1={} size={} count={count} sbz2={}",
        head[0],
        head[1],
        size,
        u16::from_le_bytes([head[6], head[7]])
    );
    // SAFETY: the ACL is `size` bytes (as its own header says, at most 1024 here).
    let b = unsafe { core::slice::from_raw_parts(acl as *const u8, size.max(8)) };
    let mut off = 8usize;
    for i in 0..count.min(30) {
        if off + 8 > b.len() {
            sdsay!(t, "{label}: ace {i} at {off} is outside the ACL");
            break;
        }
        let ty = b[off];
        let flags = b[off + 1];
        let sz = usize::from(u16::from_le_bytes([b[off + 2], b[off + 3]]));
        let mask = le32(b, off + 4);
        let sid_off = if (5..=8).contains(&ty) && off + 12 <= b.len() {
            let f = le32(b, off + 8);
            off + 12 + if f & 1 != 0 { 16 } else { 0 } + if f & 2 != 0 { 16 } else { 0 }
        } else {
            off + 8
        };
        let sid = if sid_off + 8 <= b.len() { mask_sid(b[sid_off..].as_ptr().cast()) } else { Buf::<200>::new() };
        sdsay!(
            t,
            "{label}: ace {i} off={off} type={ty:#x} flags={flags:#x} size={sz} mask={mask:#x} sid={}",
            sid.as_str()
        );
        if sz == 0 {
            break;
        }
        off += sz;
    }
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn LocalFree(p: PVOID) -> PVOID;
    fn GetModuleFileNameW(module: HANDLE, name: *mut u16, size: u32) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        sa: PVOID,
        disposition: u32,
        flags: u32,
        template: HANDLE,
    ) -> HANDLE;
    fn DeleteFileW(name: *const u16) -> BOOL;
    fn CreateDirectoryW(name: *const u16, sa: PVOID) -> BOOL;
    fn RemoveDirectoryW(name: *const u16) -> BOOL;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
    fn ConvertSidToStringSidW(sid: *const c_void, out: *mut *mut u16) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn InitializeAcl(acl: PACL, len: u32, revision: u32) -> BOOL;
    fn AddAccessAllowedAceEx(acl: PACL, revision: u32, flags: u32, mask: u32, sid: *const c_void) -> BOOL;
    fn GetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        info: u32,
        owner: *mut PSID,
        group: *mut PSID,
        dacl: *mut PACL,
        sacl: *mut PACL,
        sd: *mut PSECURITY_DESCRIPTOR,
    ) -> u32;
    fn SetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        info: u32,
        owner: PSID,
        group: PSID,
        dacl: PACL,
        sacl: PACL,
    ) -> u32;
}

poly_wtest::main!(test);

fn make_acl(buf: &mut AclBuf) {
    let system = sid_of("S-1-5-18");
    let admins = sid_of("S-1-5-32-544");
    let world = sid_of("S-1-1-0");
    // SAFETY: a 1024-byte buffer and valid SIDs.
    unsafe {
        InitializeAcl(buf.p(), 1024, 2);
        AddAccessAllowedAceEx(buf.p(), 2, 0x3, 0x1F01FF, system.as_const());
        AddAccessAllowedAceEx(buf.p(), 2, 0x3, 0x1F01FF, admins.as_const());
        AddAccessAllowedAceEx(buf.p(), 2, 0x3, 0x120089, world.as_const());
    }
}

fn get(t: &mut Test, label: &str, path: &[u16], kind: u32, info: u32) {
    let mut owner: PSID = 0x11 as PSID;
    let mut group: PSID = 0x11 as PSID;
    let mut dacl: PACL = 0x11 as PACL;
    let mut sacl: PACL = 0x11 as PACL;
    let mut sd: PSECURITY_DESCRIPTOR = 0x11 as PVOID;
    // SAFETY: a NUL-terminated path and valid out pointers.
    let r = unsafe {
        GetNamedSecurityInfoW(path.as_ptr(), kind, info, &mut owner, &mut group, &mut dacl, &mut sacl, &mut sd)
    };
    let w = |p: *const c_void| -> &'static str {
        if p as usize == 0x11 {
            "untouched"
        } else if p.is_null() {
            "null"
        } else {
            "set"
        }
    };
    say!(t, "{label}: result {r} owner={} group={} dacl={} sacl={} sd={}", w(owner), w(group), w(dacl), w(sacl), w(sd));
    if r == 0 {
        if info & OWNER_SECURITY_INFORMATION != 0 {
            say!(t, "{label}: owner sid {}", c2_sid(owner).as_str());
        }
        if info & GROUP_SECURITY_INFORMATION != 0 {
            say!(t, "{label}: group sid {}", c2_sid(group).as_str());
        }
        if info & DACL_SECURITY_INFORMATION != 0 && !dacl.is_null() {
            c2_acl(t, label, dacl);
        }
        // SAFETY: the descriptor was allocated by the call with LocalAlloc.
        unsafe { LocalFree(sd) };
    }
}

fn test(t: &mut Test) {
    let dir = c2_beside("c2gns");
    let file = c2_beside("c2gns\\f.txt");
    // SAFETY: NUL-terminated paths.
    unsafe {
        CreateDirectoryW(dir.as_ptr(), core::ptr::null_mut());
        let h = CreateFileW(file.as_ptr(), 0x1000_0000, 0, core::ptr::null_mut(), 2, 0x80, core::ptr::null_mut());
        CloseHandle(h);
    }
    let mut acl = AclBuf::new();
    make_acl(&mut acl);
    for p in [&dir, &file] {
        // SAFETY: valid path and ACL.
        let r = unsafe {
            SetNamedSecurityInfoW(
                p.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                acl.p(),
                core::ptr::null_mut(),
            )
        };
        say!(t, "prepare: result {r}");
    }
    get(t, "file dacl", &file, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION);
    get(t, "directory dacl", &dir, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION);
    get(t, "file owner+group", &file, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION);
    get(t, "file all three", &file, SE_FILE_OBJECT, 7);
    get(
        t,
        "file sacl (ambient: administrator holding SeSecurityPrivilege)",
        &file,
        SE_FILE_OBJECT,
        SACL_SECURITY_INFORMATION,
    );
    get(t, "file no information requested", &file, SE_FILE_OBJECT, 0);
    get(t, "file label", &file, SE_FILE_OBJECT, 0x10);
    get(t, "file unknown bits", &file, SE_FILE_OBJECT, 0x100);
    let nope = c2_beside("c2gns\\missing.txt");
    get(t, "missing file", &nope, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION);
    let nodir = c2_beside("c2gns\\nodir\\f.txt");
    get(t, "missing directory", &nodir, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION);
    let empty = wide("");
    get(t, "empty name", &empty, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION);
    get(t, "object type 0", &file, 0, DACL_SECURITY_INFORMATION);
    get(t, "object type 99", &file, 99, DACL_SECURITY_INFORMATION);
    get(t, "wrong type (registry for a file path)", &file, SE_REGISTRY_KEY, DACL_SECURITY_INFORMATION);
    // SAFETY: cleaning up the files made above.
    unsafe {
        DeleteFileW(file.as_ptr());
        RemoveDirectoryW(dir.as_ptr());
    }
}
