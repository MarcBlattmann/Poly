//! The desktop folder (`SHGetDesktopFolder`, `IShellFolder::ParseDisplayName`), item lists of
//! paths (`SHParseDisplayName`) and "show in folder" (`SHOpenFolderAndSelectItems`), which on
//! Poly opens Files on the folder with the item selected (ADR 0223). Sheet:
//! `specs/windows/shell32.dll/SHParseDisplayName.yaml`.

use crate::com::*;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::AtomicU32;
use poly_kernelbase::shellitem::{self as si, Parsed};

/// `IID_IShellFolder` (`shobjidl_core.h`): {000214E6-0000-0000-C000-000000000046}.
const IID_ISHELL_FOLDER: Guid = guid(0x0002_14E6, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// `SHGDN_FORPARSING` (`shobjidl_core.h`).
const SHGDN_FORPARSING: u32 = 0x8000;
/// `STRRET_WSTR` (`shtypes.h`).
const STRRET_WSTR: u32 = 0;

/// The desktop folder: one object for the process, never freed.
#[repr(C)]
struct Desktop {
    head: Head,
}
// SAFETY: the object has no state but its atomic count.
unsafe impl Sync for Desktop {}

static FOLDER_IIDS: [Guid; 1] = [IID_ISHELL_FOLDER];

/// `IShellFolder` after `IUnknown`: `ParseDisplayName` 3, `EnumObjects` 4, `BindToObject` 5,
/// `BindToStorage` 6, `CompareIDs` 7, `CreateViewObject` 8, `GetAttributesOf` 9,
/// `GetUIObjectOf` 10, `GetDisplayNameOf` 11, `SetNameOf` 12.
static FOLDER_VTBL: Vtbl<13> = Vtbl([
    query_interface as *const c_void,
    add_ref as *const c_void,
    release as *const c_void,
    parse_display_name as *const c_void,
    not_implemented_3 as *const c_void,
    not_implemented_4 as *const c_void,
    not_implemented_4 as *const c_void,
    compare_ids as *const c_void,
    not_implemented_3 as *const c_void,
    get_attributes_of as *const c_void,
    not_implemented_6 as *const c_void,
    get_display_name_of as *const c_void,
    not_implemented_5 as *const c_void,
]);

static DESKTOP: Desktop = Desktop {
    head: Head {
        vtbl: (&FOLDER_VTBL as *const Vtbl<13>).cast(),
        refs: AtomicU32::new(1),
        iids: &FOLDER_IIDS,
        drop: None,
    },
};

unsafe extern "system" fn not_implemented_3(_: *mut c_void, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_4(_: *mut c_void, _: usize, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_5(_: *mut c_void, _: usize, _: usize, _: usize, _: usize, _: usize) -> Hresult {
    E_NOTIMPL
}
unsafe extern "system" fn not_implemented_6(
    _: *mut c_void,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
    _: usize,
) -> Hresult {
    E_NOTIMPL
}

/// The item list of a parsing name, with the asked attributes: the sheet's rules (an empty name
/// is the desktop's empty list; a name that is refused or names nothing has no list).
fn parse(name: &[u16], asked: u32) -> Result<(*mut u8, u32), Hresult> {
    let (path, folder) = match si::parse(name).map_err(|e| e as Hresult)? {
        Parsed::Desktop => return Ok((crate::pidl_of_desktop(), 0)),
        Parsed::Path(p) => {
            let folder = crate::item::exists(&p).ok_or(si::E_FILE_NOT_FOUND as Hresult)?;
            (p, folder)
        }
    };
    // SAFETY: a fresh list of the task allocator.
    let pidl = unsafe { crate::pidl_of_path(&path) };
    if pidl.is_null() {
        return Err(E_OUTOFMEMORY);
    }
    Ok((pidl, si::attributes(folder) & asked))
}

unsafe extern "system" fn parse_display_name(
    _this: *mut c_void,
    _hwnd: usize,
    _bc: *mut c_void,
    name: *const u16,
    eaten: *mut u32,
    out: *mut *mut u8,
    attributes: *mut u32,
) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if name.is_null() {
            return E_INVALIDARG;
        }
        let name = wstr(name);
        let asked = if attributes.is_null() { 0 } else { *attributes };
        // The whole name is used: `pchEaten` 0, also when it names nothing (an empty name
        // leaves it as it was).
        if !eaten.is_null() && !name.is_empty() {
            *eaten = 0;
        }
        match parse(name, asked) {
            Ok((pidl, attrs)) => {
                *out = pidl;
                if !attributes.is_null() {
                    *attributes = attrs;
                }
                S_OK
            }
            Err(e) => e,
        }
    }
}

unsafe extern "system" fn compare_ids(_this: *mut c_void, _lparam: isize, a: *const u8, b: *const u8) -> Hresult {
    // SAFETY: lists of the caller.
    let (a, b) = unsafe { (crate::pidl_path(a).unwrap_or_default(), crate::pidl_path(b).unwrap_or_default()) };
    // The order in the `HRESULT`'s code: negative, 0 or positive (Learn: "IShellFolder::CompareIDs").
    (si::compare_paths(&a, &b) & 0xFFFF) as u16 as Hresult
}

unsafe extern "system" fn get_attributes_of(_this: *mut c_void, count: u32, lists: *const *const u8, io: *mut u32) -> Hresult {
    if io.is_null() {
        return E_POINTER;
    }
    // SAFETY: `count` lists of the caller and its in/out mask.
    unsafe {
        let mut all = !0u32;
        for i in 0..count as usize {
            let path = crate::pidl_path(*lists.add(i)).unwrap_or_default();
            all &= si::attributes(crate::item::exists(&path).unwrap_or(false));
        }
        *io &= all;
    }
    S_OK
}

unsafe extern "system" fn get_display_name_of(_this: *mut c_void, pidl: *const u8, flags: u32, out: *mut u8) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's list and its `STRRET` (the type at 0, the string pointer at the
    // pointer-aligned offset after it).
    unsafe {
        let Some(path) = crate::pidl_path(pidl) else { return E_INVALIDARG };
        let text: Vec<u16> =
            if flags & SHGDN_FORPARSING != 0 && flags & 1 == 0 { path } else { si::file_name(&path).to_vec() };
        out.cast::<u32>().write_unaligned(STRRET_WSTR);
        let slot = out.add(core::mem::size_of::<usize>()).cast::<*mut u16>();
        let mut p: *mut u16 = core::ptr::null_mut();
        let hr = task_string(&text, &mut p);
        slot.write_unaligned(p);
        hr
    }
}

/// `SHGetDesktopFolder(ppshf)`: the desktop's `IShellFolder`, the same object every time.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHGetDesktopFolder(out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the static object; the caller's out parameter.
    unsafe {
        let p = (&DESKTOP as *const Desktop).cast_mut().cast::<c_void>();
        add_ref(p);
        *out = p;
    }
    S_OK
}

/// `SHParseDisplayName(pszName, pbc, ppidl, sfgaoIn, psfgaoOut)`: the item list of a path and
/// the asked attributes it has.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHParseDisplayName(
    name: *const u16,
    _bc: *mut c_void,
    out: *mut *mut u8,
    asked: u32,
    got: *mut u32,
) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's pointers.
    unsafe {
        *out = core::ptr::null_mut();
        if !got.is_null() {
            *got = 0;
        }
        if name.is_null() {
            return E_INVALIDARG;
        }
        match parse(wstr(name), asked) {
            Ok((pidl, attrs)) => {
                *out = pidl;
                if !got.is_null() {
                    *got = attrs;
                }
                S_OK
            }
            Err(e) => e,
        }
    }
}

/// The highest `OFASI_*` flag (`shlobj_core.h`: `OFASI_EDIT` 1, `OFASI_OPENDESKTOP` 2).
const OFASI_ALL: u32 = 0x3;

/// `SHOpenFolderAndSelectItems(pidlFolder, cidl, apidl, dwFlags)`: Files opens the folder with
/// the item selected (ADR 0223). With `cidl` 0 `pidlFolder` is the item itself; else the items
/// are in that folder (Poly selects the first).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SHOpenFolderAndSelectItems(
    folder: *const u8,
    count: u32,
    items: *const *const u8,
    flags: u32,
) -> Hresult {
    if folder.is_null() || flags & !OFASI_ALL != 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's lists.
    let target = unsafe {
        let Some(folder) = crate::pidl_path(folder) else { return E_INVALIDARG };
        if count == 0 || items.is_null() {
            folder
        } else {
            match crate::pidl_path(*items) {
                // Poly's lists hold full paths; a list of a name in the folder is joined to it.
                Some(p) if matches!(si::parse(&p), Ok(Parsed::Path(_))) => p,
                Some(name) => si::join(&folder, &name),
                None => folder,
            }
        }
    };
    match crate::exec::desktop_request(&target, true) {
        Ok(()) => S_OK,
        Err(e) => poly_com::hresult::from_win32(e),
    }
}
