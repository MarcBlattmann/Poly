//! Shell items (`IShellItem`, `IShellItem2`) of file-system objects and of the desktop:
//! `SHCreateItemFromParsingName`, `SHCreateItemFromIDList` (ADR 0223). The item keeps the path
//! it was made for; its names, attributes, parent and comparisons are
//! [`poly_kernelbase::shellitem`]'s. Sheet: `specs/windows/shell32.dll/SHCreateItemFromParsingName.yaml`.

use crate::com::*;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_kernelbase::shellitem::{self as si, Parsed};

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn GetFileAttributesW(name: *const u16) -> u32;
}

/// `IID_IShellItem` (`shobjidl_core.h`).
pub const IID_ISHELL_ITEM: Guid = guid(0x4382_6D1E, 0xE718, 0x42EE, [0xBC, 0x55, 0xA1, 0xE2, 0x61, 0xC3, 0x7B, 0xFE]);
/// `IID_IShellItem2` (`shobjidl_core.h`).
pub const IID_ISHELL_ITEM2: Guid = guid(0x7E9F_B0D3, 0x919F, 0x4307, [0xAB, 0x2E, 0x9B, 0x18, 0x60, 0x31, 0x0C, 0x93]);
/// `MK_E_NOOBJECT` (`winerror.h`): the desktop has no parent.
const MK_E_NOOBJECT: Hresult = 0x8004_01E5_u32 as i32;
const INVALID_FILE_ATTRIBUTES: u32 = 0xFFFF_FFFF;
const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;

/// A shell item.
#[repr(C)]
pub struct ShellItem {
    head: Head,
    /// The object's path (`None`: the desktop).
    path: Option<Vec<u16>>,
    folder: bool,
}

static ITEM_IIDS: [Guid; 2] = [IID_ISHELL_ITEM, IID_ISHELL_ITEM2];

/// `IShellItem` after `IUnknown`: `BindToHandler` 3, `GetParent` 4, `GetDisplayName` 5,
/// `GetAttributes` 6, `Compare` 7; `IShellItem2`: `GetPropertyStore` 8 .. `GetBool` 20.
static ITEM_VTBL: Vtbl<21> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    bind_to_handler as *const c_void,
    get_parent as *const c_void,
    get_display_name as *const c_void,
    get_attributes as *const c_void,
    compare as *const c_void,
    not_implemented_3 as *const c_void, // GetPropertyStore(flags, riid, ppv)
    not_implemented_4 as *const c_void, // GetPropertyStoreWithCreateObject
    not_implemented_5 as *const c_void, // GetPropertyStoreForKeys
    not_implemented_3 as *const c_void, // GetPropertyDescriptionList
    not_implemented_1 as *const c_void, // Update
    not_implemented_2 as *const c_void, // GetProperty
    not_implemented_2 as *const c_void, // GetCLSID
    not_implemented_2 as *const c_void, // GetFileTime
    not_implemented_2 as *const c_void, // GetInt32
    not_implemented_2 as *const c_void, // GetString
    not_implemented_2 as *const c_void, // GetUInt32
    not_implemented_2 as *const c_void, // GetUInt64
    not_implemented_2 as *const c_void, // GetBool
]);

unsafe extern "system" fn not_implemented_1(_: *mut c_void, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_2(_: *mut c_void, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_3(_: *mut c_void, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_5(_: *mut c_void, _: usize, _: usize, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_4(_: *mut c_void, _: usize, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}

/// A new item (one reference) of `path` (`None`: the desktop).
pub fn new_item(path: Option<Vec<u16>>, folder: bool) -> *mut c_void {
    into_raw(ShellItem { head: Head::new(&ITEM_VTBL, &ITEM_IIDS, drop_box::<ShellItem>), path, folder }).cast()
}

/// Whether `path` names an existing object: `Some(true)` for a folder.
pub fn exists(path: &[u16]) -> Option<bool> {
    let name = nul(path);
    // SAFETY: a NUL-terminated name.
    let a = unsafe { GetFileAttributesW(name.as_ptr()) };
    (a != INVALID_FILE_ATTRIBUTES).then_some(a & FILE_ATTRIBUTE_DIRECTORY != 0)
}

/// The item of a parsing name, or the `HRESULT` the sheet gives for it.
pub fn item_of_name(name: &[u16]) -> Result<*mut c_void, Hresult> {
    match si::parse(name).map_err(|e| e as Hresult)? {
        Parsed::Desktop => Ok(new_item(None, true)),
        Parsed::Path(p) => {
            let folder = exists(&p).ok_or(si::E_FILE_NOT_FOUND as Hresult)?;
            Ok(new_item(Some(p), folder))
        }
    }
}

/// The file-system path of any shell item (through its `GetDisplayName(SIGDN_FILESYSPATH)`),
/// `None` when it has none.
pub unsafe fn path_of(item: *mut c_void) -> Option<Vec<u16>> {
    if item.is_null() {
        return None;
    }
    let mut text: *mut u16 = core::ptr::null_mut();
    // SAFETY: a live IShellItem (slot 5: GetDisplayName); the string is task memory.
    unsafe {
        let hr = crate::vcall!(item, 5, fn(u32, *mut *mut u16) -> Hresult, si::sigdn::FILESYSPATH, &mut text);
        if hr != S_OK || text.is_null() {
            return None;
        }
        let path = wstr(text).to_vec();
        task_free(text.cast());
        Some(path)
    }
}

unsafe fn this<'a>(p: *mut c_void) -> &'a ShellItem {
    // SAFETY: the caller's item of this table.
    unsafe { &*p.cast::<ShellItem>() }
}

unsafe extern "system" fn bind_to_handler(
    _this: *mut c_void,
    _bc: *mut c_void,
    _bhid: *const Guid,
    _iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if !out.is_null() {
        // SAFETY: the caller's out parameter.
        unsafe { *out = core::ptr::null_mut() };
    }
    E_NOTIMPL
}

unsafe extern "system" fn get_parent(p: *mut c_void, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our item; the caller's out parameter.
    unsafe {
        *out = core::ptr::null_mut();
        let Some(path) = &this(p).path else { return MK_E_NOOBJECT };
        *out = match si::parent(path) {
            Some(parent) => new_item(Some(parent), true),
            None => new_item(None, true),
        };
    }
    S_OK
}

unsafe extern "system" fn get_display_name(p: *mut c_void, sigdn: u32, out: *mut *mut u16) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our item; the caller's out parameter.
    unsafe {
        *out = core::ptr::null_mut();
        match si::display_name(this(p).path.as_deref(), sigdn) {
            Ok(text) => task_string(&text, out),
            Err(e) => e as Hresult,
        }
    }
}

unsafe extern "system" fn get_attributes(p: *mut c_void, mask: u32, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: our item; the caller's out parameter.
    unsafe {
        let item = this(p);
        let all = if item.path.is_some() { si::attributes(item.folder) } else { si::sfgao::FOLDER };
        let (got, hr) = si::masked_attributes(all, mask);
        *out = got;
        hr as Hresult
    }
}

unsafe extern "system" fn compare(p: *mut c_void, other: *mut c_void, _hint: u32, order: *mut i32) -> Hresult {
    if order.is_null() || other.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: our item, the caller's item and out parameter.
    unsafe {
        let mine = this(p).path.clone().unwrap_or_default();
        let theirs = path_of(other).unwrap_or_default();
        let o = si::compare_paths(&mine, &theirs);
        *order = o;
        if o == 0 { S_OK } else { S_FALSE }
    }
}

/// `SHCreateItemFromParsingName(pszPath, pbc, riid, ppv)`: an item of a full path (the sheet's
/// rules: COM must be started, the object must exist, some names are refused).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHCreateItemFromParsingName(
    path: *const u16,
    _bc: *mut c_void,
    iid: *const Guid,
    out: *mut *mut c_void,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if let Err(e) = com_started() {
            return e;
        }
        if path.is_null() {
            return E_INVALIDARG;
        }
        match item_of_name(wstr(path)) {
            Ok(item) => hand_out(item, iid, out),
            Err(e) => e,
        }
    }
}

/// `SHCreateItemFromIDList(pidl, riid, ppv)`: the item of one of Poly's item lists
/// ([`crate::pidl_path`]); the empty list is the desktop.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHCreateItemFromIDList(pidl: *const u8, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if pidl.is_null() {
            return E_INVALIDARG;
        }
        let item = match crate::pidl_path(pidl) {
            Some(path) => new_item(Some(path.clone()), exists(&path).unwrap_or(false)),
            None => new_item(None, true),
        };
        hand_out(item, iid, out)
    }
}
