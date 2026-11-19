//! What the marshaller knows of an interface: found through `Interface\{IID}\TypeLib` (the library
//! id and `Version`) and the registered library (sheet `RegisterTypeLib`, rule `interface_keys`),
//! its methods by vtable slot with each parameter's direction and how its value travels. `IDispatch`
//! itself and every dual or `IDispatch`-based interface start with `IDispatch`'s four methods,
//! which [`super::disp`] marshals.

use super::codec::Ty;
use crate::com::*;
use crate::typelib::{load, objects};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_com::typelib::reg::parse_version;
use poly_com::typelib::view::{Form, view};
use poly_com::typelib::*;

/// One parameter.
#[derive(Clone, Debug)]
pub(crate) struct Param {
    pub ty: Ty,
    pub input: bool,
    pub output: bool,
}

/// One method.
#[derive(Clone, Debug)]
pub(crate) struct Method {
    pub slot: u32,
    pub name: String,
    pub params: Vec<Param>,
}

/// One interface.
#[derive(Debug)]
pub(crate) struct Info {
    pub iid: Guid,
    /// Slots 3..=6 are `IDispatch`'s.
    pub dispatch: bool,
    pub methods: Vec<Method>,
    /// The highest slot (vtable size - 1).
    pub last_slot: u32,
}

impl Info {
    pub(crate) fn method(&self, slot: u32) -> Option<&Method> {
        self.methods.iter().find(|m| m.slot == slot)
    }
}

static CACHE: Locked<Vec<(Guid, usize)>> = Locked::new(Vec::new());

/// The description of `iid` (`REGDB_E_IIDNOTREG` when no registered library describes it).
pub(crate) fn info_for(iid: &Guid) -> Result<Arc<Info>, Hresult> {
    if let Some(p) = CACHE.with(|c| c.iter().find(|(g, _)| g == iid).map(|e| e.1)) {
        // SAFETY: an `Arc` leaked into the cache below; one more strong count for the caller.
        unsafe {
            Arc::increment_strong_count(p as *const Info);
            return Ok(Arc::from_raw(p as *const Info));
        }
    }
    let info = Arc::new(build(iid)?);
    let raw = Arc::into_raw(info.clone()) as usize;
    CACHE.with(|c| c.push((*iid, raw)));
    Ok(info)
}

fn build(iid: &Guid) -> Result<Info, Hresult> {
    if *iid == IID_IDISPATCH {
        return Ok(Info { iid: *iid, dispatch: true, methods: Vec::new(), last_slot: 6 });
    }
    if *iid == IID_IUNKNOWN {
        return Err(REGDB_E_IIDNOTREG);
    }
    let text: String = iid.to_text().iter().map(|&u| u as u8 as char).collect();
    let key = open_class(&alloc::format!("Interface\\{text}\\TypeLib")).ok_or(REGDB_E_IIDNOTREG)?;
    let libid_text = key.get(None).ok_or(REGDB_E_IIDNOTREG)?;
    let units: Vec<u16> = libid_text.encode_utf16().collect();
    let libid = Guid::parse(&units).ok_or(REGDB_E_IIDNOTREG)?;
    let (major, minor) = key.get(Some("Version")).and_then(|v| parse_version(&v)).unwrap_or((1, 0));
    let t = load::load_registered(&libid, major, minor, 0).map_err(|_| REGDB_E_IIDNOTREG)?;
    let lib = objects::model(t);
    let r = describe(t, &lib, iid);
    objects::lib_release(t);
    r
}

fn describe(t: *mut objects::TypeLib, lib: &Library, iid: &Guid) -> Result<Info, Hresult> {
    let index = lib.find_guid(iid).ok_or(REGDB_E_IIDNOTREG)?;
    let ptr = core::mem::size_of::<usize>();
    let mut dispatch = lib.is_dual(index);
    let mut funcs: Vec<Func> = Vec::new();
    let mut i = index;
    // The interface and its bases inside the library, down to IUnknown / IDispatch.
    for _ in 0..16 {
        let form = if lib.is_dual(i) { Form::DualInterface } else { Form::Stored };
        let v = view(lib, i, form).ok_or(REGDB_E_IIDNOTREG)?;
        if v.kind == TKIND_DISPATCH && !lib.is_dual(i) {
            // A dispinterface: IDispatch's methods only.
            dispatch = true;
            break;
        }
        funcs.extend(v.funcs.iter().cloned());
        let Some(base) = v.impl_types.first() else { break };
        if let Some(imp) = lib.import_of(base.href) {
            if imp.type_guid == IID_IDISPATCH {
                dispatch = true;
            }
            break;
        }
        match Href::decode(base.href) {
            Some(Href::Internal(j)) if j != i => {
                if lib.types[j].guid == IID_IDISPATCH {
                    dispatch = true;
                    break;
                }
                i = j;
            }
            _ => break,
        }
    }
    let mut methods = Vec::new();
    let mut last = if dispatch { 6 } else { 2 };
    for f in &funcs {
        let slot = (f.ovft.max(0) as usize / ptr) as u32;
        last = last.max(slot);
        let params = f
            .params
            .iter()
            .map(|p| {
                let output = p.flags & PARAMFLAG_FOUT != 0;
                let input = p.flags & PARAMFLAG_FIN != 0 || !output;
                Param { ty: ty_of(t, lib, &p.td, 0), input, output }
            })
            .collect();
        methods.push(Method { slot, name: f.name.clone(), params });
    }
    Ok(Info { iid: *iid, dispatch, methods, last_slot: last })
}

/// How a value of `td` travels.
fn ty_of(t: *mut objects::TypeLib, lib: &Library, td: &TypeDesc, depth: u32) -> Ty {
    if depth > 16 {
        return Ty::Bad;
    }
    match td {
        TypeDesc::Base(vt) => super::codec::of_vt(*vt),
        TypeDesc::SafeArray(inner) => match &**inner {
            TypeDesc::Base(vt) => Ty::Array(*vt),
            TypeDesc::UserDefined(h) => match user_kind(t, lib, *h) {
                UserKind::Interface(g) if g == IID_IDISPATCH => Ty::Array(VT_DISPATCH),
                UserKind::Interface(_) => Ty::Array(VT_UNKNOWN),
                UserKind::Int4 => Ty::Array(VT_I4),
                _ => Ty::Bad,
            },
            _ => Ty::Bad,
        },
        TypeDesc::Ptr(inner) => {
            if let TypeDesc::UserDefined(h) = &**inner {
                if let UserKind::Interface(g) = user_kind(t, lib, *h) {
                    return Ty::Itf(g);
                }
            }
            match ty_of(t, lib, inner, depth + 1) {
                Ty::Bad => Ty::Bad,
                x => Ty::Ptr(alloc::boxed::Box::new(x)),
            }
        }
        TypeDesc::UserDefined(h) => match user_kind(t, lib, *h) {
            UserKind::Int4 => Ty::Int(4),
            UserKind::Alias(a) => ty_of(t, lib, &a, depth + 1),
            _ => Ty::Bad,
        },
        TypeDesc::CArray(..) => Ty::Bad,
    }
}

enum UserKind {
    Interface(Guid),
    Int4,
    Alias(TypeDesc),
    Other,
}

/// What a referenced type is: an interface (by its id), an enumeration (a 4-byte integer), an alias.
fn user_kind(_t: *mut objects::TypeLib, lib: &Library, href: u32) -> UserKind {
    match Href::decode(href) {
        Some(Href::Internal(i)) | Some(Href::DualInterface(i)) => match lib.types.get(i) {
            Some(ty) if matches!(ty.kind, TKIND_INTERFACE | TKIND_DISPATCH) => UserKind::Interface(ty.guid),
            Some(ty) if ty.kind == TKIND_ENUM => UserKind::Int4,
            Some(ty) if ty.kind == TKIND_ALIAS => ty.alias.clone().map_or(UserKind::Other, UserKind::Alias),
            _ => UserKind::Other,
        },
        Some(Href::Import(_)) => match lib.import_of(href) {
            // Types of other libraries a method refers to are interfaces (stdole's IUnknown and
            // IDispatch, or another library's interface).
            Some(imp) => UserKind::Interface(imp.type_guid),
            None => UserKind::Other,
        },
        None => UserKind::Other,
    }
}
