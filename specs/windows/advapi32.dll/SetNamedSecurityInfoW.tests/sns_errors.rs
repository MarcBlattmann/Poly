//! SetNamedSecurityInfoW errors and special arguments: missing file, null and invalid names,
//! information bits with the matching pointer null (owner, group, DACL: a null DACL grants everyone),
//! an owner the caller may not assign, unknown object type, an invalid ACL, and an unknown bit.
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

const SE_FILE_OBJECT: u32 = 1;
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
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLengthSid(sid: *const c_void) -> u32;
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

fn test(t: &mut Test) {
    let file = c2_beside("c2sne.txt");
    // SAFETY: NUL-terminated path.
    unsafe {
        let h = CreateFileW(file.as_ptr(), 0x1000_0000, 0, core::ptr::null_mut(), 2, 0x80, core::ptr::null_mut());
        CloseHandle(h);
    }
    let mut acl = AclBuf::new();
    let system = sid_of("S-1-5-18");
    // SAFETY: a 1024-byte buffer.
    unsafe {
        InitializeAcl(acl.p(), 1024, 2);
        AddAccessAllowedAceEx(acl.p(), 2, 0, 0x1F01FF, system.as_const());
        let admins = sid_of("S-1-5-32-544");
        AddAccessAllowedAceEx(acl.p(), 2, 0, 0x1F01FF, admins.as_const());
    }
    let run = |t: &mut Test,
               label: &str,
               name: *const u16,
               kind: u32,
               info: u32,
               owner: PSID,
               group: PSID,
               dacl: PACL,
               sacl: PACL| {
        // SAFETY: the caller passes a NUL-terminated name (or null), SIDs and ACLs that are valid or null.
        let r = unsafe { SetNamedSecurityInfoW(name, kind, info, owner, group, dacl, sacl) };
        say!(t, "{label}: result {r}");
    };
    let nul = core::ptr::null_mut();
    let missing = c2_beside("c2sne-missing.txt");
    run(t, "missing file", missing.as_ptr(), SE_FILE_OBJECT, 4, nul, nul, acl.p(), nul);
    let empty = wide("");
    run(t, "empty name", empty.as_ptr(), SE_FILE_OBJECT, 4, nul, nul, acl.p(), nul);
    run(t, "object type 0", file.as_ptr(), 0, 4, nul, nul, acl.p(), nul);
    run(t, "object type 99", file.as_ptr(), 99, 4, nul, nul, acl.p(), nul);
    run(t, "info 0", file.as_ptr(), SE_FILE_OBJECT, 0, nul, nul, nul, nul);
    run(
        t,
        "DACL requested, DACL null (everyone full access)",
        file.as_ptr(),
        SE_FILE_OBJECT,
        4 | PROTECTED_DACL_SECURITY_INFORMATION,
        nul,
        nul,
        nul,
        nul,
    );
    let mut dacl: PACL = core::ptr::null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = core::ptr::null_mut();
    // SAFETY: valid path and out pointers.
    unsafe {
        GetNamedSecurityInfoW(file.as_ptr(), SE_FILE_OBJECT, 4, nul.cast(), nul.cast(), &mut dacl, nul.cast(), &mut sd)
    };
    say!(t, "after the null DACL: dacl pointer null={}", dacl.is_null());
    // SAFETY: freeing the descriptor.
    unsafe { LocalFree(sd) };
    run(
        t,
        "owner requested, owner null",
        file.as_ptr(),
        SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION,
        nul,
        nul,
        nul,
        nul,
    );
    run(
        t,
        "group requested, group null",
        file.as_ptr(),
        SE_FILE_OBJECT,
        GROUP_SECURITY_INFORMATION,
        nul,
        nul,
        nul,
        nul,
    );
    let mut everyone = sid_of("S-1-1-0");
    run(
        t,
        "owner Everyone (ambient: administrator with SeRestore/SeTakeOwnership)",
        file.as_ptr(),
        SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION,
        everyone.as_psid(),
        nul,
        nul,
        nul,
    );
    let mut bad = AclBuf::new();
    bad.b[0] = 9;
    run(t, "invalid ACL (revision 9)", file.as_ptr(), SE_FILE_OBJECT, 4, nul, nul, bad.p(), nul);
    run(t, "unknown information bit 0x100 only", file.as_ptr(), SE_FILE_OBJECT, 0x100, nul, nul, nul, nul);
    run(t, "DACL with an unknown bit 0x100", file.as_ptr(), SE_FILE_OBJECT, 4 | 0x100, nul, nul, acl.p(), nul);
    // SAFETY: cleaning up.
    unsafe { DeleteFileW(file.as_ptr()) };
}
