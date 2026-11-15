//! The `Path*W` functions of `shlwapi.dll` that popular programs call first (git, Firefox,
//! LibreOffice, Python, ...): the rules are `poly_kernelbase::winpath`'s, host-tested against the
//! recorded results of `specs/windows/shlwapi.dll/Path*.tests`; this file only reads and writes
//! the callers' strings.

use poly_kernelbase::winpath as wp;

#[cfg_attr(target_arch = "x86", link(name = "kernelbase", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernelbase", kind = "raw-dylib"))]
unsafe extern "system" {
    fn SetLastError(error: u32);
}

/// `ERROR_FILENAME_EXCED_RANGE` (`winerror.h`).
const ERROR_FILENAME_EXCED_RANGE: u32 = 206;

const BACKSLASH: u16 = b'\\' as u16;

/// The characters of a NUL-terminated string (empty for a null pointer).
///
/// # Safety
/// `p` is null or a NUL-terminated UTF-16 string that lives as long as `'a`.
unsafe fn wide<'a>(p: *const u16) -> &'a [u16] {
    // SAFETY: the caller's contract.
    unsafe { crate::wide(p) }
}

/// Writes `text` and a terminating NUL to `dst`.
///
/// # Safety
/// `dst` holds `text.len() + 1` writable units.
unsafe fn put(dst: *mut u16, text: &[u16]) {
    // SAFETY: the caller's contract.
    unsafe {
        core::ptr::copy(text.as_ptr(), dst, text.len());
        dst.add(text.len()).write(0);
    }
}

/// What a failed `PathCombineW` or `PathCanonicalizeW` leaves in the destination: an empty string.
/// As observed on Windows, a combined result of 260 or 261 units writes the whole `MAX_PATH`
/// buffer (`whole` is that text), every other failure only its first four units.
///
/// # Safety
/// `dst` holds `MAX_PATH` writable units.
unsafe fn put_failure(dst: *mut u16, whole: Option<&[u16]>) {
    // SAFETY: the caller's contract; at most MAX_PATH units are written.
    unsafe {
        match whole {
            Some(text) => {
                for i in 0..wp::MAX_PATH {
                    dst.add(i).write(text.get(i).copied().unwrap_or(0));
                }
                dst.write(0);
            }
            None => {
                for i in 0..4 {
                    dst.add(i).write(0);
                }
            }
        }
    }
}

/// `PathRemoveFileSpecW(pszPath)`: TRUE when the string was shortened.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathRemoveFileSpecW(path: *mut u16) -> i32 {
    if path.is_null() {
        return 0;
    }
    // SAFETY: the caller's writable string; the result is not longer than it.
    unsafe {
        let (out, changed) = wp::remove_file_spec(wide(path));
        put(path, &out);
        i32::from(changed)
    }
}

/// `PathAppendW(pszPath, pszMore)`: TRUE on success; FALSE and an empty `pszPath` on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathAppendW(path: *mut u16, more: *const u16) -> i32 {
    if path.is_null() {
        return 0;
    }
    // SAFETY: the caller's strings; `path` holds MAX_PATH units.
    unsafe {
        match wp::path_append(wide(path), wide(more)) {
            Ok(out) => {
                put(path, &out);
                1
            }
            Err(_) => {
                path.write(0);
                0
            }
        }
    }
}

/// `PathCombineW(pszDest, pszDir, pszFile)`: `pszDest`, or NULL and an empty string on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCombineW(dest: *mut u16, dir: *const u16, file: *const u16) -> *mut u16 {
    if dest.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's strings; `dest` holds MAX_PATH units.
    unsafe {
        match wp::path_combine(wide(dir), wide(file)) {
            Ok(out) => {
                put(dest, &out);
                dest
            }
            Err(len) => {
                let whole = (len == wp::MAX_PATH || len == wp::MAX_PATH + 1)
                    .then(|| wp::path_combine_text(wide(dir), wide(file)))
                    .flatten();
                put_failure(dest, whole.as_deref());
                core::ptr::null_mut()
            }
        }
    }
}

/// `PathCanonicalizeW(pszBuf, pszPath)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathCanonicalizeW(buf: *mut u16, path: *const u16) -> i32 {
    if buf.is_null() || path.is_null() {
        return 0;
    }
    // SAFETY: the caller's strings; `buf` holds MAX_PATH units.
    unsafe {
        match wp::path_canonicalize(wide(path)) {
            Ok(out) => {
                put(buf, &out);
                1
            }
            Err(wp::TooLong) => {
                put_failure(buf, None);
                SetLastError(ERROR_FILENAME_EXCED_RANGE);
                0
            }
        }
    }
}

/// `PathIsRelativeW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsRelativeW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_relative(unsafe { wide(path) }))
}

/// `PathIsRootW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsRootW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_root(unsafe { wide(path) }))
}

/// `PathIsUNCW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsUNCW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_unc(unsafe { wide(path) }))
}

/// `PathIsUNCServerW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsUNCServerW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_unc_server(unsafe { wide(path) }))
}

/// `PathIsUNCServerShareW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsUNCServerShareW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_unc_server_share(unsafe { wide(path) }))
}

/// `PathIsNetworkPathW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsNetworkPathW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_network_path(unsafe { wide(path) }))
}

/// `PathIsFileSpecW`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathIsFileSpecW(path: *const u16) -> i32 {
    // SAFETY: the caller's string.
    i32::from(!path.is_null() && wp::is_file_spec(unsafe { wide(path) }))
}

/// `PathGetDriveNumberW`: 0 to 25, or -1.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathGetDriveNumberW(path: *const u16) -> i32 {
    if path.is_null() {
        return -1;
    }
    // SAFETY: the caller's string.
    wp::drive_number(unsafe { wide(path) })
}

/// `PathStripPathW`: only the file name stays.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathStripPathW(path: *mut u16) {
    if path.is_null() {
        return;
    }
    // SAFETY: the caller's writable string; the name is moved to its start.
    unsafe {
        let s = wide(path);
        let at = wp::strip_path_start(s);
        if at > 0 {
            core::ptr::copy(path.add(at), path, s.len() - at + 1);
        }
    }
}

/// `PathStripToRootW`: TRUE when the path has a root (the string is cut to it), FALSE otherwise
/// (the string becomes empty).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathStripToRootW(path: *mut u16) -> i32 {
    if path.is_null() {
        return 0;
    }
    // SAFETY: the caller's writable string; the root is not longer than it.
    unsafe {
        let (root, has) = wp::strip_to_root(wide(path));
        put(path, &root);
        i32::from(has)
    }
}

/// `PathSkipRootW`: the text after the root, or NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathSkipRootW(path: *const u16) -> *mut u16 {
    if path.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's string; the index is at most its length.
    unsafe {
        match wp::skip_root(wide(path)) {
            Some(at) => path.add(at) as *mut u16,
            None => core::ptr::null_mut(),
        }
    }
}

/// `PathGetArgsW`: the text after the first space (the end of the string when there is none).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathGetArgsW(path: *const u16) -> *mut u16 {
    if path.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's string; the index is at most its length.
    unsafe { path.add(wp::args_start(wide(path))) as *mut u16 }
}

/// `PathAddBackslashW`: the end of the string; NULL when the backslash does not fit in the
/// `MAX_PATH` buffer.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathAddBackslashW(path: *mut u16) -> *mut u16 {
    if path.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's writable string with room for MAX_PATH units.
    unsafe {
        let len = wide(path).len();
        if len != 0 && *path.add(len - 1) != BACKSLASH {
            if len + 2 > wp::MAX_PATH {
                return core::ptr::null_mut();
            }
            path.add(len).write(BACKSLASH);
            path.add(len + 1).write(0);
            return path.add(len + 1);
        }
        path.add(len)
    }
}

/// `PathRemoveBackslashW`: where the backslash was (now the NUL), or the last character when
/// nothing was removed.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathRemoveBackslashW(path: *mut u16) -> *mut u16 {
    if path.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's writable string.
    unsafe {
        let mut v = wide(path).to_vec();
        if wp::remove_backslash(&mut v) {
            path.add(v.len()).write(0);
            return path.add(v.len());
        }
        path.add(v.len().saturating_sub(1))
    }
}
