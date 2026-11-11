//! Our `shlwapi.dll` (ADR 0155, step 9). Chrome's browser delay-loads eight of its functions;
//! without the DLL the delay load fails and the browser ends itself with a breakpoint. The
//! three it calls in a headless run are `PathMatchSpecW`, `QISearch` (ordinal 219) and `IsOS`
//! (ordinal 437); the others are there so that their delay loads bind.
//!
//! The behaviour is Learn's (`shlwapi.h`); the logic is `poly_kernelbase::shlwapi`, host-tested.
//! Nothing here was observed on Windows.

#![no_std]
#![allow(clippy::missing_safety_doc)]
// rustc exports every `#[no_mangle]` function by name; `build.rs` exports three again with their
// ordinals, which lld-link keeps (the first specification), noting that the name came twice.
#![allow(linker_messages)]

extern crate alloc;

use alloc::vec::Vec;
use poly_kernelbase::shlwapi as logic;

poly_win32_rt::process_heap_allocator!();

/// `S_OK`, `E_NOINTERFACE`, `E_POINTER`, `E_INVALIDARG` (`winerror.h`).
const S_OK: u32 = 0;
const E_NOINTERFACE: u32 = 0x8000_4002;
const E_POINTER: u32 = 0x8000_4003;
const E_INVALIDARG: u32 = 0x8007_0057;
/// `HRESULT_FROM_WIN32(ERROR_NO_ASSOCIATION)` (`winerror.h`: 1155): no application is
/// associated with the file type.
const E_NO_ASSOCIATION: u32 = 0x8007_0483;

/// The characters of a NUL-terminated string (empty for a null pointer).
///
/// # Safety
/// `p` is null or a NUL-terminated UTF-16 string that lives as long as `'a`.
unsafe fn wide<'a>(p: *const u16) -> &'a [u16] {
    if p.is_null() {
        return &[];
    }
    let mut n = 0;
    // SAFETY: the caller's contract.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// `PathMatchSpecW(pszFile, pszSpec)`: TRUE if the name matches the wildcard specification.
/// A null string matches nothing.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathMatchSpecW(file: *const u16, spec: *const u16) -> i32 {
    if file.is_null() || spec.is_null() {
        return 0;
    }
    // SAFETY: the caller's strings.
    i32::from(logic::path_match_spec(unsafe { wide(file) }, unsafe { wide(spec) }))
}

/// `PathFindExtensionW(pszPath)`: the address of the `.` of the extension, or of the
/// terminating NUL; NULL for a null path.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathFindExtensionW(path: *const u16) -> *const u16 {
    if path.is_null() {
        return path;
    }
    // SAFETY: the caller's string; the index is at most its length.
    unsafe { path.add(logic::find_extension(wide(path))) }
}

/// `PathFindFileNameW(pszPath)`: the address where the file name starts (the path itself when
/// it has no separator the name could follow).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathFindFileNameW(path: *const u16) -> *const u16 {
    if path.is_null() {
        return path;
    }
    // SAFETY: the caller's string; the index is at most its length.
    unsafe { path.add(logic::find_file_name(wide(path))) }
}

/// `PathRemoveExtensionW(pszPath)`: the extension is cut off in place.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn PathRemoveExtensionW(path: *mut u16) {
    if path.is_null() {
        return;
    }
    // SAFETY: the caller's writable string; the index is at most its length, where the NUL is.
    unsafe {
        let at = logic::find_extension(wide(path));
        path.add(at).write(0);
    }
}

/// `AssocQueryStringW(flags, str, pszAssoc, pszExtra, pszOut, pcchOut)`: Poly has no file
/// associations for Windows programs yet, so nothing is associated with any type. Learn:
/// `E_POINTER` when `pcchOut` is null.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AssocQueryStringW(
    _flags: u32,
    _what: u32,
    _assoc: *const u16,
    _extra: *const u16,
    _out: *mut u16,
    count: *mut u32,
) -> u32 {
    if count.is_null() { E_POINTER } else { E_NO_ASSOCIATION }
}

/// `SHCreateMemStream(pInit, cbInit)` (ordinal 12): "NULL on failure" (Learn). Poly's COM has no
/// stream object yet, so no stream can be made.
#[unsafe(no_mangle)]
pub extern "system" fn SHCreateMemStream(_init: *const u8, _size: u32) -> *mut u8 {
    core::ptr::null_mut()
}

/// `IsOS(dwOS)` (ordinal 437).
#[unsafe(no_mangle)]
pub extern "system" fn IsOS(os: u32) -> i32 {
    i32::from(logic::is_os(os))
}

/// `QITAB` (`shlwapi.h`): `const IID *piid; DWORD dwOffset;`, 16 bytes on x64.
#[repr(C)]
pub struct QiTab {
    piid: *const [u8; 16],
    offset: u32,
}

/// `QISearch(that, pqit, riid, ppv)` (ordinal 219): the table-driven `QueryInterface`. The
/// interface found is `that + dwOffset`; its `AddRef` (the second method of every COM
/// interface) is called.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QISearch(
    that: *mut u8,
    table: *const QiTab,
    riid: *const [u8; 16],
    out: *mut *mut u8,
) -> u32 {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(core::ptr::null_mut()) };
    if that.is_null() || table.is_null() || riid.is_null() {
        return E_INVALIDARG;
    }
    // The entries up to the one whose piid is NULL (Learn; at most 256 are read).
    let mut entries: Vec<([u8; 16], u32)> = Vec::new();
    // SAFETY: the table ends with an entry whose piid is NULL; each piid points to an IID.
    unsafe {
        let mut i = 0;
        while i < 256 && !(*table.add(i)).piid.is_null() {
            let e = &*table.add(i);
            entries.push((e.piid.read_unaligned(), e.offset));
            i += 1;
        }
    }
    // SAFETY: the caller's IID.
    let Some(offset) = logic::qi_search(&entries, &unsafe { riid.read_unaligned() }) else { return E_NOINTERFACE };
    // SAFETY: `that + offset` is an interface pointer of the caller's object: a pointer to a
    // vtable whose second entry is AddRef.
    unsafe {
        let interface = that.add(offset as usize);
        let vtable = interface.cast::<*const unsafe extern "system" fn(*mut u8) -> u32>().read();
        (*vtable.add(1))(interface);
        out.write(interface);
    }
    S_OK
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn shlwapi_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
