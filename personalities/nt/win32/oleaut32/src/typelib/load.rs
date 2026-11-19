//! Loading and registering type libraries (sheets `CreateTypeLib2`, `LoadTypeLibEx`,
//! `LoadRegTypeLib`, `RegisterTypeLib`, `UnRegisterTypeLib`, `QueryPathOfRegTypeLib`; `LoadTypeLib`
//! is `LoadTypeLibEx` with `REGKIND_DEFAULT`, which registers nothing). Files are read through
//! [`poly_com::typelib::file::read`]: Poly's own format now, `MSFT` once its format sheet exists
//! (B2b-2; until then such a file is `TYPE_E_CANTLOADLIBRARY`).

use super::objects::{TypeLib, as_ours, cached, lib_release, model, new_typelib};
use crate::com::*;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_com::typelib::reg::{
    self, guid_text, lcid_key, parse_lcid, parse_version, pick_lcid, pick_version, version_key,
};
use poly_com::typelib::*;

const REGKIND_DEFAULT: i32 = 0;
const REGKIND_REGISTER: i32 = 1;
const REGKIND_NONE: i32 = 2;
/// `LOAD_LIBRARY_AS_DATAFILE`.
const AS_DATAFILE: u32 = 2;

/// The `TYPELIB` resource `n` of the executable image `file`.
fn resource(file: &str, n: u32) -> Option<Vec<u8>> {
    let w = wide(file);
    // SAFETY: a NUL-terminated name; the module is freed below.
    let m = unsafe { LoadLibraryExW(w.as_ptr(), 0, AS_DATAFILE) };
    if m == 0 {
        return None;
    }
    let kind = wide("TYPELIB");
    // SAFETY: an integer resource name (MAKEINTRESOURCE) and a NUL-terminated type.
    let out = unsafe {
        let r = FindResourceW(m, n as usize as *const u16, kind.as_ptr());
        if r == 0 {
            None
        } else {
            let h = LoadResource(m, r);
            let p = LockResource(h);
            let size = SizeofResource(m, r) as usize;
            (!p.is_null()).then(|| core::slice::from_raw_parts(p, size).to_vec())
        }
    };
    // SAFETY: the module loaded above.
    unsafe { FreeLibrary(m) };
    out
}

/// The library at `path` (a file, or `file\N` for resource N of an executable).
fn read_library(path: &str) -> Result<Library, Hresult> {
    let bytes = if file_exists(path) {
        let b = read_file(path).map_err(|_| TYPE_E_CANTLOADLIBRARY)?;
        if b.starts_with(b"MZ") { resource(path, 1).ok_or(TYPE_E_CANTLOADLIBRARY)? } else { b }
    } else if let Some((head, n)) = reg::split_resource(path).filter(|(h, _)| file_exists(h)) {
        let b = read_file(head).map_err(|_| TYPE_E_CANTLOADLIBRARY)?;
        if !b.starts_with(b"MZ") {
            return Err(TYPE_E_CANTLOADLIBRARY);
        }
        resource(head, n).ok_or(TYPE_E_CANTLOADLIBRARY)?
    } else {
        return Err(TYPE_E_CANTLOADLIBRARY);
    };
    file::read(&bytes).map_err(|_| TYPE_E_CANTLOADLIBRARY)
}

/// The library object for `path`: the live one of that file, else a new one.
pub(crate) fn load_path(path: &str) -> Result<*mut TypeLib, Hresult> {
    let key = path.to_ascii_uppercase();
    if let Some(t) = cached(&key) {
        return Ok(t);
    }
    let lib = read_library(path)?;
    Ok(new_typelib(lib, false, String::from(path), Some(key)))
}

/// The registered path of a library (`TYPE_E_LIBNOTREGISTERED` when no registration matches).
pub(crate) fn registered_path(guid: &Guid, major: u16, minor: u16, lcid: u32) -> Result<String, Hresult> {
    let base = format!("TypeLib\\{}", guid_text(guid));
    let mut versions: Vec<(u16, u16)> = Vec::new();
    for &(root, prefix) in &CLASS_ROOTS {
        if let Some(k) = Key::open(root, &format!("{prefix}{base}")) {
            versions.extend(k.subkeys().iter().filter_map(|s| parse_version(s)));
        }
    }
    let (ma, mi) = pick_version(&versions, major, minor).ok_or(TYPE_E_LIBNOTREGISTERED)?;
    let ver = format!("{base}\\{}", version_key(ma, mi));
    let mut lcids: Vec<u32> = Vec::new();
    for &(root, prefix) in &CLASS_ROOTS {
        if let Some(k) = Key::open(root, &format!("{prefix}{ver}")) {
            lcids.extend(k.subkeys().iter().filter_map(|s| parse_lcid(s)));
        }
    }
    let l = pick_lcid(&lcids, lcid).ok_or(TYPE_E_LIBNOTREGISTERED)?;
    for platform in reg::LOOKUP_PLATFORMS {
        for &(root, prefix) in &CLASS_ROOTS {
            if let Some(k) = Key::open(root, &format!("{prefix}{ver}\\{}\\{platform}", lcid_key(l))) {
                return Ok(k.get(None).unwrap_or_default());
            }
        }
    }
    Err(TYPE_E_LIBNOTREGISTERED)
}

/// The highest registered version of a library (`TYPE_E_LIBNOTREGISTERED` when none), loaded.
pub(crate) fn load_registered_any(guid: &Guid) -> Result<*mut TypeLib, Hresult> {
    let base = format!("TypeLib\\{}", guid_text(guid));
    let mut versions: Vec<(u16, u16)> = Vec::new();
    for &(root, prefix) in &CLASS_ROOTS {
        if let Some(k) = Key::open(root, &format!("{prefix}{base}")) {
            versions.extend(k.subkeys().iter().filter_map(|s| parse_version(s)));
        }
    }
    let best = versions.into_iter().max();
    match best {
        Some((ma, mi)) => load_registered(guid, ma, mi, 0),
        None => load_registered(guid, if *guid == stdole::LIBID { 2 } else { 1 }, 0, 0),
    }
}

/// `LoadRegTypeLib`'s work: the registered file, loaded.
pub(crate) fn load_registered(guid: &Guid, major: u16, minor: u16, lcid: u32) -> Result<*mut TypeLib, Hresult> {
    let path = match registered_path(guid, major, minor, lcid) {
        // `stdole` 2.0 is part of the system: Poly's copy when its registration is missing.
        Err(_) if *guid == stdole::LIBID && major == 2 && minor == 0 => {
            return match load_path(&alloc::format!(r"C:\Windows\System32\{}", stdole::FILE_NAME)) {
                Ok(t) => Ok(t),
                Err(_) => Ok(new_typelib(stdole::library(), false, String::new(), None)),
            };
        }
        r => r?,
    };
    let path = path.trim_end_matches(' ');
    if path.is_empty() {
        return Err(STG_E_FILENOTFOUND);
    }
    load_path(path).map_err(|_| TYPE_E_CANTLOADLIBRARY)
}

/// The model of an `ITypeLib` (ours; another's as far as its attributes and names go).
fn model_of(p: *mut c_void) -> Option<Library> {
    if let Some(t) = as_ours(p) {
        return Some(model(t));
    }
    let mut a: *mut super::desc::CLibAttr = core::ptr::null_mut();
    if vcall!(p, 7, (*mut *mut super::desc::CLibAttr) -> Hresult, &mut a) < 0 || a.is_null() {
        return None;
    }
    let mut lib = Library::new(SYS_WIN64);
    // SAFETY: the attributes the library returned.
    unsafe {
        lib.guid = (*a).guid;
        lib.lcid = (*a).lcid;
        lib.syskind = (*a).syskind;
        lib.major = (*a).major;
        lib.minor = (*a).minor;
        lib.flags = (*a).flags;
    }
    vcall!(p, 12, (*mut super::desc::CLibAttr) -> (), a);
    let mut name: *mut u16 = core::ptr::null_mut();
    let mut doc: *mut u16 = core::ptr::null_mut();
    vcall!(p, 9, (i32, *mut *mut u16, *mut *mut u16, *mut u32, *mut *mut u16) -> Hresult, -1, &mut name, &mut doc, core::ptr::null_mut(), core::ptr::null_mut());
    // SAFETY: BSTRs the call returned, freed here.
    unsafe {
        lib.name = bstr_text(name).unwrap_or_default();
        lib.doc = bstr_text(doc).unwrap_or_default();
        crate::SysFreeString(name);
        crate::SysFreeString(doc);
    }
    Some(lib)
}

/// Write `RegisterTypeLib`'s keys under `HKLM\SOFTWARE\Classes`.
fn register(lib: &Library, path: &str, help_dir: Option<&str>) -> Hresult {
    for w in reg::register_writes(lib, path, help_dir) {
        let Some(k) = Key::create(HKEY_LOCAL_MACHINE, &format!("SOFTWARE\\Classes\\{}", w.key)) else {
            return TYPE_E_REGISTRYACCESS;
        };
        if !k.set(w.name, &w.value) {
            return TYPE_E_REGISTRYACCESS;
        }
    }
    S_OK
}

/// `CreateTypeLib2(syskind, szFile, ppctlib)`: a library being built; no file until it is saved.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateTypeLib2(syskind: i32, file: *const u16, out: *mut *mut c_void) -> Hresult {
    if file.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's string.
    let path = unsafe { from_wide(file) }.unwrap_or_default();
    let t = new_typelib(Library::new(syskind), true, path, None);
    put(out, (t as usize + core::mem::size_of::<usize>()) as *mut c_void);
    S_OK
}

/// `LoadTypeLibEx(szFile, regkind, pptlib)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadTypeLibEx(file: *const u16, regkind: i32, out: *mut *mut c_void) -> Hresult {
    if file.is_null() || out.is_null() || !matches!(regkind, REGKIND_DEFAULT | REGKIND_REGISTER | REGKIND_NONE) {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's string.
    let path = unsafe { from_wide(file) }.unwrap_or_default();
    match load_path(&path) {
        Ok(t) => {
            if regkind == REGKIND_REGISTER {
                let hr = register(&model(t), &path, None);
                if hr < 0 {
                    lib_release(t);
                    put(out, core::ptr::null_mut());
                    return hr;
                }
            }
            put(out, t.cast());
            S_OK
        }
        Err(hr) => {
            put(out, core::ptr::null_mut());
            hr
        }
    }
}

/// `LoadTypeLib(szFile, pptlib)`: `LoadTypeLibEx` with `REGKIND_DEFAULT`, which registers nothing
/// (sheet `LoadTypeLibEx`, rule `regkind`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadTypeLib(file: *const u16, out: *mut *mut c_void) -> Hresult {
    // SAFETY: the caller's arguments.
    unsafe { LoadTypeLibEx(file, REGKIND_DEFAULT, out) }
}

/// `LoadRegTypeLib(rguid, wVerMajor, wVerMinor, lcid, pptlib)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn LoadRegTypeLib(
    guid: *const Guid,
    major: u16,
    minor: u16,
    lcid: u32,
    out: *mut *mut c_void,
) -> Hresult {
    if guid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    match load_registered(unsafe { &*guid }, major, minor, lcid) {
        Ok(t) => {
            put(out, t.cast());
            S_OK
        }
        Err(hr) => {
            put(out, core::ptr::null_mut());
            hr
        }
    }
}

/// `RegisterTypeLib(ptlib, szFullPath, szHelpDir)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn RegisterTypeLib(tlib: *mut c_void, path: *const u16, help_dir: *const u16) -> Hresult {
    if tlib.is_null() || path.is_null() {
        return E_INVALIDARG;
    }
    let Some(lib) = model_of(tlib) else { return E_INVALIDARG };
    // SAFETY: the caller's strings.
    let (p, h) = unsafe { (from_wide(path).unwrap_or_default(), from_wide(help_dir)) };
    register(&lib, &p, h.as_deref())
}

/// `QueryPathOfRegTypeLib(guid, wMaj, wMin, lcid, lpbstrPathName)`: the registered value, the file
/// not opened.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn QueryPathOfRegTypeLib(
    guid: *const Guid,
    major: u16,
    minor: u16,
    lcid: u32,
    out: *mut *mut u16,
) -> Hresult {
    if guid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    match registered_path(unsafe { &*guid }, major, minor, lcid) {
        Ok(p) => {
            put(out, bstr(&p));
            S_OK
        }
        Err(hr) => {
            put(out, core::ptr::null_mut());
            hr
        }
    }
}

/// `UnRegisterTypeLib(libID, wVerMajor, wVerMinor, lcid, syskind)` (sheet `UnRegisterTypeLib`):
/// the registration must match exactly; the registered file is read for its interfaces, whose
/// keys go when they point to this version; the version key goes when only `FLAGS` and `HELPDIR`
/// are left, the library key when it is empty.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn UnRegisterTypeLib(
    guid: *const Guid,
    major: u16,
    minor: u16,
    lcid: u32,
    syskind: i32,
) -> Hresult {
    if guid.is_null() {
        return E_INVALIDARG;
    }
    let Some(platform) = reg::platform_key(syskind) else { return E_INVALIDARG };
    // SAFETY: the caller's GUID.
    let id = guid_text(unsafe { &*guid });
    let ver = version_key(major, minor);
    let base = format!("SOFTWARE\\Classes\\TypeLib\\{id}");
    let vkey = format!("{base}\\{ver}");
    let lkey = format!("{vkey}\\{}", lcid_key(lcid));
    let pkey = format!("{lkey}\\{platform}");
    let Some(k) = Key::open(HKEY_LOCAL_MACHINE, &pkey) else { return TYPE_E_REGISTRYACCESS };
    let path = k.get(None).unwrap_or_default();
    drop(k);
    let lib = match read_library(path.trim_end_matches(' ')) {
        Ok(l) => l,
        Err(_) => return TYPE_E_CANTLOADLIBRARY,
    };
    for t in 0..lib.types.len() {
        if !reg::registers_interface(&lib, t) {
            continue;
        }
        let ikey = format!("SOFTWARE\\Classes\\Interface\\{}", guid_text(&lib.types[t].guid));
        let points_here = Key::open(HKEY_LOCAL_MACHINE, &format!("{ikey}\\TypeLib")).is_some_and(|k| {
            k.get(None).is_some_and(|v| v.eq_ignore_ascii_case(&id)) && k.get(Some("Version")).is_none_or(|v| v == ver)
        });
        if points_here {
            delete_tree(HKEY_LOCAL_MACHINE, &ikey);
        }
    }
    delete_tree(HKEY_LOCAL_MACHINE, &pkey);
    if Key::open(HKEY_LOCAL_MACHINE, &lkey).is_some_and(|k| k.subkeys().is_empty()) {
        delete_tree(HKEY_LOCAL_MACHINE, &lkey);
    }
    let rest = Key::open(HKEY_LOCAL_MACHINE, &vkey).map(|k| k.subkeys()).unwrap_or_default();
    if rest.iter().all(|s| s.eq_ignore_ascii_case("FLAGS") || s.eq_ignore_ascii_case("HELPDIR")) {
        delete_tree(HKEY_LOCAL_MACHINE, &vkey);
    }
    if Key::open(HKEY_LOCAL_MACHINE, &base).is_some_and(|k| k.subkeys().is_empty()) {
        delete_tree(HKEY_LOCAL_MACHINE, &base);
    }
    S_OK
}
