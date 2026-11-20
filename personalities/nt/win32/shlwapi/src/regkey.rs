//! `SHDeleteKeyW` and `SHDeleteValueW` (Learn, `shlwapi.h`): the shell's helpers that delete a
//! registry key with everything below it, and one value of a subkey. A program that registers a
//! file type for the shell removes it again with them; while they did nothing, the type stayed
//! registered and `ShellExecuteExW` still opened its files (impl-chrome-dl-2).

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn RegDeleteTreeW(key: isize, sub: *const u16) -> u32;
    fn RegOpenKeyExW(key: isize, sub: *const u16, options: u32, sam: u32, out: *mut isize) -> u32;
    fn RegDeleteValueW(key: isize, name: *const u16) -> u32;
    fn RegCloseKey(key: isize) -> u32;
}

/// `KEY_SET_VALUE` (`winnt.h`).
const KEY_SET_VALUE: u32 = 0x0002;

/// `SHDeleteKeyW(hkey, pszSubKey)`: "Deletes a subkey and all its descendants" (Learn); the
/// Win32 error of the deletion, `ERROR_SUCCESS` (0) when it is gone.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHDeleteKeyW(key: isize, sub: *const u16) -> u32 {
    // SAFETY: the caller's key and NUL-terminated name. `RegDeleteTreeW` with a name deletes that
    // key with its subkeys and values (Learn, "RegDeleteTreeW").
    unsafe { RegDeleteTreeW(key, sub) }
}

/// `SHDeleteValueW(hkey, pszSubKey, pszValue)`: deletes the named value of a subkey (Learn);
/// the Win32 error, `ERROR_SUCCESS` (0) when it is gone.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHDeleteValueW(key: isize, sub: *const u16, value: *const u16) -> u32 {
    let mut k: isize = 0;
    // SAFETY: the caller's key and names; the key opened here is closed here.
    unsafe {
        let r = RegOpenKeyExW(key, sub, 0, KEY_SET_VALUE, &mut k);
        if r != 0 {
            return r;
        }
        let r = RegDeleteValueW(k, value);
        RegCloseKey(k);
        r
    }
}
