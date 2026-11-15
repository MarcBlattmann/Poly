//! `LoadStringW` (`winuser.h`): a string of a module's string table. The table is the `RT_STRING`
//! resource: blocks of 16 strings, the string `id` being entry `id & 15` of the block
//! `(id >> 4) + 1`, each entry a length in UTF-16 units and the units without a NUL. The rules are
//! `poly_kernelbase::resource`'s (host-tested; `specs/windows/user32.dll/LoadStringW.yaml`); the
//! block is found with the resource functions of `kernel32`, so the module handles of
//! `LoadLibraryExW` in all three modes work.

use super::set_last_error;
use poly_kernelbase::resource as res;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn FindResourceExW(module: *mut u8, ty: *const u16, name: *const u16, lang: u16) -> *mut u8;
    fn LoadResource(module: *mut u8, res: *mut u8) -> *mut u8;
    fn LockResource(data: *mut u8) -> *mut u8;
    fn SizeofResource(module: *mut u8, res: *mut u8) -> u32;
}

/// `ERROR_INVALID_PARAMETER` (`winerror.h`).
const ERROR_INVALID_PARAMETER: u32 = 87;

/// `LoadStringW(hInstance, uID, lpBuffer, cchBufferMax)`: the number of units copied without the
/// NUL. With `cchBufferMax` 0 the buffer argument is the address of a pointer that is set to the
/// string inside the resource (not NUL-terminated), and the result is its length.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadStringW(instance: *mut u8, id: u32, buf: *mut u16, cch: i32) -> i32 {
    if buf.is_null() && cch != 0 {
        set_last_error(ERROR_INVALID_PARAMETER);
        return 0;
    }
    let nul = |buf: *mut u16| {
        if cch > 0 {
            // SAFETY: the caller's buffer of at least one unit.
            unsafe { buf.write(0) };
        }
    };
    let (block, index) = res::string_block_and_index(id);
    // SAFETY: arguments of the kernel32 resource functions: RT_STRING and the block id as integers.
    let found =
        unsafe { FindResourceExW(instance, res::RT_STRING as usize as *const u16, block as usize as *const u16, 0) };
    if found.is_null() {
        nul(buf);
        return 0;
    }
    // SAFETY: the handles `FindResourceExW` returned for this module.
    let (data, size) = unsafe {
        let loaded = LoadResource(instance, found);
        (LockResource(loaded), SizeofResource(instance, found) as usize)
    };
    if data.is_null() {
        nul(buf);
        return 0;
    }
    // SAFETY: `size` bytes of the resource in the module.
    let bytes = unsafe { core::slice::from_raw_parts(data, size) };
    let Some((at, len)) = res::string_in_block(bytes, index) else {
        nul(buf);
        return 0;
    };
    if cch == 0 {
        // SAFETY: the caller passed the address of a pointer.
        unsafe { (buf as *mut *const u16).write_unaligned(data.add(at) as *const u16) };
        return len as i32;
    }
    let n = len.min((cch - 1).max(0) as usize);
    // SAFETY: the buffer holds `cch` units, `n + 1` of them are written; the units are the
    // resource's (unaligned: a resource block may start anywhere).
    unsafe {
        for i in 0..n {
            buf.add(i).write(data.add(at + 2 * i).cast::<u16>().read_unaligned());
        }
        buf.add(n).write(0);
    }
    n as i32
}
