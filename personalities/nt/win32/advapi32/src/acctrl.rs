//! Access-control helpers of `advapi32` (ADR 0171): the trustee builders,
//! `SetEntriesInAclW` / `GetExplicitEntriesFromAclW` (`accctrl.h`
//! layouts, x64) and the account lookups. The rules are
//! `poly_security::{explicit, account}`'s.

use super::*;
use alloc::string::String;
use poly_security::account;
use poly_security::explicit::{self, Entry, Trustee};

const TRUSTEE_IS_SID: u32 = 0;
const TRUSTEE_IS_NAME: u32 = 1;
const ERROR_NONE_MAPPED: u32 = 1332;

unsafe fn put_u32(p: *mut u8, at: usize, v: u32) {
    // SAFETY: the caller's structure.
    unsafe { (p.add(at) as *mut u32).write_unaligned(v) }
}

unsafe fn put_u64(p: *mut u8, at: usize, v: u64) {
    // SAFETY: the caller's structure.
    unsafe { (p.add(at) as *mut u64).write_unaligned(v) }
}

unsafe fn get_u32(p: *const u8, at: usize) -> u32 {
    // SAFETY: the caller's structure.
    unsafe { (p.add(at) as *const u32).read_unaligned() }
}

unsafe fn get_u64(p: *const u8, at: usize) -> u64 {
    // SAFETY: the caller's structure.
    unsafe { (p.add(at) as *const u64).read_unaligned() }
}

/// Fill the fields of a `TRUSTEE_W` that the builders write (not the two
/// padding words): multiple trustee NULL / operation 0, `form`, type 0, the
/// name or SID pointer.
unsafe fn build_trustee(t: *mut u8, form: u32, name: u64) {
    // SAFETY: the caller's 32-byte structure.
    unsafe {
        put_u64(t, 0, 0);
        put_u32(t, 8, 0);
        put_u32(t, 12, form);
        put_u32(t, 16, 0);
        put_u64(t, 24, name);
    }
}

/// `BuildTrusteeWithNameW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BuildTrusteeWithNameW(trustee: *mut u8, name: *mut u16) {
    // SAFETY: the caller's structure.
    unsafe { build_trustee(trustee, TRUSTEE_IS_NAME, name as u64) }
}

/// `BuildTrusteeWithSidW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BuildTrusteeWithSidW(trustee: *mut u8, sid: *mut u8) {
    // SAFETY: the caller's structure.
    unsafe { build_trustee(trustee, TRUSTEE_IS_SID, sid as u64) }
}

/// `BuildExplicitAccessWithNameW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn BuildExplicitAccessWithNameW(
    ea: *mut u8,
    name: *mut u16,
    access: u32,
    mode: u32,
    inheritance: u32,
) {
    // SAFETY: the caller's 48-byte structure.
    unsafe {
        put_u32(ea, 0, access);
        put_u32(ea, 4, mode);
        put_u32(ea, 8, inheritance);
        build_trustee(ea.add(16), TRUSTEE_IS_NAME, name as u64);
    }
}

/// The entry at `p` (`EXPLICIT_ACCESS_W`, 48 bytes).
unsafe fn read_entry(p: *const u8) -> Entry {
    // SAFETY: the caller's array.
    unsafe {
        let form = get_u32(p, 28);
        let ptr = get_u64(p, 40) as *const u8;
        let trustee = match form {
            TRUSTEE_IS_SID if !ptr.is_null() => Trustee::Sid(sid_bytes(ptr).to_vec()),
            TRUSTEE_IS_NAME => match wide(ptr as *const u16) {
                Some(w) => Trustee::Name(String::from_utf16_lossy(w)),
                None => Trustee::Bad,
            },
            _ => Trustee::Bad,
        };
        Entry { access: get_u32(p, 0), mode: get_u32(p, 4), inherit: get_u32(p, 8), trustee }
    }
}

/// `SetEntriesInAclW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetEntriesInAclW(
    count: u32,
    entries: *const u8,
    old_acl: *const u8,
    new_acl: *mut *mut u8,
) -> u32 {
    if new_acl.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's out pointer, array and ACL.
    unsafe {
        new_acl.write(core::ptr::null_mut());
        if count != 0 && entries.is_null() {
            return ERROR_INVALID_PARAMETER;
        }
        let list: Vec<Entry> = (0..count as usize).map(|i| read_entry(entries.add(48 * i))).collect();
        let old = (!old_acl.is_null()).then(|| {
            let size = usize::from((old_acl.add(2) as *const u16).read_unaligned());
            core::slice::from_raw_parts(old_acl, size.max(8))
        });
        match explicit::set_entries(&list, old) {
            Err(e) => e,
            Ok(None) => 0,
            Ok(Some(acl)) => match local_copy(&acl) {
                Some(p) => {
                    new_acl.write(p);
                    0
                }
                None => ERROR_NOT_ENOUGH_MEMORY,
            },
        }
    }
}

/// `GetExplicitEntriesFromAclW`: the entries and the SIDs they point at
/// share one `LocalAlloc` block.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetExplicitEntriesFromAclW(
    acl: *const u8,
    count: *mut u32,
    entries: *mut *mut u8,
) -> u32 {
    if count.is_null() || entries.is_null() {
        return ERROR_INVALID_PARAMETER;
    }
    // SAFETY: the caller's out pointers and ACL.
    unsafe {
        count.write(0);
        entries.write(core::ptr::null_mut());
        if acl.is_null() {
            return 0;
        }
        let size = usize::from((acl.add(2) as *const u16).read_unaligned());
        let list = match explicit::explicit_entries(core::slice::from_raw_parts(acl, size.max(8))) {
            Ok(l) => l,
            Err(e) => return e,
        };
        if list.is_empty() {
            return 0;
        }
        let head = 48 * list.len();
        let total = head + list.iter().map(|e| e.sid.len()).sum::<usize>();
        let block = LocalAlloc(LMEM_FIXED, total);
        if block.is_null() {
            return ERROR_NOT_ENOUGH_MEMORY;
        }
        core::ptr::write_bytes(block, 0, total);
        let mut sid_at = head;
        for (i, e) in list.iter().enumerate() {
            let p = block.add(48 * i);
            put_u32(p, 0, e.access);
            put_u32(p, 4, e.mode);
            put_u32(p, 8, e.inherit);
            build_trustee(p.add(16), TRUSTEE_IS_SID, block.add(sid_at) as u64);
            core::ptr::copy_nonoverlapping(e.sid.as_ptr(), block.add(sid_at), e.sid.len());
            sid_at += e.sid.len();
        }
        count.write(list.len() as u32);
        entries.write(block);
        0
    }
}

/// Copy `text` into `buf` (`cch` units, NUL added) when both the name and
/// the domain fit, else set both sizes to what they need and fail with
/// `ERROR_INSUFFICIENT_BUFFER`. The sizes on success are the lengths
/// without the NUL.
unsafe fn put_names(name: &[u16], buf: *mut u16, cch: *mut u32, domain: &[u16], dbuf: *mut u16, dcch: *mut u32) -> i32 {
    // SAFETY: the caller's buffers and size variables.
    unsafe {
        if *cch as usize <= name.len() || *dcch as usize <= domain.len() {
            *cch = name.len() as u32 + 1;
            *dcch = domain.len() as u32 + 1;
            return fail(ERROR_INSUFFICIENT_BUFFER);
        }
        for (b, t) in [(buf, name), (dbuf, domain)] {
            if !b.is_null() {
                core::ptr::copy_nonoverlapping(t.as_ptr(), b, t.len());
                b.add(t.len()).write(0);
            }
        }
        *cch = name.len() as u32;
        *dcch = domain.len() as u32;
    }
    TRUE
}

/// `LookupAccountSidW` (the local machine only).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LookupAccountSidW(
    _system: *const u16,
    s: *const u8,
    name: *mut u16,
    cch_name: *mut u32,
    domain: *mut u16,
    cch_domain: *mut u32,
    kind: *mut u32,
) -> i32 {
    if s.is_null() || cch_name.is_null() || cch_domain.is_null() {
        return fail(ERROR_INVALID_PARAMETER);
    }
    // SAFETY: the caller's SID, buffers and variables.
    unsafe {
        let Some(sid) = sid::Sid::from_bytes(sid_bytes(s)) else { return fail(ERROR_INVALID_PARAMETER) };
        let Some(a) = account::lookup_sid(&sid) else { return fail(ERROR_NONE_MAPPED) };
        let n: Vec<u16> = a.name.encode_utf16().collect();
        let d: Vec<u16> = a.domain.encode_utf16().collect();
        if put_names(&n, name, cch_name, &d, domain, cch_domain) == FALSE {
            return FALSE;
        }
        if !kind.is_null() {
            kind.write(a.kind);
        }
    }
    TRUE
}

/// `LookupAccountNameW` (the local machine only).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LookupAccountNameW(
    _system: *const u16,
    account_name: *const u16,
    sid_out: *mut u8,
    cb_sid: *mut u32,
    domain: *mut u16,
    cch_domain: *mut u32,
    kind: *mut u32,
) -> i32 {
    // SAFETY: the caller's name, buffers and variables.
    unsafe {
        let Some(name) = wide(account_name) else { return fail(ERROR_INVALID_PARAMETER) };
        if cb_sid.is_null() || cch_domain.is_null() {
            return fail(ERROR_INVALID_PARAMETER);
        }
        let Some((s, d, use_)) = account::lookup_name(&String::from_utf16_lossy(name)) else {
            return fail(ERROR_NONE_MAPPED);
        };
        let sb = s.to_bytes();
        let dw: Vec<u16> = d.encode_utf16().collect();
        if (*cb_sid as usize) < sb.len() || *cch_domain as usize <= dw.len() {
            *cb_sid = sb.len() as u32;
            *cch_domain = dw.len() as u32 + 1;
            return fail(ERROR_INSUFFICIENT_BUFFER);
        }
        if !sid_out.is_null() {
            core::ptr::copy_nonoverlapping(sb.as_ptr(), sid_out, sb.len());
        }
        if !domain.is_null() {
            core::ptr::copy_nonoverlapping(dw.as_ptr(), domain, dw.len());
            domain.add(dw.len()).write(0);
        }
        *cch_domain = dw.len() as u32;
        if !kind.is_null() {
            kind.write(use_);
        }
    }
    TRUE
}

// ----- security by name -------------------------------------------------------------------

#[link(name = "kernelbase", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetLastError() -> u32;
    fn RegOpenKeyExW(hkey: Handle, sub: *const u16, options: u32, sam: u32, out: *mut Handle) -> u32;
}

const SE_FILE_OBJECT: u32 = 1;
const SE_REGISTRY_KEY: u32 = 4;
const SE_KERNEL_OBJECT: u32 = 6;
const READ_CONTROL: u32 = 0x20000;
const WRITE_DAC: u32 = 0x40000;
const WRITE_OWNER: u32 = 0x80000;
const ACCESS_SYSTEM_SECURITY: u32 = 0x100_0000;
const ERROR_ACCESS_DENIED: u32 = 5;
const ERROR_INVALID_NAME: u32 = 123;
const ERROR_FILE_NOT_FOUND: u32 = 2;

/// The predefined key a `GetNamedSecurityInfoW` root name stands for
/// (`MACHINE`, `USERS`, `CURRENT_USER`, `CLASSES_ROOT`).
fn registry_root(name: &str) -> Option<Handle> {
    Some(match name.to_ascii_uppercase().as_str() {
        "CLASSES_ROOT" => 0x8000_0000u32 as i32 as isize,
        "CURRENT_USER" => 0x8000_0001u32 as i32 as isize,
        "MACHINE" => 0x8000_0002u32 as i32 as isize,
        "USERS" => 0x8000_0003u32 as i32 as isize,
        _ => return None,
    })
}

/// Open the object `name` of `object_type` with `access`: a file or
/// directory (`CreateFileW` with backup semantics) or a registry key
/// (`ROOT\subkey`).
unsafe fn open_named(name: *const u16, object_type: u32, access: u32) -> Result<Handle, u32> {
    // SAFETY: the caller's string.
    unsafe {
        let Some(w) = wide(name) else { return Err(ERROR_INVALID_PARAMETER) };
        match object_type {
            SE_FILE_OBJECT => {
                if w.is_empty() {
                    return Err(ERROR_INVALID_NAME);
                }
                let h = CreateFileW(name, access, 7, core::ptr::null(), 3, 0x0200_0000, 0);
                if h == -1 { Err(GetLastError()) } else { Ok(h) }
            }
            SE_REGISTRY_KEY => {
                let text = String::from_utf16_lossy(w);
                let (root, rest) = match text.split_once('\\') {
                    Some((r, s)) => (r, s),
                    None => (text.as_str(), ""),
                };
                let Some(hroot) = registry_root(root) else { return Err(ERROR_INVALID_PARAMETER) };
                let sub: Vec<u16> = rest.encode_utf16().chain([0]).collect();
                let mut out: Handle = 0;
                match RegOpenKeyExW(hroot, sub.as_ptr(), 0, access, &mut out) {
                    0 => Ok(out),
                    e => Err(e),
                }
            }
            SE_KERNEL_OBJECT => {
                if w.is_empty() {
                    Err(ERROR_INVALID_PARAMETER)
                } else {
                    Err(ERROR_FILE_NOT_FOUND)
                }
            }
            _ => Err(ERROR_INVALID_PARAMETER),
        }
    }
}

/// `GetNamedSecurityInfoW`: open the object for reading, then
/// `GetSecurityInfo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetNamedSecurityInfoW(
    name: *const u16,
    object_type: u32,
    info: u32,
    owner: *mut *mut u8,
    group: *mut *mut u8,
    dacl: *mut *mut u8,
    sacl: *mut *mut u8,
    descriptor: *mut *mut u8,
) -> u32 {
    let access = READ_CONTROL | if info & 8 != 0 { ACCESS_SYSTEM_SECURITY } else { 0 };
    // SAFETY: the caller's arguments.
    unsafe {
        let rc = match open_named(name, object_type, access) {
            Ok(h) => {
                let rc = GetSecurityInfo(h, object_type, info, owner, group, dacl, sacl, descriptor);
                CloseHandle(h);
                return rc;
            }
            Err(e) => e,
        };
        // A failure nulls the outputs that were asked for and the descriptor.
        for (w, p) in [(info & 1 != 0, owner), (info & 2 != 0, group), (info & 4 != 0, dacl), (info & 0x1F8 != 0, sacl)]
        {
            if w && !p.is_null() {
                p.write(core::ptr::null_mut());
            }
        }
        if !descriptor.is_null() {
            descriptor.write(core::ptr::null_mut());
        }
        rc
    }
}

/// `SetNamedSecurityInfoW`: open the object for the rights the information
/// needs, then `SetSecurityInfo` (the stored descriptor is by path, so
/// children see the inherited ACEs, ADR 0171).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SetNamedSecurityInfoW(
    name: *mut u16,
    object_type: u32,
    info: u32,
    owner: *mut u8,
    group: *mut u8,
    dacl: *mut u8,
    sacl: *mut u8,
) -> u32 {
    let mut access = 0;
    if info & (1 | 2 | 0x10) != 0 {
        access |= WRITE_OWNER;
    }
    if info & 4 != 0 {
        access |= WRITE_DAC;
    }
    if info & 8 != 0 {
        access |= ACCESS_SYSTEM_SECURITY;
    }
    // SAFETY: the caller's arguments.
    unsafe {
        let h = match open_named(name, object_type, access.max(READ_CONTROL)) {
            Ok(h) => h,
            Err(e) => return e,
        };
        let rc = if info == 0 {
            ERROR_ACCESS_DENIED
        } else {
            SetSecurityInfo(h, object_type, info, owner, group, dacl, sacl)
        };
        CloseHandle(h);
        rc
    }
}
