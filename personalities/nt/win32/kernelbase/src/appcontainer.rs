//! AppContainers (ADR 0196, the sheets of `spec-chrome-win-s2b`): the exports of `kernelbase` that
//! Chrome's broker resolves by name to make its sandbox's tokens, plus the two documented ones
//! `userenv` and `securityappcontainer.h` call for. The rules (SIDs, names, texts) are
//! `poly_security::appcontainer`; this file makes the Win32 calls: the registry mapping of a SID to
//! a name below `HKCU`, the token (`NtCreateLowBoxToken`), the object directory text.
//!
//! The signatures of `AppContainerRegisterSid`, `AppContainerLookupMoniker` and
//! `CreateAppContainerToken` are not documented; they were inferred by the spec agent from calls
//! (the sheets say so) and are those of the sheets.

use crate::base::*;
use crate::nt::Handle;
use crate::registry::{RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW};
use alloc::vec::Vec;
use poly_ntobj::NtStatus;
use poly_security::appcontainer as ac;
use poly_security::sid::Sid;

#[cfg_attr(target_arch = "x86", link(name = "ntdll", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ntdll", kind = "raw-dylib"))]
unsafe extern "system" {
    fn NtOpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> NtStatus;
    fn NtQueryInformationToken(token: Handle, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> NtStatus;
    fn NtCreateLowBoxToken(
        new: *mut Handle,
        source: Handle,
        access: u32,
        attributes: *const u8,
        package: *const u8,
        count: u32,
        capabilities: *const u8,
        handle_count: u32,
        handles: *const Handle,
    ) -> NtStatus;
    fn NtClose(handle: Handle) -> NtStatus;
}

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_READ: u32 = 0x2_0019;
const KEY_ALL_ACCESS: u32 = 0xF_003F;
const KEY_WOW64_64: u32 = 0x100;
const REG_SZ: u32 = 1;
const TOKEN_QUERY: u32 = 0x8;
const TOKEN_DUPLICATE: u32 = 0x2;
const CURRENT_PROCESS_TOKEN: Handle = -4;
const MAXIMUM_ALLOWED: u32 = 0x0200_0000;
const TOKEN_CLASS_APP_CONTAINER_SID: u32 = 31;

const ERROR_INVALID_PARAMETER: u32 = 87;
const ERROR_FILE_NOT_FOUND: u32 = 2;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NOT_APPCONTAINER: u32 = 4250;
const S_OK: i32 = 0;

/// `HRESULT_FROM_WIN32`.
fn hr(error: u32) -> i32 {
    if error == 0 { 0 } else { (0x8007_0000 | error) as i32 }
}

fn hresult(code: u32) -> i32 {
    code as i32
}

/// A NUL-terminated copy.
fn z(units: &[u16]) -> Vec<u16> {
    let mut v = units.to_vec();
    v.push(0);
    v
}

fn z_str(s: &str) -> Vec<u16> {
    z(&s.encode_utf16().collect::<Vec<u16>>())
}

/// The SID at `p` (`None` for NULL or one that is not valid).
unsafe fn sid_at(p: *const u8) -> Option<Sid> {
    if p.is_null() {
        return None;
    }
    // SAFETY: the caller's SID; `sid_arg` reads its header to know its length.
    Sid::from_bytes(unsafe { crate::secdata::sid_arg(p) })
}

/// An open key of a container's mapping, or the Win32 error.
fn open_mapping(container: &Sid, access: u32) -> Result<isize, u32> {
    let path = z_str(&ac::mapping_key(container));
    let mut key = 0isize;
    // SAFETY: a NUL-terminated path and a local out variable.
    let e = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, access | KEY_WOW64_64, &mut key) };
    if e == 0 { Ok(key) } else { Err(e) }
}

/// The string value `name` of an open key.
fn string_value(key: isize, name: &str) -> Option<Vec<u16>> {
    let n = z_str(name);
    let mut kind = 0u32;
    let mut size = 0u32;
    // SAFETY: the sizing call, then the read into a buffer of that size.
    unsafe {
        if RegQueryValueExW(key, n.as_ptr(), core::ptr::null_mut(), &mut kind, core::ptr::null_mut(), &mut size) != 0
            || kind != REG_SZ
        {
            return None;
        }
        let mut buf = alloc::vec![0u16; size as usize / 2 + 1];
        let mut got = size;
        if RegQueryValueExW(key, n.as_ptr(), core::ptr::null_mut(), &mut kind, buf.as_mut_ptr().cast(), &mut got) != 0 {
            return None;
        }
        buf.truncate((got as usize / 2).min(buf.len()));
        while buf.last() == Some(&0) {
            buf.pop();
        }
        Some(buf)
    }
}

/// `AppContainerRegisterSid(sid, moniker, displayName)`: records the mapping of a container SID to
/// a name in the user's registry, once (`ERROR_ALREADY_EXISTS` as an `HRESULT` the second time),
/// whatever process looks at it. A NULL SID and NULL or empty strings are `E_INVALIDARG`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AppContainerRegisterSid(sid: *const u8, moniker: *const u16, display: *const u16) -> i32 {
    // SAFETY: the caller's SID and strings.
    let (sid, moniker, display) = unsafe { (sid_at(sid), wide_arg(moniker), wide_arg(display)) };
    let (Some(sid), Some(moniker), Some(display)) = (sid, moniker, display) else {
        return hresult(ac::E_INVALIDARG);
    };
    if moniker.is_empty() || display.is_empty() {
        return hresult(ac::E_INVALIDARG);
    }
    if let Ok(k) = open_mapping(&sid, KEY_READ) {
        // SAFETY: a key we opened.
        unsafe { RegCloseKey(k) };
        return hresult(ac::E_ALREADY_EXISTS);
    }
    let path = z_str(&ac::mapping_key(&sid));
    let mut key = 0isize;
    // SAFETY: a NUL-terminated path, local out variables.
    let e = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            core::ptr::null(),
            0,
            KEY_ALL_ACCESS | KEY_WOW64_64,
            core::ptr::null(),
            &mut key,
            core::ptr::null_mut(),
        )
    };
    if e != 0 {
        return hr(e);
    }
    let set = |name: &str, text: &[u16]| {
        let (n, v) = (z_str(name), z(text));
        // SAFETY: an open key and NUL-terminated strings.
        unsafe { RegSetValueExW(key, n.as_ptr(), 0, REG_SZ, v.as_ptr().cast(), (v.len() * 2) as u32) }
    };
    let e = match set("Moniker", moniker) {
        0 => set("DisplayName", display),
        e => e,
    };
    // SAFETY: the key we created.
    unsafe { RegCloseKey(key) };
    hr(e)
}

/// `AppContainerLookupMoniker(sid, &moniker)`: the name registered for a container SID, in a
/// `LocalAlloc` block. `0x80070002` for a SID nobody registered, `0x8007109A` for one that is no
/// package SID, `0x80072023` for a moniker longer than 64 characters.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AppContainerLookupMoniker(sid: *const u8, out: *mut *mut u16) -> i32 {
    // SAFETY: the caller's SID.
    let Some(sid) = (unsafe { sid_at(sid) }) else { return hresult(ac::E_INVALIDARG) };
    if out.is_null() {
        return hresult(ac::E_INVALIDARG);
    }
    if !ac::is_package_sid(&sid) {
        return hresult(ac::E_NOT_APPCONTAINER);
    }
    let key = match open_mapping(&sid, KEY_READ) {
        Ok(k) => k,
        Err(e) => return hr(e),
    };
    let name = string_value(key, "Moniker");
    // SAFETY: the key we opened.
    unsafe { RegCloseKey(key) };
    let Some(name) = name else { return hresult(ac::E_FILE_NOT_FOUND) };
    if name.len() > ac::MAX_NAME {
        return hresult(ac::E_MONIKER_TOO_LONG);
    }
    let bytes = (name.len() + 1) * 2;
    let p = crate::memory::LocalAlloc(0, bytes) as *mut u16;
    if p.is_null() {
        return hr(8);
    }
    // SAFETY: `p` holds `name.len() + 1` units; the caller's out pointer.
    unsafe {
        core::ptr::copy_nonoverlapping(name.as_ptr(), p, name.len());
        p.add(name.len()).write(0);
        out.write(p);
    }
    S_OK
}

/// Remove a container's mapping (`DeleteAppContainerProfile` of the name it derives from).
/// Nothing to remove is no failure.
pub fn unregister_sid(sid: &Sid) {
    let parent = z_str(ac::MAPPINGS_KEY);
    let name = z_str(&poly_security::sddl::to_string(sid));
    let mut key = 0isize;
    // SAFETY: NUL-terminated paths and local variables.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, parent.as_ptr(), 0, KEY_ALL_ACCESS | KEY_WOW64_64, &mut key) == 0 {
            RegDeleteKeyExW(key, name.as_ptr(), KEY_WOW64_64, 0);
            RegCloseKey(key);
        }
    }
}

/// `DeriveCapabilitySidsFromName(name, &groups, &nGroups, &capabilities, &nCapabilities)`
/// (`securitybaseapi.h`, exported by `kernelbase` only): one group SID and one capability SID of a
/// name, each in its own `LocalAlloc` block, in two `LocalAlloc` arrays of one pointer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeriveCapabilitySidsFromName(
    name: *const u16,
    groups: *mut *mut *mut u8,
    n_groups: *mut u32,
    capabilities: *mut *mut *mut u8,
    n_capabilities: *mut u32,
) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { wide_arg(name) }) else { return fail(ERROR_INVALID_PARAMETER, FALSE) };
    if groups.is_null() || n_groups.is_null() || capabilities.is_null() || n_capabilities.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let (group, capability) = ac::capability_sids(name);
    let block = |bytes: &[u8]| -> *mut u8 {
        let p = crate::memory::LocalAlloc(0, bytes.len());
        if !p.is_null() {
            // SAFETY: `p` holds `bytes.len()` bytes.
            unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
        }
        p
    };
    let array = |sid: &Sid| -> *mut *mut u8 {
        let item = block(&sid.to_bytes());
        let a = crate::memory::LocalAlloc(0, core::mem::size_of::<*mut u8>()) as *mut *mut u8;
        if a.is_null() || item.is_null() {
            return core::ptr::null_mut();
        }
        // SAFETY: `a` holds one pointer.
        unsafe { a.write(item) };
        a
    };
    let (g, c) = (array(&group), array(&capability));
    if g.is_null() || c.is_null() {
        return fail(8, FALSE);
    }
    // SAFETY: the caller's out variables.
    unsafe {
        groups.write(g);
        n_groups.write(1);
        capabilities.write(c);
        n_capabilities.write(1);
    }
    TRUE
}

/// `SECURITY_CAPABILITIES` (`winnt.h`).
#[repr(C)]
pub struct SecurityCapabilities {
    app_container_sid: *const u8,
    capabilities: *const u8,
    capability_count: u32,
    reserved: u32,
}

/// What a token for `container` and `capabilities` is checked against, as Win32 errors: the
/// container is a package SID (`ERROR_NOT_APPCONTAINER`; a child container's SID with 12
/// sub-authorities is `ERROR_FILE_NOT_FOUND`, there is no parent), the capabilities are capability
/// SIDs, none twice (`ERROR_INVALID_PARAMETER`).
unsafe fn check_capabilities(sc: &SecurityCapabilities) -> Result<(), u32> {
    // SAFETY: the caller's SID.
    let Some(sid) = (unsafe { sid_at(sc.app_container_sid) }) else { return Err(ERROR_INVALID_PARAMETER) };
    if !ac::is_package_sid(&sid) {
        let child = sid.authority == ac::APP_PACKAGE_AUTHORITY
            && sid.subs.first() == Some(&ac::APP_PACKAGE_BASE_RID)
            && sid.subs.len() == 12;
        return Err(if child { ERROR_FILE_NOT_FOUND } else { ERROR_NOT_APPCONTAINER });
    }
    let n = sc.capability_count as usize;
    if (n == 0) != sc.capabilities.is_null() {
        return Err(ERROR_INVALID_PARAMETER);
    }
    let mut seen: Vec<Sid> = Vec::new();
    for i in 0..n {
        // SAFETY: the caller's array of `n` `SID_AND_ATTRIBUTES` (a pointer and a `DWORD`).
        let p = unsafe { (sc.capabilities.add(16 * i) as *const *const u8).read_unaligned() };
        // SAFETY: the caller's SID.
        let Some(c) = (unsafe { sid_at(p) }) else { return Err(ERROR_INVALID_PARAMETER) };
        if !ac::is_capability_sid(&c) || seen.contains(&c) {
            return Err(ERROR_INVALID_PARAMETER);
        }
        seen.push(c);
    }
    Ok(())
}

/// The lowbox token for `sc` made from `source` (a handle of the caller's, or NULL for the
/// process's own token): the value of `CreateAppContainerToken` and of `CreateProcessW`'s
/// `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`.
pub(crate) unsafe fn make_lowbox(source: Handle, sc: &SecurityCapabilities) -> Result<Handle, u32> {
    // SAFETY: the caller's structure.
    unsafe { check_capabilities(sc) }?;
    let mut own: Handle = 0;
    let src = if source == 0 {
        // SAFETY: a local out variable.
        let st = unsafe { NtOpenProcessToken(-1, TOKEN_DUPLICATE | TOKEN_QUERY, &mut own) };
        if !st.is_success() {
            // SAFETY: an ntdll export.
            return Err(unsafe { crate::nt::RtlNtStatusToDosError(st) });
        }
        own
    } else {
        source
    };
    let mut new: Handle = 0;
    // SAFETY: the SID and the list are the caller's, checked above; the kernel reads them.
    let st = unsafe {
        NtCreateLowBoxToken(
            &mut new,
            src,
            MAXIMUM_ALLOWED,
            core::ptr::null(),
            sc.app_container_sid,
            sc.capability_count,
            sc.capabilities,
            0,
            core::ptr::null(),
        )
    };
    if own != 0 {
        // SAFETY: the token we opened.
        unsafe { NtClose(own) };
    }
    if st.is_success() {
        Ok(new)
    } else {
        // SAFETY: an ntdll export.
        Err(unsafe { crate::nt::RtlNtStatusToDosError(st) })
    }
}

/// The token of a child `CreateProcessW` starts in the container of `sc`: its profile has to
/// exist (a mapping for the SID), else error 2.
pub(crate) unsafe fn start_in_container(sc: *const SecurityCapabilities) -> Result<Handle, u32> {
    // SAFETY: the caller's structure.
    let sc = unsafe { &*sc };
    // SAFETY: the caller's SID.
    let Some(sid) = (unsafe { sid_at(sc.app_container_sid) }) else { return Err(ERROR_INVALID_PARAMETER) };
    if ac::is_package_sid(&sid) {
        match open_mapping(&sid, KEY_READ) {
            // SAFETY: a key we opened.
            Ok(k) => unsafe { RegCloseKey(k) },
            Err(_) => return Err(ERROR_FILE_NOT_FOUND),
        };
    }
    // SAFETY: as above.
    unsafe { make_lowbox(0, sc) }
}

/// `CreateAppContainerToken(source, &securityCapabilities, &token)`: a primary lowbox token.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateAppContainerToken(
    source: Handle,
    sc: *const SecurityCapabilities,
    out: *mut Handle,
) -> i32 {
    if sc.is_null() || out.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's structure.
    match unsafe { make_lowbox(source, &*sc) } {
        Ok(h) => {
            // SAFETY: the caller's out variable.
            unsafe { out.write(h) };
            TRUE
        }
        Err(e) => {
            // The handle is NULL after a failure (the sheet's tests read NULL back).
            // SAFETY: the caller's out variable.
            unsafe { out.write(0) };
            fail(e, FALSE)
        }
    }
}

/// `GetAppContainerNamedObjectPath(token, sid, length, buffer, &needed)` (`securityappcontainer.h`):
/// the object directory of the container named by `token` or `sid` (neither: the process's
/// own token), `AppContainerNamedObjects\<SID>`, in characters with the NUL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAppContainerNamedObjectPath(
    token: Handle,
    sid: *const u8,
    length: u32,
    buffer: *mut u16,
    needed: *mut u32,
) -> i32 {
    if token != 0 && !sid.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    let container = if !sid.is_null() {
        // SAFETY: the caller's SID.
        match unsafe { sid_at(sid) } {
            Some(s) if ac::is_package_sid(&s) => s,
            Some(_) => return fail(ERROR_NOT_APPCONTAINER, FALSE),
            None => return fail(ERROR_INVALID_PARAMETER, FALSE),
        }
    } else {
        let h = if token == 0 { CURRENT_PROCESS_TOKEN } else { token };
        let mut buf = [0u8; 128];
        let mut ret = 0u32;
        // SAFETY: a local buffer of the length given.
        let st = unsafe { NtQueryInformationToken(h, TOKEN_CLASS_APP_CONTAINER_SID, buf.as_mut_ptr(), 128, &mut ret) };
        if !st.is_success() {
            return fail_status(st, FALSE);
        }
        let ptr = u64::from_le_bytes(buf[..8].try_into().expect("8 bytes"));
        if ptr == 0 {
            return fail(ERROR_NOT_APPCONTAINER, FALSE);
        }
        // The SID follows the pointer in the same buffer.
        match Sid::from_bytes(&buf[8..]) {
            Some(s) => s,
            None => return fail(ERROR_NOT_APPCONTAINER, FALSE),
        }
    };
    let text = z_str(&ac::named_object_path(&container));
    if needed.is_null() {
        return fail(ERROR_INVALID_PARAMETER, FALSE);
    }
    // SAFETY: the caller's out variable.
    unsafe { needed.write(text.len() as u32) };
    if (length as usize) < text.len() || buffer.is_null() {
        return fail(ERROR_INSUFFICIENT_BUFFER, FALSE);
    }
    // SAFETY: the caller's buffer holds `length` characters, at least `text.len()`.
    unsafe { core::ptr::copy_nonoverlapping(text.as_ptr(), buffer, text.len()) };
    TRUE
}
