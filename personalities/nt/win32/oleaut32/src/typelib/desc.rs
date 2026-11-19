//! The C structures `ITypeLib` / `ITypeInfo` hand out and take (`oaidl.h`): `TYPEDESC`,
//! `ELEMDESC`, `FUNCDESC`, `VARDESC`, `TYPEATTR`, `TLIBATTR`, made from the model and read into it.
//! What a `Get*` method returns is one heap block whose first field is the structure; its
//! `Release*` method frees the whole block.

use crate::com::*;
use alloc::boxed::Box;
use alloc::vec::Vec;
use poly_com::typelib::view::TypeView;
use poly_com::typelib::{Func, Library, Param, TypeDesc, VT_CARRAY, VT_PTR, VT_SAFEARRAY, VT_USERDEFINED, Var};

/// `TYPEDESC`: the union (a pointer to a `TYPEDESC`, an `ARRAYDESC`, or an `HREFTYPE`) and `vt`.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CTypeDesc {
    pub u: usize,
    pub vt: u16,
}

/// `PARAMDESC` (the arm of `ELEMDESC`'s union used here).
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CParamDesc {
    pub ex: usize,
    pub flags: u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct CElemDesc {
    pub td: CTypeDesc,
    pub pd: CParamDesc,
}

#[repr(C)]
pub(crate) struct CFuncDesc {
    pub memid: i32,
    pub scodes: *mut i32,
    pub params: *mut CElemDesc,
    pub funckind: i32,
    pub invkind: i32,
    pub callconv: i32,
    pub c_params: i16,
    pub c_params_opt: i16,
    pub ovft: i16,
    pub c_scodes: i16,
    pub ret: CElemDesc,
    pub flags: u16,
}

#[repr(C)]
pub(crate) struct CVarDesc {
    pub memid: i32,
    pub schema: *mut u16,
    pub u: usize,
    pub elem: CElemDesc,
    pub flags: u16,
    pub varkind: i32,
}

#[repr(C)]
pub(crate) struct CTypeAttr {
    pub guid: Guid,
    pub lcid: u32,
    pub reserved: u32,
    pub memid_ctor: i32,
    pub memid_dtor: i32,
    pub schema: *mut u16,
    pub cb_size_instance: u32,
    pub typekind: i32,
    pub c_funcs: u16,
    pub c_vars: u16,
    pub c_impl_types: u16,
    pub cb_size_vft: u16,
    pub cb_alignment: u16,
    pub flags: u16,
    pub major: u16,
    pub minor: u16,
    pub alias: CTypeDesc,
    pub idl: [usize; 2],
}

#[repr(C)]
pub(crate) struct CLibAttr {
    pub guid: Guid,
    pub lcid: u32,
    pub syskind: i32,
    pub major: u16,
    pub minor: u16,
    pub flags: u16,
}

/// What the nested type descriptions of one block need to live.
#[derive(Default)]
pub(crate) struct Arena {
    tds: Vec<Box<CTypeDesc>>,
    arrays: Vec<Vec<u64>>,
}

impl Arena {
    /// The C form of `t`; nested parts live in the arena.
    pub(crate) fn td(&mut self, t: &TypeDesc) -> CTypeDesc {
        match t {
            TypeDesc::Base(vt) => CTypeDesc { u: 0, vt: *vt },
            TypeDesc::Ptr(i) | TypeDesc::SafeArray(i) => {
                let inner = Box::new(self.td(i));
                let p = &*inner as *const CTypeDesc as usize;
                self.tds.push(inner);
                CTypeDesc { u: p, vt: t.vt() }
            }
            TypeDesc::UserDefined(h) => CTypeDesc { u: *h as usize, vt: VT_USERDEFINED },
            TypeDesc::CArray(i, dims) => {
                // ARRAYDESC: tdescElem (16 bytes on x64), cDims, then SAFEARRAYBOUNDs; kept in u64s.
                let elem = self.td(i);
                let words = 2 + 1 + dims.len();
                let mut v = alloc::vec![0u64; words];
                let base = v.as_mut_ptr() as *mut u8;
                // SAFETY: `v` holds the ARRAYDESC's bytes: the element type, the count, the bounds.
                unsafe {
                    base.cast::<CTypeDesc>().write(elem);
                    let after = core::mem::size_of::<CTypeDesc>();
                    base.add(after).cast::<u16>().write(dims.len() as u16);
                    for (k, (n, lo)) in dims.iter().enumerate() {
                        base.add(after + 4 + 8 * k).cast::<u32>().write(*n);
                        base.add(after + 8 + 8 * k).cast::<i32>().write(*lo);
                    }
                }
                let p = v.as_ptr() as usize;
                self.arrays.push(v);
                CTypeDesc { u: p, vt: VT_CARRAY }
            }
        }
    }
}

/// The model's form of a C type description.
///
/// # Safety
/// `t` and what it points to are valid type descriptions.
pub(crate) unsafe fn read_td(t: &CTypeDesc, depth: u32) -> Option<TypeDesc> {
    if depth > 32 {
        return None;
    }
    Some(match t.vt {
        VT_PTR | VT_SAFEARRAY => {
            if t.u == 0 {
                return None;
            }
            // SAFETY: the caller's chain.
            let inner = unsafe { read_td(&*(t.u as *const CTypeDesc), depth + 1)? };
            if t.vt == VT_PTR { TypeDesc::Ptr(Box::new(inner)) } else { TypeDesc::SafeArray(Box::new(inner)) }
        }
        VT_USERDEFINED => TypeDesc::UserDefined(t.u as u32),
        VT_CARRAY => {
            if t.u == 0 {
                return None;
            }
            let base = t.u as *const u8;
            // SAFETY: the caller's ARRAYDESC.
            unsafe {
                let elem = read_td(&*base.cast::<CTypeDesc>(), depth + 1)?;
                let after = core::mem::size_of::<CTypeDesc>();
                let n = base.add(after).cast::<u16>().read() as usize;
                let dims = (0..n)
                    .map(|k| {
                        (
                            base.add(after + 4 + 8 * k).cast::<u32>().read(),
                            base.add(after + 8 + 8 * k).cast::<i32>().read(),
                        )
                    })
                    .collect();
                TypeDesc::CArray(Box::new(elem), dims)
            }
        }
        vt => TypeDesc::Base(vt),
    })
}

/// A `FUNCDESC` block.
#[repr(C)]
struct FuncBlock {
    fd: CFuncDesc,
    elems: Vec<CElemDesc>,
    arena: Arena,
}

/// A new `FUNCDESC` for `f` (freed by [`free_func`]).
pub(crate) fn make_func(f: &Func) -> *mut CFuncDesc {
    let mut arena = Arena::default();
    let mut elems: Vec<CElemDesc> =
        f.params.iter().map(|p| CElemDesc { td: arena.td(&p.td), pd: CParamDesc { ex: 0, flags: p.flags } }).collect();
    let ret = CElemDesc { td: arena.td(&f.ret), pd: CParamDesc { ex: 0, flags: f.ret_flags } };
    let params = if elems.is_empty() { core::ptr::null_mut() } else { elems.as_mut_ptr() };
    let b = Box::new(FuncBlock {
        fd: CFuncDesc {
            memid: f.memid,
            scodes: core::ptr::null_mut(),
            params,
            funckind: f.funckind,
            invkind: f.invkind,
            callconv: f.callconv,
            c_params: f.params.len() as i16,
            c_params_opt: f.opt_params,
            ovft: f.ovft,
            c_scodes: 0,
            ret,
            flags: f.flags,
        },
        elems,
        arena,
    });
    Box::into_raw(b).cast()
}

/// Free a block from [`make_func`].
pub(crate) fn free_func(p: *mut CFuncDesc) {
    if !p.is_null() {
        // SAFETY: a block from `make_func` (the FUNCDESC is its first field).
        drop(unsafe { Box::from_raw(p.cast::<FuncBlock>()) });
    }
}

/// The model's form of a `FUNCDESC` given to `AddFuncDesc` (names come later).
///
/// # Safety
/// `fd` is a valid `FUNCDESC` with `cParams` parameters.
pub(crate) unsafe fn read_func(fd: &CFuncDesc) -> Option<Func> {
    let mut params = Vec::new();
    for i in 0..fd.c_params.max(0) as usize {
        if fd.params.is_null() {
            return None;
        }
        // SAFETY: the caller's array of `cParams` ELEMDESCs.
        let e = unsafe { &*fd.params.add(i) };
        // SAFETY: as above.
        params.push(Param { td: unsafe { read_td(&e.td, 0)? }, flags: e.pd.flags, name: alloc::string::String::new() });
    }
    Some(Func {
        memid: fd.memid,
        funckind: fd.funckind,
        invkind: fd.invkind,
        callconv: fd.callconv,
        params,
        opt_params: fd.c_params_opt,
        ovft: fd.ovft,
        // SAFETY: the caller's result description.
        ret: unsafe { read_td(&fd.ret.td, 0)? },
        ret_flags: fd.ret.pd.flags,
        flags: fd.flags,
        name: alloc::string::String::new(),
        doc: alloc::string::String::new(),
        help_context: 0,
        named: false,
    })
}

#[repr(C)]
struct VarBlock {
    vd: CVarDesc,
    arena: Arena,
    value: [u64; 3],
}

/// A new `VARDESC` for `v` (freed by [`free_var`]).
pub(crate) fn make_var(v: &Var) -> *mut CVarDesc {
    let mut arena = Arena::default();
    let td = arena.td(&v.td);
    let mut b = Box::new(VarBlock {
        vd: CVarDesc {
            memid: v.memid,
            schema: core::ptr::null_mut(),
            u: v.value as usize,
            elem: CElemDesc { td, pd: CParamDesc { ex: 0, flags: 0 } },
            flags: v.flags,
            varkind: v.varkind,
        },
        arena,
        value: [0; 3],
    });
    if v.varkind == 2 {
        // VAR_CONST: lpvarValue points to a VARIANT holding the value (VT_I4).
        b.value[0] = 3;
        b.value[1] = v.value as u64;
        b.vd.u = b.value.as_ptr() as usize;
    }
    Box::into_raw(b).cast()
}

pub(crate) fn free_var(p: *mut CVarDesc) {
    if !p.is_null() {
        // SAFETY: a block from `make_var`.
        drop(unsafe { Box::from_raw(p.cast::<VarBlock>()) });
    }
}

#[repr(C)]
struct AttrBlock {
    a: CTypeAttr,
    arena: Arena,
}

/// A new `TYPEATTR` for a type view (freed by [`free_attr`]).
pub(crate) fn make_attr(lib: &Library, v: &TypeView) -> *mut CTypeAttr {
    let mut arena = Arena::default();
    let alias = match &v.alias {
        Some(t) => arena.td(t),
        None => CTypeDesc { u: 0, vt: 0 },
    };
    let b = Box::new(AttrBlock {
        a: CTypeAttr {
            guid: v.guid,
            lcid: lib.lcid,
            reserved: 0,
            memid_ctor: -1,
            memid_dtor: -1,
            schema: core::ptr::null_mut(),
            cb_size_instance: v.cb_size_instance,
            typekind: v.kind,
            c_funcs: v.funcs.len() as u16,
            c_vars: v.vars.len() as u16,
            c_impl_types: v.impl_types.len() as u16,
            cb_size_vft: v.cb_size_vft,
            cb_alignment: v.align,
            flags: v.flags,
            major: v.major,
            minor: v.minor,
            alias,
            idl: [0; 2],
        },
        arena,
    });
    Box::into_raw(b).cast()
}

pub(crate) fn free_attr(p: *mut CTypeAttr) {
    if !p.is_null() {
        // SAFETY: a block from `make_attr`.
        drop(unsafe { Box::from_raw(p.cast::<AttrBlock>()) });
    }
}

/// A new `TLIBATTR` (freed by [`free_lib_attr`]).
pub(crate) fn make_lib_attr(lib: &Library) -> *mut CLibAttr {
    Box::into_raw(Box::new(CLibAttr {
        guid: lib.guid,
        lcid: lib.lcid,
        syskind: lib.syskind,
        major: lib.major,
        minor: lib.minor,
        flags: lib.flags,
    }))
}

pub(crate) fn free_lib_attr(p: *mut CLibAttr) {
    if !p.is_null() {
        // SAFETY: a block from `make_lib_attr`.
        drop(unsafe { Box::from_raw(p) });
    }
}
