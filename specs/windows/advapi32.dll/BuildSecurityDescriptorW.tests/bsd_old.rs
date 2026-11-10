//! BuildSecurityDescriptorW with an old descriptor: what it keeps of the old owner, group, control
//! flags and lists, how new access entries (grant, set, deny, revoke) are merged into the old
//! DACL, and the errors for an old descriptor in absolute form and for a trustee name nobody has.
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, non_upper_case_globals, non_snake_case, clippy::all)]

// The declarations below are from Microsoft Learn (prototypes) and the
// public SDK headers winnt.h, winbase.h, fileapi.h, namedpipeapi.h,
// winternl.h, ntifs.h (FILE_NAME_INFORMATION), accctrl.h, aclapi.h, sddl.h,
// securitybaseapi.h, ntstatus.h and winerror.h (constants, layouts); the
// helpers are this test's own.

use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type BOOL = i32;

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

/// Where observation lines go: the test's output file, or (in a child process) a pipe to the parent.
trait Sink {
    fn put(&mut self, s: &str);
}

impl Sink for Test {
    fn put(&mut self, s: &str) {
        self.line(s);
    }
}

/// One observation line, formatted.
macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<600>::new();
        let _ = write!(b, $($a)*);
        $t.put(b.as_str());
    }};
}

/// A NUL-terminated UTF-16 copy of an ASCII string (up to 299 units).
fn wide(s: &str) -> [u16; 300] {
    let mut w = [0u16; 300];
    for (i, c) in s.bytes().enumerate().take(299) {
        w[i] = u16::from(c);
    }
    w
}

/// UTF-16 units as text: printable ASCII as it is, anything else as `<xxxx>`.
fn units_text(u: &[u16]) -> Buf<400> {
    let mut b = Buf::<400>::new();
    for &c in u {
        if (0x20..0x7f).contains(&c) {
            let _ = write!(b, "{}", c as u8 as char);
        } else {
            let _ = write!(b, "<{c:04x}>");
        }
    }
    b
}

type PSID = *mut c_void;

const NOT_USED_ACCESS: u32 = 0;
const GRANT_ACCESS: u32 = 1;
const SET_ACCESS: u32 = 2;
const DENY_ACCESS: u32 = 3;
const REVOKE_ACCESS: u32 = 4;
const SET_AUDIT_SUCCESS: u32 = 5;
const SET_AUDIT_FAILURE: u32 = 6;
const NO_INHERITANCE: u32 = 0;
const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_NAME: u32 = 1;
const TOKEN_QUERY: u32 = 8;
const TOKEN_USER_CLASS: u32 = 1; // TokenUser (winnt.h TOKEN_INFORMATION_CLASS)
const SDDL_REVISION_1: u32 = 1;
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

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
    inheritance: u32,
    trustee: TRUSTEE_W,
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn BuildSecurityDescriptorW(
        owner: *const TRUSTEE_W,
        group: *const TRUSTEE_W,
        access_count: u32,
        access: *const EXPLICIT_ACCESS_W,
        audit_count: u32,
        audit: *const EXPLICIT_ACCESS_W,
        old: *const c_void,
        size: *mut u32,
        new: *mut *mut c_void,
    ) -> u32;
    fn BuildExplicitAccessWithNameW(
        ea: *mut EXPLICIT_ACCESS_W,
        name: *mut u16,
        access: u32,
        mode: u32,
        inheritance: u32,
    );
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        s: *const u16,
        revision: u32,
        sd: *mut *mut c_void,
        size: *mut u32,
    ) -> BOOL;
    fn ConvertStringSidToSidW(s: *const u16, sid: *mut PSID) -> BOOL;
    fn ConvertSidToStringSidW(sid: *const c_void, s: *mut *mut u16) -> BOOL;
    fn GetSecurityDescriptorControl(sd: *const c_void, control: *mut u16, revision: *mut u32) -> BOOL;
    fn GetSecurityDescriptorOwner(sd: *const c_void, owner: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorGroup(sd: *const c_void, group: *mut PSID, defaulted: *mut BOOL) -> BOOL;
    fn GetSecurityDescriptorDacl(
        sd: *const c_void,
        present: *mut BOOL,
        acl: *mut *mut c_void,
        defaulted: *mut BOOL,
    ) -> BOOL;
    fn GetSecurityDescriptorSacl(
        sd: *const c_void,
        present: *mut BOOL,
        acl: *mut *mut c_void,
        defaulted: *mut BOOL,
    ) -> BOOL;
    fn GetSecurityDescriptorLength(sd: *const c_void) -> u32;
    fn IsValidSecurityDescriptor(sd: *const c_void) -> BOOL;
    fn InitializeSecurityDescriptor(sd: *mut c_void, revision: u32) -> BOOL;
    fn GetAce(acl: *const c_void, index: u32, ace: *mut *mut c_void) -> BOOL;
    fn OpenProcessToken(process: HANDLE, access: u32, token: *mut HANDLE) -> BOOL;
    fn GetTokenInformation(token: HANDLE, class: u32, buf: *mut c_void, len: u32, ret: *mut u32) -> BOOL;
    fn EqualSid(a: *const c_void, b: *const c_void) -> BOOL;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentProcess() -> HANDLE;
    fn CloseHandle(h: HANDLE) -> BOOL;
    fn GetLastError() -> u32;
    fn SetLastError(e: u32);
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn LocalSize(p: *mut c_void) -> usize;
}

/// The process token's user SID, kept in `buf` (TOKEN_USER: a pointer to the SID comes first).
fn token_user(buf: &mut [u64; 16]) -> PSID {
    let mut token: HANDLE = core::ptr::null_mut();
    let mut ret = 0u32;
    // SAFETY: live out pointers; `buf` is 128 bytes, 8-byte aligned.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return core::ptr::null_mut();
        }
        let ok = GetTokenInformation(token, TOKEN_USER_CLASS, buf.as_mut_ptr().cast(), 128, &mut ret);
        CloseHandle(token);
        if ok == 0 {
            return core::ptr::null_mut();
        }
    }
    buf[0] as PSID
}

/// A SID as text; the token's user as `<token user>`, other account SIDs with the machine part masked.
fn sid_text(sid: *const c_void) -> Buf<200> {
    let mut b = Buf::<200>::new();
    if sid.is_null() {
        let _ = write!(b, "none");
        return b;
    }
    let mut user_buf = [0u64; 16];
    let user = token_user(&mut user_buf);
    // SAFETY: both are valid SIDs.
    if !user.is_null() && unsafe { EqualSid(sid, user) } != 0 {
        let _ = write!(b, "<token user>");
        return b;
    }
    let mut out: *mut u16 = core::ptr::null_mut();
    // SAFETY: `sid` is a SID inside a descriptor or made by ConvertStringSidToSidW.
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 {
        let _ = write!(b, "<invalid sid>");
        return b;
    }
    let mut full = Buf::<200>::new();
    let mut i = 0;
    // SAFETY: the string is NUL-terminated and was allocated by the call with LocalAlloc.
    unsafe {
        while *out.add(i) != 0 {
            let _ = write!(full, "{}", *out.add(i) as u8 as char);
            i += 1;
        }
        LocalFree(out.cast());
    }
    let s = full.as_str();
    if s.starts_with("S-1-5-21-") && s.matches('-').count() >= 7 {
        let _ = write!(b, "S-1-5-21-<machine>-{}", s.rsplit('-').next().unwrap_or("?"));
    } else {
        let _ = write!(b, "{s}");
    }
    b
}

/// The lines for one ACL of a descriptor (winnt.h ACL: AclRevision u8 @0, AclSize u16 @2, AceCount u16 @4;
/// ACE_HEADER: AceType u8 @0, AceFlags u8 @1, AceSize u16 @2; then the mask u32 @4 and the SID @8).
fn dump_acl(t: &mut impl Sink, what: &str, which: &str, present: BOOL, defaulted: BOOL, acl: *const c_void) {
    if present == 0 {
        say!(t, "{what}: {which} absent");
        return;
    }
    if acl.is_null() {
        say!(t, "{what}: {which} present, NULL, defaulted {defaulted}");
        return;
    }
    let p = acl as *const u8;
    // SAFETY: an ACL header is 8 bytes.
    let (rev, size, count) = unsafe { (*p, *(p.add(2) as *const u16), *(p.add(4) as *const u16)) };
    say!(t, "{what}: {which} present, defaulted {defaulted}, revision {rev}, {size} bytes, {count} entries");
    for i in 0..u32::from(count) {
        let mut ace: *mut c_void = core::ptr::null_mut();
        // SAFETY: `acl` is a valid ACL with `count` entries.
        if unsafe { GetAce(acl, i, &mut ace) } == 0 {
            say!(t, "{what}: {which} entry {i}: unreadable");
            continue;
        }
        let a = ace as *const u8;
        // SAFETY: every ACE type used here is a header, a mask and a SID.
        let (ty, flags, asize, mask) = unsafe { (*a, *a.add(1), *(a.add(2) as *const u16), *(a.add(4) as *const u32)) };
        // SAFETY: the SID starts 8 bytes into the ACE.
        let sid = unsafe { a.add(8) } as *const c_void;
        say!(
            t,
            "{what}: {which} entry {i}: type {ty} flags {flags:#x} size {asize} mask {mask:#x} sid {}",
            sid_text(sid).as_str()
        );
    }
}

/// Everything a security descriptor holds, as lines starting with `what`.
fn dump_sd(t: &mut impl Sink, what: &str, sd: *const c_void) {
    if sd.is_null() {
        say!(t, "{what}: no descriptor");
        return;
    }
    let mut control = 0u16;
    let mut revision = 0u32;
    let mut owner: PSID = core::ptr::null_mut();
    let mut group: PSID = core::ptr::null_mut();
    let (mut od, mut gd, mut dp, mut dd, mut sp, mut sdef) = (0, 0, 0, 0, 0, 0);
    let mut dacl: *mut c_void = core::ptr::null_mut();
    let mut sacl: *mut c_void = core::ptr::null_mut();
    // SAFETY: `sd` is a descriptor made by advapi32; every out pointer is a live local.
    unsafe {
        let valid = IsValidSecurityDescriptor(sd);
        let length = GetSecurityDescriptorLength(sd);
        GetSecurityDescriptorControl(sd, &mut control, &mut revision);
        GetSecurityDescriptorOwner(sd, &mut owner, &mut od);
        GetSecurityDescriptorGroup(sd, &mut group, &mut gd);
        GetSecurityDescriptorDacl(sd, &mut dp, &mut dacl, &mut dd);
        GetSecurityDescriptorSacl(sd, &mut sp, &mut sacl, &mut sdef);
        say!(t, "{what}: valid {valid} length {length} control {control:#x} revision {revision}");
    }
    say!(t, "{what}: owner {} defaulted {od}", sid_text(owner).as_str());
    say!(t, "{what}: group {} defaulted {gd}", sid_text(group).as_str());
    dump_acl(t, what, "dacl", dp, dd, dacl);
    dump_acl(t, what, "sacl", sp, sdef, sacl);
}

/// A SID from its string form (kept for the life of the process).
fn sid(s: &str) -> PSID {
    let mut w = [0u16; 80];
    for (i, c) in s.bytes().enumerate().take(79) {
        w[i] = u16::from(c);
    }
    let mut out: PSID = core::ptr::null_mut();
    // SAFETY: `w` is NUL-terminated; `out` is a live local.
    unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut out) };
    out
}

fn trustee_sid(s: PSID) -> TRUSTEE_W {
    TRUSTEE_W { multiple: core::ptr::null_mut(), multiple_op: 0, form: TRUSTEE_IS_SID, kind: 0, name: s.cast() }
}

fn trustee_name(name: *mut u16) -> TRUSTEE_W {
    TRUSTEE_W { multiple: core::ptr::null_mut(), multiple_op: 0, form: TRUSTEE_IS_NAME, kind: 0, name }
}

fn entry(mode: u32, access: u32, inheritance: u32, trustee: TRUSTEE_W) -> EXPLICIT_ACCESS_W {
    EXPLICIT_ACCESS_W { access, mode, inheritance, trustee }
}

/// A self-relative descriptor from SDDL (freed with LocalFree), or NULL.
fn from_sddl(s: &str) -> *mut c_void {
    let w = wide(s);
    let mut sd: *mut c_void = core::ptr::null_mut();
    // SAFETY: `w` is NUL-terminated; `sd` is a live local.
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            w.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            core::ptr::null_mut(),
        )
    };
    sd
}

/// One BuildSecurityDescriptorW call: what it returned, the last error (set to 0xDEAD before), the
/// size and pointer it left (0xAAAAAAAA and a marker before), and the descriptor it made.
fn build(
    t: &mut impl Sink,
    what: &str,
    owner: *const TRUSTEE_W,
    group: *const TRUSTEE_W,
    access: &[EXPLICIT_ACCESS_W],
    audit: &[EXPLICIT_ACCESS_W],
    old: *const c_void,
) {
    let marker = 0x1111usize as *mut c_void;
    let mut size: u32 = 0xAAAA_AAAA;
    let mut new = marker;
    let ap = if access.is_empty() { core::ptr::null() } else { access.as_ptr() };
    let up = if audit.is_empty() { core::ptr::null() } else { audit.as_ptr() };
    // SAFETY: every pointer is NULL or to live data of the stated count; the out pointers are live locals.
    let (r, e) = unsafe {
        SetLastError(0xDEAD);
        let r = BuildSecurityDescriptorW(
            owner,
            group,
            access.len() as u32,
            ap,
            audit.len() as u32,
            up,
            old,
            &mut size,
            &mut new,
        );
        (r, GetLastError())
    };
    let mut et = Buf::<24>::new();
    if e == 0xDEAD {
        let _ = write!(et, "unchanged");
    } else {
        let _ = write!(et, "{e}");
    }
    let mut st = Buf::<24>::new();
    if size == 0xAAAA_AAAA {
        let _ = write!(st, "unchanged");
    } else {
        let _ = write!(st, "{size}");
    }
    let pt = if new == marker {
        "unchanged"
    } else if new.is_null() {
        "NULL"
    } else {
        "set"
    };
    say!(t, "{what}: ret {r} last error {} size {} pointer {pt}", et.as_str(), st.as_str());
    if r == 0 && new != marker && !new.is_null() {
        // SAFETY: the function documents the result as LocalAlloc memory the caller frees.
        let local = unsafe { LocalSize(new) };
        say!(t, "{what}: LocalSize {local}");
        dump_sd(t, what, new);
        // SAFETY: freed once.
        unsafe { LocalFree(new) };
    }
}

poly_wtest::main!(test);

fn test(t: &mut Test) {
    let null: *const TRUSTEE_W = core::ptr::null();
    let everyone = sid("S-1-1-0");
    let users = sid("S-1-5-32-545");
    let system = sid("S-1-5-18");
    let service = sid("S-1-5-19");

    // An absolute descriptor (InitializeSecurityDescriptor) as the old one.
    let mut abs = [0u64; 8];
    // SAFETY: SECURITY_DESCRIPTOR is 40 bytes on x64; `abs` is 64.
    let ok = unsafe { InitializeSecurityDescriptor(abs.as_mut_ptr().cast(), SECURITY_DESCRIPTOR_REVISION) };
    say!(t, "absolute descriptor initialised {ok}");
    build(t, "old is absolute", null, null, &[], &[], abs.as_ptr().cast());

    let old = from_sddl("O:BAG:SYD:(A;;GR;;;WD)(D;;GW;;;BU)(A;;GA;;;SY)");
    dump_sd(t, "old", old);
    build(t, "old, nothing new", null, null, &[], &[], old);
    build(t, "old, grant Everyone 0x2", null, null, &[entry(GRANT_ACCESS, 2, 0, trustee_sid(everyone))], &[], old);
    build(t, "old, set Everyone 0x2", null, null, &[entry(SET_ACCESS, 2, 0, trustee_sid(everyone))], &[], old);
    build(t, "old, deny Everyone 0x2", null, null, &[entry(DENY_ACCESS, 2, 0, trustee_sid(everyone))], &[], old);
    build(t, "old, revoke Users", null, null, &[entry(REVOKE_ACCESS, 0, 0, trustee_sid(users))], &[], old);
    build(
        t,
        "old, grant LOCAL SERVICE 0x1 inherit 3",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 3, trustee_sid(service))],
        &[],
        old,
    );
    let three = [
        entry(REVOKE_ACCESS, 0, 0, trustee_sid(users)),
        entry(SET_ACCESS, 1, 0, trustee_sid(system)),
        entry(DENY_ACCESS, 4, 0, trustee_sid(service)),
    ];
    build(t, "old, revoke Users + set SYSTEM 0x1 + deny LOCAL SERVICE 0x4", null, null, &three, &[], old);
    let owner = trustee_sid(service);
    build(t, "old, new owner", &owner, null, &[], &[], old);
    build(t, "old, new group", null, &owner, &[], &[], old);
    build(
        t,
        "old, audit Everyone 0x10000",
        null,
        null,
        &[],
        &[entry(SET_AUDIT_SUCCESS, 0x10000, 0, trustee_sid(everyone))],
        old,
    );

    let mut nobody = wide("PolySpec4NoSuchAccount");
    build(
        t,
        "old, grant to a name nobody has",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_name(nobody.as_mut_ptr()))],
        &[],
        old,
    );
    let bad_owner = trustee_name(nobody.as_mut_ptr());
    build(t, "old, owner is a name nobody has", &bad_owner, null, &[], &[], old);
    build(t, "old, entry with mode 0", null, null, &[entry(NOT_USED_ACCESS, 1, 0, trustee_sid(everyone))], &[], old);
    build(t, "old, entry with mode 7", null, null, &[entry(7, 1, 0, trustee_sid(everyone))], &[], old);
    // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
    unsafe { LocalFree(old) };

    // Where new entries go in old lists of other shapes.
    let old = from_sddl("D:(D;;GW;;;BU)(A;;GR;;;WD)(A;;GA;;;SY)");
    build(
        t,
        "deny-first old, grant LOCAL SERVICE 0x1",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_sid(service))],
        &[],
        old,
    );
    build(
        t,
        "deny-first old, deny LOCAL SERVICE 0x4",
        null,
        null,
        &[entry(DENY_ACCESS, 4, 0, trustee_sid(service))],
        &[],
        old,
    );
    build(
        t,
        "deny-first old, set Everyone 0x2",
        null,
        null,
        &[entry(SET_ACCESS, 2, 0, trustee_sid(everyone))],
        &[],
        old,
    );
    build(
        t,
        "deny-first old, revoke Everyone",
        null,
        null,
        &[entry(REVOKE_ACCESS, 0, 0, trustee_sid(everyone))],
        &[],
        old,
    );
    build(
        t,
        "deny-first old, grant Users GENERIC_WRITE",
        null,
        null,
        &[entry(GRANT_ACCESS, GENERIC_WRITE, 0, trustee_sid(users))],
        &[],
        old,
    );
    build(
        t,
        "deny-first old, deny Everyone GENERIC_READ",
        null,
        null,
        &[entry(DENY_ACCESS, GENERIC_READ, 0, trustee_sid(everyone))],
        &[],
        old,
    );
    // SAFETY: as above.
    unsafe { LocalFree(old) };
    let old = from_sddl("D:(A;;GR;;;WD)(A;;GA;;;SY)(D;;GW;;;BU)");
    build(
        t,
        "deny-last old, grant LOCAL SERVICE 0x1",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_sid(service))],
        &[],
        old,
    );
    build(
        t,
        "deny-last old, deny LOCAL SERVICE 0x4",
        null,
        null,
        &[entry(DENY_ACCESS, 4, 0, trustee_sid(service))],
        &[],
        old,
    );
    // SAFETY: as above.
    unsafe { LocalFree(old) };
    let old = from_sddl("D:(A;;GA;;;SY)(A;ID;GR;;;WD)");
    dump_sd(t, "inherited-entry old", old);
    build(
        t,
        "inherited-entry old, grant LOCAL SERVICE 0x1",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_sid(service))],
        &[],
        old,
    );
    build(
        t,
        "inherited-entry old, deny LOCAL SERVICE 0x4",
        null,
        null,
        &[entry(DENY_ACCESS, 4, 0, trustee_sid(service))],
        &[],
        old,
    );
    // SAFETY: as above.
    unsafe { LocalFree(old) };

    // Control flags and a SACL of the old descriptor.
    let old = from_sddl("O:SYG:SYD:PAI(A;OICI;GA;;;SY)S:PAI(AU;SA;GA;;;WD)(ML;;NW;;;LW)");
    dump_sd(t, "protected old", old);
    build(t, "protected old, nothing new", null, null, &[], &[], old);
    build(
        t,
        "protected old, grant Everyone 0x1",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_sid(everyone))],
        &[],
        old,
    );
    build(
        t,
        "protected old, audit failure Users 0x2",
        null,
        null,
        &[],
        &[entry(SET_AUDIT_FAILURE, 2, 0, trustee_sid(users))],
        old,
    );
    // SAFETY: as above.
    unsafe { LocalFree(old) };

    // An old descriptor with nothing in it, and one with a NULL DACL.
    let old = from_sddl("");
    dump_sd(t, "empty old", old);
    build(t, "empty old, nothing new", null, null, &[], &[], old);
    build(
        t,
        "empty old, grant Everyone 0x1",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_sid(everyone))],
        &[],
        old,
    );
    // SAFETY: as above.
    unsafe { LocalFree(old) };
    let old = from_sddl("D:NO_ACCESS_CONTROL");
    dump_sd(t, "null-dacl old", old);
    build(t, "null-dacl old, nothing new", null, null, &[], &[], old);
    build(
        t,
        "null-dacl old, grant Everyone 0x1",
        null,
        null,
        &[entry(GRANT_ACCESS, 1, 0, trustee_sid(everyone))],
        &[],
        old,
    );
    // SAFETY: as above.
    unsafe { LocalFree(old) };
}
