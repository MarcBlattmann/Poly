//! How a stored type reads back through `ITypeInfo` (sheet `CreateTypeLib2`, rules
//! `interface_type` and `dual_interface`).
//!
//! A dual interface (the dual flag on an interface or dispatch type) has two forms. The one the
//! library hands out (`GetTypeInfo`, `GetTypeInfoOfGuid`) is a dispatch type (`TKIND_DISPATCH`,
//! without `FOLEAUTOMATION`) whose functions are `IUnknown`'s three and `IDispatch`'s four (member
//! ids `0x60000000..` and `0x60010000..`), then the type's own, each with funckind `FUNC_DISPATCH`
//! and the `HRESULT` and `[out, retval]` parameter folded into the result; its vtable size is
//! `IDispatch`'s. The other is the interface form (`TKIND_INTERFACE`, functions as added, vtable
//! after `IDispatch`'s seven slots), reached with `GetRefTypeOfImplType(-1)` (Learn:
//! "ITypeInfo::GetRefTypeOfImplType": "If the TYPEINFO describes a dual interface ... index -1 ...
//! the interface").

use super::*;
use alloc::vec;

/// Which form of a type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    /// As the library hands it out.
    Stored,
    /// The interface form of a dual interface.
    DualInterface,
}

/// A type as `ITypeInfo` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeView {
    pub guid: Guid,
    pub kind: i32,
    pub flags: u16,
    pub major: u16,
    pub minor: u16,
    pub cb_size_vft: u16,
    pub cb_size_instance: u32,
    pub align: u16,
    pub funcs: Vec<Func>,
    pub vars: Vec<Var>,
    pub impl_types: Vec<ImplType>,
    pub alias: Option<TypeDesc>,
}

/// The member ids `IUnknown`'s and `IDispatch`'s methods have in a dual interface's dispatch form.
pub const MEMID_IUNKNOWN: i32 = 0x6000_0000;
pub const MEMID_IDISPATCH: i32 = 0x6001_0000;

/// `IUnknown`'s and `IDispatch`'s methods as a dual interface's dispatch form lists them.
fn dispatch_base_funcs(ptr: usize) -> Vec<Func> {
    let mut out = Vec::new();
    for (i, f) in stdole::iunknown_funcs().into_iter().chain(stdole::idispatch_funcs()).enumerate() {
        let memid = if i < 3 { MEMID_IUNKNOWN + i as i32 } else { MEMID_IDISPATCH + (i as i32 - 3) };
        out.push(Func { memid, funckind: FUNC_DISPATCH, ovft: (i * ptr) as i16, ..f });
    }
    out
}

/// A function's dispatch form: `FUNC_DISPATCH`, and an `HRESULT` result with an `[out, retval]`
/// parameter folded into a result of that parameter's type (`VT_VOID` without one).
pub fn fold(f: &Func) -> Func {
    let mut g = f.clone();
    g.funckind = FUNC_DISPATCH;
    if f.ret == TypeDesc::Base(VT_HRESULT) {
        match f.params.last() {
            Some(p) if p.flags & PARAMFLAG_FRETVAL != 0 => {
                g.ret = match &p.td {
                    TypeDesc::Ptr(inner) => (**inner).clone(),
                    other => other.clone(),
                };
                g.ret_flags = 0;
                g.params.pop();
            }
            _ => g.ret = TypeDesc::Base(VT_VOID),
        }
    }
    g
}

/// The view of type `t` of `lib` in `form` (`None`: no such type, or the interface form of a type
/// that is not dual).
pub fn view(lib: &Library, t: usize, form: Form) -> Option<TypeView> {
    let ty = lib.types.get(t)?;
    let ptr = lib.ptr_size();
    let dual = lib.is_dual(t);
    let mut v = TypeView {
        guid: ty.guid,
        kind: ty.kind,
        flags: ty.flags,
        major: ty.major,
        minor: ty.minor,
        cb_size_vft: 0,
        cb_size_instance: 0,
        align: ty.align,
        funcs: ty.funcs.clone(),
        vars: ty.vars.clone(),
        impl_types: ty.impl_types.clone(),
        alias: ty.alias.clone(),
    };
    match (form, dual) {
        (Form::DualInterface, false) => return None,
        (Form::DualInterface, true) => {
            v.kind = TKIND_INTERFACE;
            v.flags = ty.flags | TYPEFLAG_FOLEAUTOMATION;
            for (i, f) in v.funcs.iter_mut().enumerate() {
                f.funckind = FUNC_PUREVIRTUAL;
                f.ovft = ((7 + i) * ptr) as i16;
            }
            v.cb_size_vft = ((7 + ty.funcs.len()) * ptr) as u16;
        }
        (Form::Stored, true) => {
            v.kind = TKIND_DISPATCH;
            v.flags = ty.flags & !TYPEFLAG_FOLEAUTOMATION;
            let mut funcs = dispatch_base_funcs(ptr);
            for (i, f) in ty.funcs.iter().enumerate() {
                let mut g = fold(f);
                g.ovft = ((7 + i) * ptr) as i16;
                funcs.push(g);
            }
            v.funcs = funcs;
            v.cb_size_vft = (7 * ptr) as u16;
        }
        (Form::Stored, false) => {
            v.cb_size_vft = match ty.kind {
                TKIND_INTERFACE if ty.vft != 0 => ty.vft,
                TKIND_INTERFACE => {
                    let last = ty.funcs.iter().map(|f| f.ovft as usize + ptr).max();
                    let base = if ty.impl_types.is_empty() { 0 } else { 3 * ptr };
                    last.unwrap_or(base).max(base) as u16
                }
                TKIND_DISPATCH => (7 * ptr) as u16,
                _ => 0,
            };
            if ty.kind == TKIND_RECORD {
                v.cb_size_instance = record_size(&ty.vars, ptr);
            }
        }
    }
    Some(v)
}

/// The size of a record whose fields are `vars` (their offsets and sizes).
fn record_size(vars: &[Var], ptr: usize) -> u32 {
    vars.iter().filter(|v| v.varkind == 0).map(|v| v.value as u32 + size_of_td(&v.td, ptr)).max().unwrap_or(0)
}

/// The size of a value of `td` (pointers and references count as a pointer).
pub fn size_of_td(td: &TypeDesc, ptr: usize) -> u32 {
    match td {
        TypeDesc::Base(vt) => match *vt {
            VT_I1 | VT_UI1 => 1,
            VT_I2 | VT_UI2 | VT_BOOL => 2,
            VT_I4 | VT_UI4 | VT_R4 | VT_ERROR | VT_HRESULT | VT_INT | VT_UINT => 4,
            VT_I8 | VT_UI8 | VT_R8 | VT_CY | VT_DATE => 8,
            VT_VARIANT => {
                if ptr == 8 {
                    24
                } else {
                    16
                }
            }
            VT_DECIMAL => 16,
            _ => ptr as u32,
        },
        TypeDesc::CArray(t, dims) => size_of_td(t, ptr) * dims.iter().map(|d| d.0).product::<u32>(),
        _ => ptr as u32,
    }
}

/// The interface the stored form of type `t` holds functions of, for the marshaller: the type's
/// functions with their vtable offsets, `HRESULT` results and parameters as declared (a dual
/// interface's interface form; any other interface as stored).
pub fn vtable_funcs(lib: &Library, t: usize) -> Option<Vec<Func>> {
    let form = if lib.is_dual(t) { Form::DualInterface } else { Form::Stored };
    let v = view(lib, t, form)?;
    if v.kind != TKIND_INTERFACE {
        return None;
    }
    Some(v.funcs)
}

/// The names `ITypeInfo::GetNames` returns for a function: its name, then its parameters'.
pub fn names_of(f: &Func) -> Vec<String> {
    let mut v = vec![f.name.clone()];
    v.extend(f.params.iter().map(|p| p.name.clone()));
    v
}
