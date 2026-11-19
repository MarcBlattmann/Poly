//! What an installer tells the shell about file types and default programs: `SHChangeNotify`
//! (sheet `shell32.dll/SHChangeNotify.yaml`) and the object `SHCreateAssociationRegistration`
//! returns, `IApplicationAssociationRegistration` (sheet
//! `shell32.dll/SHCreateAssociationRegistration.yaml`). The association rules are
//! `poly_kernelbase::assoc`'s; this file reads the registry for them.

use crate::com::*;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_kernelbase::assoc::{self as logic, Hive};

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegGetValueW(
        key: isize,
        sub: *const u16,
        value: *const u16,
        flags: u32,
        kind: *mut u32,
        data: *mut u8,
        size: *mut u32,
    ) -> u32;
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, access: u32, out: *mut isize) -> u32;
    fn RegCloseKey(key: isize) -> u32;
}

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetLastError(error: u32);
}

/// `HKEY_CURRENT_USER`, `HKEY_LOCAL_MACHINE` (`winreg.h`), sign-extended as the header's cast makes them.
const HKEY_CURRENT_USER: isize = 0x8000_0001_u32 as i32 as isize;
const HKEY_LOCAL_MACHINE: isize = 0x8000_0002_u32 as i32 as isize;
/// `RRF_RT_REG_SZ` (`winreg.h`): a `REG_SZ`, or a `REG_EXPAND_SZ` that comes expanded (as a `REG_SZ`).
const RRF_STRINGS: u32 = 0x2;
/// `KEY_READ` (`winnt.h`).
const KEY_READ: u32 = 0x2_0019;

/// `SHChangeNotify(wEventId, uFlags, dwItem1, dwItem2)`: Poly's desktop watches the file system
/// itself and reads no Windows associations yet, so there is nobody to tell: the call returns at
/// once and changes nothing, with the last error 0 (the sheet's `returns`, `last_error`).
#[unsafe(no_mangle)]
pub extern "system" fn SHChangeNotify(_event: i32, _flags: u32, _item1: *const c_void, _item2: *const c_void) {
    // SAFETY: a plain call.
    unsafe { SetLastError(0) };
}

fn nul_of(text: &[u16]) -> Vec<u16> {
    nul(text)
}

/// The registry root and the path below it of a key of `hive`.
fn place(hive: Hive, key: &[u16]) -> (isize, Vec<u16>) {
    let (root, base) = match hive {
        Hive::UserClasses => (HKEY_CURRENT_USER, "Software\\Classes"),
        Hive::MachineClasses => (HKEY_LOCAL_MACHINE, "SOFTWARE\\Classes"),
        Hive::Machine => (HKEY_LOCAL_MACHINE, ""),
        Hive::User => (HKEY_CURRENT_USER, ""),
    };
    let mut path: Vec<u16> = base.encode_utf16().collect();
    if !path.is_empty() && !key.is_empty() {
        path.push(u16::from(b'\\'));
    }
    path.extend_from_slice(key);
    (root, path)
}

/// Poly's registry, as far as the association-registration rules read it (values and keys).
struct System;

impl logic::Registry for System {
    fn value(&self, hive: Hive, key: &[u16], name: Option<&[u16]>) -> Option<Vec<u16>> {
        let (root, path) = place(hive, key);
        let path = nul_of(&path);
        let name = name.map(nul_of);
        let name_ptr = name.as_ref().map_or(core::ptr::null(), |v| v.as_ptr());
        let mut buf = alloc::vec![0u8; 4096];
        let mut size = buf.len() as u32;
        let mut kind = 0u32;
        // SAFETY: NUL-terminated names; the buffer and its size.
        let r =
            unsafe { RegGetValueW(root, path.as_ptr(), name_ptr, RRF_STRINGS, &mut kind, buf.as_mut_ptr(), &mut size) };
        if r != 0 {
            return None;
        }
        Some(
            buf[..(size as usize).min(buf.len())]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&c| c != 0)
                .collect(),
        )
    }

    fn has_key(&self, hive: Hive, key: &[u16]) -> bool {
        let (root, path) = place(hive, key);
        let path = nul_of(&path);
        let mut h: isize = 0;
        // SAFETY: a NUL-terminated path and an out pointer; the key is closed again.
        unsafe {
            if RegOpenKeyExW(root, path.as_ptr(), 0, KEY_READ, &mut h) != 0 {
                return false;
            }
            RegCloseKey(h);
        }
        true
    }

    /// Not read by the association-registration rules (only `AssocQueryStringW` lists values).
    fn value_names(&self, _hive: Hive, _key: &[u16]) -> Vec<Vec<u16>> {
        Vec::new()
    }

    /// Not read by the association-registration rules.
    fn file_description(&self, _path: &[u16]) -> Option<Vec<u16>> {
        None
    }
}

/// `IID_IApplicationAssociationRegistration` (`shobjidl_core.h`):
/// {4E530B0A-E611-4C77-A3AC-9031D022281B}.
const IID_IAPPLICATION_ASSOCIATION_REGISTRATION: Guid =
    guid(0x4E53_0B0A, 0xE611, 0x4C77, [0xA3, 0xAC, 0x90, 0x31, 0xD0, 0x22, 0x28, 0x1B]);

#[repr(C)]
struct Registration {
    head: Head,
}

static REGISTRATION_IIDS: [Guid; 1] = [IID_IAPPLICATION_ASSOCIATION_REGISTRATION];

/// `IApplicationAssociationRegistration` after `IUnknown` (`shobjidl_core.h`):
/// `QueryCurrentDefault` 3, `QueryAppIsDefault` 4, `QueryAppIsDefaultAll` 5, `SetAppAsDefault` 6,
/// `SetAppAsDefaultAll` 7, `ClearUserAssociations` 8.
static REGISTRATION_VTBL: Vtbl<9> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    query_current_default as *const c_void,
    query_app_is_default as *const c_void,
    query_app_is_default_all as *const c_void,
    set_app_as_default as *const c_void,
    set_app_as_default_all as *const c_void,
    clear_user_associations as *const c_void,
]);

/// `QueryCurrentDefault(pszQuery, atQueryType, alQueryLevel, ppszAssociation)`: the default's
/// ProgID (or its command at `AL_MACHINE`) in a `CoTaskMemAlloc` string.
unsafe extern "system" fn query_current_default(
    _this: *mut c_void,
    query: *const u16,
    kind: u32,
    level: u32,
    out: *mut *mut u16,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out parameter and string.
    unsafe {
        *out = core::ptr::null_mut();
        if query.is_null() {
            return E_INVALIDARG;
        }
        match logic::current_default(&System, wstr(query), kind, level) {
            Ok(text) => task_string(&text, out),
            Err(hr) => hr as Hresult,
        }
    }
}

/// `QueryAppIsDefault(pszQuery, atQueryType, alQueryLevel, pszAppRegistryName, pfDefault)`: this
/// build answers TRUE for every application (the sheet's `query_app_is_default`).
unsafe extern "system" fn query_app_is_default(
    _this: *mut c_void,
    query: *const u16,
    _kind: u32,
    level: u32,
    _app: *const u16,
    out: *mut i32,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    if query.is_null() || level > logic::AL_USER {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = 1 };
    S_OK
}

/// `QueryAppIsDefaultAll(alQueryLevel, pszAppRegistryName, pfDefault)`: TRUE (the sheet's
/// `query_app_is_default_all`).
unsafe extern "system" fn query_app_is_default_all(
    _this: *mut c_void,
    level: u32,
    _app: *const u16,
    out: *mut i32,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    if level > logic::AL_USER {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's out parameter.
    unsafe { *out = 1 };
    S_OK
}

/// `SetAppAsDefault(pszAppRegistryName, pszSet, atSetType)`: a program may not make itself the
/// default (`E_FAIL`, the sheet's `set_app_as_default`).
unsafe extern "system" fn set_app_as_default(
    _this: *mut c_void,
    _app: *const u16,
    _set: *const u16,
    _kind: u32,
) -> Hresult {
    E_FAIL
}

/// `SetAppAsDefaultAll(pszAppRegistryName)`: `S_OK` for a registered application, changing
/// nothing.
unsafe extern "system" fn set_app_as_default_all(_this: *mut c_void, app: *const u16) -> Hresult {
    if app.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's string.
    logic::set_app_as_default_all(&System, unsafe { wstr(app) }) as Hresult
}

/// `ClearUserAssociations()`: Poly keeps no choice of the user for Windows programs, so there is
/// nothing to clear.
unsafe extern "system" fn clear_user_associations(_this: *mut c_void) -> Hresult {
    S_OK
}

/// `SHCreateAssociationRegistration(riid, ppv)`: a new `IApplicationAssociationRegistration`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHCreateAssociationRegistration(iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    let object =
        into_raw(Registration { head: Head::new(&REGISTRATION_VTBL, &REGISTRATION_IIDS, drop_box::<Registration>) });
    // SAFETY: a fresh object; the caller's out parameter.
    unsafe { hand_out(object.cast(), iid, out) }
}
