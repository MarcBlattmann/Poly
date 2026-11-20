//! Our `shell32.dll` (ADR 0155): `CommandLineToArgvW` and `SHGetFolderPathW`, which Chrome's
//! processes call when they start (its crash handler reads its arguments with the first and
//! finds the user's local application data with the second), and since step 9
//! `SHGetKnownFolderPath`, `SHGetPropertyStoreForWindow` (the browser treats a failure of either
//! as fatal) and `Shell_NotifyIconW` (delay-loaded: the export has to exist). The behaviour is
//! Learn's (`shellapi.h`, `shlobj_core.h`, `propsys.h`); the logic is
//! `poly_kernelbase::{shell, knownfolder, propstore}`. Chrome's downloads (ADR 0223) added
//! shell items ([`item`]), the file operation object ([`fileop`]), the taskbar and jump lists
//! ([`taskbar`]), the desktop folder and item lists ([`folder`]) and `ShellExecuteExW`
//! ([`exec`]); the classes reach `ole32` through `DllGetClassObject` ([`com`]), Poly's desktop
//! through win32k's link. Every other shell32 function is bound to polystub by the loader.

#![no_std]
#![allow(clippy::missing_safety_doc)]
// rustc exports every `#[no_mangle]` function by name; `build.rs` exports three again with their
// ordinals, which lld-link keeps (the first specification), noting that the name came twice.
#![allow(linker_messages)]

extern crate alloc;

mod elevate;

use alloc::vec::Vec;
use poly_kernelbase::shell;
use poly_win32_rt as _;

/// `ERROR_INVALID_PARAMETER` (`winerror.h`).
const ERROR_INVALID_PARAMETER: u32 = 87;
/// `ERROR_NOT_ENOUGH_MEMORY` (`winerror.h`).
const ERROR_NOT_ENOUGH_MEMORY: u32 = 8;
/// `MAX_PATH` (`minwindef.h`): the size of the buffer `SHGetFolderPathW` fills.
const MAX_PATH: usize = 260;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetProcessHeap() -> *mut u8;
    fn HeapAlloc(heap: *mut u8, flags: u32, bytes: usize) -> *mut u8;
    fn HeapFree(heap: *mut u8, flags: u32, p: *mut u8) -> i32;
    fn LocalAlloc(flags: u32, bytes: usize) -> *mut u8;
    fn SetLastError(error: u32);
    fn GetModuleFileNameW(module: *mut u8, buf: *mut u16, size: u32) -> u32;
    fn GetEnvironmentVariableW(name: *const u16, buf: *mut u16, size: u32) -> u32;
    fn CreateDirectoryW(name: *const u16, security: *const u8) -> i32;
    fn GetFileAttributesW(name: *const u16) -> u32;
    fn GetLastError() -> u32;
    fn AcquireSRWLockExclusive(lock: *mut usize);
    fn ReleaseSRWLockExclusive(lock: *mut usize);
}

#[cfg_attr(target_arch = "x86", link(name = "user32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "user32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn IsWindow(hwnd: usize) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "advapi32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "advapi32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetTokenInformation(token: *mut u8, class: u32, info: *mut u8, length: u32, ret: *mut u32) -> i32;
}

#[cfg_attr(target_arch = "x86", link(name = "ole32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "ole32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn CoTaskMemAlloc(bytes: usize) -> *mut u8;
    fn CoGetMalloc(context: u32, out: *mut *mut u8) -> i32;
}

/// The process heap (16-byte aligned) for `alloc`.
struct ProcessHeap;

// SAFETY: HeapAlloc returns 16-byte aligned blocks; stricter alignments are refused.
unsafe impl core::alloc::GlobalAlloc for ProcessHeap {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        // SAFETY: the process heap.
        unsafe { HeapAlloc(GetProcessHeap(), 0, layout.size().max(1)) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _: core::alloc::Layout) {
        // SAFETY: a block from `alloc`.
        unsafe { HeapFree(GetProcessHeap(), 0, ptr) };
    }
}

#[global_allocator]
static ALLOC: ProcessHeap = ProcessHeap;

/// `CommandLineToArgvW`: the arguments of a command line, in one `LocalAlloc` block the
/// caller frees with `LocalFree`. An empty command line gives the executable's path.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CommandLineToArgvW(cmd_line: *const u16, num_args: *mut i32) -> *mut *mut u16 {
    if cmd_line.is_null() || num_args.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_INVALID_PARAMETER) };
        return core::ptr::null_mut();
    }
    let mut len = 0;
    // SAFETY: a NUL-terminated string.
    while unsafe { *cmd_line.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: `len` characters before the NUL.
    let mut args = shell::command_line_to_argv(unsafe { core::slice::from_raw_parts(cmd_line, len) });
    if args.is_empty() {
        let mut path = [0u16; 1024];
        // SAFETY: the buffer holds 1024 characters.
        let n = unsafe { GetModuleFileNameW(core::ptr::null_mut(), path.as_mut_ptr(), 1024) } as usize;
        args.push(Vec::from(&path[..n.min(1023)]));
    }
    let size = shell::argv_block_size(&args, core::mem::size_of::<usize>());
    // SAFETY: LMEM_FIXED (0): the handle is the block's address.
    let block = unsafe { LocalAlloc(0, size) };
    if block.is_null() {
        // SAFETY: a plain call.
        unsafe { SetLastError(ERROR_NOT_ENOUGH_MEMORY) };
        return core::ptr::null_mut();
    }
    // SAFETY: `block` is `size` writable bytes; `num_args` is the caller's out-pointer.
    unsafe {
        shell::write_argv_block(
            &args,
            block as usize as u64,
            core::mem::size_of::<usize>(),
            core::slice::from_raw_parts_mut(block, size),
        );
        num_args.write(args.len() as i32);
    }
    block.cast()
}

/// `SHGetFolderPathW`: the path of a `CSIDL` folder (no trailing backslash) in the caller's
/// `MAX_PATH` buffer. `hwnd`, the token (the current user) and `SHGFP_TYPE_*` (current and
/// default paths are the same on Poly) do not change the result.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFolderPathW(
    _hwnd: *mut u8,
    folder: i32,
    _token: *mut u8,
    _flags: u32,
    path: *mut u16,
) -> u32 {
    if path.is_null() {
        return shell::E_INVALIDARG;
    }
    let f = match shell::folder(folder as u32) {
        Ok(f) => f,
        Err(e) => return e,
    };
    let mut name = [0u16; 32];
    for (to, c) in name.iter_mut().zip(f.variable.encode_utf16()) {
        *to = c;
    }
    let mut out = [0u16; MAX_PATH];
    // SAFETY: `name` is NUL-terminated (shorter than 32); `out` holds MAX_PATH characters.
    let n = unsafe { GetEnvironmentVariableW(name.as_ptr(), out.as_mut_ptr(), MAX_PATH as u32) } as usize;
    let suffix = f.suffix.encode_utf16().count();
    if n == 0 || n + suffix >= MAX_PATH {
        return shell::E_FAIL;
    }
    for (to, c) in out[n..].iter_mut().zip(f.suffix.encode_utf16()) {
        *to = c;
    }
    if folder as u32 & shell::CSIDL_FLAG_CREATE != 0 {
        // SAFETY: `out` is NUL-terminated. An existing folder is fine.
        unsafe { CreateDirectoryW(out.as_ptr(), core::ptr::null()) };
    }
    // SAFETY: Learn: the caller's buffer is MAX_PATH characters.
    unsafe { core::ptr::copy_nonoverlapping(out.as_ptr(), path, n + suffix + 1) };
    shell::S_OK
}

/// `SHGetFolderPathA`: `SHGetFolderPathW` with the path in the ANSI code page (1252: a path of the
/// profile has no character it lacks).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetFolderPathA(
    hwnd: *mut u8,
    folder: i32,
    token: *mut u8,
    flags: u32,
    path: *mut u8,
) -> u32 {
    if path.is_null() {
        return shell::E_INVALIDARG;
    }
    let mut wide = [0u16; MAX_PATH];
    // SAFETY: a MAX_PATH buffer.
    let hr = unsafe { SHGetFolderPathW(hwnd, folder, token, flags, wide.as_mut_ptr()) };
    if hr != shell::S_OK {
        return hr;
    }
    let n = wide.iter().position(|&c| c == 0).unwrap_or(MAX_PATH - 1);
    let ansi = poly_kernelbase::text::to_ansi(&wide[..n]);
    // SAFETY: Learn: the caller's buffer is MAX_PATH bytes.
    unsafe {
        core::ptr::copy_nonoverlapping(ansi.as_ptr(), path, ansi.len());
        path.add(ansi.len()).write(0);
    }
    shell::S_OK
}

// ----- SHGetKnownFolderPath (step 9) --------------------------------------------------------------

/// The value of an environment variable, without the NUL (`None` when it is not set).
fn variable(name: &str) -> Option<Vec<u16>> {
    let mut z: Vec<u16> = name.encode_utf16().collect();
    z.push(0);
    let mut out = [0u16; MAX_PATH];
    // SAFETY: `z` is NUL-terminated; `out` holds MAX_PATH characters.
    let n = unsafe { GetEnvironmentVariableW(z.as_ptr(), out.as_mut_ptr(), MAX_PATH as u32) } as usize;
    (n != 0 && n < MAX_PATH).then(|| out[..n].to_vec())
}

/// Whether directory `path` (NUL-terminated) exists.
fn directory_exists(path: &[u16]) -> bool {
    /// `FILE_ATTRIBUTE_DIRECTORY`, `INVALID_FILE_ATTRIBUTES` (`fileapi.h`).
    const DIRECTORY: u32 = 0x10;
    const INVALID: u32 = u32::MAX;
    // SAFETY: NUL-terminated.
    let a = unsafe { GetFileAttributesW(path.as_ptr()) };
    a != INVALID && a & DIRECTORY != 0
}

/// Whose folders `hToken` asks for (`specs/windows/shell32.dll/SHGetKnownFolderPath.yaml`, rule
/// `token`): NULL is the calling user, -1 the default user, and anything else has to be a token
/// handle opened with `TOKEN_QUERY` (its user is the only one Poly has). The failures:
/// `E_ACCESSDENIED` without that right, `E_HANDLE` for a value that is no token.
fn whose_folders(token: *mut u8) -> Result<bool, u32> {
    use poly_kernelbase::knownfolder as kf;
    if token.is_null() {
        return Ok(false);
    }
    if token as usize == usize::MAX {
        return Ok(true);
    }
    /// `TokenType` (`TOKEN_INFORMATION_CLASS`, `winnt.h`): needs `TOKEN_QUERY`, nothing else.
    const TOKEN_TYPE: u32 = 8;
    let (mut info, mut ret) = ([0u8; 4], 0u32);
    // SAFETY: a 4-byte buffer and a live out variable.
    if unsafe { GetTokenInformation(token, TOKEN_TYPE, info.as_mut_ptr(), 4, &mut ret) } != 0 {
        return Ok(false);
    }
    // SAFETY: a plain call.
    Err(if unsafe { GetLastError() } == 5 { kf::E_ACCESSDENIED } else { kf::E_HANDLE })
}

/// `SHGetKnownFolderPath(rfid, dwFlags, hToken, ppszPath)`: the folder's path in a
/// `CoTaskMemAlloc` block, without a trailing backslash. A folder whose directory is not there
/// is `HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)` unless `KF_FLAG_DONT_VERIFY` says not to look;
/// `KF_FLAG_CREATE` makes it. `*ppszPath` is NULL on failure.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetKnownFolderPath(
    id: *const [u8; 16],
    flags: u32,
    token: *mut u8,
    path: *mut *mut u16,
) -> u32 {
    use poly_kernelbase::knownfolder as kf;
    if path.is_null() {
        return kf::E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { path.write(core::ptr::null_mut()) };
    if id.is_null() {
        return kf::E_INVALIDARG;
    }
    let default_user = match whose_folders(token) {
        Ok(d) => d,
        Err(e) => return e,
    };
    // SAFETY: the caller's KNOWNFOLDERID (16 bytes, any alignment).
    let id = unsafe { id.read_unaligned() };
    // An id that is no folder, and a virtual folder, fail whatever the flags (the sheet).
    let mut folder = match kf::known_folder_path(&id, variable) {
        Ok(p) => p,
        Err(e) => return e,
    };
    if !kf::flags_accepted(flags) {
        return kf::E_INVALIDARG;
    }
    if default_user && let Some(profile) = variable("USERPROFILE") {
        folder = kf::for_default_user(&folder, &profile);
    }
    folder.push(0);
    if flags & kf::KF_FLAG_CREATE != 0 {
        // SAFETY: NUL-terminated. An existing folder is fine.
        unsafe { CreateDirectoryW(folder.as_ptr(), core::ptr::null()) };
    } else if flags & kf::KF_FLAG_DONT_VERIFY == 0 && !directory_exists(&folder) {
        return kf::E_FILE_NOT_FOUND;
    }
    // SAFETY: a plain allocation.
    let block = unsafe { CoTaskMemAlloc(folder.len() * 2) }.cast::<u16>();
    if block.is_null() {
        return kf::E_OUTOFMEMORY;
    }
    // SAFETY: `block` holds `folder.len()` characters; the out-pointer is the caller's.
    unsafe {
        core::ptr::copy_nonoverlapping(folder.as_ptr(), block, folder.len());
        path.write(block);
    }
    kf::S_OK
}

// ----- SHGetPropertyStoreForWindow (step 9) -------------------------------------------------------

use poly_kernelbase::propstore::{self as ps, Kind, Store, Value};

/// The lock of [`STORES`] (an `SRWLOCK`, which is one pointer and starts as zero).
static STORES_LOCK: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// The property stores of this process's windows, by window handle.
struct Stores(core::cell::UnsafeCell<Vec<(usize, Store)>>);
// SAFETY: every access is under `STORES_LOCK`.
unsafe impl Sync for Stores {}
static STORES: Stores = Stores(core::cell::UnsafeCell::new(Vec::new()));

/// Forget the stores of windows that are gone: "the values go away with the window".
fn drop_dead_stores(stores: &mut Vec<(usize, Store)>) {
    // SAFETY: a plain call, on any value.
    stores.retain(|(w, _)| unsafe { IsWindow(*w) } != 0);
}

/// Run `f` on the store of `hwnd` (made on first use), under the lock. The store of a window
/// that is gone is empty again.
fn with_store<R>(hwnd: usize, f: impl FnOnce(&mut Store) -> R) -> R {
    // SAFETY: the SRW lock's word; the vector is only touched between acquire and release.
    unsafe {
        AcquireSRWLockExclusive(STORES_LOCK.as_ptr());
        let stores = &mut *STORES.0.get();
        drop_dead_stores(stores);
        let at = match stores.iter().position(|(w, _)| *w == hwnd) {
            Some(i) => i,
            None => {
                stores.push((hwnd, Store::new()));
                stores.len() - 1
            }
        };
        let r = f(&mut stores[at].1);
        ReleaseSRWLockExclusive(STORES_LOCK.as_ptr());
        r
    }
}

/// The object `SHGetPropertyStoreForWindow` returns: an `IPropertyStore` whose values are the
/// window's store.
#[repr(C)]
struct WindowStore {
    vtbl: *const StoreVtbl,
    refs: core::sync::atomic::AtomicU32,
    hwnd: usize,
}

/// `IPropertyStore`'s methods in vtable order (`propsys.h`): `IUnknown`'s three, then
/// `GetCount`, `GetAt`, `GetValue`, `SetValue`, `Commit`.
#[repr(C)]
struct StoreVtbl {
    query_interface: unsafe extern "system" fn(*mut WindowStore, *const [u8; 16], *mut *mut u8) -> u32,
    add_ref: unsafe extern "system" fn(*mut WindowStore) -> u32,
    release: unsafe extern "system" fn(*mut WindowStore) -> u32,
    get_count: unsafe extern "system" fn(*mut WindowStore, *mut u32) -> u32,
    get_at: unsafe extern "system" fn(*mut WindowStore, u32, *mut [u8; 20]) -> u32,
    get_value: unsafe extern "system" fn(*mut WindowStore, *const [u8; 20], *mut [u8; 24]) -> u32,
    set_value: unsafe extern "system" fn(*mut WindowStore, *const [u8; 20], *const [u8; 24]) -> u32,
    commit: unsafe extern "system" fn(*mut WindowStore) -> u32,
}

static STORE_VTBL: StoreVtbl = StoreVtbl {
    query_interface: store_query_interface,
    add_ref: store_add_ref,
    release: store_release,
    get_count: store_get_count,
    get_at: store_get_at,
    get_value: store_get_value,
    set_value: store_set_value,
    commit: store_commit,
};

unsafe extern "system" fn store_query_interface(
    this: *mut WindowStore,
    iid: *const [u8; 16],
    out: *mut *mut u8,
) -> u32 {
    if out.is_null() {
        return ps::E_POINTER;
    }
    // SAFETY: the caller's out-pointer and IID.
    unsafe {
        out.write(core::ptr::null_mut());
        if iid.is_null() {
            return ps::E_INVALIDARG;
        }
        let iid = iid.read_unaligned();
        if iid != ps::IID_IPROPERTYSTORE && iid != poly_kernelbase::shlwapi::IID_IUNKNOWN {
            return ps::E_NOINTERFACE;
        }
        store_add_ref(this);
        out.write(this.cast());
    }
    ps::S_OK
}

unsafe extern "system" fn store_add_ref(this: *mut WindowStore) -> u32 {
    // SAFETY: a live object.
    unsafe { (*this).refs.fetch_add(1, core::sync::atomic::Ordering::AcqRel) + 1 }
}

unsafe extern "system" fn store_release(this: *mut WindowStore) -> u32 {
    // SAFETY: a live object; freed when the last reference goes (the window's values stay).
    unsafe {
        let left = (*this).refs.fetch_sub(1, core::sync::atomic::Ordering::AcqRel) - 1;
        if left == 0 {
            HeapFree(GetProcessHeap(), 0, this.cast());
        }
        left
    }
}

unsafe extern "system" fn store_get_count(this: *mut WindowStore, count: *mut u32) -> u32 {
    if count.is_null() {
        return ps::E_POINTER;
    }
    // SAFETY: a live object.
    let counted = with_store(unsafe { (*this).hwnd }, |s| s.count_checked());
    // The sheet: a store nothing was set in fails with E_FAIL and count 0.
    // SAFETY: the caller's out-pointer.
    unsafe { count.write_unaligned(counted.unwrap_or(0)) };
    counted.err().unwrap_or(ps::S_OK)
}

unsafe extern "system" fn store_get_at(this: *mut WindowStore, index: u32, key: *mut [u8; 20]) -> u32 {
    if key.is_null() {
        return ps::E_POINTER;
    }
    // SAFETY: a live object; the caller's PROPERTYKEY.
    match with_store(unsafe { (*this).hwnd }, |s| s.key_at(index)) {
        Some(k) => {
            // SAFETY: as above.
            unsafe { key.write_unaligned(k) };
            ps::S_OK
        }
        None => ps::E_INVALIDARG,
    }
}

unsafe extern "system" fn store_get_value(this: *mut WindowStore, key: *const [u8; 20], out: *mut [u8; 24]) -> u32 {
    if key.is_null() || out.is_null() {
        return ps::E_POINTER;
    }
    // SAFETY: a live object; the caller's key.
    let value = with_store(unsafe { (*this).hwnd }, |s| s.get(&unsafe { key.read_unaligned() }).cloned());
    let mut variant = [0u8; 24];
    match value {
        // Learn: a key the store does not have gives VT_EMPTY and S_OK.
        None => {}
        Some(Value::Scalar(b)) => variant = b,
        Some(Value::Text(t)) => {
            // SAFETY: a plain allocation of the string with its NUL.
            let p = unsafe { CoTaskMemAlloc((t.len() + 1) * 2) }.cast::<u16>();
            if p.is_null() {
                return ps::E_OUTOFMEMORY;
            }
            // SAFETY: `p` holds `t.len() + 1` characters.
            unsafe {
                core::ptr::copy_nonoverlapping(t.as_ptr(), p, t.len());
                p.add(t.len()).write(0);
            }
            variant[0..2].copy_from_slice(&ps::VT_LPWSTR.to_le_bytes());
            variant[8..16].copy_from_slice(&(p as u64).to_le_bytes());
        }
    }
    // SAFETY: the caller's PROPVARIANT.
    unsafe { out.write_unaligned(variant) };
    ps::S_OK
}

unsafe extern "system" fn store_set_value(this: *mut WindowStore, key: *const [u8; 20], value: *const [u8; 24]) -> u32 {
    if key.is_null() || value.is_null() {
        return ps::E_POINTER;
    }
    // SAFETY: the caller's key and PROPVARIANT.
    let (key, variant) = unsafe { (key.read_unaligned(), value.read_unaligned()) };
    let new = match ps::kind(u16::from_le_bytes([variant[0], variant[1]])) {
        Kind::Empty => None,
        // A number set into a property that is a string reads back as its text.
        Kind::Scalar => match ps::integer_text(&variant).filter(|_| ps::is_text_key(&key)) {
            Some(text) => Some(Value::Text(text)),
            None => Some(Value::Scalar(variant)),
        },
        Kind::Text => {
            let p = u64::from_le_bytes(variant[8..16].try_into().expect("8 bytes")) as *const u16;
            if p.is_null() {
                return ps::E_INVALIDARG;
            }
            let mut n = 0;
            // SAFETY: a VT_LPWSTR points to a NUL-terminated string.
            while unsafe { *p.add(n) } != 0 {
                n += 1;
            }
            // SAFETY: `n` characters before the NUL.
            Some(Value::Text(unsafe { core::slice::from_raw_parts(p, n) }.to_vec()))
        }
        Kind::Unsupported => return ps::E_INVALIDARG,
    };
    // SAFETY: a live object.
    let hwnd = unsafe { (*this).hwnd };
    // The window is gone: its store takes no value (the sheet: `HRESULT_FROM_WIN32(
    // ERROR_INVALID_WINDOW_HANDLE)`).
    // SAFETY: a plain call, on any value.
    if unsafe { IsWindow(hwnd) } == 0 {
        return ps::E_INVALID_WINDOW;
    }
    with_store(hwnd, |s| s.set(&key, new));
    ps::S_OK
}

unsafe extern "system" fn store_commit(_this: *mut WindowStore) -> u32 {
    // The values are in place as soon as they are set.
    ps::S_OK
}

/// `SHGetPropertyStoreForWindow(hwnd, riid, ppv)`: an `IPropertyStore` (or its `IUnknown`) on
/// the window's properties. The properties live in this process, per window handle; a null
/// window is `E_INVALIDARG`, another interface `E_NOINTERFACE`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetPropertyStoreForWindow(hwnd: usize, iid: *const [u8; 16], out: *mut *mut u8) -> u32 {
    if out.is_null() {
        return ps::E_POINTER;
    }
    // A NULL window leaves `*ppv` as it was (the sheet).
    if hwnd == 0 {
        return ps::E_INVALIDARG;
    }
    // SAFETY: the caller's out-pointer.
    unsafe { out.write(core::ptr::null_mut()) };
    if iid.is_null() {
        return ps::E_INVALIDARG;
    }
    // SAFETY: the process heap; the block is one WindowStore.
    let object = unsafe { HeapAlloc(GetProcessHeap(), 0, core::mem::size_of::<WindowStore>()) }.cast::<WindowStore>();
    if object.is_null() {
        return ps::E_OUTOFMEMORY;
    }
    // SAFETY: `object` is a fresh block of the right size and alignment (16 bytes).
    unsafe {
        object.write(WindowStore { vtbl: &STORE_VTBL, refs: core::sync::atomic::AtomicU32::new(1), hwnd });
        // The caller's interface takes its own reference; ours goes.
        let hr = store_query_interface(object, iid, out);
        store_release(object);
        hr
    }
}

/// `ILFree(pidl)` (ordinal 155): "Frees an ITEMIDLIST structure allocated by the Shell" (Learn);
/// the Shell's allocator is the COM task allocator, which is the process heap here. NULL is
/// ignored.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ILFree(pidl: *mut u8) {
    if !pidl.is_null() {
        // SAFETY: a block of the task allocator.
        unsafe { HeapFree(GetProcessHeap(), 0, pidl) };
    }
}

// ----- item ID lists ------------------------------------------------------------------------------

/// Poly's `ITEMIDLIST` (`shtypes.h`) for a file system object: one `SHITEMID` (`cb`, a `USHORT` that
/// counts itself, then `cb - 2` bytes) holding the object's full path in UTF-16 without the NUL, then
/// the `cb` 0 that ends a list. The Shell's lists are opaque to programs: they pass them back to the
/// functions here (`SHGetPathFromIDListW`) and free them (`ILFree`).
pub(crate) unsafe fn pidl_of_path(path: &[u16]) -> *mut u8 {
    let cb = 2 + 2 * path.len();
    if cb > 0xFFFF {
        return core::ptr::null_mut();
    }
    // SAFETY: the process heap; the block holds the item and the terminator.
    unsafe {
        let p = HeapAlloc(GetProcessHeap(), 0, cb + 2);
        if p.is_null() {
            return p;
        }
        p.cast::<u16>().write_unaligned(cb as u16);
        core::ptr::copy_nonoverlapping(path.as_ptr().cast::<u8>(), p.add(2), 2 * path.len());
        p.add(cb).cast::<u16>().write_unaligned(0);
        p
    }
}

/// The desktop's list: no item, only the `cb` 0 that ends a list (`SHGetPathFromIDListW` gives
/// no path for it).
pub(crate) fn pidl_of_desktop() -> *mut u8 {
    // SAFETY: the process heap (the task allocator); two bytes, both 0.
    unsafe {
        let p = HeapAlloc(GetProcessHeap(), 0, 2);
        if !p.is_null() {
            p.cast::<u16>().write_unaligned(0);
        }
        p
    }
}

/// The path one of Poly's lists holds (`None` for the desktop's empty list, NULL, or a list
/// whose item has no room for a path).
pub(crate) unsafe fn pidl_path(pidl: *const u8) -> Option<Vec<u16>> {
    if pidl.is_null() {
        return None;
    }
    // SAFETY: a list the Shell functions here made: `cb`, then `cb - 2` bytes of path.
    unsafe {
        let cb = usize::from(pidl.cast::<u16>().read_unaligned());
        if cb < 4 {
            return None;
        }
        let units = (cb - 2) / 2;
        Some((0..units).map(|i| pidl.add(2 + 2 * i).cast::<u16>().read_unaligned()).collect())
    }
}

/// `SHGetSpecialFolderLocation(hwnd, csidl, ppidl)`: the list of a `CSIDL_*` folder, `S_OK`, or the
/// `HRESULT` `SHGetFolderPathW` gives for a folder Poly lacks.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetSpecialFolderLocation(_hwnd: *mut u8, csidl: i32, out: *mut *mut u8) -> u32 {
    if out.is_null() {
        return shell::E_INVALIDARG;
    }
    let mut path = [0u16; MAX_PATH];
    // SAFETY: a MAX_PATH buffer.
    let hr = unsafe { SHGetFolderPathW(core::ptr::null_mut(), csidl, core::ptr::null_mut(), 0, path.as_mut_ptr()) };
    if hr != shell::S_OK {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
        return hr;
    }
    let n = path.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
    // SAFETY: the process heap; the caller's out parameter.
    let pidl = unsafe { pidl_of_path(&path[..n]) };
    unsafe { *out = pidl };
    if pidl.is_null() { 0x8007_000E } else { shell::S_OK }
}

/// `SHGetPathFromIDListW(pidl, pszPath)`: the path of one of Poly's lists into a `MAX_PATH` buffer;
/// FALSE for a list that is not one (an empty list, or an item with no room for a path).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetPathFromIDListW(pidl: *const u8, path: *mut u16) -> i32 {
    if pidl.is_null() || path.is_null() {
        return 0;
    }
    // SAFETY: a list the Shell functions here made.
    unsafe {
        let cb = usize::from(pidl.cast::<u16>().read_unaligned());
        if cb < 4 || (cb - 2) / 2 >= MAX_PATH {
            return 0;
        }
        let units = (cb - 2) / 2;
        core::ptr::copy_nonoverlapping(pidl.add(2), path.cast::<u8>(), 2 * units);
        path.add(units).write(0);
    }
    1
}

/// `ILCreateFromPathW(pszPath)`: the list of a path, NULL for a path of 0 characters or more than `MAX_PATH`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ILCreateFromPathW(path: *const u16) -> *mut u8 {
    if path.is_null() {
        return core::ptr::null_mut();
    }
    let mut n = 0;
    // SAFETY: a NUL-terminated path.
    unsafe {
        while n < MAX_PATH && *path.add(n) != 0 {
            n += 1;
        }
        if n == 0 || n >= MAX_PATH {
            return core::ptr::null_mut();
        }
        pidl_of_path(core::slice::from_raw_parts(path, n))
    }
}

/// `SHGetSpecialFolderPathW(hwnd, pszPath, csidl, fCreate)`: the path of a `CSIDL_*` folder into a
/// `MAX_PATH` buffer, TRUE on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetSpecialFolderPathW(_hwnd: *mut u8, path: *mut u16, csidl: i32, create: i32) -> i32 {
    let folder = if create != 0 { csidl | shell::CSIDL_FLAG_CREATE as i32 } else { csidl };
    // SAFETY: the caller's MAX_PATH buffer.
    i32::from(unsafe { SHGetFolderPathW(core::ptr::null_mut(), folder, core::ptr::null_mut(), 0, path) } == shell::S_OK)
}

/// `Shell_NotifyIconW(dwMessage, lpData)`: "Returns TRUE if successful, or FALSE otherwise"
/// (Learn). Poly's panel has no notification area for Windows programs yet, so no icon can be
/// added, changed or removed.
#[unsafe(no_mangle)]
pub extern "system" fn Shell_NotifyIconW(_message: u32, _data: *const u8) -> i32 {
    0
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn shell32_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}

mod assoc;
mod com;
mod delayed;
mod dispatch;
mod exec;
mod fileop;
mod folder;
mod idle;
mod item;
mod link;
mod taskbar;

mod apps;

/// `SHGetMalloc(ppMalloc)`: the shell's allocator is the task allocator (Learn: "Retrieves a pointer to
/// the Shell's IMalloc interface", which `CoGetMalloc` with `MEMCTX_TASK` returns too).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetMalloc(out: *mut *mut u8) -> i32 {
    // SAFETY: the caller's out parameter.
    unsafe { CoGetMalloc(1, out) }
}
