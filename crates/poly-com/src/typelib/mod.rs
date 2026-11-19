//! Type libraries (ADR 0220, B2b addendum): what `oleaut32`'s `ITypeLib`, `ITypeInfo`,
//! `ICreateTypeLib2` and the automation marshaller work on, as host-tested data.
//!
//! - this module: the model of a library (its attributes, its types with their functions,
//!   parameters, type descriptions and references to types of other libraries) and the builder
//!   rules of `ICreateTypeLib2` / `ICreateTypeInfo` (Learn: "ICreateTypeInfo::AddFuncDesc",
//!   "ICreateTypeInfo::LayOut", "ICreateTypeInfo::SetTypeFlags"; sheet `CreateTypeLib2`).
//! - [`view`]: how a stored type reads back through `ITypeInfo` — in particular a dual interface,
//!   which reads as a dispatch type holding `IUnknown`'s and `IDispatch`'s methods first, and whose
//!   interface form is reached through `GetRefTypeOfImplType(-1)`.
//! - [`file`]: the file Poly's `CreateTypeLib2` writes and its `LoadTypeLib` reads first, and the
//!   reader interface the `MSFT` format (B2b-2) plugs into ([`msft`]).
//! - [`reg`]: the registry rules (`RegisterTypeLib`, `LoadRegTypeLib`, `QueryPathOfRegTypeLib`,
//!   `UnRegisterTypeLib`): version and LCID matching, the keys written, resource paths.
//! - [`stdole`]: the standard library `stdole` 2.0 (`IUnknown`, `IDispatch`, ...), which every
//!   library refers to for the base of its interfaces.
//!
//! Constants from `oaidl.h`, `wtypes.h` and `oleauto.h`; error codes from `winerror.h`.

pub mod file;
pub mod msft;
pub mod reg;
pub mod stdole;
pub mod view;

use crate::guid::Guid;
use crate::hresult::Hresult;
use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

// ----- constants (oaidl.h, wtypes.h, winerror.h) ---------------------------------------------

pub const TKIND_ENUM: i32 = 0;
pub const TKIND_RECORD: i32 = 1;
pub const TKIND_MODULE: i32 = 2;
pub const TKIND_INTERFACE: i32 = 3;
pub const TKIND_DISPATCH: i32 = 4;
pub const TKIND_COCLASS: i32 = 5;
pub const TKIND_ALIAS: i32 = 6;
pub const TKIND_UNION: i32 = 7;

pub const TYPEFLAG_FAPPOBJECT: u16 = 0x1;
pub const TYPEFLAG_FHIDDEN: u16 = 0x10;
pub const TYPEFLAG_FDUAL: u16 = 0x40;
pub const TYPEFLAG_FNONEXTENSIBLE: u16 = 0x80;
pub const TYPEFLAG_FOLEAUTOMATION: u16 = 0x100;
pub const TYPEFLAG_FRESTRICTED: u16 = 0x200;
pub const TYPEFLAG_FDISPATCHABLE: u16 = 0x1000;
/// Every defined `TYPEFLAGS` bit (`FAPPOBJECT` .. `FPROXY`).
pub const TYPEFLAGS_ALL: u16 = 0x7FFF;

pub const LIBFLAG_FRESTRICTED: u16 = 0x1;
pub const LIBFLAG_FCONTROL: u16 = 0x2;
pub const LIBFLAG_FHIDDEN: u16 = 0x4;
pub const LIBFLAG_FHASDISKIMAGE: u16 = 0x8;

pub const FUNC_VIRTUAL: i32 = 0;
pub const FUNC_PUREVIRTUAL: i32 = 1;
pub const FUNC_NONVIRTUAL: i32 = 2;
pub const FUNC_STATIC: i32 = 3;
pub const FUNC_DISPATCH: i32 = 4;

pub const INVOKE_FUNC: i32 = 1;
pub const INVOKE_PROPERTYGET: i32 = 2;
pub const INVOKE_PROPERTYPUT: i32 = 4;
pub const INVOKE_PROPERTYPUTREF: i32 = 8;

pub const CC_STDCALL: i32 = 4;

pub const PARAMFLAG_FIN: u16 = 0x1;
pub const PARAMFLAG_FOUT: u16 = 0x2;
pub const PARAMFLAG_FLCID: u16 = 0x4;
pub const PARAMFLAG_FRETVAL: u16 = 0x8;
pub const PARAMFLAG_FOPT: u16 = 0x10;

pub const FUNCFLAG_FRESTRICTED: u16 = 0x1;

pub const SYS_WIN16: i32 = 0;
pub const SYS_WIN32: i32 = 1;
pub const SYS_MAC: i32 = 2;
pub const SYS_WIN64: i32 = 3;

pub const VT_EMPTY: u16 = 0;
pub const VT_NULL: u16 = 1;
pub const VT_I2: u16 = 2;
pub const VT_I4: u16 = 3;
pub const VT_R4: u16 = 4;
pub const VT_R8: u16 = 5;
pub const VT_CY: u16 = 6;
pub const VT_DATE: u16 = 7;
pub const VT_BSTR: u16 = 8;
pub const VT_DISPATCH: u16 = 9;
pub const VT_ERROR: u16 = 10;
pub const VT_BOOL: u16 = 11;
pub const VT_VARIANT: u16 = 12;
pub const VT_UNKNOWN: u16 = 13;
pub const VT_DECIMAL: u16 = 14;
pub const VT_I1: u16 = 16;
pub const VT_UI1: u16 = 17;
pub const VT_UI2: u16 = 18;
pub const VT_UI4: u16 = 19;
pub const VT_I8: u16 = 20;
pub const VT_UI8: u16 = 21;
pub const VT_INT: u16 = 22;
pub const VT_UINT: u16 = 23;
pub const VT_VOID: u16 = 24;
pub const VT_HRESULT: u16 = 25;
pub const VT_PTR: u16 = 26;
pub const VT_SAFEARRAY: u16 = 27;
pub const VT_CARRAY: u16 = 28;
pub const VT_USERDEFINED: u16 = 29;
pub const VT_LPSTR: u16 = 30;
pub const VT_LPWSTR: u16 = 31;
pub const VT_RECORD: u16 = 36;
pub const VT_INT_PTR: u16 = 37;
pub const VT_UINT_PTR: u16 = 38;
pub const VT_ARRAY: u16 = 0x2000;
pub const VT_BYREF: u16 = 0x4000;

/// `MEMBERID_NIL`.
pub const MEMBERID_NIL: i32 = -1;

pub const TYPE_E_ELEMENTNOTFOUND: Hresult = 0x8002_802B_u32 as i32;
pub const TYPE_E_AMBIGUOUSNAME: Hresult = 0x8002_802C_u32 as i32;
pub const TYPE_E_NAMECONFLICT: Hresult = 0x8002_802D_u32 as i32;
pub const TYPE_E_UNDEFINEDTYPE: Hresult = 0x8002_8027_u32 as i32;
pub const TYPE_E_BADMODULEKIND: Hresult = 0x8002_88BD_u32 as i32;
pub const TYPE_E_DUPLICATEID: Hresult = 0x8002_88C6_u32 as i32;
pub const TYPE_E_LIBNOTREGISTERED: Hresult = 0x8002_801D_u32 as i32;
pub const TYPE_E_REGISTRYACCESS: Hresult = 0x8002_801C_u32 as i32;
pub const TYPE_E_CANTLOADLIBRARY: Hresult = 0x8002_9C4A_u32 as i32;
pub const TYPE_E_INVALIDSTATE: Hresult = 0x8002_8029_u32 as i32;
pub const TYPE_E_WRONGTYPEKIND: Hresult = 0x8002_802A_u32 as i32;
pub const STG_E_FILENOTFOUND: Hresult = 0x8003_0002_u32 as i32;
pub const STG_E_PATHNOTFOUND: Hresult = 0x8003_0003_u32 as i32;

// ----- the model --------------------------------------------------------------------------------

/// A type description (`TYPEDESC`): a base type, a pointer to one, a safe array of one, a
/// reference to another type (an `HREFTYPE`), or a C array.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeDesc {
    Base(u16),
    Ptr(Box<TypeDesc>),
    SafeArray(Box<TypeDesc>),
    UserDefined(u32),
    CArray(Box<TypeDesc>, Vec<(u32, i32)>),
}

impl TypeDesc {
    /// The `vt` of the outermost level.
    pub fn vt(&self) -> u16 {
        match self {
            TypeDesc::Base(v) => *v,
            TypeDesc::Ptr(_) => VT_PTR,
            TypeDesc::SafeArray(_) => VT_SAFEARRAY,
            TypeDesc::UserDefined(_) => VT_USERDEFINED,
            TypeDesc::CArray(..) => VT_CARRAY,
        }
    }

    /// What a pointer or safe array points to.
    pub fn inner(&self) -> Option<&TypeDesc> {
        match self {
            TypeDesc::Ptr(t) | TypeDesc::SafeArray(t) | TypeDesc::CArray(t, _) => Some(t),
            _ => None,
        }
    }
}

/// One parameter of a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub td: TypeDesc,
    pub flags: u16,
    pub name: String,
}

/// One function (`FUNCDESC` and its names) as it was added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Func {
    pub memid: i32,
    pub funckind: i32,
    pub invkind: i32,
    pub callconv: i32,
    pub params: Vec<Param>,
    pub opt_params: i16,
    /// The vtable offset (computed at layout).
    pub ovft: i16,
    pub ret: TypeDesc,
    pub ret_flags: u16,
    pub flags: u16,
    pub name: String,
    pub doc: String,
    pub help_context: u32,
    /// Whether `SetFuncAndParamNames` has named it.
    pub named: bool,
}

/// One variable or record field (`VARDESC`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Var {
    pub memid: i32,
    pub name: String,
    pub td: TypeDesc,
    pub flags: u16,
    /// `VAR_PERINSTANCE` (0), `VAR_STATIC` (1), `VAR_CONST` (2), `VAR_DISPATCH` (3).
    pub varkind: i32,
    /// The offset of a field, or the value of a constant.
    pub value: i64,
}

/// One implemented (base) type: its `HREFTYPE` and `IMPLTYPEFLAGS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImplType {
    pub href: u32,
    pub flags: i32,
}

/// One type of a library as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeEntry {
    pub name: String,
    pub doc: String,
    pub help_context: u32,
    pub guid: Guid,
    pub kind: i32,
    pub flags: u16,
    pub major: u16,
    pub minor: u16,
    pub align: u16,
    pub impl_types: Vec<ImplType>,
    pub funcs: Vec<Func>,
    pub vars: Vec<Var>,
    pub alias: Option<TypeDesc>,
    /// `LayOut` succeeded since the last change.
    pub laid_out: bool,
    /// The vtable size `LayOut` computed (bytes).
    pub vft: u16,
}

/// A type of another library a library refers to (an imported `HREFTYPE`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub lib_guid: Guid,
    pub lib_major: u16,
    pub lib_minor: u16,
    pub lib_lcid: u32,
    pub type_guid: Guid,
    pub type_name: String,
}

/// A whole library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Library {
    pub guid: Guid,
    pub lcid: u32,
    pub syskind: i32,
    pub major: u16,
    pub minor: u16,
    pub flags: u16,
    pub name: String,
    pub doc: String,
    pub help_file: String,
    pub help_context: u32,
    pub types: Vec<TypeEntry>,
    pub imports: Vec<Import>,
}

// ----- HREFTYPE values ---------------------------------------------------------------------------
//
// Poly's own encoding (programs treat an HREFTYPE as opaque): bits 0..1 say what it names, the rest
// is an index.

const HREF_INTERNAL: u32 = 0;
const HREF_IMPORT: u32 = 1;
const HREF_DUAL_INTERFACE: u32 = 2;

/// What an `HREFTYPE` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Href {
    /// Type `i` of this library (as stored; a dual interface reads as its dispatch form).
    Internal(usize),
    /// Import `i` of this library.
    Import(usize),
    /// The interface form of the dual type `i` (what `GetRefTypeOfImplType(-1)` returns).
    DualInterface(usize),
}

impl Href {
    pub fn encode(self) -> u32 {
        match self {
            Href::Internal(i) => ((i as u32) << 2) | HREF_INTERNAL,
            Href::Import(i) => ((i as u32) << 2) | HREF_IMPORT,
            Href::DualInterface(i) => ((i as u32) << 2) | HREF_DUAL_INTERFACE,
        }
    }

    pub fn decode(h: u32) -> Option<Href> {
        let i = (h >> 2) as usize;
        match h & 3 {
            HREF_INTERNAL => Some(Href::Internal(i)),
            HREF_IMPORT => Some(Href::Import(i)),
            HREF_DUAL_INTERFACE => Some(Href::DualInterface(i)),
            _ => None,
        }
    }
}

/// Case-insensitive comparison of names (ASCII letters fold; Learn: names in a library are not
/// case sensitive).
pub fn same_name(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Library {
    /// An empty library being built with `CreateTypeLib2(syskind)`.
    pub fn new(syskind: i32) -> Library {
        Library {
            guid: Guid::default(),
            lcid: 0,
            syskind,
            major: 0,
            minor: 0,
            flags: 0,
            name: String::new(),
            doc: String::new(),
            help_file: String::new(),
            help_context: 0,
            types: Vec::new(),
            imports: Vec::new(),
        }
    }

    /// The size of a vtable slot: the running process's pointer (a 32-bit library loaded in a
    /// 64-bit process lays out with 8-byte slots, sheet `LoadRegTypeLib`, rule `stdole`).
    pub fn ptr_size(&self) -> usize {
        core::mem::size_of::<usize>()
    }

    /// `ICreateTypeLib::CreateTypeInfo`: a new type at the end; `TYPE_E_NAMECONFLICT` when a type
    /// of the library already has the name (in any case).
    pub fn create_type(&mut self, name: &str, kind: i32) -> Result<usize, Hresult> {
        if !name.is_empty() && self.types.iter().any(|t| same_name(&t.name, name)) {
            return Err(TYPE_E_NAMECONFLICT);
        }
        self.types.push(TypeEntry {
            name: String::from(name),
            doc: String::new(),
            help_context: 0,
            guid: Guid::default(),
            kind,
            flags: 0,
            major: 0,
            minor: 0,
            align: if self.ptr_size() == 8 { 8 } else { 4 },
            impl_types: Vec::new(),
            funcs: Vec::new(),
            vars: Vec::new(),
            alias: None,
            laid_out: false,
            vft: 0,
        });
        Ok(self.types.len() - 1)
    }

    /// The index of a type by GUID.
    pub fn find_guid(&self, g: &Guid) -> Option<usize> {
        self.types.iter().position(|t| t.guid == *g && *g != Guid::default())
    }

    /// The index of a type by name (any case).
    pub fn find_name(&self, name: &str) -> Option<usize> {
        self.types.iter().position(|t| same_name(&t.name, name))
    }

    /// The `HREFTYPE` of an import, adding it when new.
    pub fn import(&mut self, imp: Import) -> u32 {
        let i = match self.imports.iter().position(|x| x.lib_guid == imp.lib_guid && x.type_guid == imp.type_guid) {
            Some(i) => i,
            None => {
                self.imports.push(imp);
                self.imports.len() - 1
            }
        };
        Href::Import(i).encode()
    }

    /// The import an `HREFTYPE` names, when it names one.
    pub fn import_of(&self, href: u32) -> Option<&Import> {
        match Href::decode(href)? {
            Href::Import(i) => self.imports.get(i),
            _ => None,
        }
    }

    /// `ICreateTypeInfo::SetTypeFlags`: undefined bits and `FOLEAUTOMATION` on a dispatch type
    /// are `TYPE_E_BADMODULEKIND` (sheet).
    pub fn set_type_flags(&mut self, t: usize, flags: u16) -> Result<(), Hresult> {
        let ty = self.types.get_mut(t).ok_or(TYPE_E_ELEMENTNOTFOUND)?;
        if flags & !TYPEFLAGS_ALL != 0 || flags == 0xFFFF {
            return Err(TYPE_E_BADMODULEKIND);
        }
        if ty.kind == TKIND_DISPATCH && flags & TYPEFLAG_FOLEAUTOMATION != 0 {
            return Err(TYPE_E_BADMODULEKIND);
        }
        ty.flags = flags;
        ty.laid_out = false;
        Ok(())
    }

    /// `ICreateTypeInfo::AddFuncDesc` at `index` (inserting; the count is the highest index).
    pub fn add_func(&mut self, t: usize, index: usize, f: Func) -> Result<(), Hresult> {
        let ty = self.types.get_mut(t).ok_or(TYPE_E_ELEMENTNOTFOUND)?;
        if index > ty.funcs.len() {
            return Err(TYPE_E_ELEMENTNOTFOUND);
        }
        ty.funcs.insert(index, f);
        ty.laid_out = false;
        Ok(())
    }

    /// `ICreateTypeInfo::SetFuncAndParamNames`: the function's name, then its parameters'; a
    /// name another function of the type already has is `TYPE_E_AMBIGUOUSNAME`.
    pub fn set_func_names(&mut self, t: usize, index: usize, names: &[String]) -> Result<(), Hresult> {
        let ty = self.types.get_mut(t).ok_or(TYPE_E_ELEMENTNOTFOUND)?;
        let Some(first) = names.first() else { return Err(crate::hresult::E_INVALIDARG) };
        if index >= ty.funcs.len() {
            return Err(TYPE_E_ELEMENTNOTFOUND);
        }
        // Property accessors share a name (get / put of one property).
        let inv = ty.funcs[index].invkind;
        let clash = ty.funcs.iter().enumerate().any(|(i, f)| {
            i != index && f.named && same_name(&f.name, first) && (f.invkind == inv || (f.invkind | inv) & 1 != 0)
        });
        if clash {
            return Err(TYPE_E_AMBIGUOUSNAME);
        }
        let f = &mut ty.funcs[index];
        if names.len() > f.params.len() + 1 {
            return Err(crate::hresult::E_INVALIDARG);
        }
        f.name = first.clone();
        for (p, n) in f.params.iter_mut().zip(names[1..].iter()) {
            p.name = n.clone();
        }
        f.named = true;
        Ok(())
    }

    /// The vtable size of the type `t` (in bytes) as its functions lay out: the base interface's
    /// part (`base_vft`, from the referenced type) and one pointer per function.
    pub fn layout(&mut self, t: usize, base_vft: usize) -> Result<(), Hresult> {
        let ptr = self.ptr_size();
        let ty = self.types.get_mut(t).ok_or(TYPE_E_ELEMENTNOTFOUND)?;
        for (i, f) in ty.funcs.iter().enumerate() {
            if ty.funcs[..i].iter().any(|g| g.memid == f.memid && g.invkind == f.invkind) {
                return Err(TYPE_E_DUPLICATEID);
            }
        }
        if matches!(ty.kind, TKIND_INTERFACE | TKIND_DISPATCH) {
            for (i, f) in ty.funcs.iter_mut().enumerate() {
                f.ovft = (base_vft + i * ptr) as i16;
            }
            ty.vft = (base_vft + ty.funcs.len() * ptr) as u16;
        }
        ty.laid_out = true;
        Ok(())
    }

    /// What `SaveAllChanges` refuses: an interface holding functions with no base interface
    /// (`TYPE_E_UNDEFINEDTYPE`, sheet), a type whose layout fails.
    pub fn check_save(&self) -> Result<(), Hresult> {
        for ty in &self.types {
            if matches!(ty.kind, TKIND_INTERFACE) && !ty.funcs.is_empty() && ty.impl_types.is_empty() {
                return Err(TYPE_E_UNDEFINEDTYPE);
            }
            for (i, f) in ty.funcs.iter().enumerate() {
                if ty.funcs[..i].iter().any(|g| g.memid == f.memid && g.invkind == f.invkind) {
                    return Err(TYPE_E_DUPLICATEID);
                }
            }
        }
        Ok(())
    }

    /// Whether type `t` is a dual interface (stored with the dual flag, made as an interface or as
    /// a dispatch type).
    pub fn is_dual(&self, t: usize) -> bool {
        self.types
            .get(t)
            .is_some_and(|ty| ty.flags & TYPEFLAG_FDUAL != 0 && matches!(ty.kind, TKIND_INTERFACE | TKIND_DISPATCH))
    }
}

#[cfg(test)]
mod tests;
