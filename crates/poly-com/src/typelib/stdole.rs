//! The standard library `stdole` 2.0 (`{00020430-0000-0000-C000-000000000046}`, file
//! `stdole2.tlb`), Poly's own copy: the types every other library refers to for the base of its
//! interfaces. Contents from the sheet `LoadRegTypeLib` (rule `stdole`: name, doc string, syskind,
//! flags, the first six types and their order) and the declarations of `IUnknown`, `IDispatch`,
//! `IEnumVARIANT`, `GUID`, `DISPPARAMS` and `EXCEPINFO` in the SDK's `unknwn.idl` / `oaidl.idl`.
//! Poly installs it as `C:\Windows\System32\stdole2.tlb` in its own file format.

use super::*;
use alloc::vec;

pub const LIBID: Guid = Guid { data1: 0x0002_0430, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IUNKNOWN: Guid = Guid { data1: 0, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IDISPATCH: Guid = Guid { data1: 0x0002_0400, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
pub const IID_IENUMVARIANT: Guid =
    Guid { data1: 0x0002_0404, data2: 0, data3: 0, data4: [0xC0, 0, 0, 0, 0, 0, 0, 0x46] };
/// The file name it is registered under.
pub const FILE_NAME: &str = "stdole2.tlb";

/// The indices of its types.
pub const T_GUID: usize = 0;
pub const T_DISPPARAMS: usize = 1;
pub const T_EXCEPINFO: usize = 2;
pub const T_IUNKNOWN: usize = 3;
pub const T_IDISPATCH: usize = 4;
pub const T_IENUMVARIANT: usize = 5;

fn b(vt: u16) -> TypeDesc {
    TypeDesc::Base(vt)
}

fn ptr(t: TypeDesc) -> TypeDesc {
    TypeDesc::Ptr(Box::new(t))
}

fn udt(t: usize) -> TypeDesc {
    TypeDesc::UserDefined(Href::Internal(t).encode())
}

fn p(name: &str, td: TypeDesc, flags: u16) -> Param {
    Param { td, flags, name: String::from(name) }
}

fn f(memid: i32, name: &str, params: Vec<Param>, ret: TypeDesc) -> Func {
    Func {
        memid,
        funckind: FUNC_PUREVIRTUAL,
        invkind: INVOKE_FUNC,
        callconv: CC_STDCALL,
        params,
        opt_params: 0,
        ovft: 0,
        ret,
        ret_flags: 0,
        flags: FUNCFLAG_FRESTRICTED,
        name: String::from(name),
        doc: String::new(),
        help_context: 0,
        named: true,
    }
}

const IN: u16 = PARAMFLAG_FIN;
const OUT: u16 = PARAMFLAG_FOUT;

/// `IUnknown`'s three methods (references to `GUID` as `stdole` holds them).
fn iunknown_raw(guid_ref: TypeDesc) -> Vec<Func> {
    vec![
        f(
            0x6000_0000,
            "QueryInterface",
            vec![p("riid", ptr(guid_ref), IN), p("ppvObj", ptr(ptr(b(VT_VOID))), OUT)],
            b(VT_HRESULT),
        ),
        f(0x6000_0001, "AddRef", vec![], b(VT_UI4)),
        f(0x6000_0002, "Release", vec![], b(VT_UI4)),
    ]
}

/// `IDispatch`'s four methods.
fn idispatch_raw(guid_ref: TypeDesc, dp_ref: TypeDesc, ei_ref: TypeDesc) -> Vec<Func> {
    vec![
        f(0x6001_0000, "GetTypeInfoCount", vec![p("pctinfo", ptr(b(VT_UINT)), OUT)], b(VT_HRESULT)),
        f(
            0x6001_0001,
            "GetTypeInfo",
            vec![p("itinfo", b(VT_UINT), IN), p("lcid", b(VT_UI4), IN), p("pptinfo", ptr(ptr(b(VT_VOID))), OUT)],
            b(VT_HRESULT),
        ),
        f(
            0x6001_0002,
            "GetIDsOfNames",
            vec![
                p("riid", ptr(guid_ref.clone()), IN),
                p("rgszNames", ptr(ptr(b(VT_I1))), IN),
                p("cNames", b(VT_UINT), IN),
                p("lcid", b(VT_UI4), IN),
                p("rgdispid", ptr(b(VT_I4)), OUT),
            ],
            b(VT_HRESULT),
        ),
        f(
            0x6001_0003,
            "Invoke",
            vec![
                p("dispidMember", b(VT_I4), IN),
                p("riid", ptr(guid_ref), IN),
                p("lcid", b(VT_UI4), IN),
                p("wFlags", b(VT_UI2), IN),
                p("pdispparams", ptr(dp_ref), IN),
                p("pvarResult", ptr(b(VT_VARIANT)), OUT),
                p("pexcepinfo", ptr(ei_ref), OUT),
                p("puArgErr", ptr(b(VT_UINT)), OUT),
            ],
            b(VT_HRESULT),
        ),
    ]
}

/// `IUnknown`'s methods as another library's dual interface lists them (no references into
/// `stdole`: its `GUID` reads as `void`).
pub fn iunknown_funcs() -> Vec<Func> {
    iunknown_raw(b(VT_VOID))
}

/// `IDispatch`'s methods likewise.
pub fn idispatch_funcs() -> Vec<Func> {
    idispatch_raw(b(VT_VOID), b(VT_VOID), b(VT_VOID))
}

fn var(name: &str, td: TypeDesc, offset: i64) -> Var {
    Var { memid: MEMBERID_NIL, name: String::from(name), td, flags: 0, varkind: 0, value: offset }
}

fn record(lib: &mut Library, name: &str, vars: Vec<Var>) {
    let t = lib.create_type(name, TKIND_RECORD).unwrap_or(0);
    lib.types[t].vars = vars;
    lib.types[t].laid_out = true;
}

fn interface(lib: &mut Library, name: &str, guid: Guid, flags: u16, base: Option<usize>, funcs: Vec<Func>) {
    let t = lib.create_type(name, TKIND_INTERFACE).unwrap_or(0);
    let ty = &mut lib.types[t];
    ty.guid = guid;
    ty.flags = flags;
    if let Some(bi) = base {
        ty.impl_types.push(ImplType { href: Href::Internal(bi).encode(), flags: 0 });
    }
    ty.funcs = funcs;
    let base_vft = match base {
        Some(T_IUNKNOWN) => 3,
        Some(T_IDISPATCH) => 7,
        _ => 0,
    } * lib.ptr_size();
    let _ = lib.layout(t, base_vft);
}

/// The library.
pub fn library() -> Library {
    let mut lib = Library::new(SYS_WIN32);
    lib.guid = LIBID;
    lib.major = 2;
    lib.minor = 0;
    lib.flags = LIBFLAG_FHASDISKIMAGE;
    lib.name = String::from("stdole");
    lib.doc = String::from("OLE Automation");
    let w = lib.ptr_size() as i64;
    record(
        &mut lib,
        "GUID",
        vec![
            var("Data1", b(VT_UI4), 0),
            var("Data2", b(VT_UI2), 4),
            var("Data3", b(VT_UI2), 6),
            var("Data4", TypeDesc::CArray(Box::new(b(VT_UI1)), vec![(8, 0)]), 8),
        ],
    );
    record(
        &mut lib,
        "DISPPARAMS",
        vec![
            var("rgvarg", ptr(b(VT_VARIANT)), 0),
            var("rgdispidNamedArgs", ptr(b(VT_I4)), w),
            var("cArgs", b(VT_UINT), 2 * w),
            var("cNamedArgs", b(VT_UINT), 2 * w + 4),
        ],
    );
    record(
        &mut lib,
        "EXCEPINFO",
        vec![
            var("wCode", b(VT_UI2), 0),
            var("wReserved", b(VT_UI2), 2),
            var("bstrSource", b(VT_BSTR), w),
            var("bstrDescription", b(VT_BSTR), 2 * w),
            var("bstrHelpFile", b(VT_BSTR), 3 * w),
            var("dwHelpContext", b(VT_UI4), 4 * w),
            var("pvReserved", ptr(b(VT_VOID)), 5 * w),
            var("pfnDeferredFillIn", ptr(b(VT_VOID)), 6 * w),
            var("scode", b(VT_ERROR), 7 * w),
        ],
    );
    interface(&mut lib, "IUnknown", IID_IUNKNOWN, TYPEFLAG_FHIDDEN, None, iunknown_raw(udt(T_GUID)));
    interface(
        &mut lib,
        "IDispatch",
        IID_IDISPATCH,
        0,
        Some(T_IUNKNOWN),
        idispatch_raw(udt(T_GUID), udt(T_DISPPARAMS), udt(T_EXCEPINFO)),
    );
    let mut enum_funcs = vec![
        f(
            0x6002_0000,
            "Next",
            vec![p("celt", b(VT_UI4), IN), p("rgvar", ptr(b(VT_VARIANT)), IN), p("pceltFetched", ptr(b(VT_UI4)), OUT)],
            b(VT_HRESULT),
        ),
        f(0x6002_0001, "Skip", vec![p("celt", b(VT_UI4), IN)], b(VT_HRESULT)),
        f(0x6002_0002, "Reset", vec![], b(VT_HRESULT)),
        f(0x6002_0003, "Clone", vec![p("ppenum", ptr(ptr(udt(T_IENUMVARIANT))), OUT)], b(VT_HRESULT)),
    ];
    for g in &mut enum_funcs {
        g.flags = 0;
    }
    interface(&mut lib, "IEnumVARIANT", IID_IENUMVARIANT, TYPEFLAG_FHIDDEN, Some(T_IUNKNOWN), enum_funcs);
    lib
}

/// The file Poly installs as `stdole2.tlb`.
pub fn file_bytes() -> Vec<u8> {
    super::file::encode(&library())
}
