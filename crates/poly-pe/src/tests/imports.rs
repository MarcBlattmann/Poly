use std::string::String;
use std::vec;

use super::builder::*;
use super::*;

type Seen = (usize, Vec<u8>, usize, String, u32);

fn describe(s: &ImportSymbol<'_>) -> String {
    match s {
        ImportSymbol::Name { hint, name } => std::format!("{}@{hint}", std::str::from_utf8(name).unwrap()),
        ImportSymbol::Ordinal(n) => std::format!("#{n}"),
    }
}

fn u64_le(b: &[u8], at: u32, bits: Bits) -> u64 {
    let at = at as usize;
    match bits {
        Bits::Pe32Plus => u64::from_le_bytes(b[at..at + 8].try_into().unwrap()),
        Bits::Pe32 => u64::from(u32::from_le_bytes(b[at..at + 4].try_into().unwrap())),
    }
}

#[test]
fn import_directory_walk() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let p = bits.pointer_size() as u32;
        let pe = Pe::parse(&s.file).unwrap();
        let descs: Vec<ImportDescriptor<'_>> = pe.imports().unwrap().unwrap().map(Result::unwrap).collect();
        assert_eq!(descs.len(), 2);
        assert_eq!(descs[0].dll_name().unwrap(), b"KERNEL32.dll");
        assert_eq!(descs[1].dll_name().unwrap(), b"ntdll.dll");
        assert!(!descs[0].is_bound());
        assert_eq!(descs[0].first_thunk, s.iats[0]);
        let entries: Vec<ImportEntry<'_>> = descs[0].entries().unwrap().map(Result::unwrap).collect();
        assert_eq!(
            entries,
            [
                ImportEntry {
                    index: 0,
                    iat_rva: s.iats[0],
                    symbol: ImportSymbol::Name { hint: 0x10, name: b"GetProcAddress" }
                },
                ImportEntry {
                    index: 1,
                    iat_rva: s.iats[0] + p,
                    symbol: ImportSymbol::Name { hint: 0x20, name: b"LoadLibraryA" }
                },
                ImportEntry { index: 2, iat_rva: s.iats[0] + 2 * p, symbol: ImportSymbol::Ordinal(42) },
            ]
        );
        let ntdll: Vec<String> = descs[1].entries().unwrap().map(|e| describe(&e.unwrap().symbol)).collect();
        assert_eq!(ntdll, ["RtlAllocateHeap@0", "#7"]);
        assert!(!pe.is_bound().unwrap());
        assert!(pe.bound_imports().unwrap().is_none());
    }
}

#[test]
fn bind_imports_fills_the_iat() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let p = bits.pointer_size() as u32;
        let info = *Pe::parse(&s.file).unwrap().info();
        let mut image = map(&s.file);
        let before = image.clone();
        let mut seen: Vec<Seen> = Vec::new();
        let n = bind_imports(&mut image, &info, |r| {
            seen.push((r.module_index, r.dll.to_vec(), r.index, describe(&r.symbol), r.iat_rva));
            Ok::<u64, ()>(0x7000_0000 + (r.module_index as u64) * 0x1000 + r.index as u64 * 0x10)
        })
        .unwrap();
        assert_eq!(n, 5);
        let k32 = b"KERNEL32.dll".to_vec();
        let nt = b"ntdll.dll".to_vec();
        assert_eq!(
            seen,
            [
                (0, k32.clone(), 0, "GetProcAddress@16".into(), s.iats[0]),
                (0, k32.clone(), 1, "LoadLibraryA@32".into(), s.iats[0] + p),
                (0, k32, 2, "#42".into(), s.iats[0] + 2 * p),
                (1, nt.clone(), 0, "RtlAllocateHeap@0".into(), s.iats[1]),
                (1, nt, 1, "#7".into(), s.iats[1] + p),
            ]
        );
        assert_eq!(u64_le(&image, s.iats[0], bits), 0x7000_0000);
        assert_eq!(u64_le(&image, s.iats[0] + 2 * p, bits), 0x7000_0020);
        assert_eq!(u64_le(&image, s.iats[1] + p, bits), 0x7000_1010);
        // The terminating slots and everything outside the IATs are untouched.
        assert_eq!(u64_le(&image, s.iats[0] + 3 * p, bits), 0);
        let changed: Vec<usize> = (0..image.len()).filter(|&i| image[i] != before[i]).collect();
        assert!(changed.iter().all(|&i| {
            let rva = i as u32;
            (s.iats[0]..s.iats[0] + 3 * p).contains(&rva) || (s.iats[1]..s.iats[1] + 2 * p).contains(&rva)
        }));
        // The lookup tables still hold names, so binding again works.
        assert_eq!(bind_imports(&mut image, &info, |_| Ok::<u64, ()>(1)).unwrap(), 5);
    }
}

#[test]
fn resolver_errors_stop_binding() {
    let s = sample_exe(Bits::Pe32Plus);
    let info = *Pe::parse(&s.file).unwrap().info();
    let mut image = map(&s.file);
    let r = bind_imports(&mut image, &info, |r| match r.symbol {
        ImportSymbol::Name { name: b"LoadLibraryA", .. } => Err("not found"),
        _ => Ok(0x1234),
    });
    assert_eq!(r, Err(BindError::Resolve { module_index: 0, index: 1, iat_rva: s.iats[0] + 8, error: "not found" }));
    assert_eq!(u64_le(&image, s.iats[0], Bits::Pe32Plus), 0x1234);
    // Slot 1 still holds its lookup value (the hint/name RVA).
    assert_ne!(u64_le(&image, s.iats[0] + 8, Bits::Pe32Plus), 0x1234);

    // A PE32 image cannot take a 64-bit address.
    let s = sample_exe(Bits::Pe32);
    let info = *Pe::parse(&s.file).unwrap().info();
    let mut image = map(&s.file);
    let r = bind_imports(&mut image, &info, |_| Ok::<u64, ()>(0x1_0000_0000));
    assert_eq!(r, Err(BindError::Image(Error::BadVa(0x1_0000_0000))));
}

#[test]
fn names_from_the_iat_without_a_lookup_table() {
    let s = sample_exe(Bits::Pe32Plus);
    let info = *Pe::parse(&s.file).unwrap().info();
    let mut image = map(&s.file);
    // Clear OriginalFirstThunk of both descriptors.
    for k in 0..2 {
        let at = s.idata as usize + k * 20;
        image[at..at + 4].fill(0);
    }
    {
        let pe = Pe::parse_mapped(&image).unwrap();
        let d = pe.imports().unwrap().unwrap().next().unwrap().unwrap();
        assert_eq!(d.original_first_thunk, 0);
        assert_eq!(d.entries().unwrap().count(), 3);
    }
    let n = bind_imports(&mut image, &info, |r| Ok::<u64, ()>(0x5000 + r.index as u64)).unwrap();
    assert_eq!(n, 5);
    assert_eq!(u64_le(&image, s.iats[0] + 16, Bits::Pe32Plus), 0x5002);

    // A bound descriptor without a lookup table cannot be rebound.
    let mut image = map(&s.file);
    let at = s.idata as usize;
    image[at..at + 4].fill(0);
    image[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
    let pe = Pe::parse_mapped(&image).unwrap();
    let d = pe.imports().unwrap().unwrap().next().unwrap().unwrap();
    assert!(d.is_bound());
    assert!(matches!(d.entries(), Err(Error::Malformed(_))));
    assert!(pe.is_bound().unwrap());
    assert!(matches!(
        bind_imports(&mut image, &info, |_| Ok::<u64, ()>(1)),
        Err(BindError::Image(Error::Malformed(_)))
    ));
}

#[test]
fn bound_import_directory() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    let idata = b.next_rva();
    let (bytes, size, _) = imports(idata, Bits::Pe32Plus, &[("KERNEL32.dll", &[Sym::Name(1, "Sleep")])]);
    b.section(".idata", RDATA, bytes, None);
    b.dir(dir::IMPORT, idata, size);
    b.dir(dir::BOUND_IMPORT, 0x300, 0x40);
    let mut f = b.build();
    // New-style binding: time stamp -1 in the descriptor.
    let off = Pe::parse(&f).unwrap().rva_to_offset(idata).unwrap();
    put32(&mut f, off + 4, u32::MAX);
    // KERNEL32.dll, bound at 0x11111111, forwarding to ntdll.dll.
    put32(&mut f, 0x300, 0x1111_1111);
    put16(&mut f, 0x304, 0x18);
    put16(&mut f, 0x306, 1);
    put32(&mut f, 0x308, 0x2222_2222);
    put16(&mut f, 0x30c, 0x25);
    put_bytes(&mut f, 0x318, b"KERNEL32.dll\0ntdll.dll\0");

    let pe = Pe::parse(&f).unwrap();
    assert!(pe.is_bound().unwrap());
    let d = pe.imports().unwrap().unwrap().next().unwrap().unwrap();
    assert!(d.is_bound());
    assert_eq!(d.time_date_stamp, u32::MAX);
    let bound: Vec<BoundImport<'_>> = pe.bound_imports().unwrap().unwrap().map(Result::unwrap).collect();
    assert_eq!(bound.len(), 1);
    assert_eq!((bound[0].time_date_stamp, bound[0].module_name), (0x1111_1111, &b"KERNEL32.dll"[..]));
    let fwd: Vec<BoundForwarder<'_>> = bound[0].forwarders().map(Result::unwrap).collect();
    assert_eq!(fwd, [BoundForwarder { time_date_stamp: 0x2222_2222, module_name: b"ntdll.dll" }]);

    // The lookup table is still there, so the loader rebinds regardless.
    let info = *pe.info();
    let mut image = map(&f);
    assert_eq!(bind_imports(&mut image, &info, |_| Ok::<u64, ()>(0x42)).unwrap(), 1);

    // A forwarder count running off the table, and a name offset past it.
    put16(&mut f, 0x306, 50);
    let pe = Pe::parse(&f).unwrap();
    let bound = pe.bound_imports().unwrap().unwrap().next().unwrap().unwrap();
    assert!(bound.forwarders().any(|r| r.is_err()));
    put16(&mut f, 0x304, 0x400);
    let pe = Pe::parse(&f).unwrap();
    assert!(pe.bound_imports().unwrap().unwrap().next().unwrap().is_err());
}

#[test]
fn delay_imports() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let p = bits.pointer_size() as u32;
        let pe = Pe::parse(&s.file).unwrap();
        let all: Vec<DelayImport<'_>> = pe.delay_imports().unwrap().unwrap().map(Result::unwrap).collect();
        assert_eq!(all.len(), 1);
        let d = &all[0];
        assert_eq!(d.dll_name().unwrap(), b"delayed.dll");
        assert_eq!(d.attributes, 1);
        assert_eq!(d.module_handle_rva, s.data + DATA_MODULE);
        assert_eq!((d.iat_rva, d.name_table_rva), (s.didat + 0x60, s.didat + 0x40));
        let entries: Vec<ImportEntry<'_>> = d.entries().map(Result::unwrap).collect();
        assert_eq!(
            entries,
            [
                ImportEntry {
                    index: 0,
                    iat_rva: s.didat + 0x60,
                    symbol: ImportSymbol::Name { hint: 5, name: b"DelayedFunction" }
                },
                ImportEntry { index: 1, iat_rva: s.didat + 0x60 + p, symbol: ImportSymbol::Ordinal(3) },
            ]
        );

        let info = *pe.info();
        let mut image = map(&s.file);
        let before = image.clone();
        let n = bind_delay_imports(&mut image, &info, |r| {
            assert_eq!(r.dll, b"delayed.dll");
            Ok::<u64, ()>(0x6000_0000 + r.index as u64)
        })
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(u64_le(&image, s.didat + 0x60, bits), 0x6000_0000);
        assert_eq!(u64_le(&image, s.didat + 0x60 + p, bits), 0x6000_0001);
        // The regular IAT and the module handle slot are left alone.
        assert_eq!(&image[s.iats[0] as usize..][..16], &before[s.iats[0] as usize..][..16]);
        assert_eq!(u64_le(&image, s.data + DATA_MODULE, bits), 0);
    }
}

#[test]
fn reserved_bits_in_lookup_entries() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let ilt = pe.imports().unwrap().unwrap().next().unwrap().unwrap().original_first_thunk;
    for bad in [(1u64 << 63) | (1 << 20) | 5, 1 << 40, 0x8000_0000] {
        let mut image = map(&s.file);
        image[ilt as usize..ilt as usize + 8].copy_from_slice(&bad.to_le_bytes());
        let pe = Pe::parse_mapped(&image).unwrap();
        let d = pe.imports().unwrap().unwrap().next().unwrap().unwrap();
        let first = d.entries().unwrap().next().unwrap();
        assert!(matches!(first, Err(Error::Malformed(_))), "{bad:#x}");
    }
}

#[test]
fn self_feeding_iat_terminates() {
    // The IAT sits one slot after the lookup table, so every address the
    // resolver writes becomes the next lookup entry. Binding must still end.
    let s = sample_exe(Bits::Pe32Plus);
    let info = *Pe::parse(&s.file).unwrap().info();
    let mut image = map(&s.file);
    let desc = s.idata as usize;
    let ilt = u32::from_le_bytes(image[desc..desc + 4].try_into().unwrap());
    image[desc + 16..desc + 20].copy_from_slice(&(ilt + 8).to_le_bytes());
    let mut calls = 0usize;
    let r = bind_imports(&mut image, &info, |_| {
        calls += 1;
        Ok::<u64, ()>((1 << 63) | 9)
    });
    assert!(r.is_err());
    assert!(calls <= image.len() / 8);
}

#[test]
fn malformed_import_directories() {
    let s = sample_exe(Bits::Pe32Plus);
    // A descriptor without a name.
    let mut image = map(&s.file);
    let at = s.idata as usize + 12;
    image[at..at + 4].fill(0);
    let pe = Pe::parse_mapped(&image).unwrap();
    let mut it = pe.imports().unwrap().unwrap();
    assert!(matches!(it.next(), Some(Err(Error::Malformed(_)))));
    assert!(it.next().is_none());

    // A directory whose table runs off the end of its section (in a file)
    // or of the image (mapped).
    let mut f = s.file.clone();
    let (_, idata) = Pe::parse(&f).unwrap().section_by_name(b".idata").unwrap();
    let end = idata.virtual_address + idata.virtual_size;
    put32(&mut f, NT_OFFSET + 24 + 112 + 8, end - 8);
    let pe = Pe::parse(&f).unwrap();
    assert!(pe.imports().unwrap().unwrap().next().unwrap().is_err());
    put32(&mut f, NT_OFFSET + 24 + 112 + 8, 0x7fff_0000);
    let pe = Pe::parse(&f).unwrap();
    assert_eq!(pe.imports().unwrap().unwrap().next().unwrap().unwrap_err(), Error::BadRva(0x7fff_0000));
    let info = *pe.info();
    let mut image = map(&s.file);
    assert_eq!(
        bind_imports(&mut image, &info, |_| Ok::<u64, ()>(0)),
        Err(BindError::Image(Error::BadRva(0x7fff_0000)))
    );

    // A DLL name without a terminator.
    let mut image = map(&s.file);
    let name_rva = u32::from_le_bytes(image[s.idata as usize + 12..][..4].try_into().unwrap()) as usize;
    let len = image.len();
    image[name_rva..len].fill(b'A');
    let pe = Pe::parse_mapped(&image).unwrap();
    let d = pe.imports().unwrap().unwrap().next().unwrap().unwrap();
    assert_eq!(d.dll_name().unwrap_err(), Error::BadString);
}

#[test]
fn no_import_directory() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.section(".text", CODE, vec![0xc3], None);
    let f = b.build();
    let pe = Pe::parse(&f).unwrap();
    assert!(pe.imports().unwrap().is_none());
    assert!(pe.delay_imports().unwrap().is_none());
    let info = *pe.info();
    let mut image = map(&f);
    assert_eq!(bind_imports(&mut image, &info, |_| Err::<u64, ()>(())), Ok(0));
    assert_eq!(bind_delay_imports(&mut image, &info, |_| Err::<u64, ()>(())), Ok(0));
}
