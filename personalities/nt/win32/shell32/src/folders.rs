//! The special folders and item identifier lists of shell32: `SHGetFolderPathW/A`,
//! `SHGetSpecialFolderPathW/A`, `SHGetSpecialFolderLocation`, `SHGetPathFromIDListW/A`,
//! `ILCreateFromPathW`, `ILClone`, `ILGetSize` and `SHParseDisplayName`. The tables and the list
//! layout are `poly_kernelbase::csidl`'s (host-tested); this file asks the process for the
//! environment and the file system.
//!
//! Sources: Learn's pages of the functions and "CSIDL"; the sheets
//! `specs/windows/shell32.dll/SH*.yaml`.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_kernelbase::csidl::{self, Kind, Target};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn SetLastError(error: u32);
    fn CreateDirectoryW(name: *const u16, security: *const u8) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn FindFirstFileW(name: *const u16, data: *mut u8) -> isize;
    fn FindNextFileW(h: isize, data: *mut u8) -> i32;
    fn FindClose(h: isize) -> i32;
}

const MAX_PATH: usize = 260;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
const INVALID_HANDLE_VALUE: isize = -1;
/// `WIN32_FIND_DATAW`: the size, the attributes at 0 and the name at 44.
const FIND_DATA_SIZE: usize = 592;
const FIND_NAME_AT: usize = 44;

pub fn z(text: &[u16]) -> Vec<u16> {
    let mut v = text.to_vec();
    v.push(0);
    v
}

/// The value of an environment variable.
fn env(name: &str) -> Option<Vec<u16>> {
    super::variable(name)
}

pub fn attributes(path: &[u16]) -> u32 {
    // SAFETY: a NUL-terminated path.
    unsafe { GetFileAttributesW(z(path).as_ptr()) }
}

pub fn is_directory(path: &[u16]) -> bool {
    let a = attributes(path);
    a != INVALID_FILE_ATTRIBUTES && a & FILE_ATTRIBUTE_DIRECTORY != 0
}

/// Make the directory and the ones above it that are missing; true when it is there afterwards.
pub fn create_directories(path: &[u16]) -> bool {
    if is_directory(path) {
        return true;
    }
    let b = u16::from(b'\\');
    if let Some(cut) = path.iter().rposition(|&c| c == b)
        && cut > 2
    {
        create_directories(&path[..cut]);
    }
    // SAFETY: a NUL-terminated path.
    unsafe { CreateDirectoryW(z(path).as_ptr(), core::ptr::null()) };
    is_directory(path)
}

/// Whether directory `path` has a subdirectory.
pub fn has_subdirectory(path: &[u16]) -> bool {
    let mut pattern = path.to_vec();
    if pattern.last() != Some(&u16::from(b'\\')) {
        pattern.push(u16::from(b'\\'));
    }
    pattern.push(u16::from(b'*'));
    let mut data = [0u8; FIND_DATA_SIZE];
    // SAFETY: a NUL-terminated pattern and a WIN32_FIND_DATAW.
    let h = unsafe { FindFirstFileW(z(&pattern).as_ptr(), data.as_mut_ptr()) };
    if h == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut found = false;
    loop {
        let attrs = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let name = find_name(&data);
        let dots = name == [u16::from(b'.')] || name == [u16::from(b'.'), u16::from(b'.')];
        if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 && !dots {
            found = true;
            break;
        }
        // SAFETY: the search handle and the same buffer.
        if unsafe { FindNextFileW(h, data.as_mut_ptr()) } == 0 {
            break;
        }
    }
    // SAFETY: the search handle.
    unsafe { FindClose(h) };
    found
}

fn find_name(data: &[u8]) -> Vec<u16> {
    data[FIND_NAME_AT..FIND_NAME_AT + 2 * MAX_PATH]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect()
}

/// The name and attributes the file system has for `path` (its case, not the caller's), if it exists.
fn actual_name(path: &[u16]) -> Option<(Vec<u16>, u32)> {
    let mut data = [0u8; FIND_DATA_SIZE];
    // SAFETY: a NUL-terminated path and a WIN32_FIND_DATAW.
    let h = unsafe { FindFirstFileW(z(path).as_ptr(), data.as_mut_ptr()) };
    if h == INVALID_HANDLE_VALUE {
        return None;
    }
    // SAFETY: the search handle.
    unsafe { FindClose(h) };
    Some((find_name(&data), u32::from_le_bytes([data[0], data[1], data[2], data[3]])))
}

/// Copy `bytes` into a block of the process heap (the shell's allocator is the COM task
/// allocator, which is the process heap here; `ILFree` frees it).
pub fn alloc_list(bytes: &[u8]) -> *mut u8 {
    // SAFETY: the process heap.
    let p = unsafe { HeapAlloc(GetProcessHeap(), 0, bytes.len()) };
    if !p.is_null() {
        // SAFETY: `bytes.len()` writable bytes.
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len()) };
    }
    p
}

/// The bytes of a list, with its terminator (the first 32 KiB at most).
unsafe fn read_list(pidl: *const u8) -> Vec<u8> {
    let mut at = 0usize;
    while at < 32768 {
        // SAFETY: the caller's list: sizes, then data, ended by a size of 0.
        let cb = usize::from(unsafe { u16::from_le_bytes([*pidl.add(at), *pidl.add(at + 1)]) });
        at += 2.max(cb);
        if cb == 0 {
            break;
        }
    }
    // SAFETY: `at` bytes were read above.
    unsafe { core::slice::from_raw_parts(pidl, at) }.to_vec()
}

// ----- paths ------------------------------------------------------------------------------------

/// `SHGetFolderPathW`'s work: the text of the path or the `HRESULT`.
fn folder_path(csidl_value: i32, token: *mut u8, flags_of_call: u32) -> Result<Vec<u16>, u32> {
    let Some(entry) = csidl::entry(csidl_value) else { return Err(csidl::E_INVALIDARG) };
    if matches!(entry.kind, Kind::Virtual(_)) {
        // SAFETY: a plain call.
        unsafe { SetLastError(0) };
        return Err(csidl::E_FAIL);
    }
    if !token.is_null()
        && let Err(e) = super::whose_folders(token)
    {
        // SAFETY: a plain call.
        unsafe { SetLastError(if csidl::is_user_folder(&entry) { 298 } else { 6 }) };
        return Err(e);
    }
    let Some(text) = csidl::path_text(&entry, env, false) else { return Err(csidl::E_FAIL) };
    let flags = csidl_value as u32 & csidl::FLAG_MASK;
    let _ = flags_of_call;
    if flags & csidl::FLAG_CREATE != 0 {
        if !create_directories(&text) {
            return Err(csidl::E_FAIL);
        }
    } else if flags & csidl::FLAG_DONT_VERIFY == 0 && !is_directory(&text) {
        return Err(csidl::E_FILE_NOT_FOUND);
    }
    if text.len() >= MAX_PATH {
        return Err(csidl::E_FAIL);
    }
    Ok(text)
}

/// `SHGetFolderPathW(hwnd, csidl, hToken, dwFlags, pszPath)`: the path in the caller's
/// `MAX_PATH` buffer, `S_OK`; the buffer untouched on a failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFolderPathW(
    _hwnd: *mut u8,
    csidl_value: i32,
    token: *mut u8,
    flags: u32,
    path: *mut u16,
) -> u32 {
    if path.is_null() {
        return csidl::E_INVALIDARG;
    }
    match folder_path(csidl_value, token, flags) {
        Ok(text) => {
            // SAFETY: the caller's MAX_PATH buffer.
            unsafe { core::ptr::copy_nonoverlapping(z(&text).as_ptr(), path, text.len() + 1) };
            csidl::S_OK
        }
        Err(e) => e,
    }
}

fn narrow(text: &[u16]) -> Vec<u8> {
    let mut v: Vec<u8> = text.iter().map(|&c| if c < 0x80 { c as u8 } else { b'?' }).collect();
    v.push(0);
    v
}

/// `SHGetFolderPathA`: as the wide form, the path as ANSI text.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFolderPathA(
    _hwnd: *mut u8,
    csidl_value: i32,
    token: *mut u8,
    flags: u32,
    path: *mut u8,
) -> u32 {
    if path.is_null() {
        return csidl::E_INVALIDARG;
    }
    match folder_path(csidl_value, token, flags) {
        Ok(text) => {
            let a = narrow(&text);
            // SAFETY: the caller's MAX_PATH buffer.
            unsafe { core::ptr::copy_nonoverlapping(a.as_ptr(), path, a.len()) };
            csidl::S_OK
        }
        Err(e) => e,
    }
}

/// `SHGetSpecialFolderPathW(hwnd, pszPath, csidl, fCreate)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetSpecialFolderPathW(_hwnd: *mut u8, path: *mut u16, csidl_value: i32, create: i32) -> i32 {
    let value = csidl_value | if create != 0 { csidl::FLAG_CREATE as i32 } else { 0 };
    // SAFETY: the caller's buffer.
    i32::from(unsafe { SHGetFolderPathW(core::ptr::null_mut(), value, core::ptr::null_mut(), 0, path) } == csidl::S_OK)
}

/// `SHGetSpecialFolderPathA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetSpecialFolderPathA(_hwnd: *mut u8, path: *mut u8, csidl_value: i32, create: i32) -> i32 {
    let value = csidl_value | if create != 0 { csidl::FLAG_CREATE as i32 } else { 0 };
    // SAFETY: the caller's buffer.
    i32::from(unsafe { SHGetFolderPathA(core::ptr::null_mut(), value, core::ptr::null_mut(), 0, path) } == csidl::S_OK)
}

// ----- lists ------------------------------------------------------------------------------------

/// `SHGetSpecialFolderLocation(hwnd, csidl, ppidl)`: the list of a special folder.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetSpecialFolderLocation(_hwnd: *mut u8, csidl_value: i32, ppidl: *mut *mut u8) -> u32 {
    if ppidl.is_null() {
        return csidl::E_INVALIDARG;
    }
    let Some(entry) = csidl::entry(csidl_value) else { return csidl::E_INVALIDARG };
    let list = match entry.kind {
        _ if entry.id == 0 => csidl::empty_list(),
        Kind::Virtual(clsid) => csidl::virtual_list(&clsid),
        Kind::Path { .. } => {
            let Some(text) = csidl::path_text(&entry, env, true) else { return csidl::E_FAIL };
            if csidl_value as u32 & csidl::FLAG_CREATE != 0 {
                create_directories(&text);
            }
            let Some(list) = csidl::path_list(&text, false, 0) else { return csidl::E_FAIL };
            list
        }
    };
    let p = alloc_list(&list);
    if p.is_null() {
        return 0x8007_000E;
    }
    // SAFETY: the caller's out pointer.
    unsafe { *ppidl = p };
    csidl::S_OK
}

/// The text a list leads to: `None` when it leads to no path.
fn list_path(pidl: *const u8) -> Option<Vec<u16>> {
    if pidl.is_null() {
        return None;
    }
    // SAFETY: the caller's list.
    match csidl::target(&unsafe { read_list(pidl) }) {
        Target::Desktop => {
            let e = csidl::entry(0x10)?;
            csidl::path_text(&e, env, true)
        }
        Target::Path(p) => Some(p),
        Target::Virtual | Target::Unknown => None,
    }
}

/// `SHGetPathFromIDListW(pidl, pszPath)`: the path of a file system list in a `MAX_PATH` buffer;
/// FALSE with the empty string for a virtual folder and for no list.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetPathFromIDListW(pidl: *const u8, path: *mut u16) -> i32 {
    if path.is_null() {
        return 0;
    }
    match list_path(pidl) {
        Some(p) if p.len() < MAX_PATH => {
            // SAFETY: the caller's MAX_PATH buffer.
            unsafe { core::ptr::copy_nonoverlapping(z(&p).as_ptr(), path, p.len() + 1) };
            1
        }
        _ => {
            // SAFETY: the caller's buffer.
            unsafe { *path = 0 };
            0
        }
    }
}

/// `SHGetPathFromIDListA`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetPathFromIDListA(pidl: *const u8, path: *mut u8) -> i32 {
    if path.is_null() {
        return 0;
    }
    match list_path(pidl) {
        Some(p) if p.len() < MAX_PATH => {
            let a = narrow(&p);
            // SAFETY: the caller's MAX_PATH buffer.
            unsafe { core::ptr::copy_nonoverlapping(a.as_ptr(), path, a.len()) };
            1
        }
        _ => {
            // SAFETY: the caller's buffer.
            unsafe { *path = 0 };
            0
        }
    }
}

static SERIAL: AtomicU32 = AtomicU32::new(1);

/// The list of an existing absolute path, with the file system's spelling of the names.
fn list_of_path(path: &[u16]) -> Option<Vec<u8>> {
    let b = u16::from(b'\\');
    if path.contains(&u16::from(b'/')) || path.len() < 3 || path[1] != u16::from(b':') || path[2] != b {
        return None;
    }
    let mut cur: Vec<u16> = path[..3].to_vec();
    if !is_directory(&cur) {
        return None;
    }
    let mut last_is_file = false;
    for name in path[3..].split(|&c| c == b).filter(|n| !n.is_empty()) {
        let mut probe = cur.clone();
        probe.extend_from_slice(name);
        let (actual, attrs) = actual_name(&probe)?;
        cur.extend_from_slice(&actual);
        if attrs & FILE_ATTRIBUTE_DIRECTORY != 0 {
            cur.push(b);
            last_is_file = false;
        } else {
            last_is_file = true;
        }
    }
    while cur.len() > 3 && cur.last() == Some(&b) {
        cur.pop();
    }
    let serial = SERIAL.fetch_add(1, Ordering::Relaxed);
    csidl::path_list(&cur, last_is_file, serial)
}

/// `ILCreateFromPathW(pszPath)` (ordinal 190): the list of an existing absolute path, NULL for
/// anything else.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ILCreateFromPathW(path: *const u16) -> *mut u8 {
    if path.is_null() {
        return core::ptr::null_mut();
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated path.
    while unsafe { *path.add(n) } != 0 {
        n += 1;
    }
    // SAFETY: `n` units before the NUL.
    let text = unsafe { core::slice::from_raw_parts(path, n) };
    match list_of_path(text) {
        Some(list) => alloc_list(&list),
        None => core::ptr::null_mut(),
    }
}

/// `SHParseDisplayName(pszName, pbc, ppidl, sfgaoIn, psfgaoOut)`: the list of an existing path.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHParseDisplayName(
    name: *const u16,
    _bind_context: *mut u8,
    ppidl: *mut *mut u8,
    _attributes_in: u32,
    attributes_out: *mut u32,
) -> u32 {
    if ppidl.is_null() {
        return csidl::E_INVALIDARG;
    }
    // SAFETY: the caller's out pointer.
    unsafe { *ppidl = core::ptr::null_mut() };
    if name.is_null() {
        return csidl::E_INVALIDARG;
    }
    // SAFETY: a NUL-terminated name.
    let p = unsafe { ILCreateFromPathW(name) };
    if p.is_null() {
        return csidl::E_FILE_NOT_FOUND;
    }
    if !attributes_out.is_null() {
        // SAFETY: the caller's out pointer.
        unsafe { *attributes_out = 0 };
    }
    // SAFETY: the caller's out pointer.
    unsafe { *ppidl = p };
    csidl::S_OK
}

/// `ILClone(pidl)`: a copy of the list.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ILClone(pidl: *const u8) -> *mut u8 {
    if pidl.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: the caller's list.
    alloc_list(&unsafe { read_list(pidl) })
}

/// `ILGetSize(pidl)`: the bytes of the list with its terminator; 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ILGetSize(pidl: *const u8) -> u32 {
    if pidl.is_null() {
        return 0;
    }
    // SAFETY: the caller's list.
    unsafe { read_list(pidl) }.len() as u32
}
