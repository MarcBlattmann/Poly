//! Real linker output: a DLL, an executable that imports from it and a COFF
//! object, built from the C sources in `fixtures/` by `fixtures/build.sh`
//! (clang + lld-link, no Windows SDK). The expected values were checked
//! against `llvm-readobj` when the fixtures were made.

use poly_pe::*;

static EXE: &[u8] = include_bytes!("fixtures/fixture.exe");
static DLL: &[u8] = include_bytes!("fixtures/fixture.dll");
static OBJ: &[u8] = include_bytes!("fixtures/fixture.obj");

const EXE_BASE: u64 = 0x1_4000_0000;
const DLL_BASE: u64 = 0x1_8000_0000;

fn map(pe: &Pe<'_>) -> Vec<u8> {
    let plan = pe.load_plan().unwrap();
    let mut image = vec![0; plan.size_of_image as usize];
    plan.map_into(&mut image).unwrap();
    image
}

fn ptr_at(image: &[u8], rva: u32) -> u64 {
    u64::from_le_bytes(image[rva as usize..rva as usize + 8].try_into().unwrap())
}

#[test]
fn exe_headers_and_sections() {
    let pe = Pe::parse(EXE).unwrap();
    let info = pe.info();
    assert_eq!(info.machine(), machine::AMD64);
    assert_eq!(info.bits(), Bits::Pe32Plus);
    assert_eq!(info.image_base(), EXE_BASE);
    assert_eq!(info.entry_point(), 0x1020);
    assert_eq!(info.size_of_image(), 0x9000);
    assert_eq!(pe.optional_header().size_of_headers, 0x400);
    assert_eq!(pe.optional_header().subsystem, subsystem::WINDOWS_CUI);
    assert!(info.dynamic_base() && info.high_entropy_va() && info.nx_compat() && !info.is_dll());
    let names: Vec<&[u8]> = (0..pe.sections().len()).map(|i| pe.section_name(i).unwrap()).collect();
    assert_eq!(names, [&b".text"[..], b".rdata", b".data", b".pdata", b".CRT", b".tls", b".rsrc", b".reloc"]);
    let plan = pe.load_plan().unwrap();
    let text = plan.regions().nth(1).unwrap();
    assert_eq!((text.rva, text.protection), (0x1000, Protection { read: true, write: false, execute: true }));
}

#[test]
fn exe_imports_and_delay_imports() {
    let pe = Pe::parse(EXE).unwrap();
    let descs: Vec<ImportDescriptor<'_>> = pe.imports().unwrap().unwrap().map(Result::unwrap).collect();
    assert_eq!(descs.len(), 1);
    assert_eq!(descs[0].dll_name().unwrap(), b"fixture.dll");
    let syms: Vec<ImportSymbol<'_>> = descs[0].entries().unwrap().map(|e| e.unwrap().symbol).collect();
    assert!(matches!(syms[0], ImportSymbol::Name { name: b"fixture_add", .. }));
    assert!(matches!(syms[1], ImportSymbol::Name { name: b"fixture_counter", .. }));
    assert_eq!(syms[2], ImportSymbol::Ordinal(7));
    assert_eq!(syms.len(), 3);

    let delay: Vec<DelayImport<'_>> = pe.delay_imports().unwrap().unwrap().map(Result::unwrap).collect();
    assert_eq!(delay.len(), 1);
    let d = &delay[0];
    assert_eq!(d.dll_name().unwrap(), b"delayed.dll");
    assert_eq!((d.attributes, d.module_handle_rva, d.iat_rva, d.name_table_rva), (1, 0x3008, 0x3010, 0x2170));
    let e: Vec<ImportEntry<'_>> = d.entries().map(Result::unwrap).collect();
    assert_eq!(e.len(), 1);
    assert!(matches!(e[0].symbol, ImportSymbol::Name { name: b"delayed_function", .. }));
    assert!(!pe.is_bound().unwrap());
}

#[test]
fn exe_relocations() {
    let pe = Pe::parse(EXE).unwrap();
    let mut fixups = Vec::new();
    for block in pe.relocations().unwrap().unwrap() {
        for r in block.unwrap().entries() {
            if r.kind != reloc_kind::ABSOLUTE {
                assert_eq!(r.kind, reloc_kind::DIR64);
                fixups.push(r.rva);
            }
        }
    }
    assert_eq!(fixups, [0x2000, 0x2008, 0x2010, 0x2018, 0x2080, 0x3010, 0x5008]);
}

#[test]
fn exe_tls_load_config_exceptions_debug_resources() {
    let pe = Pe::parse(EXE).unwrap();
    let tls = pe.tls().unwrap().unwrap();
    assert_eq!(tls.start_address_of_raw_data, EXE_BASE + 0x6000);
    assert_eq!(tls.end_address_of_raw_data, EXE_BASE + 0x600c);
    assert_eq!(tls.address_of_index, EXE_BASE + 0x3020);
    assert_eq!(tls.address_of_callbacks, EXE_BASE + 0x5008);
    assert_eq!((tls.size_of_zero_fill, tls.alignment()), (0, Some(4)));
    let template = pe.tls_template(&tls).unwrap();
    assert_eq!(template.len(), 12);
    // tls_value = 42 follows _tls_start in the template.
    assert!(template.windows(4).any(|w| w == 42u32.to_le_bytes()));
    let callbacks: Vec<u64> = pe.tls_callbacks(&tls).unwrap().map(Result::unwrap).collect();
    assert_eq!(callbacks.len(), 1);
    let cb = pe.info().va_to_rva(callbacks[0]).unwrap();
    assert!(pe.sections().get(0).unwrap().contains_rva(cb));

    let lc = pe.load_config().unwrap().unwrap();
    assert_eq!(lc.size, 0x98);
    assert_eq!(lc.security_cookie, EXE_BASE + 0x3000);
    assert_eq!(lc.guard_flags, 0);

    let funcs = pe.runtime_functions().unwrap().unwrap();
    assert_eq!(funcs.len(), 1);
    let f = funcs.get(0).unwrap();
    assert_eq!(funcs.lookup(inside_first_function(&pe)), Some(f));
    let u = pe.unwind_info(&f).unwrap();
    assert_eq!((u.version, u.flags, u.size_of_prolog), (1, 0, 10));
    let ops: Vec<UnwindOp> = u.codes().map(|c| c.unwrap().op).collect();
    assert_eq!(ops[0], UnwindOp::Alloc { size: 72 });
    assert_eq!(ops.len(), 5);

    let cv = pe.pdb_info().unwrap().unwrap();
    assert_eq!((cv.age, cv.path), (1, &b"fixture.pdb"[..]));

    let named = pe.find_resource(ResourceKey::Id(10), ResourceKey::Name("fixture"), Some(1033)).unwrap().unwrap();
    assert_eq!(pe.resource_bytes(&named).unwrap(), b"named");
    let by_id = pe.find_resource(ResourceKey::Id(10), ResourceKey::Id(1), None).unwrap().unwrap();
    assert_eq!(pe.resource_bytes(&by_id).unwrap(), b"poly");
}

/// An address inside the one function with unwind data (the entry point
/// calls other functions, so it is the non-leaf one).
fn inside_first_function(pe: &Pe<'_>) -> u32 {
    pe.runtime_functions().unwrap().unwrap().get(0).unwrap().begin_address + 1
}

#[test]
fn dll_exports() {
    let pe = Pe::parse(DLL).unwrap();
    assert!(pe.info().is_dll());
    assert_eq!(pe.info().image_base(), DLL_BASE);
    assert_eq!(pe.info().entry_point(), 0);
    let e = pe.exports().unwrap().unwrap();
    assert_eq!(e.dll_name().unwrap(), b"fixture.dll");
    assert_eq!(e.ordinal_base, 7);
    assert_eq!(e.by_ordinal(7).unwrap(), Some(ExportTarget::Rva(0x1010)));
    assert_eq!(e.by_name(b"fixture_add").unwrap(), Some(ExportTarget::Rva(0x1000)));
    assert_eq!(e.by_name(b"fixture_counter").unwrap(), Some(ExportTarget::Rva(0x3000)));
    assert_eq!(e.ordinal_of(b"fixture_counter_ptr").unwrap(), Some(11));
    assert_eq!(e.by_name(b"fixture_hidden").unwrap(), None);
    match e.by_name(b"FixtureAllocate").unwrap() {
        Some(ExportTarget::Forwarder(f)) => {
            assert_eq!((f.dll, f.target), (&b"NTDLL"[..], ForwardTarget::Name(b"RtlAllocateHeap")));
        }
        other => panic!("{other:?}"),
    }
    let names: Vec<&[u8]> = e.names().map(|n| n.unwrap().0).collect();
    assert_eq!(names, [&b"FixtureAllocate"[..], b"fixture_add", b"fixture_counter", b"fixture_counter_ptr"]);
}

/// The whole loading sequence: map both images at bases of our choosing,
/// relocate them and bind the executable's imports to the DLL's exports.
#[test]
fn load_exe_against_dll() {
    let dll = Pe::parse(DLL).unwrap();
    let dll_base = 0x7ffb_1234_0000;
    let mut dll_image = map(&dll);
    let stats = relocate(&mut dll_image, dll.info(), dll_base).unwrap();
    assert_eq!(stats.applied, 1);
    assert_eq!(ptr_at(&dll_image, 0x3008), dll_base + 0x3000);
    let exports = dll.exports().unwrap().unwrap();

    let exe = Pe::parse(EXE).unwrap();
    let exe_base = 0x7ff7_0000_0000;
    let mut image = map(&exe);
    let info = *exe.info();
    assert_eq!(relocate(&mut image, &info, exe_base).unwrap().applied, 7);
    let bound = bind_imports(&mut image, &info, |req| {
        assert_eq!(req.dll, b"fixture.dll");
        let target = match req.symbol {
            ImportSymbol::Name { hint, name } => exports.by_name_with_hint(name, hint),
            ImportSymbol::Ordinal(n) => exports.by_ordinal(n.into()),
        };
        match target {
            Ok(Some(ExportTarget::Rva(rva))) => Ok(dll_base + u64::from(rva)),
            _ => Err(()),
        }
    })
    .unwrap();
    assert_eq!(bound, 3);
    let iat = info.directory(dir::IAT).unwrap().virtual_address;
    assert_eq!(ptr_at(&image, iat), dll_base + 0x1000);
    assert_eq!(ptr_at(&image, iat + 8), dll_base + 0x3000);
    assert_eq!(ptr_at(&image, iat + 16), dll_base + 0x1010);

    // The delay-loaded DLL does not exist.
    let r = bind_delay_imports(&mut image, &info, |req| Err(req.dll.to_vec()));
    assert_eq!(
        r,
        Err(BindError::Resolve { module_index: 0, index: 0, iat_rva: 0x3010, error: b"delayed.dll".to_vec() })
    );

    // The relocated, bound image reads back consistently.
    let loaded = Pe::parse_mapped(&image).unwrap();
    assert_eq!(loaded.info().image_base(), exe_base);
    let tls = loaded.tls().unwrap().unwrap();
    assert_eq!(tls.start_address_of_raw_data, exe_base + 0x6000);
    assert_eq!(loaded.tls_index_rva(&tls).unwrap(), 0x3020);
    assert_eq!(loaded.load_config().unwrap().unwrap().security_cookie, exe_base + 0x3000);
}

#[test]
fn object_long_section_names() {
    let obj = CoffObject::parse(OBJ).unwrap();
    assert_eq!(obj.header.machine, machine::AMD64);
    let names: Vec<&[u8]> = (0..obj.sections().len()).map(|i| obj.section_name(i).unwrap()).collect();
    assert_eq!(names, [&b".text"[..], b".data", b".bss", b".poly_long_name", b".llvm_addrsig"]);
    assert_eq!(obj.section_data(3).unwrap(), &1u32.to_le_bytes());
    // An object is not an image.
    assert_eq!(Pe::parse(OBJ).unwrap_err(), Error::NotMz);
}

#[test]
fn fixture_prefixes_never_panic() {
    for f in [EXE, DLL] {
        for n in (0..=f.len()).step_by(3) {
            let Ok(pe) = Pe::parse(&f[..n]) else { continue };
            let _ = pe.load_plan();
            if let Ok(Some(imports)) = pe.imports() {
                for d in imports.flatten() {
                    let _ = d.entries().map(|e| e.count());
                }
            }
            if let Ok(Some(e)) = pe.exports() {
                let _ = e.by_name(b"fixture_add");
            }
            let _ = pe.pdb_info();
            let _ = pe.tls();
        }
        let _ = CoffObject::parse(&OBJ[..OBJ.len() / 2]);
    }
}
