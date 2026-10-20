use std::vec;

use super::builder::*;
use super::*;

#[test]
fn lookup_every_name_and_ordinal() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_dll(bits);
        let pe = Pe::parse(&s.file).unwrap();
        assert!(pe.info().is_dll());
        let e = pe.exports().unwrap().unwrap();
        assert_eq!(e.dll_name().unwrap(), b"sample.dll");
        assert_eq!(e.ordinal_base, DLL_ORDINAL_BASE);
        assert_eq!(e.number_of_functions, 204);
        assert_eq!(e.number_of_names, 202);
        for (name, index) in &s.names {
            let ordinal = DLL_ORDINAL_BASE + u32::from(*index);
            assert_eq!(e.ordinal_of(name.as_bytes()).unwrap(), Some(ordinal), "{name}");
            let by_name = e.by_name(name.as_bytes()).unwrap().unwrap();
            assert_eq!(by_name, e.by_ordinal(ordinal).unwrap().unwrap());
            if *index < 200 {
                assert_eq!(by_name, ExportTarget::Rva(s.text + u32::from(*index) * 4));
            }
        }
        // Ordinal only, an empty slot, out of range on both sides.
        assert_eq!(e.by_ordinal(DLL_ORDINAL_BASE + 200).unwrap(), Some(ExportTarget::Rva(s.text + 0x3f0)));
        assert_eq!(e.by_ordinal(DLL_ORDINAL_BASE + 201).unwrap(), None);
        assert_eq!(e.by_ordinal(DLL_ORDINAL_BASE + 204).unwrap(), None);
        assert_eq!(e.by_ordinal(DLL_ORDINAL_BASE - 1).unwrap(), None);
        assert_eq!(e.by_ordinal(0).unwrap(), None);
        assert_eq!(e.by_ordinal(u32::MAX).unwrap(), None);
        // Names that are not there, including neighbours of real ones.
        for missing in [&b""[..], b"A", b"Func", b"Func000_", b"Func000_AA", b"Func199_B", b"zzz", b"heapalloc"] {
            assert_eq!(e.by_name(missing).unwrap(), None, "{missing:?}");
            assert_eq!(e.ordinal_of(missing).unwrap(), None);
        }
    }
}

#[test]
fn name_table_is_sorted_and_complete() {
    let s = sample_dll(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let e = pe.exports().unwrap().unwrap();
    let names: Vec<(&[u8], u32)> = e.names().map(Result::unwrap).collect();
    assert_eq!(names.len(), s.names.len());
    assert!(names.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(names[0], (&b"Func000_W"[..], DLL_ORDINAL_BASE + 199));
    // Every non-empty function slot, in ordinal order.
    let functions: Vec<Export<'_>> = e.functions().map(Result::unwrap).collect();
    assert_eq!(functions.len(), 203);
    assert_eq!(functions[0], Export { ordinal: DLL_ORDINAL_BASE, target: ExportTarget::Rva(s.text) });
    assert!(functions.iter().all(|f| f.ordinal != DLL_ORDINAL_BASE + 201));
}

#[test]
fn forwarders() {
    let s = sample_dll(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let e = pe.exports().unwrap().unwrap();
    assert_eq!(
        e.by_name(b"HeapAlloc").unwrap(),
        Some(ExportTarget::Forwarder(Forwarder {
            raw: b"NTDLL.RtlAllocateHeap",
            dll: b"NTDLL",
            target: ForwardTarget::Name(b"RtlAllocateHeap"),
        }))
    );
    assert_eq!(
        e.by_name(b"Sleepy").unwrap(),
        Some(ExportTarget::Forwarder(Forwarder {
            raw: b"api-ms-win-core-synch-l1-2-0.#12",
            dll: b"api-ms-win-core-synch-l1-2-0",
            target: ForwardTarget::Ordinal(12),
        }))
    );
    // A forwarder is recognised by its RVA inside the export directory; an
    // export just past the directory is an ordinary RVA.
    let dir = pe.info().directory(dir::EXPORT).unwrap();
    assert!(dir.contains(dir.virtual_address) && !dir.contains(dir.virtual_address + dir.size));

    assert_eq!(Forwarder::parse(b"A.B.C").unwrap().dll, b"A.B");
    assert!(Forwarder::parse(b"NoDot").is_err());
    assert!(Forwarder::parse(b".Name").is_err());
    assert!(Forwarder::parse(b"Dll.").is_err());
    assert!(Forwarder::parse(b"Dll.#").is_err());
    assert!(Forwarder::parse(b"Dll.#65536").is_err());
    assert!(Forwarder::parse(b"Dll.#1x").is_err());
    assert_eq!(Forwarder::parse(b"Dll.#65535").unwrap().target, ForwardTarget::Ordinal(65535));
}

#[test]
fn hints() {
    let s = sample_dll(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let e = pe.exports().unwrap().unwrap();
    let target = e.by_name(b"Func123_A").unwrap();
    let right = (0..e.number_of_names as usize).find(|&i| e.name(i).unwrap() == b"Func123_A").unwrap() as u16;
    assert_eq!(e.by_name_with_hint(b"Func123_A", right).unwrap(), target);
    assert_eq!(e.by_name_with_hint(b"Func123_A", 0).unwrap(), target);
    assert_eq!(e.by_name_with_hint(b"Func123_A", u16::MAX).unwrap(), target);
    assert_eq!(e.by_name_with_hint(b"Nope", right).unwrap(), None);
}

#[test]
fn resolve_imports_against_exports() {
    // A loader in miniature: bind the sample executable's ntdll imports to
    // the sample DLL loaded at some base.
    let exe = sample_exe(Bits::Pe32Plus);
    let dll = sample_dll(Bits::Pe32Plus);
    let dll_pe = Pe::parse(&dll.file).unwrap();
    let exports = dll_pe.exports().unwrap().unwrap();
    let dll_base = 0x7ffa_0000_0000u64;
    let info = *Pe::parse(&exe.file).unwrap().info();
    let mut image = map(&exe.file);
    let r = bind_imports(&mut image, &info, |req| {
        let target = match req.symbol {
            ImportSymbol::Name { hint, name } => exports.by_name_with_hint(name, hint),
            ImportSymbol::Ordinal(n) => exports.by_ordinal(n.into()),
        };
        match target {
            Ok(Some(ExportTarget::Rva(rva))) => Ok(dll_base + u64::from(rva)),
            Ok(Some(ExportTarget::Forwarder(f))) => Err(std::format!("forwarded to {:?}", f.dll)),
            _ => Err(std::format!("missing {:?}", req.symbol)),
        }
    });
    // GetProcAddress is not in the sample DLL.
    match r {
        Err(BindError::Resolve { module_index: 0, index: 0, error, .. }) => assert!(error.starts_with("missing")),
        other => panic!("{other:?}"),
    }
    // Ordinal 7 = function index 2.
    assert_eq!(exports.by_ordinal(7).unwrap(), Some(ExportTarget::Rva(dll.text + 8)));
}

#[test]
fn malformed_export_directories() {
    let s = sample_dll(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let edata = pe.info().directory(dir::EXPORT).unwrap().virtual_address;
    let off = pe.rva_to_offset(edata).unwrap();

    // Absurd counts: the tables would run off the section.
    for field in [20, 24] {
        let mut f = s.file.clone();
        put32(&mut f, off + field, u32::MAX);
        assert!(Pe::parse(&f).unwrap().exports().is_err());
    }
    // Tables outside the image.
    for field in [28, 32, 36] {
        let mut f = s.file.clone();
        put32(&mut f, off + field, 0x7fff_0000);
        assert!(Pe::parse(&f).unwrap().exports().is_err());
    }
    // A name pointer leading nowhere: lookups fail without panicking.
    let mut f = s.file.clone();
    let names_rva = u32::from_le_bytes(f[off + 32..off + 36].try_into().unwrap());
    let names_off = pe.rva_to_offset(names_rva).unwrap();
    for i in 0..202 {
        put32(&mut f, names_off + i * 4, 0x7fff_0000);
    }
    let pe2 = Pe::parse(&f).unwrap();
    let e = pe2.exports().unwrap().unwrap();
    assert!(e.by_name(b"Func100_A").is_err());
    assert!(e.names().all(|n| n.is_err()));
    // An ordinal table entry past the address table is an empty result.
    let mut f = s.file.clone();
    let ords_rva = u32::from_le_bytes(f[off + 36..off + 40].try_into().unwrap());
    let ords_off = pe.rva_to_offset(ords_rva).unwrap();
    for i in 0..202 {
        put16(&mut f, ords_off + i * 2, 0xffff);
    }
    let pe3 = Pe::parse(&f).unwrap();
    let e = pe3.exports().unwrap().unwrap();
    assert_eq!(e.by_name(b"Func100_A").unwrap(), None);
    // A directory too short for its header.
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    let r = b.section(".edata", RDATA, vec![0; 16], None);
    b.dir(dir::EXPORT, r, 40);
    let f = b.build();
    assert_eq!(Pe::parse(&f).unwrap().exports().unwrap_err(), Error::Truncated);
}

#[test]
fn unsorted_names_never_panic() {
    let s = sample_dll(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let edata = pe.info().directory(dir::EXPORT).unwrap().virtual_address;
    let off = pe.rva_to_offset(edata).unwrap();
    let names_off = pe.rva_to_offset(u32::from_le_bytes(s.file[off + 32..off + 36].try_into().unwrap())).unwrap();
    let mut f = s.file.clone();
    // Reverse the name pointer table.
    let mut ptrs: Vec<[u8; 4]> = (0..202).map(|i| f[names_off + i * 4..][..4].try_into().unwrap()).collect();
    ptrs.reverse();
    for (i, p) in ptrs.iter().enumerate() {
        f[names_off + i * 4..][..4].copy_from_slice(p);
    }
    let pe = Pe::parse(&f).unwrap();
    let e = pe.exports().unwrap().unwrap();
    for (name, _) in &s.names {
        let _ = e.by_name(name.as_bytes()).unwrap();
    }
}
