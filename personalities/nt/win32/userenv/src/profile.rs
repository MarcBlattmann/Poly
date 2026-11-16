//! AppContainer profiles (ADR 0196, the sheets of `CreateAppContainerProfile` and
//! `GetAppContainerFolderPath`): a name, its derived SID, the registry mapping of the SID to the
//! name (`kernelbase`'s `AppContainerRegisterSid`) and the folder
//! `%LOCALAPPDATA%\Packages\<name>\AC` with `AC\Temp`. The rules (names, texts, errors) are
//! `poly_security::appcontainer`; this file makes the calls.

use alloc::string::String;
use alloc::vec::Vec;
use poly_security::appcontainer as ac;
use poly_security::sid::Sid;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetProcessHeap() -> isize;
    fn HeapAlloc(heap: isize, flags: u32, bytes: usize) -> *mut u8;
    fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;
    fn CreateDirectoryW(path: *const u16, security: *const u8) -> i32;
    fn RemoveDirectoryW(path: *const u16) -> i32;
    fn GetFileAttributesW(path: *const u16) -> u32;
    fn GetLastError() -> u32;
    fn AppContainerRegisterSid(sid: *const u8, moniker: *const u16, display: *const u16) -> i32;
    fn AppContainerLookupMoniker(sid: *const u8, moniker: *mut *mut u16) -> i32;
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, out: *mut isize) -> u32;
    fn RegDeleteKeyExW(key: isize, sub: *const u16, sam: u32, reserved: u32) -> u32;
    fn RegCloseKey(key: isize) -> u32;
    fn LocalFree(mem: *mut u8) -> *mut u8;
}

const HKEY_CURRENT_USER: isize = 0x8000_0001u32 as i32 as isize;
const KEY_ALL_ACCESS: u32 = 0xF_003F;
const KEY_WOW64_64: u32 = 0x100;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const ERROR_ALREADY_EXISTS: u32 = 183;

fn hr(error: u32) -> i32 {
    (0x8007_0000 | error) as i32
}

fn code(c: u32) -> i32 {
    c as i32
}

fn z(units: &[u16]) -> Vec<u16> {
    let mut v = units.to_vec();
    v.push(0);
    v
}

fn z_str(s: &str) -> Vec<u16> {
    z(&s.encode_utf16().collect::<Vec<u16>>())
}

/// A NUL-terminated string of the caller's.
unsafe fn units<'a>(p: *const u16) -> Option<&'a [u16]> {
    if p.is_null() {
        return None;
    }
    let mut n = 0;
    // SAFETY: NUL-terminated per the contract.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        Some(core::slice::from_raw_parts(p, n))
    }
}

/// `%LOCALAPPDATA%`, or the profile of Poly's one user.
fn local_app_data() -> String {
    let name = z_str("LOCALAPPDATA");
    let mut buf = [0u16; 400];
    // SAFETY: a local buffer of the size given.
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n > 0 && n < buf.len() {
        String::from_utf16_lossy(&buf[..n])
    } else {
        String::from("C:\\Users\\poly\\AppData\\Local")
    }
}

fn exists(path: &str) -> bool {
    let p = z_str(path);
    // SAFETY: a NUL-terminated path.
    unsafe { GetFileAttributesW(p.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

fn make_dir(path: &str) -> Result<(), u32> {
    let p = z_str(path);
    // SAFETY: a NUL-terminated path.
    if unsafe { CreateDirectoryW(p.as_ptr(), core::ptr::null()) } != 0 {
        return Ok(());
    }
    // SAFETY: no preconditions.
    match unsafe { GetLastError() } {
        ERROR_ALREADY_EXISTS => Ok(()),
        e => Err(e),
    }
}

/// The folder of a profile: `...\Packages\<name>`; the container's own files are below `\AC`.
fn package_folder(name: &[u16]) -> String {
    alloc::format!("{}\\Packages\\{}", local_app_data(), String::from_utf16_lossy(name))
}

/// Make `\AC` and `\AC\Temp` of the profile `name`.
fn make_folders(name: &[u16]) -> Result<String, u32> {
    let packages = alloc::format!("{}\\Packages", local_app_data());
    let package = package_folder(name);
    let ac = alloc::format!("{package}\\AC");
    for dir in [packages, package, ac.clone(), alloc::format!("{ac}\\Temp")] {
        make_dir(&dir)?;
    }
    Ok(ac)
}

/// A copy of `bytes` in the process heap (a block `CoTaskMemFree` frees).
fn task_mem(bytes: &[u8]) -> *mut u8 {
    // SAFETY: the process heap.
    let p = unsafe { HeapAlloc(GetProcessHeap(), 0, bytes.len().max(1)) };
    if !p.is_null() {
        // SAFETY: `p` holds `bytes.len()` bytes.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
    }
    p
}

fn wide_bytes(units: &[u16]) -> Vec<u8> {
    z(units).iter().flat_map(|u| u.to_le_bytes()).collect()
}

/// `CreateAppContainerProfile(name, displayName, description, capabilities, count, &sid)`
/// (`userenv.h`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateAppContainerProfile(
    name: *const u16,
    display: *const u16,
    description: *const u16,
    capabilities: *const u8,
    count: u32,
    sid: *mut *mut u8,
) -> i32 {
    // SAFETY: the caller's strings.
    let (name, display, description) = unsafe { (units(name), units(display), units(description)) };
    let (Some(name), Some(display), Some(description)) = (name, display, description) else {
        return code(ac::E_INVALIDARG);
    };
    if let Err(e) = ac::check_profile_name(name) {
        return code(e);
    }
    if ac::check_text(display, ac::MAX_DISPLAY_NAME).is_err()
        || ac::check_text(description, ac::MAX_DESCRIPTION).is_err()
        || (count != 0 && capabilities.is_null())
        || sid.is_null()
    {
        return code(ac::E_INVALIDARG);
    }
    let derived = ac::package_sid(name);
    if exists(&alloc::format!("{}\\AC", package_folder(name))) {
        return code(ac::E_ALREADY_EXISTS);
    }
    if let Err(e) = make_folders(name) {
        return hr(e);
    }
    let (name_z, display_z, bytes) = (z(name), z(display), derived.to_bytes());
    // SAFETY: NUL-terminated strings and a SID of ours; a mapping that exists stays.
    let r = unsafe { AppContainerRegisterSid(bytes.as_ptr(), name_z.as_ptr(), display_z.as_ptr()) };
    if r != 0 && r != code(ac::E_ALREADY_EXISTS) {
        return r;
    }
    let out = task_mem(&bytes);
    if out.is_null() {
        return hr(8);
    }
    // SAFETY: the caller's out pointer.
    unsafe { sid.write(out) };
    0
}

/// `DeleteAppContainerProfile(name)`: the mapping and the folder go; a name that never was a
/// profile is no failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DeleteAppContainerProfile(name: *const u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(name) = (unsafe { units(name) }) else { return code(ac::E_INVALIDARG) };
    if let Err(e) = ac::check_profile_name(name) {
        return code(e);
    }
    let derived = ac::package_sid(name);
    let parent = z_str(ac::MAPPINGS_KEY);
    let key_name = z_str(&poly_security::sddl::to_string(&derived));
    let mut key = 0isize;
    // SAFETY: NUL-terminated paths and local variables.
    unsafe {
        if RegOpenKeyExW(HKEY_CURRENT_USER, parent.as_ptr(), 0, KEY_ALL_ACCESS | KEY_WOW64_64, &mut key) == 0 {
            RegDeleteKeyExW(key, key_name.as_ptr(), KEY_WOW64_64, 0);
            RegCloseKey(key);
        }
    }
    let package = package_folder(name);
    for dir in [alloc::format!("{package}\\AC\\Temp"), alloc::format!("{package}\\AC"), package] {
        let p = z_str(&dir);
        // SAFETY: a NUL-terminated path; a folder with files stays.
        unsafe { RemoveDirectoryW(p.as_ptr()) };
    }
    0
}

/// `GetAppContainerFolderPath(sidText, &path)`: the profile's `AC` folder, in a `CoTaskMem` string,
/// for the SID of a registered container (the folder is made if it is not there).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAppContainerFolderPath(sid_text: *const u16, path: *mut *mut u16) -> i32 {
    // SAFETY: the caller's string.
    let Some(text) = (unsafe { units(sid_text) }) else { return code(ac::E_INVALIDARG) };
    if path.is_null() {
        return code(ac::E_INVALIDARG);
    }
    let Some(sid) = poly_security::sddl::parse(text) else { return code(ac::E_INVALID_SID) };
    if !ac::is_package_sid(&sid) {
        return code(ac::E_NOT_APPCONTAINER);
    }
    let name = match moniker_of(&sid) {
        Ok(n) => n,
        Err(e) => return e,
    };
    let folder = match make_folders(&name) {
        Ok(f) => f,
        Err(e) => return hr(e),
    };
    let out = task_mem(&wide_bytes(&folder.encode_utf16().collect::<Vec<u16>>()));
    if out.is_null() {
        return hr(8);
    }
    // SAFETY: the caller's out pointer.
    unsafe { path.write(out.cast()) };
    0
}

/// The name registered for `sid`, or the `HRESULT` of the lookup.
fn moniker_of(sid: &Sid) -> Result<Vec<u16>, i32> {
    let bytes = sid.to_bytes();
    let mut p: *mut u16 = core::ptr::null_mut();
    // SAFETY: a SID of ours and a local out variable.
    let r = unsafe { AppContainerLookupMoniker(bytes.as_ptr(), &mut p) };
    if r != 0 {
        return Err(r);
    }
    // SAFETY: a NUL-terminated string that the call made.
    let name = unsafe { units(p) }.map(<[u16]>::to_vec).unwrap_or_default();
    // SAFETY: the block `LocalAlloc` made.
    unsafe { LocalFree(p.cast()) };
    Ok(name)
}

/// `GetAppContainerRegistryLocation(desiredAccess, &key)`: the key of the calling container.
/// A process that is no container (Poly's callers are not, the sheet) gets `E_UNEXPECTED`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAppContainerRegistryLocation(_access: u32, key: *mut isize) -> i32 {
    if !key.is_null() {
        // SAFETY: the caller's out pointer.
        unsafe { key.write(0) };
    }
    code(ac::E_UNEXPECTED)
}
