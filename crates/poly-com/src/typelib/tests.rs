use super::view::{Form, view, vtable_funcs};
use super::*;
use alloc::string::ToString;
use alloc::vec;

const G: Guid = Guid { data1: 0xB2B0_A001, data2: 0x1234, data3: 0x4000, data4: [0x80, 0, 0x70, 0x6F, 0x6C, 0x79, 0x77, 0x73] };

fn func(memid: i32, name: &str, params: Vec<Param>) -> Func {
    Func {
        memid,
        funckind: FUNC_PUREVIRTUAL,
        invkind: INVOKE_FUNC,
        callconv: CC_STDCALL,
        params,
        opt_params: 0,
        ovft: 0,
        ret: TypeDesc::Base(VT_HRESULT),
        ret_flags: 0,
        flags: 0,
        name: name.to_string(),
        doc: String::new(),
        help_context: 0,
        named: true,
    }
}

fn param(name: &str, td: TypeDesc, flags: u16) -> Param {
    Param { td, flags, name: name.to_string() }
}

fn pi4() -> TypeDesc {
    TypeDesc::Ptr(Box::new(TypeDesc::Base(VT_I4)))
}

/// A library like the tests': a custom interface and a dual one.
fn sample(dual_kind: i32) -> Library {
    let mut lib = Library::new(SYS_WIN64);
    lib.guid = G;
    lib.major = 1;
    lib.name = "PolyWsTl".to_string();
    lib.doc = "Poly test library".to_string();
    let unk = lib.import(Import {
        lib_guid: stdole::LIBID,
        lib_major: 2,
        lib_minor: 0,
        lib_lcid: 0,
        type_guid: stdole::IID_IUNKNOWN,
        type_name: "IUnknown".to_string(),
    });
    let disp = lib.import(Import {
        lib_guid: stdole::LIBID,
        lib_major: 2,
        lib_minor: 0,
        lib_lcid: 0,
        type_guid: stdole::IID_IDISPATCH,
        type_name: "IDispatch".to_string(),
    });
    let e = lib.create_type("ITlEcho", TKIND_INTERFACE).unwrap();
    lib.types[e].guid = Guid { data1: 0xB2B0_A002, ..G };
    lib.set_type_flags(e, TYPEFLAG_FOLEAUTOMATION | TYPEFLAG_FNONEXTENSIBLE).unwrap();
    lib.types[e].impl_types.push(ImplType { href: unk, flags: 0 });
    let add = func(
        0x6001,
        "AddLong",
        vec![
            param("a", TypeDesc::Base(VT_I4), PARAMFLAG_FIN),
            param("b", TypeDesc::Base(VT_I4), PARAMFLAG_FIN),
            param("r", pi4(), PARAMFLAG_FOUT | PARAMFLAG_FRETVAL),
        ],
    );
    lib.add_func(e, 0, add.clone()).unwrap();
    lib.layout(e, 24).unwrap();
    let d = lib.create_type("ITlDual", dual_kind).unwrap();
    lib.types[d].guid = Guid { data1: 0xB2B0_A004, ..G };
    let flags = if dual_kind == TKIND_DISPATCH {
        TYPEFLAG_FDUAL | TYPEFLAG_FDISPATCHABLE | TYPEFLAG_FNONEXTENSIBLE
    } else {
        TYPEFLAG_FDUAL | TYPEFLAG_FOLEAUTOMATION | TYPEFLAG_FDISPATCHABLE | TYPEFLAG_FNONEXTENSIBLE
    };
    lib.set_type_flags(d, flags).unwrap();
    lib.types[d].impl_types.push(ImplType { href: disp, flags: 0 });
    lib.add_func(d, 0, Func { memid: 1, ..add.clone() }).unwrap();
    lib.add_func(d, 1, func(4, "FailHr", vec![param("hr", TypeDesc::Base(VT_I4), PARAMFLAG_FIN)])).unwrap();
    lib.layout(d, 56).unwrap();
    lib
}

#[test]
fn interface_reads_back_as_added() {
    let lib = sample(TKIND_INTERFACE);
    let v = view(&lib, 0, Form::Stored).unwrap();
    assert_eq!((v.kind, v.flags, v.cb_size_vft), (TKIND_INTERFACE, 0x180, 32));
    assert_eq!(v.funcs[0].ovft, 24);
    assert_eq!(v.funcs[0].params.len(), 3);
    assert_eq!(v.funcs[0].params[2].flags, 0xa);
    assert!(view(&lib, 0, Form::DualInterface).is_none());
}

#[test]
fn dual_reads_back_as_dispatch_with_folded_results() {
    for kind in [TKIND_INTERFACE, TKIND_DISPATCH] {
        let lib = sample(kind);
        let v = view(&lib, 1, Form::Stored).unwrap();
        assert_eq!((v.kind, v.flags, v.cb_size_vft), (TKIND_DISPATCH, 0x10c0, 56));
        assert_eq!(v.funcs.len(), 9);
        let ids: Vec<i32> = v.funcs.iter().map(|f| f.memid).collect();
        assert_eq!(&ids[..7], &[0x6000_0000, 0x6000_0001, 0x6000_0002, 0x6001_0000, 0x6001_0001, 0x6001_0002, 0x6001_0003]);
        assert_eq!(v.funcs[2].ovft, 16);
        let add = &v.funcs[7];
        assert_eq!((add.funckind, add.params.len(), add.ret.clone(), add.ovft), (FUNC_DISPATCH, 2, TypeDesc::Base(VT_I4), 56));
        assert_eq!(v.funcs[8].ret, TypeDesc::Base(VT_VOID));
        let i = view(&lib, 1, Form::DualInterface).unwrap();
        assert_eq!((i.kind, i.cb_size_vft, i.funcs.len()), (TKIND_INTERFACE, 72, 2));
        assert_eq!((i.funcs[0].funckind, i.funcs[0].params.len(), i.funcs[1].ovft), (FUNC_PUREVIRTUAL, 3, 64));
        assert_eq!(vtable_funcs(&lib, 1).unwrap().len(), 2);
    }
}

#[test]
fn builder_rules() {
    let mut lib = Library::new(SYS_WIN64);
    let t = lib.create_type("T3", TKIND_INTERFACE).unwrap();
    assert_eq!(lib.create_type("t3", TKIND_INTERFACE), Err(TYPE_E_NAMECONFLICT));
    assert!(lib.create_type("", TKIND_INTERFACE).is_ok());
    assert!(lib.create_type("Has Blank", 99).is_ok());
    lib.add_func(t, 0, func(0x6001, "First", vec![])).unwrap();
    assert_eq!(lib.add_func(t, 5, func(0x6002, "Gap", vec![])), Err(TYPE_E_ELEMENTNOTFOUND));
    lib.set_func_names(t, 0, &["First".to_string()]).unwrap();
    lib.add_func(t, 1, func(0x6001, "Second", vec![])).unwrap();
    lib.set_func_names(t, 1, &["Second".to_string()]).unwrap();
    lib.add_func(t, 2, func(0x6003, "x", vec![])).unwrap();
    assert_eq!(lib.set_func_names(t, 2, &["first".to_string()]), Err(TYPE_E_AMBIGUOUSNAME));
    assert_eq!(lib.layout(t, 24), Err(TYPE_E_DUPLICATEID));
    assert_eq!(lib.set_type_flags(t, 0xFFFF), Err(TYPE_E_BADMODULEKIND));
    assert_eq!(lib.set_type_flags(t, 0x8000), Err(TYPE_E_BADMODULEKIND));
    assert_eq!(lib.check_save(), Err(TYPE_E_UNDEFINEDTYPE));
    let d = lib.create_type("D", TKIND_DISPATCH).unwrap();
    assert_eq!(lib.set_type_flags(d, TYPEFLAG_FOLEAUTOMATION), Err(TYPE_E_BADMODULEKIND));
    assert!(lib.set_type_flags(d, TYPEFLAG_FDUAL | TYPEFLAG_FDISPATCHABLE).is_ok());
}

#[test]
fn file_round_trip_and_junk() {
    let lib = sample(TKIND_DISPATCH);
    let bytes = file::encode(&lib);
    assert_eq!(file::read(&bytes), Ok(lib));
    assert_eq!(file::read(b""), Err(file::ReadError::NotALibrary));
    assert_eq!(file::read(b"existing contents"), Err(file::ReadError::NotALibrary));
    assert_eq!(file::read(&bytes[..bytes.len() - 1]), Err(file::ReadError::NotALibrary));
    assert_eq!(file::read(b"MSFT\x02\x00\x01\x00"), Err(file::ReadError::Unsupported));
    let s = stdole::library();
    assert_eq!(file::read(&stdole::file_bytes()), Ok(s.clone()));
    let names: Vec<&str> = s.types.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(&names[..6], &["GUID", "DISPPARAMS", "EXCEPINFO", "IUnknown", "IDispatch", "IEnumVARIANT"]);
    let d = view(&s, stdole::T_IDISPATCH, Form::Stored).unwrap();
    let offs: Vec<i16> = d.funcs.iter().map(|f| f.ovft).collect();
    assert_eq!(offs, [24, 32, 40, 48]);
    assert_eq!(d.cb_size_vft, 56);
}

#[test]
fn registration_writes() {
    let lib = sample(TKIND_DISPATCH);
    let w = reg::register_writes(&lib, r"C:\polyws\x.tlb", None);
    let id = reg::guid_text(&G);
    assert_eq!(w[0], reg::Write { key: alloc::format!("TypeLib\\{id}\\1.0"), name: None, value: "Poly test library".to_string() });
    assert_eq!(w[1].key, alloc::format!("TypeLib\\{id}\\1.0\\0\\win64"));
    assert_eq!(w[2].value, "0");
    assert_eq!(w[3].value, r"C:\polyws");
    // ITlEcho and ITlDual (dispatch-made dual) are registered.
    assert_eq!(w.iter().filter(|x| x.key.starts_with("Interface\\") && x.key.ends_with('}')).count(), 2);
    assert!(w.iter().any(|x| x.name == Some("Version") && x.value == "1.0"));
}
