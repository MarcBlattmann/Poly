//! The type-library objects: one block per library serving `ITypeLib` and, for a library being
//! built (`CreateTypeLib2`), `ICreateTypeLib2`; one block per handed-out type serving `ITypeInfo`
//! and, while its library is being built, `ICreateTypeInfo2`. The decisions are
//! `poly_com::typelib`'s.

use super::desc::*;
use crate::com::*;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};
use poly_com::typelib::view::{Form, view};
use poly_com::typelib::*;

const PTR: usize = core::mem::size_of::<usize>();

pub(crate) const DISP_E_UNKNOWNNAME: Hresult = 0x8002_0006_u32 as i32;
const STG_E_ACCESSDENIED: Hresult = 0x8003_0005_u32 as i32;
const STG_E_WRITEFAULT: Hresult = 0x8003_001D_u32 as i32;

// ----- the library block -----------------------------------------------------------------------

#[repr(C)]
pub(crate) struct TypeLib {
    tl: *const Vtbl<13>,
    ctl: *const Vtbl<17>,
    refs: AtomicU32,
    pub(crate) lib: Locked<Library>,
    /// Made by `CreateTypeLib2`: answers `ICreateTypeLib2`, saves to `path`.
    creating: bool,
    /// A copy sent by value from another process (`IDispatch::GetTypeInfo` through a proxy): its
    /// type infos answer `IMarshal`, as proxies do (sheet `DllGetClassObject`, rule `dual_dispatch`).
    pub(crate) copy: core::sync::atomic::AtomicBool,
    path: String,
    /// The key it is cached under (`LoadTypeLib` of the same file gives the same object).
    cache_key: Option<String>,
}

/// The live libraries: (cache key or "", block). Imports are resolved against these first.
static LIBS: Locked<Vec<(String, usize)>> = Locked::new(Vec::new());

/// A new library block (one reference).
pub(crate) fn new_typelib(lib: Library, creating: bool, path: String, cache_key: Option<String>) -> *mut TypeLib {
    let key = cache_key.clone().unwrap_or_default();
    let p = Box::into_raw(Box::new(TypeLib {
        tl: &TL_VTBL,
        ctl: &CTL_VTBL,
        refs: AtomicU32::new(1),
        lib: Locked::new(lib),
        creating,
        copy: core::sync::atomic::AtomicBool::new(false),
        path,
        cache_key,
    }));
    LIBS.with(|l| l.push((key, p as usize)));
    p
}

/// A cached library for `key`, one reference added.
pub(crate) fn cached(key: &str) -> Option<*mut TypeLib> {
    LIBS.with(|l| {
        let &(_, p) = l.iter().find(|(k, _)| !k.is_empty() && k == key)?;
        let t = p as *mut TypeLib;
        // SAFETY: a live block (a block at 0 references leaves the table under this lock first).
        unsafe {
            if (*t).refs.load(Ordering::Acquire) == 0 {
                return None;
            }
            (*t).refs.fetch_add(1, Ordering::AcqRel);
        }
        Some(t)
    })
}

/// A live library with this id and major version (minor at least `minor`), one reference added.
fn live_by_guid(guid: &Guid, major: u16, minor: u16) -> Option<*mut TypeLib> {
    LIBS.with(|l| {
        for &(_, p) in l.iter() {
            let t = p as *mut TypeLib;
            // SAFETY: as above.
            unsafe {
                if (*t).refs.load(Ordering::Acquire) == 0 || (*t).creating {
                    continue;
                }
                let ok = (*t).lib.with(|x| x.guid == *guid && x.major == major && x.minor >= minor);
                if ok {
                    (*t).refs.fetch_add(1, Ordering::AcqRel);
                    return Some(t);
                }
            }
        }
        None
    })
}

/// Whether `p` is the `ITypeLib` of one of our blocks.
pub(crate) fn as_ours(p: *mut c_void) -> Option<*mut TypeLib> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a COM pointer: its first word is its vtable.
    let v = unsafe { *(p as *const usize) };
    if v == (&raw const TL_VTBL) as usize {
        Some(p.cast())
    } else if v == (&raw const CTL_VTBL) as usize {
        Some((p as usize - PTR) as *mut TypeLib)
    } else {
        None
    }
}

pub(crate) fn lib_add_ref(t: *mut TypeLib) {
    // SAFETY: a live block.
    unsafe { (*t).refs.fetch_add(1, Ordering::AcqRel) };
}

pub(crate) fn lib_release(t: *mut TypeLib) -> u32 {
    // SAFETY: a live block.
    let n = unsafe { (*t).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        let gone = LIBS.with(|l| {
            // SAFETY: as above.
            if unsafe { (*t).refs.load(Ordering::Acquire) } != 0 {
                return false;
            }
            l.retain(|&(_, p)| p != t as usize);
            true
        });
        if gone {
            // SAFETY: the last reference of a block from `new_typelib`.
            drop(unsafe { Box::from_raw(t) });
        }
    }
    n
}

/// The library's model (a copy).
pub(crate) fn model(t: *mut TypeLib) -> Library {
    // SAFETY: a live block.
    unsafe { (*t).lib.with(|l| l.clone()) }
}

fn lib_of(this: *mut c_void) -> *mut TypeLib {
    this.cast()
}

fn lib_of_ctl(this: *mut c_void) -> *mut TypeLib {
    (this as usize - PTR) as *mut TypeLib
}

unsafe fn lib_qi(t: *mut TypeLib, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; a live block.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let g = *iid;
        let p = if g == IID_IUNKNOWN || g == IID_ITYPELIB {
            t as usize
        } else if (*t).creating && (g == IID_ICREATETYPELIB || g == IID_ICREATETYPELIB2) {
            t as usize + PTR
        } else {
            return E_NOINTERFACE;
        };
        lib_add_ref(t);
        *out = p as *mut c_void;
    }
    S_OK
}

/// A new type-info block for type `index` of `t` in `form` (as its `ITypeInfo`).
pub(crate) fn new_typeinfo(t: *mut TypeLib, index: usize, form: Form) -> *mut c_void {
    lib_add_ref(t);
    Box::into_raw(Box::new(TypeInfo { ti: &TI_VTBL, cti: &CTI_VTBL, refs: AtomicU32::new(1), lib: t, index, form }))
        .cast()
}

// ITypeLib

unsafe extern "system" fn tl_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our ITypeLib.
    unsafe { lib_qi(lib_of(this), iid, out) }
}
unsafe extern "system" fn tl_add_ref(this: *mut c_void) -> u32 {
    let t = lib_of(this);
    lib_add_ref(t);
    // SAFETY: a live block.
    unsafe { (*t).refs.load(Ordering::Acquire) }
}
unsafe extern "system" fn tl_release(this: *mut c_void) -> u32 {
    lib_release(lib_of(this))
}
unsafe extern "system" fn tl_get_type_info_count(this: *mut c_void) -> u32 {
    // SAFETY: a live block.
    unsafe { (*lib_of(this)).lib.with(|l| l.types.len() as u32) }
}
unsafe extern "system" fn tl_get_type_info(this: *mut c_void, index: u32, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let t = lib_of(this);
    // SAFETY: a live block.
    let n = unsafe { (*t).lib.with(|l| l.types.len()) };
    if index as usize >= n {
        put(out, core::ptr::null_mut());
        return TYPE_E_ELEMENTNOTFOUND;
    }
    put(out, new_typeinfo(t, index as usize, Form::Stored));
    S_OK
}
unsafe extern "system" fn tl_get_type_info_type(this: *mut c_void, index: u32, kind: *mut i32) -> Hresult {
    if kind.is_null() {
        return E_INVALIDARG;
    }
    let lib = model(lib_of(this));
    match view(&lib, index as usize, Form::Stored) {
        Some(v) => {
            put(kind, v.kind);
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}
unsafe extern "system" fn tl_get_type_info_of_guid(this: *mut c_void, g: *const Guid, out: *mut *mut c_void) -> Hresult {
    if g.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    let t = lib_of(this);
    // SAFETY: the caller's GUID; a live block.
    let found = unsafe { (*t).lib.with(|l| l.find_guid(&*g)) };
    match found {
        Some(i) => {
            put(out, new_typeinfo(t, i, Form::Stored));
            S_OK
        }
        None => {
            put(out, core::ptr::null_mut());
            TYPE_E_ELEMENTNOTFOUND
        }
    }
}
unsafe extern "system" fn tl_get_lib_attr(this: *mut c_void, out: *mut *mut CLibAttr) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let lib = model(lib_of(this));
    put(out, make_lib_attr(&lib));
    S_OK
}
unsafe extern "system" fn tl_get_type_comp(_this: *mut c_void, out: *mut *mut c_void) -> Hresult {
    put(out, core::ptr::null_mut());
    E_NOTIMPL
}
/// Write the four documentation outputs.
fn put_doc(name: *mut *mut u16, doc: *mut *mut u16, ctx: *mut u32, file: *mut *mut u16, d: (&str, &str, u32, &str)) {
    put(name, bstr(d.0));
    put(doc, if d.1.is_empty() { core::ptr::null_mut() } else { bstr(d.1) });
    put(ctx, d.2);
    put(file, if d.3.is_empty() { core::ptr::null_mut() } else { bstr(d.3) });
}
unsafe extern "system" fn tl_get_documentation(
    this: *mut c_void,
    index: i32,
    name: *mut *mut u16,
    doc: *mut *mut u16,
    ctx: *mut u32,
    file: *mut *mut u16,
) -> Hresult {
    let lib = model(lib_of(this));
    if index == -1 {
        put_doc(name, doc, ctx, file, (&lib.name, &lib.doc, lib.help_context, &lib.help_file));
        return S_OK;
    }
    match lib.types.get(index as usize) {
        Some(ty) if index >= 0 => {
            put_doc(name, doc, ctx, file, (&ty.name, &ty.doc, ty.help_context, &lib.help_file));
            S_OK
        }
        _ => TYPE_E_ELEMENTNOTFOUND,
    }
}
unsafe extern "system" fn tl_is_name(this: *mut c_void, name: *mut u16, _hash: u32, found: *mut i32) -> Hresult {
    if name.is_null() || found.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's string.
    let n = unsafe { from_wide(name) }.unwrap_or_default();
    let lib = model(lib_of(this));
    let hit = lib.types.iter().any(|t| {
        same_name(&t.name, &n)
            || t.funcs.iter().any(|f| same_name(&f.name, &n) || f.params.iter().any(|p| same_name(&p.name, &n)))
            || t.vars.iter().any(|v| same_name(&v.name, &n))
    });
    put(found, i32::from(hit));
    S_OK
}
unsafe extern "system" fn tl_find_name(
    this: *mut c_void,
    name: *mut u16,
    _hash: u32,
    infos: *mut *mut c_void,
    ids: *mut i32,
    found: *mut u16,
) -> Hresult {
    if name.is_null() || found.is_null() || infos.is_null() || ids.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's string and count.
    let (n, max) = unsafe { (from_wide(name).unwrap_or_default(), *found) };
    let t = lib_of(this);
    let lib = model(t);
    let mut k = 0u16;
    for (i, ty) in lib.types.iter().enumerate() {
        if k >= max {
            break;
        }
        let memid = if same_name(&ty.name, &n) {
            Some(MEMBERID_NIL)
        } else {
            ty.funcs.iter().find(|f| same_name(&f.name, &n)).map(|f| f.memid)
        };
        if let Some(m) = memid {
            // SAFETY: the caller's arrays of `max` entries.
            unsafe {
                *infos.add(k as usize) = new_typeinfo(t, i, Form::Stored);
                *ids.add(k as usize) = m;
            }
            k += 1;
        }
    }
    put(found, k);
    S_OK
}
unsafe extern "system" fn tl_release_lib_attr(_this: *mut c_void, a: *mut CLibAttr) {
    free_lib_attr(a);
}

static TL_VTBL: Vtbl<13> = Vtbl([
    tl_qi as *const (),
    tl_add_ref as *const (),
    tl_release as *const (),
    tl_get_type_info_count as *const (),
    tl_get_type_info as *const (),
    tl_get_type_info_type as *const (),
    tl_get_type_info_of_guid as *const (),
    tl_get_lib_attr as *const (),
    tl_get_type_comp as *const (),
    tl_get_documentation as *const (),
    tl_is_name as *const (),
    tl_find_name as *const (),
    tl_release_lib_attr as *const (),
]);

// ICreateTypeLib2

unsafe extern "system" fn ctl_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our ICreateTypeLib2.
    unsafe { lib_qi(lib_of_ctl(this), iid, out) }
}
unsafe extern "system" fn ctl_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: as above.
    unsafe { tl_add_ref(lib_of_ctl(this).cast()) }
}
unsafe extern "system" fn ctl_release(this: *mut c_void) -> u32 {
    lib_release(lib_of_ctl(this))
}
unsafe extern "system" fn ctl_create_type_info(
    this: *mut c_void,
    name: *const u16,
    kind: i32,
    out: *mut *mut c_void,
) -> Hresult {
    if name.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    let t = lib_of_ctl(this);
    // SAFETY: the caller's string; a live block.
    let n = unsafe { from_wide(name) }.unwrap_or_default();
    // SAFETY: as above.
    match unsafe { (*t).lib.with(|l| l.create_type(&n, kind)) } {
        Ok(i) => {
            let p = new_typeinfo(t, i, Form::Stored);
            put(out, (p as usize + PTR) as *mut c_void);
            S_OK
        }
        Err(hr) => {
            put(out, core::ptr::null_mut());
            hr
        }
    }
}
fn with_lib<R>(this: *mut c_void, f: impl FnOnce(&mut Library) -> R) -> R {
    // SAFETY: a live block.
    unsafe { (*lib_of_ctl(this)).lib.with(f) }
}
unsafe extern "system" fn ctl_set_name(this: *mut c_void, name: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let Some(n) = (unsafe { from_wide(name) }) else { return E_INVALIDARG };
    with_lib(this, |l| l.name = n);
    S_OK
}
unsafe extern "system" fn ctl_set_version(this: *mut c_void, major: u16, minor: u16) -> Hresult {
    with_lib(this, |l| {
        l.major = major;
        l.minor = minor;
    });
    S_OK
}
unsafe extern "system" fn ctl_set_guid(this: *mut c_void, g: *const Guid) -> Hresult {
    if g.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    let g = unsafe { *g };
    with_lib(this, |l| l.guid = g);
    S_OK
}
unsafe extern "system" fn ctl_set_doc_string(this: *mut c_void, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let Some(d) = (unsafe { from_wide(s) }) else { return E_INVALIDARG };
    with_lib(this, |l| l.doc = d);
    S_OK
}
unsafe extern "system" fn ctl_set_help_file_name(this: *mut c_void, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let Some(d) = (unsafe { from_wide(s) }) else { return E_INVALIDARG };
    with_lib(this, |l| l.help_file = d);
    S_OK
}
unsafe extern "system" fn ctl_set_help_context(this: *mut c_void, c: u32) -> Hresult {
    with_lib(this, |l| l.help_context = c);
    S_OK
}
unsafe extern "system" fn ctl_set_lcid(this: *mut c_void, lcid: u32) -> Hresult {
    with_lib(this, |l| l.lcid = lcid);
    S_OK
}
unsafe extern "system" fn ctl_set_lib_flags(this: *mut c_void, flags: u32) -> Hresult {
    with_lib(this, |l| l.flags = flags as u16);
    S_OK
}
unsafe extern "system" fn ctl_save_all_changes(this: *mut c_void) -> Hresult {
    let t = lib_of_ctl(this);
    let mut lib = model(t);
    if let Err(hr) = lib.check_save() {
        return hr;
    }
    for i in 0..lib.types.len() {
        if !lib.types[i].laid_out {
            let base = base_vft(t, &lib, i);
            let _ = lib.layout(i, base);
        }
    }
    // The laid-out types are kept; the file carries the disk-image flag.
    // SAFETY: a live block.
    unsafe { (*t).lib.with(|l| l.types = lib.types.clone()) };
    lib.flags |= LIBFLAG_FHASDISKIMAGE;
    // SAFETY: as above.
    let path = unsafe { (*t).path.clone() };
    match write_file(&path, &file::encode(&lib)) {
        Ok(()) => S_OK,
        Err(2) => STG_E_FILENOTFOUND,
        Err(3) => STG_E_PATHNOTFOUND,
        Err(5) => STG_E_ACCESSDENIED,
        Err(_) => STG_E_WRITEFAULT,
    }
}
unsafe extern "system" fn ok_ignore(_this: *mut c_void) -> Hresult {
    S_OK
}
unsafe extern "system" fn not_impl() -> Hresult {
    E_NOTIMPL
}

static CTL_VTBL: Vtbl<17> = Vtbl([
    ctl_qi as *const (),
    ctl_add_ref as *const (),
    ctl_release as *const (),
    ctl_create_type_info as *const (),
    ctl_set_name as *const (),
    ctl_set_version as *const (),
    ctl_set_guid as *const (),
    ctl_set_doc_string as *const (),
    ctl_set_help_file_name as *const (),
    ctl_set_help_context as *const (),
    ctl_set_lcid as *const (),
    ctl_set_lib_flags as *const (),
    ctl_save_all_changes as *const (),
    not_impl as *const (),
    ok_ignore as *const (),
    ok_ignore as *const (),
    ok_ignore as *const (),
]);

// ----- the type-info block ---------------------------------------------------------------------

#[repr(C)]
pub(crate) struct TypeInfo {
    ti: *const Vtbl<22>,
    cti: *const Vtbl<26>,
    refs: AtomicU32,
    pub(crate) lib: *mut TypeLib,
    pub(crate) index: usize,
    pub(crate) form: Form,
}

/// Whether `p` is one of our type infos (either interface).
pub(crate) fn typeinfo_of(p: *mut c_void) -> Option<*mut TypeInfo> {
    if p.is_null() {
        return None;
    }
    // SAFETY: a COM pointer.
    let v = unsafe { *(p as *const usize) };
    if v == (&raw const TI_VTBL) as usize {
        Some(p.cast())
    } else if v == (&raw const CTI_VTBL) as usize {
        Some((p as usize - PTR) as *mut TypeInfo)
    } else {
        None
    }
}

fn ti_of(this: *mut c_void) -> *mut TypeInfo {
    this.cast()
}
fn ti_of_cti(this: *mut c_void) -> *mut TypeInfo {
    (this as usize - PTR) as *mut TypeInfo
}

unsafe fn ti_qi(t: *mut TypeInfo, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's pointers; a live block.
    unsafe {
        *out = core::ptr::null_mut();
        if iid.is_null() {
            return E_INVALIDARG;
        }
        let g = *iid;
        let p = if g == IID_IUNKNOWN || g == IID_ITYPEINFO {
            t as usize
        } else if g == IID_IMARSHAL && (*(*t).lib).copy.load(Ordering::Acquire) {
            *out = (&raw const COPY_MARSHAL) as *mut c_void;
            return S_OK;
        } else if (*(*t).lib).creating && g == IID_ICREATETYPEINFO {
            t as usize + PTR
        } else {
            return E_NOINTERFACE;
        };
        (*t).refs.fetch_add(1, Ordering::AcqRel);
        *out = p as *mut c_void;
    }
    S_OK
}
unsafe fn ti_release_block(t: *mut TypeInfo) -> u32 {
    // SAFETY: a live block.
    let n = unsafe { (*t).refs.fetch_sub(1, Ordering::AcqRel) } - 1;
    if n == 0 {
        // SAFETY: the last reference of a block from `new_typeinfo`.
        let b = unsafe { Box::from_raw(t) };
        lib_release(b.lib);
    }
    n
}

/// The view of a type-info block's type.
pub(crate) fn view_of(t: *mut TypeInfo) -> Option<(Library, poly_com::typelib::view::TypeView)> {
    // SAFETY: a live block.
    let (lib, index, form) = unsafe { (model((*t).lib), (*t).index, (*t).form) };
    let v = view(&lib, index, form)?;
    Some((lib, v))
}

unsafe extern "system" fn ti_qi_entry(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our ITypeInfo.
    unsafe { ti_qi(ti_of(this), iid, out) }
}
unsafe extern "system" fn ti_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: a live block.
    unsafe { (*ti_of(this)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}
unsafe extern "system" fn ti_release(this: *mut c_void) -> u32 {
    // SAFETY: our ITypeInfo.
    unsafe { ti_release_block(ti_of(this)) }
}
unsafe extern "system" fn ti_get_type_attr(this: *mut c_void, out: *mut *mut CTypeAttr) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let Some((lib, v)) = view_of(ti_of(this)) else { return TYPE_E_ELEMENTNOTFOUND };
    put(out, make_attr(&lib, &v));
    S_OK
}
unsafe extern "system" fn ti_get_func_desc(this: *mut c_void, index: u32, out: *mut *mut CFuncDesc) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let Some((_, v)) = view_of(ti_of(this)) else { return TYPE_E_ELEMENTNOTFOUND };
    match v.funcs.get(index as usize) {
        Some(f) => {
            put(out, make_func(f));
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}
unsafe extern "system" fn ti_get_var_desc(this: *mut c_void, index: u32, out: *mut *mut CVarDesc) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let Some((_, v)) = view_of(ti_of(this)) else { return TYPE_E_ELEMENTNOTFOUND };
    match v.vars.get(index as usize) {
        Some(x) => {
            put(out, make_var(x));
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}
unsafe extern "system" fn ti_get_names(
    this: *mut c_void,
    memid: i32,
    names: *mut *mut u16,
    max: u32,
    count: *mut u32,
) -> Hresult {
    if names.is_null() || count.is_null() {
        return E_INVALIDARG;
    }
    let Some((_, v)) = view_of(ti_of(this)) else { return TYPE_E_ELEMENTNOTFOUND };
    let list: Vec<String> = if let Some(f) = v.funcs.iter().find(|f| f.memid == memid) {
        poly_com::typelib::view::names_of(f)
    } else if let Some(x) = v.vars.iter().find(|x| x.memid == memid) {
        alloc::vec![x.name.clone()]
    } else {
        put(count, 0);
        return TYPE_E_ELEMENTNOTFOUND;
    };
    let n = list.len().min(max as usize);
    for (i, s) in list.iter().take(n).enumerate() {
        // SAFETY: the caller's array of `max` entries.
        unsafe { *names.add(i) = bstr(s) };
    }
    put(count, n as u32);
    S_OK
}
unsafe extern "system" fn ti_get_ref_type_of_impl_type(this: *mut c_void, index: u32, out: *mut u32) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let t = ti_of(this);
    // SAFETY: a live block.
    let (lib, i, form) = unsafe { (model((*t).lib), (*t).index, (*t).form) };
    if index == u32::MAX {
        if form == Form::Stored && lib.is_dual(i) {
            put(out, Href::DualInterface(i).encode());
            return S_OK;
        }
        return TYPE_E_ELEMENTNOTFOUND;
    }
    match view(&lib, i, form).and_then(|v| v.impl_types.get(index as usize).copied()) {
        Some(it) => {
            put(out, it.href);
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}
unsafe extern "system" fn ti_get_impl_type_flags(this: *mut c_void, index: u32, out: *mut i32) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let Some((_, v)) = view_of(ti_of(this)) else { return TYPE_E_ELEMENTNOTFOUND };
    match v.impl_types.get(index as usize) {
        Some(it) => {
            put(out, it.flags);
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}

/// `GetIDsOfNames` on a view: the member's id, then its parameters' positions; an unknown name is
/// `DISP_E_UNKNOWNNAME` with -1 for it (names are not case sensitive).
pub(crate) fn ids_of_names(v: &poly_com::typelib::view::TypeView, names: &[String], ids: &mut [i32]) -> Hresult {
    let mut hr = S_OK;
    for x in ids.iter_mut() {
        *x = -1;
    }
    let Some(first) = names.first() else { return S_OK };
    let f = v.funcs.iter().find(|f| same_name(&f.name, first));
    let memid = f.map(|f| f.memid).or_else(|| v.vars.iter().find(|x| same_name(&x.name, first)).map(|x| x.memid));
    match memid {
        Some(m) => ids[0] = m,
        None => return DISP_E_UNKNOWNNAME,
    }
    for (k, n) in names.iter().enumerate().skip(1) {
        match f.and_then(|f| f.params.iter().position(|p| same_name(&p.name, n))) {
            Some(i) => ids[k] = i as i32,
            None => hr = DISP_E_UNKNOWNNAME,
        }
    }
    hr
}

unsafe extern "system" fn ti_get_ids_of_names(this: *mut c_void, names: *const *const u16, n: u32, ids: *mut i32) -> Hresult {
    if names.is_null() || ids.is_null() {
        return E_INVALIDARG;
    }
    let Some((_, v)) = view_of(ti_of(this)) else { return TYPE_E_ELEMENTNOTFOUND };
    // SAFETY: the caller's arrays of `n` entries.
    let list: Vec<String> = (0..n as usize).map(|i| unsafe { from_wide(*names.add(i)) }.unwrap_or_default()).collect();
    // SAFETY: as above.
    let out = unsafe { core::slice::from_raw_parts_mut(ids, n as usize) };
    ids_of_names(&v, &list, out)
}
unsafe extern "system" fn ti_get_documentation(
    this: *mut c_void,
    memid: i32,
    name: *mut *mut u16,
    doc: *mut *mut u16,
    ctx: *mut u32,
    file: *mut *mut u16,
) -> Hresult {
    let t = ti_of(this);
    // SAFETY: a live block.
    let (lib, i, form) = unsafe { (model((*t).lib), (*t).index, (*t).form) };
    let Some(ty) = lib.types.get(i) else { return TYPE_E_ELEMENTNOTFOUND };
    if memid == MEMBERID_NIL {
        put_doc(name, doc, ctx, file, (&ty.name, &ty.doc, ty.help_context, &lib.help_file));
        return S_OK;
    }
    let Some(v) = view(&lib, i, form) else { return TYPE_E_ELEMENTNOTFOUND };
    if let Some(f) = v.funcs.iter().find(|f| f.memid == memid) {
        put_doc(name, doc, ctx, file, (&f.name, &f.doc, f.help_context, &lib.help_file));
        return S_OK;
    }
    if let Some(x) = v.vars.iter().find(|x| x.memid == memid) {
        put_doc(name, doc, ctx, file, (&x.name, "", 0, &lib.help_file));
        return S_OK;
    }
    TYPE_E_ELEMENTNOTFOUND
}
unsafe extern "system" fn ti_get_ref_type_info(this: *mut c_void, href: u32, out: *mut *mut c_void) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    put(out, core::ptr::null_mut());
    // SAFETY: a live block.
    let t = unsafe { (*ti_of(this)).lib };
    match ref_type_info(t, href) {
        Ok(p) => {
            put(out, p);
            S_OK
        }
        Err(hr) => hr,
    }
}

/// The `ITypeInfo` an `HREFTYPE` of library `t` names.
pub(crate) fn ref_type_info(t: *mut TypeLib, href: u32) -> Result<*mut c_void, Hresult> {
    let lib = model(t);
    match Href::decode(href) {
        Some(Href::Internal(i)) if i < lib.types.len() => Ok(new_typeinfo(t, i, Form::Stored)),
        Some(Href::DualInterface(i)) if lib.is_dual(i) => Ok(new_typeinfo(t, i, Form::DualInterface)),
        Some(Href::Import(_)) => {
            let imp = lib.import_of(href).ok_or(TYPE_E_ELEMENTNOTFOUND)?.clone();
            resolve_import(&imp)
        }
        _ => Err(TYPE_E_ELEMENTNOTFOUND),
    }
}

/// The type an import names: from a live library of that id, else through the registry; `stdole`
/// also from Poly's built-in copy.
pub(crate) fn resolve_import(imp: &Import) -> Result<*mut c_void, Hresult> {
    let lib = match live_by_guid(&imp.lib_guid, imp.lib_major, imp.lib_minor) {
        Some(l) => l,
        None => match super::load::load_registered(&imp.lib_guid, imp.lib_major, imp.lib_minor, imp.lib_lcid) {
            Ok(l) => l,
            Err(hr) if imp.lib_guid == stdole::LIBID => {
                let _ = hr;
                new_typelib(stdole::library(), false, String::new(), None)
            }
            Err(hr) => return Err(hr),
        },
    };
    let found = model(lib).find_guid(&imp.type_guid);
    let r = match found {
        Some(i) => Ok(new_typeinfo(lib, i, Form::Stored)),
        None => Err(TYPE_E_ELEMENTNOTFOUND),
    };
    lib_release(lib);
    r
}

unsafe extern "system" fn ti_get_containing_type_lib(this: *mut c_void, out: *mut *mut c_void, index: *mut u32) -> Hresult {
    let t = ti_of(this);
    // SAFETY: a live block.
    let (lib, i) = unsafe { ((*t).lib, (*t).index) };
    if !out.is_null() {
        lib_add_ref(lib);
        put(out, lib.cast());
    }
    put(index, i as u32);
    S_OK
}
unsafe extern "system" fn ti_release_type_attr(_this: *mut c_void, a: *mut CTypeAttr) {
    free_attr(a);
}
unsafe extern "system" fn ti_release_func_desc(_this: *mut c_void, f: *mut CFuncDesc) {
    free_func(f);
}
unsafe extern "system" fn ti_release_var_desc(_this: *mut c_void, v: *mut CVarDesc) {
    free_var(v);
}
unsafe extern "system" fn null_out_not_impl(_this: *mut c_void, out: *mut *mut c_void) -> Hresult {
    put(out, core::ptr::null_mut());
    E_NOTIMPL
}

static TI_VTBL: Vtbl<22> = Vtbl([
    ti_qi_entry as *const (),
    ti_add_ref as *const (),
    ti_release as *const (),
    ti_get_type_attr as *const (),
    null_out_not_impl as *const (),
    ti_get_func_desc as *const (),
    ti_get_var_desc as *const (),
    ti_get_names as *const (),
    ti_get_ref_type_of_impl_type as *const (),
    ti_get_impl_type_flags as *const (),
    ti_get_ids_of_names as *const (),
    super::invoke::ti_invoke as *const (),
    ti_get_documentation as *const (),
    not_impl as *const (),
    ti_get_ref_type_info as *const (),
    not_impl as *const (),
    not_impl as *const (),
    not_impl as *const (),
    ti_get_containing_type_lib as *const (),
    ti_release_type_attr as *const (),
    ti_release_func_desc as *const (),
    ti_release_var_desc as *const (),
]);

// ICreateTypeInfo2

fn with_type<R>(this: *mut c_void, f: impl FnOnce(&mut Library, usize) -> R) -> R {
    let t = ti_of_cti(this);
    // SAFETY: a live block.
    unsafe {
        let i = (*t).index;
        (*(*t).lib).lib.with(|l| f(l, i))
    }
}
unsafe extern "system" fn cti_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    // SAFETY: our ICreateTypeInfo.
    unsafe { ti_qi(ti_of_cti(this), iid, out) }
}
unsafe extern "system" fn cti_add_ref(this: *mut c_void) -> u32 {
    // SAFETY: a live block.
    unsafe { (*ti_of_cti(this)).refs.fetch_add(1, Ordering::AcqRel) + 1 }
}
unsafe extern "system" fn cti_release(this: *mut c_void) -> u32 {
    // SAFETY: our ICreateTypeInfo.
    unsafe { ti_release_block(ti_of_cti(this)) }
}
unsafe extern "system" fn cti_set_guid(this: *mut c_void, g: *const Guid) -> Hresult {
    if g.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    let g = unsafe { *g };
    with_type(this, |l, i| l.types[i].guid = g);
    S_OK
}
unsafe extern "system" fn cti_set_type_flags(this: *mut c_void, flags: u32) -> Hresult {
    if flags > 0xFFFF {
        return TYPE_E_BADMODULEKIND;
    }
    with_type(this, |l, i| l.set_type_flags(i, flags as u16)).err().unwrap_or(S_OK)
}
unsafe extern "system" fn cti_set_doc_string(this: *mut c_void, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let Some(d) = (unsafe { from_wide(s) }) else { return E_INVALIDARG };
    with_type(this, |l, i| l.types[i].doc = d);
    S_OK
}
unsafe extern "system" fn cti_set_help_context(this: *mut c_void, c: u32) -> Hresult {
    with_type(this, |l, i| l.types[i].help_context = c);
    S_OK
}
unsafe extern "system" fn cti_set_version(this: *mut c_void, major: u16, minor: u16) -> Hresult {
    with_type(this, |l, i| {
        l.types[i].major = major;
        l.types[i].minor = minor;
    });
    S_OK
}
unsafe extern "system" fn cti_add_ref_type_info(this: *mut c_void, other: *mut c_void, out: *mut u32) -> Hresult {
    if other.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    let me = ti_of_cti(this);
    // SAFETY: a live block.
    let my_lib = unsafe { (*me).lib };
    if let Some(o) = typeinfo_of(other) {
        // SAFETY: a live block of ours.
        let (olib, oi, oform) = unsafe { ((*o).lib, (*o).index, (*o).form) };
        if olib == my_lib {
            let h = if oform == Form::DualInterface { Href::DualInterface(oi) } else { Href::Internal(oi) };
            put(out, h.encode());
            return S_OK;
        }
        let m = model(olib);
        let ty = &m.types[oi];
        let imp = Import {
            lib_guid: m.guid,
            lib_major: m.major,
            lib_minor: m.minor,
            lib_lcid: m.lcid,
            type_guid: ty.guid,
            type_name: ty.name.clone(),
        };
        // SAFETY: as above.
        put(out, unsafe { (*my_lib).lib.with(|l| l.import(imp)) });
        return S_OK;
    }
    // A foreign type info: its library's and its own attributes through COM.
    match foreign_import(other) {
        Some(imp) => {
            // SAFETY: as above.
            put(out, unsafe { (*my_lib).lib.with(|l| l.import(imp)) });
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}

/// The import describing a foreign `ITypeInfo`.
fn foreign_import(ti: *mut c_void) -> Option<Import> {
    let mut lib: *mut c_void = core::ptr::null_mut();
    let mut idx = 0u32;
    if vcall!(ti, 18, (*mut *mut c_void, *mut u32) -> Hresult, &mut lib, &mut idx) < 0 || lib.is_null() {
        return None;
    }
    let mut la: *mut CLibAttr = core::ptr::null_mut();
    vcall!(lib, 7, (*mut *mut CLibAttr) -> Hresult, &mut la);
    let mut ta: *mut CTypeAttr = core::ptr::null_mut();
    vcall!(ti, 3, (*mut *mut CTypeAttr) -> Hresult, &mut ta);
    let mut name: *mut u16 = core::ptr::null_mut();
    vcall!(ti, 12, (i32, *mut *mut u16, *mut c_void, *mut c_void, *mut c_void) -> Hresult, -1, &mut name, core::ptr::null_mut(), core::ptr::null_mut(), core::ptr::null_mut());
    let r = if la.is_null() || ta.is_null() {
        None
    } else {
        // SAFETY: the attribute blocks the objects returned.
        unsafe {
            Some(Import {
                lib_guid: (*la).guid,
                lib_major: (*la).major,
                lib_minor: (*la).minor,
                lib_lcid: (*la).lcid,
                type_guid: (*ta).guid,
                type_name: bstr_text(name).unwrap_or_default(),
            })
        }
    };
    // SAFETY: a BSTR the call returned (or NULL).
    unsafe { crate::SysFreeString(name) };
    if !ta.is_null() {
        vcall!(ti, 19, (*mut CTypeAttr) -> (), ta);
    }
    if !la.is_null() {
        vcall!(lib, 12, (*mut CLibAttr) -> (), la);
    }
    release(lib);
    r
}

unsafe extern "system" fn cti_add_func_desc(this: *mut c_void, index: u32, fd: *const CFuncDesc) -> Hresult {
    if fd.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's FUNCDESC.
    let Some(f) = (unsafe { read_func(&*fd) }) else { return E_INVALIDARG };
    with_type(this, |l, i| l.add_func(i, index as usize, f)).err().unwrap_or(S_OK)
}
unsafe extern "system" fn cti_add_impl_type(this: *mut c_void, index: u32, href: u32) -> Hresult {
    with_type(this, |l, i| {
        let ty = &mut l.types[i];
        if index as usize > ty.impl_types.len() {
            return TYPE_E_ELEMENTNOTFOUND;
        }
        ty.impl_types.insert(index as usize, ImplType { href, flags: 0 });
        ty.laid_out = false;
        S_OK
    })
}
unsafe extern "system" fn cti_set_impl_type_flags(this: *mut c_void, index: u32, flags: i32) -> Hresult {
    with_type(this, |l, i| match l.types[i].impl_types.get_mut(index as usize) {
        Some(it) => {
            it.flags = flags;
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    })
}
unsafe extern "system" fn cti_set_alignment(this: *mut c_void, a: u16) -> Hresult {
    with_type(this, |l, i| l.types[i].align = a);
    S_OK
}
unsafe extern "system" fn cti_add_var_desc(this: *mut c_void, index: u32, vd: *const CVarDesc) -> Hresult {
    if vd.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's VARDESC.
    let (td, memid, flags, varkind, u) = unsafe { (read_td(&(*vd).elem.td, 0), (*vd).memid, (*vd).flags, (*vd).varkind, (*vd).u) };
    let Some(td) = td else { return E_INVALIDARG };
    let value = if varkind == 2 && u != 0 {
        // SAFETY: a constant's VARIANT; its value (an integer) at offset 8.
        unsafe { ((u as *const u8).add(8) as *const i64).read_unaligned() }
    } else {
        u as i64
    };
    with_type(this, |l, i| {
        let ty = &mut l.types[i];
        if index as usize > ty.vars.len() {
            return TYPE_E_ELEMENTNOTFOUND;
        }
        ty.vars.insert(index as usize, Var { memid, name: String::new(), td, flags, varkind, value });
        S_OK
    })
}
unsafe extern "system" fn cti_set_func_and_param_names(
    this: *mut c_void,
    index: u32,
    names: *const *const u16,
    n: u32,
) -> Hresult {
    if names.is_null() || n == 0 {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's array of `n` strings.
    let list: Vec<String> = (0..n as usize).map(|i| unsafe { from_wide(*names.add(i)) }.unwrap_or_default()).collect();
    with_type(this, |l, i| l.set_func_names(i, index as usize, &list)).err().unwrap_or(S_OK)
}
unsafe extern "system" fn cti_set_var_name(this: *mut c_void, index: u32, name: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let Some(n) = (unsafe { from_wide(name) }) else { return E_INVALIDARG };
    with_type(this, |l, i| match l.types[i].vars.get_mut(index as usize) {
        Some(v) => {
            v.name = n;
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    })
}
unsafe extern "system" fn cti_set_type_desc_alias(this: *mut c_void, td: *const CTypeDesc) -> Hresult {
    if td.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's TYPEDESC.
    let Some(t) = (unsafe { read_td(&*td, 0) }) else { return E_INVALIDARG };
    with_type(this, |l, i| l.types[i].alias = Some(t));
    S_OK
}
unsafe extern "system" fn cti_set_func_doc_string(this: *mut c_void, index: u32, s: *const u16) -> Hresult {
    // SAFETY: the caller's string.
    let Some(d) = (unsafe { from_wide(s) }) else { return E_INVALIDARG };
    with_type(this, |l, i| match l.types[i].funcs.get_mut(index as usize) {
        Some(f) => {
            f.doc = d;
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    })
}
unsafe extern "system" fn cti_set_func_help_context(this: *mut c_void, index: u32, c: u32) -> Hresult {
    with_type(this, |l, i| match l.types[i].funcs.get_mut(index as usize) {
        Some(f) => {
            f.help_context = c;
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    })
}
unsafe extern "system" fn ok2(_this: *mut c_void, _a: usize, _b: usize) -> Hresult {
    S_OK
}

/// The vtable size of the base of type `i` (what its functions start after).
fn base_vft(t: *mut TypeLib, lib: &Library, i: usize) -> usize {
    let Some(it) = lib.types[i].impl_types.first() else { return 0 };
    if let Some(imp) = lib.import_of(it.href) {
        if imp.type_guid == IID_IUNKNOWN {
            return 3 * PTR;
        }
        if imp.type_guid == IID_IDISPATCH {
            return 7 * PTR;
        }
    }
    match Href::decode(it.href) {
        Some(Href::Internal(j)) if j < lib.types.len() && j != i => {
            if lib.is_dual(j) {
                return view(lib, j, Form::DualInterface).map_or(7 * PTR, |v| v.cb_size_vft as usize);
            }
            return view(lib, j, Form::Stored).map_or(3 * PTR, |v| v.cb_size_vft as usize);
        }
        _ => {}
    }
    match ref_type_info(t, it.href) {
        Ok(ti) => {
            let mut ta: *mut CTypeAttr = core::ptr::null_mut();
            vcall!(ti, 3, (*mut *mut CTypeAttr) -> Hresult, &mut ta);
            let n = if ta.is_null() {
                3 * PTR
            } else {
                // SAFETY: the block the call returned.
                let n = unsafe { (*ta).cb_size_vft } as usize;
                vcall!(ti, 19, (*mut CTypeAttr) -> (), ta);
                n
            };
            release(ti);
            n
        }
        Err(_) => 3 * PTR,
    }
}

unsafe extern "system" fn cti_lay_out(this: *mut c_void) -> Hresult {
    let me = ti_of_cti(this);
    // SAFETY: a live block.
    let (t, i) = unsafe { ((*me).lib, (*me).index) };
    let lib = model(t);
    let base = base_vft(t, &lib, i);
    // SAFETY: as above.
    unsafe { (*t).lib.with(|l| l.layout(i, base)) }.err().unwrap_or(S_OK)
}

static CTI_VTBL: Vtbl<26> = Vtbl([
    cti_qi as *const (),
    cti_add_ref as *const (),
    cti_release as *const (),
    cti_set_guid as *const (),
    cti_set_type_flags as *const (),
    cti_set_doc_string as *const (),
    cti_set_help_context as *const (),
    cti_set_version as *const (),
    cti_add_ref_type_info as *const (),
    cti_add_func_desc as *const (),
    cti_add_impl_type as *const (),
    cti_set_impl_type_flags as *const (),
    cti_set_alignment as *const (),
    ok2 as *const (),
    cti_add_var_desc as *const (),
    cti_set_func_and_param_names as *const (),
    cti_set_var_name as *const (),
    cti_set_type_desc_alias as *const (),
    not_impl as *const (),
    cti_set_func_doc_string as *const (),
    ok2 as *const (),
    cti_set_func_help_context as *const (),
    ok2 as *const (),
    ok2 as *const (),
    ok2 as *const (),
    cti_lay_out as *const (),
]);

// ----- the IMarshal of a copied type info -------------------------------------------------------
//
// A type info that came by value from another process is a copy; asked for `IMarshal` it answers
// with this object (its methods are not implemented: the copy is not marshalled on).

unsafe extern "system" fn cm_qi(this: *mut c_void, iid: *const Guid, out: *mut *mut c_void) -> Hresult {
    if out.is_null() || iid.is_null() {
        return E_POINTER;
    }
    // SAFETY: the caller's IID.
    let g = unsafe { *iid };
    if g == IID_IUNKNOWN || g == IID_IMARSHAL {
        put(out, this);
        S_OK
    } else {
        put(out, core::ptr::null_mut());
        E_NOINTERFACE
    }
}
unsafe extern "system" fn cm_ref(_this: *mut c_void) -> u32 {
    1
}

static CM_VTBL: Vtbl<9> = Vtbl([
    cm_qi as *const (),
    cm_ref as *const (),
    cm_ref as *const (),
    not_impl as *const (),
    not_impl as *const (),
    not_impl as *const (),
    not_impl as *const (),
    not_impl as *const (),
    not_impl as *const (),
]);

struct CopyMarshal(*const Vtbl<9>);
// SAFETY: an immutable pointer to a static table.
unsafe impl Sync for CopyMarshal {}
static COPY_MARSHAL: CopyMarshal = CopyMarshal(&CM_VTBL);
