//! The updater's sequence: a key made by RegCreateKeyExW (KEY_WRITE, or less), GetSecurityInfo on the handle, a rebuilt DACL, SetSecurityInfo on the SAME handle.
//! As the key's owner (also when the key's DACL gives the owner nothing: the owner's implied rights) and as a caller that is not the owner.
// poly-arch: x86
#![no_std]
#![no_main]
#![allow(dead_code, non_camel_case_types, static_mut_refs, clippy::all)]

// ---- shared helpers of the registry-key security tests (this kit's own code) ----
use core::ffi::c_void;
use core::fmt::Write as _;
use poly_wtest::*;

type LONG = i32;
type HKEY = usize;
type PSID = *mut c_void;
type PACL = *mut c_void;
type PSD = *mut c_void;

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
    fn push(&mut self, c: u8) {
        if self.n < N {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
}

impl<const N: usize> core::fmt::Write for Buf<N> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for &c in s.as_bytes() {
            self.push(c);
        }
        Ok(())
    }
}

macro_rules! say {
    ($t:expr, $($a:tt)*) => {{
        let mut b = Buf::<590>::new();
        let _ = write!(b, $($a)*);
        $t.line(b.as_str());
    }};
}

const HKEY_CLASSES_ROOT: HKEY = 0x8000_0000u32 as i32 as isize as usize;
const HKEY_CURRENT_USER: HKEY = 0x8000_0001u32 as i32 as isize as usize;
const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002u32 as i32 as isize as usize;
const HKEY_USERS: HKEY = 0x8000_0003u32 as i32 as isize as usize;
const HKEY_CURRENT_CONFIG: HKEY = 0x8000_0005u32 as i32 as isize as usize;

const DELETE: u32 = 0x0001_0000;
const READ_CONTROL: u32 = 0x0002_0000;
const WRITE_DAC: u32 = 0x0004_0000;
const WRITE_OWNER: u32 = 0x0008_0000;
const SYNCHRONIZE: u32 = 0x0010_0000;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_SET_VALUE: u32 = 0x0002;
const KEY_CREATE_SUB_KEY: u32 = 0x0004;
const KEY_ENUMERATE_SUB_KEYS: u32 = 0x0008;
const KEY_READ: u32 = 0x0002_0019;
const KEY_WRITE: u32 = 0x0002_0006;
const KEY_ALL_ACCESS: u32 = 0x000F_003F;
const KEY_WOW64_64KEY: u32 = 0x0100;
const KEY_WOW64_32KEY: u32 = 0x0200;

const SE_FILE_OBJECT: u32 = 1;
const SE_SERVICE: u32 = 2;
const SE_PRINTER: u32 = 3;
const SE_REGISTRY_KEY: u32 = 4;
const SE_KERNEL_OBJECT: u32 = 6;
const SE_REGISTRY_WOW64_32KEY: u32 = 9;

const OWNER_SECURITY_INFORMATION: u32 = 1;
const GROUP_SECURITY_INFORMATION: u32 = 2;
const DACL_SECURITY_INFORMATION: u32 = 4;
const SACL_SECURITY_INFORMATION: u32 = 8;
const LABEL_SECURITY_INFORMATION: u32 = 0x10;
const PROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x8000_0000;
const UNPROTECTED_DACL_SECURITY_INFORMATION: u32 = 0x2000_0000;

const ACL_REVISION: u32 = 2;
const CONTAINER_INHERIT_ACE: u32 = 0x2;
const OBJECT_INHERIT_ACE: u32 = 0x1;
const INHERIT_ONLY_ACE: u32 = 0x8;
const INHERITED_ACE: u32 = 0x10;

#[link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn RegCreateKeyExW(
        key: HKEY,
        sub: *const u16,
        reserved: u32,
        class: *const u16,
        options: u32,
        sam: u32,
        security: *const c_void,
        result: *mut HKEY,
        disposition: *mut u32,
    ) -> LONG;
    fn RegOpenKeyExW(key: HKEY, sub: *const u16, options: u32, sam: u32, result: *mut HKEY) -> LONG;
    fn RegCloseKey(key: HKEY) -> LONG;
    fn RegDeleteKeyExW(key: HKEY, sub: *const u16, sam: u32, reserved: u32) -> LONG;
    fn RegGetKeySecurity(key: HKEY, info: u32, sd: PSD, len: *mut u32) -> LONG;
    fn RegSetKeySecurity(key: HKEY, info: u32, sd: PSD) -> LONG;
    fn GetSecurityInfo(
        handle: usize,
        kind: u32,
        info: u32,
        owner: *mut PSID,
        group: *mut PSID,
        dacl: *mut PACL,
        sacl: *mut PACL,
        sd: *mut PSD,
    ) -> u32;
    fn SetSecurityInfo(handle: usize, kind: u32, info: u32, owner: PSID, group: PSID, dacl: PACL, sacl: PACL) -> u32;
    fn GetNamedSecurityInfoW(
        name: *const u16,
        kind: u32,
        info: u32,
        owner: *mut PSID,
        group: *mut PSID,
        dacl: *mut PACL,
        sacl: *mut PACL,
        sd: *mut PSD,
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
    fn GetSecurityDescriptorControl(sd: PSD, control: *mut u16, rev: *mut u32) -> i32;
    fn GetSecurityDescriptorOwner(sd: PSD, owner: *mut PSID, defaulted: *mut i32) -> i32;
    fn GetSecurityDescriptorGroup(sd: PSD, group: *mut PSID, defaulted: *mut i32) -> i32;
    fn GetSecurityDescriptorDacl(sd: PSD, present: *mut i32, dacl: *mut PACL, defaulted: *mut i32) -> i32;
    fn GetSecurityDescriptorLength(sd: PSD) -> u32;
    fn IsValidSecurityDescriptor(sd: PSD) -> i32;
    fn GetAclInformation(acl: PACL, info: *mut c_void, len: u32, class: u32) -> i32;
    fn GetAce(acl: PACL, index: u32, ace: *mut *mut u8) -> i32;
    fn InitializeAcl(acl: PACL, len: u32, rev: u32) -> i32;
    fn AddAce(acl: PACL, rev: u32, start: u32, ace: *const c_void, len: u32) -> i32;
    fn AddAccessAllowedAceEx(acl: PACL, rev: u32, flags: u32, mask: u32, sid: PSID) -> i32;
    fn AddAccessDeniedAceEx(acl: PACL, rev: u32, flags: u32, mask: u32, sid: PSID) -> i32;
    fn ConvertSidToStringSidW(sid: PSID, out: *mut *mut u16) -> i32;
    fn ConvertStringSidToSidW(text: *const u16, sid: *mut PSID) -> i32;
    fn GetLengthSid(sid: PSID) -> u32;
    fn InitializeSecurityDescriptor(sd: PSD, rev: u32) -> i32;
    fn SetSecurityDescriptorDacl(sd: PSD, present: i32, dacl: PACL, defaulted: i32) -> i32;
    fn SetSecurityDescriptorControl(sd: PSD, mask: u16, set: u16) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn LocalFree(p: *mut c_void) -> *mut c_void;
    fn CreateEventW(sa: *const c_void, manual: i32, initial: i32, name: *const u16) -> usize;
    fn CloseHandle(h: usize) -> i32;
    fn SetLastError(e: u32);
    fn GetLastError() -> u32;
}

/// A NUL-ended UTF-16 copy of an ASCII text.
struct W {
    b: [u16; 300],
}

fn w(s: &str) -> W {
    let mut x = W { b: [0; 300] };
    for (i, c) in s.bytes().enumerate() {
        x.b[i] = u16::from(c);
    }
    x
}

/// The key under HKLM\SOFTWARE that every program of this kit makes and removes.
const BASE: &str = "SOFTWARE\\PolyKeySec_B7";

fn create(root: HKEY, path: &str, sam: u32) -> (LONG, HKEY) {
    let p = w(path);
    let mut h: HKEY = 0;
    let mut d = 0u32;
    // SAFETY: a call into advapi32 with NUL-ended text and output slots of this test.
    let r =
        unsafe { RegCreateKeyExW(root, p.b.as_ptr(), 0, core::ptr::null(), 0, sam, core::ptr::null(), &mut h, &mut d) };
    (r, h)
}

fn open(root: HKEY, path: &str, sam: u32) -> (LONG, HKEY) {
    let p = w(path);
    let mut h: HKEY = 0;
    // SAFETY: a call into advapi32 with NUL-ended text and an output slot of this test.
    let r = unsafe { RegOpenKeyExW(root, p.b.as_ptr(), 0, sam, &mut h) };
    (r, h)
}

fn close(h: HKEY) {
    // SAFETY: closes a key handle of this test.
    unsafe { RegCloseKey(h) };
}

fn delete(root: HKEY, path: &str, sam: u32) -> LONG {
    let p = w(path);
    // SAFETY: a call into advapi32 with NUL-ended text.
    unsafe { RegDeleteKeyExW(root, p.b.as_ptr(), sam, 0) }
}

/// Gives the key a NULL DACL (everyone: full access) so that it can be deleted again; the owner may always do this. No output.
fn reset_dacl(root: HKEY, path: &str, sam: u32) {
    let (r, h) = open(root, path, WRITE_DAC | sam);
    if r != 0 {
        return;
    }
    let mut sd = [0u64; 8];
    // SAFETY: a descriptor buffer of this test.
    unsafe {
        InitializeSecurityDescriptor(sd.as_mut_ptr() as PSD, 1);
        SetSecurityDescriptorDacl(sd.as_mut_ptr() as PSD, 1, core::ptr::null_mut(), 0);
        RegSetKeySecurity(h, DACL_SECURITY_INFORMATION, sd.as_mut_ptr() as PSD);
    }
    close(h);
}

/// Removes `path` below HKLM and its (up to three) levels of test-made children, in both views; no output.
fn clean_all() {
    for sam in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
        for tail in ["\\A\\B\\C", "\\A\\B", "\\A\\D", "\\A", "\\B", "\\C", "\\D", ""] {
            let mut p = Buf::<300>::new();
            let _ = write!(p, "{}{}", BASE, tail);
            reset_dacl(HKEY_LOCAL_MACHINE, p.as_str(), sam);
            delete(HKEY_LOCAL_MACHINE, p.as_str(), sam);
        }
    }
}

/// The text form of a SID; well-known SIDs as `S-1-...`, domain/machine SIDs with the machine part and big RIDs hidden.
fn sid_text(sid: PSID) -> Buf<160> {
    let mut out = Buf::<160>::new();
    if sid.is_null() {
        let _ = write!(out, "NULL");
        return out;
    }
    let mut s: *mut u16 = core::ptr::null_mut();
    // SAFETY: a SID pointer into a descriptor of this test and an output slot.
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut s) };
    if ok == 0 || s.is_null() {
        let _ = write!(out, "<bad sid>");
        return out;
    }
    let mut text = Buf::<160>::new();
    let mut i = 0;
    // SAFETY: the returned text is NUL-ended and lives until LocalFree.
    unsafe {
        while *s.add(i) != 0 && i < 150 {
            text.push(*s.add(i) as u8);
            i += 1;
        }
        LocalFree(s as *mut c_void);
    }
    let t = text.as_str();
    if let Some(rest) = t.strip_prefix("S-1-5-21-") {
        // S-1-5-21-a-b-c-RID: keep a RID below 1000 (a built-in one), hide the rest
        let rid = rest.rsplit('-').next().unwrap_or("");
        let n: u32 = rid.parse().unwrap_or(99999);
        if n < 1000 {
            let _ = write!(out, "S-1-5-21-<machine>-{}", n);
        } else {
            let _ = write!(out, "S-1-5-21-<machine>-<account>");
        }
    } else {
        let _ = write!(out, "{}", t);
    }
    out
}

/// Prints the ACL: absent, or its ACEs (type, flags, mask, SID).
fn show_acl(t: &mut Test, label: &str, acl: PACL) {
    if acl.is_null() {
        say!(t, "{}: NULL", label);
        return;
    }
    let mut info = [0u32; 3];
    // SAFETY: an ACL pointer into a descriptor of this test; ACL_SIZE_INFORMATION is three DWORDs.
    let ok = unsafe { GetAclInformation(acl, info.as_mut_ptr() as *mut c_void, 12, 2) };
    if ok == 0 {
        say!(t, "{}: GetAclInformation failed {}", label, unsafe { GetLastError() });
        return;
    }
    let mut rev = [0u32; 1];
    // SAFETY: as above; ACL_REVISION_INFORMATION is one DWORD.
    unsafe { GetAclInformation(acl, rev.as_mut_ptr() as *mut c_void, 4, 1) };
    say!(t, "{}: revision {}, {} ACEs", label, rev[0], info[0]);
    for i in 0..info[0] {
        let mut ace: *mut u8 = core::ptr::null_mut();
        // SAFETY: an index below the ACE count and an output slot.
        let ok = unsafe { GetAce(acl, i, &mut ace) };
        if ok == 0 || ace.is_null() {
            say!(t, "  ace {}: GetAce failed", i);
            continue;
        }
        // SAFETY: an ACCESS_ALLOWED/DENIED ACE header, mask and SID follow it (winnt.h).
        let (ty, fl, mask, sid) = unsafe {
            let ty = *ace;
            let fl = *ace.add(1);
            let mask = core::ptr::read_unaligned(ace.add(4) as *const u32);
            (ty, fl, mask, ace.add(8) as PSID)
        };
        let s = sid_text(sid);
        say!(t, "  ace {}: type {} flags {:#04x} mask {:#010x} sid {}", i, ty, fl, mask, s.as_str());
    }
}

/// Prints the parts of a self-relative descriptor that `info` asked for (control bits always).
fn show_sd(t: &mut Test, label: &str, sd: PSD, info: u32) {
    if sd.is_null() {
        say!(t, "{}: no descriptor", label);
        return;
    }
    let mut ctl = 0u16;
    let mut rev = 0u32;
    // SAFETY: a descriptor pointer returned by the system and output slots.
    unsafe { GetSecurityDescriptorControl(sd, &mut ctl, &mut rev) };
    say!(t, "{}: revision {} control {:#06x}", label, rev, ctl);
    let mut owner: PSID = core::ptr::null_mut();
    let mut group: PSID = core::ptr::null_mut();
    let mut dacl: PACL = core::ptr::null_mut();
    let (mut d1, mut d2, mut d3, mut present) = (0i32, 0i32, 0i32, 0i32);
    // SAFETY: as above.
    unsafe {
        GetSecurityDescriptorOwner(sd, &mut owner, &mut d1);
        GetSecurityDescriptorGroup(sd, &mut group, &mut d2);
        GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut d3);
    }
    if info & OWNER_SECURITY_INFORMATION != 0 {
        let o = sid_text(owner);
        say!(t, "  owner {} (defaulted {})", o.as_str(), d1);
    }
    if info & GROUP_SECURITY_INFORMATION != 0 {
        let g = sid_text(group);
        say!(t, "  group {} (defaulted {})", g.as_str(), d2);
    }
    if info & DACL_SECURITY_INFORMATION != 0 {
        say!(t, "  dacl present {} (defaulted {})", present, d3);
        if present != 0 {
            show_acl(t, "  dacl", dacl);
        }
    }
}

/// A buffer for a descriptor of the key calls (the size is asked first).
struct SdBuf {
    b: [u64; 128],
}

fn sd_buf() -> SdBuf {
    SdBuf { b: [0; 128] }
}

/// `RegGetKeySecurity` into `buf`: (status, needed length).
fn reg_get_sd(h: HKEY, info: u32, buf: &mut SdBuf) -> (LONG, u32) {
    let mut len = 1024u32;
    // SAFETY: a 1024-byte buffer and its length.
    let r = unsafe { RegGetKeySecurity(h, info, buf.b.as_mut_ptr() as PSD, &mut len) };
    (r, len)
}

/// `GetSecurityInfo` of a handle: the status, the DACL pointer and the descriptor (to LocalFree).
fn get_info(h: usize, kind: u32, info: u32) -> (u32, PACL, PSD) {
    let mut dacl: PACL = core::ptr::null_mut();
    let mut sd: PSD = core::ptr::null_mut();
    let mut owner: PSID = core::ptr::null_mut();
    let mut group: PSID = core::ptr::null_mut();
    let po = if info & OWNER_SECURITY_INFORMATION != 0 { &mut owner as *mut PSID } else { core::ptr::null_mut() };
    let pg = if info & GROUP_SECURITY_INFORMATION != 0 { &mut group as *mut PSID } else { core::ptr::null_mut() };
    let pd = if info & DACL_SECURITY_INFORMATION != 0 { &mut dacl as *mut PACL } else { core::ptr::null_mut() };
    // SAFETY: output slots of this test for each requested part.
    let r = unsafe { GetSecurityInfo(h, kind, info, po, pg, pd, core::ptr::null_mut(), &mut sd) };
    (r, dacl, sd)
}

fn free(p: *mut c_void) {
    if !p.is_null() {
        // SAFETY: a LocalAlloc'ed block returned by the system.
        unsafe { LocalFree(p) };
    }
}

fn sid_of(text: &str) -> PSID {
    let t = w(text);
    let mut sid: PSID = core::ptr::null_mut();
    // SAFETY: NUL-ended text and an output slot; the SID is LocalAlloc'ed and leaked by the test (it is a few bytes).
    unsafe { ConvertStringSidToSidW(t.b.as_ptr(), &mut sid) };
    sid
}

/// A DACL (in `buf`) made of the ACEs of `old` followed by one allowed ACE: mask, flags and SID text.
fn dacl_plus(buf: &mut SdBuf, old: PACL, flags: u32, mask: u32, sid: PSID) -> PACL {
    let acl = buf.b.as_mut_ptr() as PACL;
    // SAFETY: a 1024-byte buffer; the old ACL comes from the system; each ACE is copied whole.
    unsafe {
        InitializeAcl(acl, 1024, ACL_REVISION);
        if !old.is_null() {
            let mut info = [0u32; 3];
            GetAclInformation(old, info.as_mut_ptr() as *mut c_void, 12, 2);
            for i in 0..info[0] {
                let mut ace: *mut u8 = core::ptr::null_mut();
                GetAce(old, i, &mut ace);
                let size = core::ptr::read_unaligned(ace.add(2) as *const u16) as u32;
                AddAce(acl, ACL_REVISION, 0xFFFF_FFFF, ace as *const c_void, size);
            }
        }
        AddAccessAllowedAceEx(acl, ACL_REVISION, flags, mask, sid);
    }
    acl
}

/// The number of ACEs of an ACL (0 for NULL).
fn ace_count(acl: PACL) -> u32 {
    if acl.is_null() {
        return 0;
    }
    let mut info = [0u32; 3];
    // SAFETY: an ACL from the system; ACL_SIZE_INFORMATION is three DWORDs.
    unsafe { GetAclInformation(acl, info.as_mut_ptr() as *mut c_void, 12, 2) };
    info[0]
}

/// The flags of ACE `i`.
fn ace_flags(acl: PACL, i: u32) -> u8 {
    let mut ace: *mut u8 = core::ptr::null_mut();
    // SAFETY: an index below the ACE count.
    unsafe {
        GetAce(acl, i, &mut ace);
        *ace.add(1)
    }
}

/// Opens `path` below `root` (READ_CONTROL), prints the asked parts of its descriptor.
fn show_key(t: &mut Test, label: &str, root: HKEY, path: &str, info: u32) {
    let (r, h) = open(root, path, READ_CONTROL);
    if r != 0 {
        say!(t, "{}: open -> {}", label, r);
        return;
    }
    let (r, _d, sd) = get_info(h, SE_REGISTRY_KEY, info);
    say!(t, "{}: GetSecurityInfo -> {}", label, r);
    if r == 0 {
        show_sd(t, "  sd", sd, info);
    }
    free(sd);
    close(h);
}

#[link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn OpenProcessToken(process: usize, access: u32, token: *mut usize) -> i32;
    fn LookupPrivilegeValueW(system: *const u16, name: *const u16, luid: *mut u64) -> i32;
    fn AdjustTokenPrivileges(
        token: usize,
        disable_all: i32,
        new: *const u8,
        len: u32,
        old: *mut u8,
        ret: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn GetCurrentProcess() -> usize;
}

#[link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated")]
unsafe extern "system" {
    fn NtQueryObject(handle: usize, class: u32, info: *mut u8, len: u32, ret: *mut u32) -> i32;
}

/// Turns the named privilege on for this process; true when AdjustTokenPrivileges reports no error.
fn set_privilege(name: &str, on: bool) -> bool {
    let n = w(name);
    let mut luid = 0u64;
    let mut tok = 0usize;
    // SAFETY: output slots of this test; the token is closed below.
    unsafe {
        if OpenProcessToken(GetCurrentProcess(), 0x28, &mut tok) == 0 {
            return false;
        }
        if LookupPrivilegeValueW(core::ptr::null(), n.b.as_ptr(), &mut luid) == 0 {
            CloseHandle(tok);
            return false;
        }
        // TOKEN_PRIVILEGES with one LUID_AND_ATTRIBUTES (SE_PRIVILEGE_ENABLED = 2)
        let mut tp = [0u8; 16];
        tp[0..4].copy_from_slice(&1u32.to_le_bytes());
        tp[4..12].copy_from_slice(&luid.to_le_bytes());
        tp[12..16].copy_from_slice(&(if on { 2u32 } else { 0u32 }).to_le_bytes());
        SetLastError(0);
        let ok = AdjustTokenPrivileges(tok, 0, tp.as_ptr(), 16, core::ptr::null_mut(), core::ptr::null_mut());
        let err = GetLastError();
        CloseHandle(tok);
        ok != 0 && err == 0
    }
}

/// The access mask NtQueryObject(ObjectBasicInformation) shows for a handle (0 if the call fails).
fn granted_access(h: usize) -> u32 {
    let mut info = [0u8; 64];
    let mut ret = 0u32;
    // SAFETY: a 64-byte buffer for OBJECT_BASIC_INFORMATION.
    let st = unsafe { NtQueryObject(h, 0, info.as_mut_ptr(), 56, &mut ret) };
    if st < 0 {
        return st as u32;
    }
    u32::from_le_bytes([info[4], info[5], info[6], info[7]])
}

/// The status of opening `path` below `root` with `sam` (0 when it opens).
fn open_status(root: HKEY, path: &str, sam: u32) -> LONG {
    let (r, h) = open(root, path, sam);
    if r == 0 {
        close(h);
    }
    r
}

poly_wtest::main!(test);

const HL: HKEY = HKEY_LOCAL_MACHINE;
const A: &str = "SOFTWARE\\PolyKeySec_B7\\A";

/// GetSecurityInfo(DACL) on `h`, a DACL rebuilt from its ACEs plus one, SetSecurityInfo on the same handle; the handle's access mask before and after.
fn sequence(t: &mut Test, label: &str, h: HKEY, nbuf: &mut SdBuf) {
    let inter = sid_of("S-1-5-4");
    let before = granted_access(h);
    let (r1, dacl, sd) = get_info(h, SE_REGISTRY_KEY, DACL_SECURITY_INFORMATION);
    let n = ace_count(dacl);
    let acl = dacl_plus(nbuf, dacl, CONTAINER_INHERIT_ACE, KEY_QUERY_VALUE, inter);
    // SAFETY: an ACL of this test and a key handle of this test.
    let r2 = unsafe {
        SetSecurityInfo(
            h,
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            acl,
            core::ptr::null_mut(),
        )
    };
    let after = granted_access(h);
    free(sd);
    say!(
        t,
        "{}: handle access {:#010x}; GetSecurityInfo(DACL) -> {} ({} ACEs); SetSecurityInfo(DACL, +1 ACE) on the same handle -> {}; handle access afterwards unchanged: {}",
        label,
        before,
        r1,
        n,
        r2,
        before == after
    );
}

/// The number of ACEs of the key's DACL read through a fresh READ_CONTROL handle (999 if it cannot be opened).
fn acl_len(path: &str) -> u32 {
    let (r, h) = open(HL, path, READ_CONTROL);
    if r != 0 {
        return 999;
    }
    let (_, dacl, sd) = get_info(h, SE_REGISTRY_KEY, DACL_SECURITY_INFORMATION);
    let n = ace_count(dacl);
    free(sd);
    close(h);
    n
}

fn test(t: &mut Test) {
    clean_all();
    let mut nbuf = sd_buf();
    // 1: as the owner, with the handle rights the updater used and with fewer
    for (name, sam) in [
        ("created with KEY_WRITE", KEY_WRITE),
        ("created with KEY_SET_VALUE|KEY_CREATE_SUB_KEY (no READ_CONTROL)", KEY_SET_VALUE | KEY_CREATE_SUB_KEY),
        ("created with KEY_READ", KEY_READ),
        ("created with KEY_QUERY_VALUE", KEY_QUERY_VALUE),
    ] {
        let (r, h) = create(HL, BASE, sam);
        t.expect("the key is made", r as i64, 0);
        let before = acl_len(BASE);
        sequence(t, name, h, &mut nbuf);
        say!(t, "  the key's DACL: {} ACEs before, {} after", before, acl_len(BASE));
        close(h);
    }
    // the owner of the new key is the caller's default owner; `other` is the well-known account that is not
    let (_, hb) = open(HL, BASE, READ_CONTROL);
    let (_, _d, sd) = get_info(hb, SE_REGISTRY_KEY, OWNER_SECURITY_INFORMATION);
    let mut owner: PSID = core::ptr::null_mut();
    let mut dflt = 0i32;
    // SAFETY: a descriptor from the system.
    unsafe { GetSecurityDescriptorOwner(sd, &mut owner, &mut dflt) };
    let mine = sid_text(owner);
    let admins = mine.as_str() == "S-1-5-32-544";
    let (mine_sid, other_sid) = if admins { ("S-1-5-32-544", "S-1-5-18") } else { ("S-1-5-18", "S-1-5-32-544") };
    say!(t, "the new key's owner is {} (the caller's default owner)", mine.as_str());
    free(sd);
    close(hb);
    // 2: the owner whose DACL gives it nothing: only the other account has access; the owner keeps READ_CONTROL and WRITE_DAC
    let (_, hk) = create(HL, BASE, KEY_WRITE);
    let (_, hall) = open(HL, BASE, KEY_ALL_ACCESS);
    let other = sid_of(other_sid);
    let mut only = sd_buf();
    let acl = dacl_plus(&mut only, core::ptr::null_mut(), 0, KEY_ALL_ACCESS, other);
    // SAFETY: an ACL of this test and a key handle of this test.
    let r = unsafe {
        SetSecurityInfo(
            hall,
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            acl,
            core::ptr::null_mut(),
        )
    };
    say!(t, "SetSecurityInfo(all-access handle, protected DACL with only the other account) -> {}", r);
    close(hall);
    for (name, sam) in [
        ("KEY_WRITE", KEY_WRITE),
        ("KEY_READ", KEY_READ),
        ("READ_CONTROL", READ_CONTROL),
        ("WRITE_DAC", WRITE_DAC),
        ("READ_CONTROL|WRITE_DAC", READ_CONTROL | WRITE_DAC),
        ("WRITE_OWNER", WRITE_OWNER),
    ] {
        say!(t, "  a new open with {} -> {}", name, open_status(HL, BASE, sam));
    }
    sequence(t, "the KEY_WRITE handle opened before the DACL changed, owner not in the DACL", hk, &mut nbuf);
    say!(t, "  the key's DACL: {} ACEs", acl_len(BASE));
    close(hk);
    // the owner gives the key back its access (a NULL DACL) so that the next steps can make a subkey
    reset_dacl(HL, BASE, 0);
    // 3: a caller that is not the owner: the key is owned by `other` (set with SeRestorePrivilege), the DACL gives the caller KEY_WRITE|KEY_READ|DELETE only
    let (_, h) = create(HL, A, KEY_ALL_ACCESS);
    let privilege = set_privilege("SeRestorePrivilege", true);
    let ro = {
        let o = sid_of(other_sid);
        // SAFETY: a key handle opened with all access, a SID of this test.
        unsafe {
            SetSecurityInfo(
                h,
                SE_REGISTRY_KEY,
                OWNER_SECURITY_INFORMATION,
                o,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
            )
        }
    };
    set_privilege("SeRestorePrivilege", false);
    say!(t, "SeRestorePrivilege enabled: {}; SetSecurityInfo(owner = the other account) -> {}", privilege, ro);
    let me = sid_of(mine_sid);
    let mut two = sd_buf();
    let a0 = dacl_plus(&mut two, core::ptr::null_mut(), 0, KEY_ALL_ACCESS, sid_of(other_sid));
    let a1 = dacl_plus(&mut nbuf, a0, 0, DELETE | KEY_WRITE | KEY_READ, me);
    // SAFETY: as above; the caller is still the owner here, or has WRITE_DAC from the all-access handle.
    let r = unsafe {
        SetSecurityInfo(
            h,
            SE_REGISTRY_KEY,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            a1,
            core::ptr::null_mut(),
        )
    };
    say!(t, "SetSecurityInfo(DACL: the other account all access, the caller KEY_WRITE|KEY_READ|DELETE) -> {}", r);
    close(h);
    show_key(t, "key owned by the other account", HL, A, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION);
    for (name, sam) in [
        ("KEY_WRITE", KEY_WRITE),
        ("KEY_READ", KEY_READ),
        ("KEY_SET_VALUE|KEY_CREATE_SUB_KEY", KEY_SET_VALUE | KEY_CREATE_SUB_KEY),
        ("WRITE_DAC", WRITE_DAC),
    ] {
        let (ro, hk) = open(HL, A, sam);
        if ro != 0 {
            say!(t, "non-owner open with {} -> {}", name, ro);
            continue;
        }
        let before = acl_len(A);
        let mut nb = sd_buf();
        sequence(t, name, hk, &mut nb);
        say!(t, "  the key's DACL: {} ACEs before, {} after", before, acl_len(A));
        close(hk);
    }
    // the keys are removed by the caller (DELETE is in its ACE) or, for BASE, by its owner
    delete(HL, A, 0);
    clean_all();
}
