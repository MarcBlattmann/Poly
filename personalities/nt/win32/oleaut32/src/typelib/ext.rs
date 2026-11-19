//! `ITypeLib2` and `ITypeInfo2` (Learn: "ITypeLib2", "ITypeInfo2", "CUSTDATA", "ClearCustData"),
//! `ITypeInfo::GetDllEntry` and the custom data, help strings and statistics a library carries
//! (format sheet `docs/windows/typelib-file-format.md`; sheets `TypeLibFile.Layout`,
//! `TypeLibFile.TypeInfo`, `TypeLibFile.Functions`).

use super::objects::{TypeInfo, TypeLib, model, view_of};
use crate::com::*;
use crate::variant::set;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use poly_com::typelib::*;
use poly_kernelbase::oleaut::VARIANT_SIZE;

/// `CUSTDATA`: the count and a task-memory array of `CUSTDATAITEM` (a GUID and a VARIANT).
#[repr(C)]
pub(crate) struct CCustData {
    count: u32,
    items: *mut u8,
}

const ITEM: usize = 16 + VARIANT_SIZE;

/// Write `v` as a VARIANT at `out` (a BSTR is allocated).
pub(crate) fn put_value(out: *mut u8, v: &CustValue) {
    match v {
        CustValue::I4(x) => set(out, VT_I4, *x as u32 as u64),
        CustValue::Ui4(x) => set(out, VT_UI4, u64::from(*x)),
        CustValue::Bstr(s) => set(out, VT_BSTR, bstr(s) as u64),
        CustValue::Other { vt, bits } => set(out, *vt, *bits),
    }
}

fn fill(out: *mut CCustData, list: &CustData) -> Hresult {
    if out.is_null() {
        return E_INVALIDARG;
    }
    let items = if list.is_empty() { core::ptr::null_mut() } else { task_alloc(ITEM * list.len()) as *mut u8 };
    if !list.is_empty() && items.is_null() {
        return E_OUTOFMEMORY;
    }
    for (k, (g, v)) in list.iter().enumerate() {
        // SAFETY: item `k` of the array just allocated.
        unsafe {
            let at = items.add(ITEM * k);
            at.cast::<Guid>().write_unaligned(*g);
            put_value(at.add(16), v);
        }
    }
    // SAFETY: the caller's CUSTDATA.
    unsafe { out.write(CCustData { count: list.len() as u32, items }) };
    S_OK
}

fn one(list: &CustData, g: *const Guid, out: *mut u8) -> Hresult {
    if g.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    // SAFETY: the caller's GUID.
    let g = unsafe { *g };
    match list.iter().find(|(x, _)| *x == g) {
        Some((_, v)) => put_value(out, v),
        // Learn: VT_EMPTY when there is no such entry.
        None => set(out, VT_EMPTY, 0),
    }
    S_OK
}

/// `ClearCustData(pCustData)`: free what a `CUSTDATA` holds.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ClearCustData(cd: *mut CCustData) {
    if cd.is_null() {
        return;
    }
    // SAFETY: a CUSTDATA one of the GetAll*CustData methods filled.
    unsafe {
        let c = &mut *cd;
        for k in 0..c.count as usize {
            crate::VariantClear(c.items.add(ITEM * k + 16));
        }
        task_free(c.items.cast());
        c.count = 0;
        c.items = core::ptr::null_mut();
    }
}

/// The help file as the API reports it: the directory of the library's file, a backslash, the
/// stored name and a NUL character (format sheet section 3); a library not loaded from a file
/// reports the name as stored.
pub(crate) fn help_file(dir: &str, name: &str) -> String {
    if name.is_empty() || dir.is_empty() {
        return String::from(name);
    }
    alloc::format!("{dir}\\{name}\0")
}

/// Distinct names (ignoring case) and their characters, for a library made here.
fn statistics(lib: &Library) -> (u32, u32) {
    let mut names: Vec<String> = Vec::new();
    let mut add = |s: &str| {
        if !s.is_empty() && !names.iter().any(|n| same_name(n, s)) {
            names.push(String::from(s));
        }
    };
    add(&lib.name);
    for t in &lib.types {
        add(&t.name);
        for f in &t.funcs {
            add(&f.name);
            for p in &f.params {
                add(&p.name);
            }
        }
        for v in &t.vars {
            add(&v.name);
        }
    }
    (names.len() as u32, names.iter().map(|n| n.chars().count() as u32).sum())
}

fn put_doc2(hs: *mut *mut u16, ctx: *mut u32, dll: *mut *mut u16, d: (&str, u32, &str)) {
    put(hs, if d.0.is_empty() { core::ptr::null_mut() } else { bstr(d.0) });
    put(ctx, d.1);
    put(dll, if d.2.is_empty() { core::ptr::null_mut() } else { bstr(d.2) });
}

// ----- ITypeLib2 (slots 13 to 16) ------------------------------------------------------------

fn lib_of(this: *mut c_void) -> Library {
    model(this as *mut TypeLib)
}

pub(crate) unsafe extern "system" fn tl2_get_cust_data(this: *mut c_void, g: *const Guid, out: *mut u8) -> Hresult {
    one(&lib_of(this).cust, g, out)
}
pub(crate) unsafe extern "system" fn tl2_get_lib_statistics(
    this: *mut c_void,
    unique: *mut u32,
    chars: *mut u32,
) -> Hresult {
    let lib = lib_of(this);
    let (u, c) = lib.stats.unwrap_or_else(|| statistics(&lib));
    put(unique, u);
    put(chars, c);
    S_OK
}
pub(crate) unsafe extern "system" fn tl2_get_documentation2(
    this: *mut c_void,
    index: i32,
    _lcid: u32,
    hs: *mut *mut u16,
    ctx: *mut u32,
    dll: *mut *mut u16,
) -> Hresult {
    let lib = lib_of(this);
    if index == -1 {
        put_doc2(hs, ctx, dll, (&lib.doc, lib.help_string_context, &lib.help_string_dll));
        return S_OK;
    }
    match lib.types.get(index as usize) {
        Some(t) if index >= 0 => {
            put_doc2(hs, ctx, dll, (&t.doc, t.help_string_context, &lib.help_string_dll));
            S_OK
        }
        _ => TYPE_E_ELEMENTNOTFOUND,
    }
}
pub(crate) unsafe extern "system" fn tl2_get_all_cust_data(this: *mut c_void, out: *mut CCustData) -> Hresult {
    fill(out, &lib_of(this).cust)
}

// ----- ITypeInfo2 (slots 22 to 36) and GetDllEntry (slot 13) ------------------------------------

fn info(this: *mut c_void) -> Option<(Library, usize, poly_com::typelib::view::TypeView)> {
    let t = this as *mut TypeInfo;
    // SAFETY: a live block.
    let index = unsafe { (*t).index };
    let (lib, v) = view_of(t)?;
    Some((lib, index, v))
}

pub(crate) unsafe extern "system" fn ti_get_dll_entry(
    this: *mut c_void,
    memid: i32,
    invkind: i32,
    dll: *mut *mut u16,
    name: *mut *mut u16,
    ordinal: *mut u16,
) -> Hresult {
    let Some((_, _, v)) = info(this) else { return E_UNEXPECTED };
    if v.kind != TKIND_MODULE {
        return TYPE_E_BADMODULEKIND;
    }
    let Some(f) = v.funcs.iter().find(|f| f.memid == memid && f.invkind == invkind) else {
        return TYPE_E_ELEMENTNOTFOUND;
    };
    let e = f.entry.clone().unwrap_or_default();
    put(dll, if e.dll.is_empty() { core::ptr::null_mut() } else { bstr(&e.dll) });
    put(name, e.name.as_deref().map_or(core::ptr::null_mut(), bstr));
    put(ordinal, e.ordinal);
    S_OK
}

pub(crate) unsafe extern "system" fn ti2_get_type_kind(this: *mut c_void, out: *mut i32) -> Hresult {
    let Some((_, _, v)) = info(this) else { return E_UNEXPECTED };
    put(out, v.kind);
    S_OK
}
pub(crate) unsafe extern "system" fn ti2_get_type_flags(this: *mut c_void, out: *mut u32) -> Hresult {
    let Some((_, _, v)) = info(this) else { return E_UNEXPECTED };
    put(out, u32::from(v.flags));
    S_OK
}
pub(crate) unsafe extern "system" fn ti2_get_func_index_of_memid(
    this: *mut c_void,
    memid: i32,
    inv: i32,
    out: *mut u32,
) -> Hresult {
    let Some((_, _, v)) = info(this) else { return E_UNEXPECTED };
    match v.funcs.iter().position(|f| f.memid == memid && f.invkind == inv) {
        Some(i) => {
            put(out, i as u32);
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}
pub(crate) unsafe extern "system" fn ti2_get_var_index_of_memid(
    this: *mut c_void,
    memid: i32,
    out: *mut u32,
) -> Hresult {
    let Some((_, _, v)) = info(this) else { return E_UNEXPECTED };
    match v.vars.iter().position(|x| x.memid == memid) {
        Some(i) => {
            put(out, i as u32);
            S_OK
        }
        None => TYPE_E_ELEMENTNOTFOUND,
    }
}

/// The custom data of the type itself, of function `f`, of its parameter `p`, or of variable `x`.
fn cust_of(this: *mut c_void, what: Which) -> Result<CustData, Hresult> {
    let (lib, i, v) = info(this).ok_or(E_UNEXPECTED)?;
    let empty = Vec::new;
    Ok(match what {
        Which::Type => lib.types.get(i).map_or_else(empty, |t| t.cust.clone()),
        Which::Func(f) => v.funcs.get(f).ok_or(TYPE_E_ELEMENTNOTFOUND)?.cust.clone(),
        Which::Param(f, p) => {
            v.funcs.get(f).ok_or(TYPE_E_ELEMENTNOTFOUND)?.params.get(p).ok_or(TYPE_E_ELEMENTNOTFOUND)?.cust.clone()
        }
        Which::Var(x) => v.vars.get(x).ok_or(TYPE_E_ELEMENTNOTFOUND)?.cust.clone(),
        Which::Impl(k) => {
            v.impl_types.get(k).ok_or(TYPE_E_ELEMENTNOTFOUND)?;
            Vec::new()
        }
    })
}

#[derive(Clone, Copy)]
enum Which {
    Type,
    Func(usize),
    Param(usize, usize),
    Var(usize),
    Impl(usize),
}

fn get_one(this: *mut c_void, w: Which, g: *const Guid, out: *mut u8) -> Hresult {
    match cust_of(this, w) {
        Ok(c) => one(&c, g, out),
        Err(hr) => hr,
    }
}
fn get_all(this: *mut c_void, w: Which, out: *mut CCustData) -> Hresult {
    match cust_of(this, w) {
        Ok(c) => fill(out, &c),
        Err(hr) => hr,
    }
}

pub(crate) unsafe extern "system" fn ti2_get_cust_data(this: *mut c_void, g: *const Guid, out: *mut u8) -> Hresult {
    get_one(this, Which::Type, g, out)
}
pub(crate) unsafe extern "system" fn ti2_get_func_cust_data(
    this: *mut c_void,
    f: u32,
    g: *const Guid,
    out: *mut u8,
) -> Hresult {
    get_one(this, Which::Func(f as usize), g, out)
}
pub(crate) unsafe extern "system" fn ti2_get_param_cust_data(
    this: *mut c_void,
    f: u32,
    p: u32,
    g: *const Guid,
    out: *mut u8,
) -> Hresult {
    get_one(this, Which::Param(f as usize, p as usize), g, out)
}
pub(crate) unsafe extern "system" fn ti2_get_var_cust_data(
    this: *mut c_void,
    x: u32,
    g: *const Guid,
    out: *mut u8,
) -> Hresult {
    get_one(this, Which::Var(x as usize), g, out)
}
pub(crate) unsafe extern "system" fn ti2_get_impl_type_cust_data(
    this: *mut c_void,
    k: u32,
    g: *const Guid,
    out: *mut u8,
) -> Hresult {
    get_one(this, Which::Impl(k as usize), g, out)
}
pub(crate) unsafe extern "system" fn ti2_get_documentation2(
    this: *mut c_void,
    memid: i32,
    _lcid: u32,
    hs: *mut *mut u16,
    ctx: *mut u32,
    dll: *mut *mut u16,
) -> Hresult {
    let Some((lib, i, v)) = info(this) else { return E_UNEXPECTED };
    let Some(t) = lib.types.get(i) else { return TYPE_E_ELEMENTNOTFOUND };
    if memid == MEMBERID_NIL {
        put_doc2(hs, ctx, dll, (&t.doc, t.help_string_context, &lib.help_string_dll));
        return S_OK;
    }
    if let Some(f) = v.funcs.iter().find(|f| f.memid == memid) {
        put_doc2(hs, ctx, dll, (&f.doc, f.help_string_context, &lib.help_string_dll));
        return S_OK;
    }
    if let Some(x) = v.vars.iter().find(|x| x.memid == memid) {
        put_doc2(hs, ctx, dll, (&x.doc, x.help_string_context, &lib.help_string_dll));
        return S_OK;
    }
    TYPE_E_ELEMENTNOTFOUND
}
pub(crate) unsafe extern "system" fn ti2_get_all_cust_data(this: *mut c_void, out: *mut CCustData) -> Hresult {
    get_all(this, Which::Type, out)
}
pub(crate) unsafe extern "system" fn ti2_get_all_func_cust_data(
    this: *mut c_void,
    f: u32,
    out: *mut CCustData,
) -> Hresult {
    get_all(this, Which::Func(f as usize), out)
}
pub(crate) unsafe extern "system" fn ti2_get_all_param_cust_data(
    this: *mut c_void,
    f: u32,
    p: u32,
    out: *mut CCustData,
) -> Hresult {
    get_all(this, Which::Param(f as usize, p as usize), out)
}
pub(crate) unsafe extern "system" fn ti2_get_all_var_cust_data(
    this: *mut c_void,
    x: u32,
    out: *mut CCustData,
) -> Hresult {
    get_all(this, Which::Var(x as usize), out)
}
pub(crate) unsafe extern "system" fn ti2_get_all_impl_type_cust_data(
    this: *mut c_void,
    k: u32,
    out: *mut CCustData,
) -> Hresult {
    get_all(this, Which::Impl(k as usize), out)
}
