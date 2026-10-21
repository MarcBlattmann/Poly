use std::vec;

use super::builder::*;
use super::*;

/// File offset of section `i`'s header in a built image.
fn section_header(bits: Bits, i: usize) -> usize {
    let opt = if bits == Bits::Pe32Plus { 240 } else { 224 };
    NT_OFFSET + 24 + opt + 40 * i
}

const OPT: usize = NT_OFFSET + 24;

#[test]
fn plan_regions() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let plan = pe.load_plan().unwrap();
    assert_eq!(plan.image_base, s.base);
    assert_eq!(plan.size_of_image, pe.optional_header().size_of_image);
    assert_eq!(plan.size_of_image % SECTION_ALIGN, 0);
    assert_eq!((plan.section_alignment, plan.file_alignment), (SECTION_ALIGN, FILE_ALIGN));
    assert_eq!(plan.entry_point, s.text);
    assert!(plan.relocatable && plan.has_relocations);
    assert!(!plan.needs_relocation(s.base) && plan.needs_relocation(s.base + 0x10000));
    assert!(plan.can_load_at(0x7ff6_1234_0000));

    let regions: Vec<Region> = plan.regions().collect();
    assert_eq!(regions.len(), 1 + pe.sections().len());
    assert_eq!(
        regions[0],
        Region {
            rva: 0,
            size: SECTION_ALIGN,
            data_offset: 0,
            data_size: HEADERS,
            protection: Protection::READ_ONLY,
            characteristics: 0,
            section: None,
        }
    );
    assert_eq!(
        regions[1],
        Region {
            rva: s.text,
            size: SECTION_ALIGN,
            data_offset: HEADERS as usize,
            data_size: 0x80,
            protection: Protection { read: true, write: false, execute: true },
            characteristics: CODE,
            section: Some(0),
        }
    );
    let bss = regions.iter().find(|r| r.rva == s.bss).unwrap();
    assert_eq!((bss.size, bss.data_size), (0x2000, 0));
    assert!(bss.protection.write);
    // Regions do not overlap and stay inside the image.
    for w in regions.windows(2) {
        assert!(w[0].rva + w[0].size <= w[1].rva);
    }
    let last = regions.last().unwrap();
    assert!(last.rva + last.size <= plan.size_of_image);
}

#[test]
fn map_copies_and_zero_fills() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    let text = b.section(".text", CODE, vec![0x11; 0x30], None);
    // Raw data longer than VirtualSize: only VirtualSize bytes are loaded.
    let data = b.section(".data", DATA, vec![0x55; 0x200], Some(0x10));
    // VirtualSize longer than the raw data: the rest is zero.
    let big = b.section(".big", DATA, vec![0x77; 0x20], Some(0x3000));
    let f = b.build();
    let plan = Pe::parse(&f).unwrap().load_plan().unwrap();
    let mut image = vec![0xaa; plan.size_of_image as usize + 7];
    plan.map_into(&mut image).unwrap();
    assert_eq!(&image[..HEADERS as usize], &f[..HEADERS as usize]);
    assert!(image[HEADERS as usize..text as usize].iter().all(|&c| c == 0));
    assert!(image[text as usize..][..0x30].iter().all(|&c| c == 0x11));
    assert!(image[text as usize + 0x30..data as usize].iter().all(|&c| c == 0));
    assert!(image[data as usize..][..0x10].iter().all(|&c| c == 0x55));
    assert!(image[data as usize + 0x10..big as usize].iter().all(|&c| c == 0));
    assert!(image[big as usize..][..0x20].iter().all(|&c| c == 0x77));
    assert!(image[big as usize + 0x20..plan.size_of_image as usize].iter().all(|&c| c == 0));
    // Bytes past the image are not touched.
    assert!(image[plan.size_of_image as usize..].iter().all(|&c| c == 0xaa));

    let mut small = vec![0; plan.size_of_image as usize - 1];
    assert_eq!(plan.map_into(&mut small).unwrap_err(), Error::BufferTooSmall);
}

#[test]
fn plan_rejects_inconsistent_layouts() {
    let f = sample_exe(Bits::Pe32Plus).file;
    let s0 = section_header(Bits::Pe32Plus, 0);
    let s1 = section_header(Bits::Pe32Plus, 1);
    let plan = |f: &[u8]| Pe::parse(f).unwrap().load_plan().map(|_| ());
    let layout = |r: Result<(), Error>| matches!(r, Err(Error::BadLayout(_)));

    let mut x = f.clone();
    put32(&mut x, OPT + 32, 0x1800);
    assert!(layout(plan(&x)), "section alignment not a power of two");
    let mut x = f.clone();
    put32(&mut x, OPT + 36, 0);
    assert!(layout(plan(&x)), "file alignment zero");
    let mut x = f.clone();
    put32(&mut x, OPT + 36, 0x2000);
    assert!(layout(plan(&x)), "file alignment above section alignment");
    let mut x = f.clone();
    put32(&mut x, OPT + 32, 0x800);
    assert!(layout(plan(&x)), "small section alignment needs equal file alignment");
    let mut x = f.clone();
    put32(&mut x, s1 + 12, 0x2010);
    assert!(layout(plan(&x)), "misaligned section");
    let mut x = f.clone();
    put32(&mut x, s1 + 12, 0x1000);
    assert!(layout(plan(&x)), "overlapping sections");
    let mut x = f.clone();
    put32(&mut x, s0 + 12, 0x3000);
    assert!(layout(plan(&x)), "sections out of order");
    let mut x = f.clone();
    put32(&mut x, OPT + 56, 0x2000);
    assert!(layout(plan(&x)), "section past SizeOfImage");
    let mut x = f.clone();
    put32(&mut x, OPT + 56, 0);
    assert!(layout(plan(&x)), "SizeOfImage zero");
    let mut x = f.clone();
    put32(&mut x, OPT + 56, u32::MAX);
    assert!(layout(plan(&x)), "SizeOfImage overflows when aligned");
    let mut x = f.clone();
    put32(&mut x, s0 + 8, u32::MAX);
    assert!(layout(plan(&x)), "section size overflows");
    let mut x = f.clone();
    put32(&mut x, OPT + 16, 0x100_0000);
    assert!(layout(plan(&x)), "entry point outside");
    let mut x = f.clone();
    put32(&mut x, OPT + 60, 0x100);
    assert!(layout(plan(&x)), "headers smaller than the section table");
    let mut x = f.clone();
    put32(&mut x, OPT + 60, 0x1000);
    put32(&mut x, s0 + 12, 0x1000);
    // SizeOfHeaders 0x1000 still fits before the first section at 0x1000.
    assert!(plan(&x).is_ok());
    let mut x = f.clone();
    put32(&mut x, OPT + 60, 0x1001);
    assert!(layout(plan(&x)), "headers overlap the first section");
    let mut x = f.clone();
    let len = x.len() as u32;
    put32(&mut x, OPT + 60, len + 0x200);
    assert!(plan(&x).is_err(), "headers past the end of the file");
    let mut x = f.clone();
    let len = x.len() as u32;
    put32(&mut x, s0 + 20, len);
    assert_eq!(plan(&x).unwrap_err(), Error::Truncated, "raw data past the end of the file");
    let mut x = f.clone();
    put32(&mut x, s0 + 20, u32::MAX);
    assert_eq!(plan(&x).unwrap_err(), Error::Truncated, "raw data pointer overflows");
}

#[test]
fn low_alignment_image() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.section(".text", CODE, vec![0xc3; 0x10], None);
    let mut f = b.build();
    put32(&mut f, OPT + 32, 0x200);
    put32(&mut f, OPT + 56, 0x1200);
    let plan = Pe::parse(&f).unwrap().load_plan().unwrap();
    assert_eq!(plan.section_alignment, 0x200);
    assert_eq!(plan.size_of_image, 0x1200);
    let r: Vec<Region> = plan.regions().collect();
    assert_eq!((r[0].size, r[1].size), (0x400, 0x200));
}

#[test]
fn relocs_stripped_image_loads_only_at_its_base() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.characteristics |= file_flags::RELOCS_STRIPPED;
    b.section(".text", CODE, vec![0xc3], None);
    let f = b.build();
    let pe = Pe::parse(&f).unwrap();
    let plan = pe.load_plan().unwrap();
    assert!(!plan.relocatable && !plan.has_relocations);
    assert!(plan.can_load_at(plan.image_base) && !plan.can_load_at(plan.image_base + 0x10000));
    let mut image = map(&f);
    let info = *pe.info();
    assert_eq!(relocate(&mut image, &info, info.image_base() + 0x10000).unwrap_err(), Error::RelocsStripped);
    assert_eq!(relocate(&mut image, &info, info.image_base()).unwrap(), RelocStats::default());
}

#[test]
fn mapped_image_parses_like_the_file() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let file = Pe::parse(&s.file).unwrap();
        let image = map(&s.file);
        let mapped = Pe::parse_mapped(&image).unwrap();
        assert_eq!(file.info(), mapped.info());
        let names = |pe: &Pe<'_>| -> Vec<Vec<u8>> {
            let mut v = Vec::new();
            for d in pe.imports().unwrap().unwrap() {
                let d = d.unwrap();
                v.push(d.dll_name().unwrap().to_vec());
                for e in d.entries().unwrap() {
                    if let ImportSymbol::Name { name, .. } = e.unwrap().symbol {
                        v.push(name.to_vec());
                    }
                }
            }
            v
        };
        assert_eq!(names(&file), names(&mapped));
        assert_eq!(file.tls().unwrap(), mapped.tls().unwrap());
        let t = file.tls().unwrap().unwrap();
        assert_eq!(file.tls_template(&t).unwrap(), mapped.tls_template(&t).unwrap());
        assert_eq!(file.load_config().unwrap(), mapped.load_config().unwrap());
        assert_eq!(file.pdb_info().unwrap(), mapped.pdb_info().unwrap());
        let res = |pe: &Pe<'_>| {
            let d = pe.find_resource(ResourceKey::Id(10), ResourceKey::Id(1), None).unwrap().unwrap();
            pe.resource_bytes(&d).unwrap().to_vec()
        };
        assert_eq!(res(&file), res(&mapped));
        // The plan of a mapped image copies from RVAs.
        let plan = mapped.load_plan().unwrap();
        let mut again = vec![0; plan.size_of_image as usize];
        plan.map_into(&mut again).unwrap();
        assert_eq!(again, image);
    }
}

fn read_ptr(image: &[u8], rva: u32, bits: Bits) -> u64 {
    let at = rva as usize;
    match bits {
        Bits::Pe32Plus => u64::from_le_bytes(image[at..at + 8].try_into().unwrap()),
        Bits::Pe32 => u64::from(u32::from_le_bytes(image[at..at + 4].try_into().unwrap())),
    }
}

#[test]
fn relocation_blocks_list_every_fixup() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let pe = Pe::parse(&s.file).unwrap();
        let mut found = Vec::new();
        let mut padding = 0;
        for block in pe.relocations().unwrap().unwrap() {
            let block = block.unwrap();
            assert_eq!(block.page_rva & 0xfff, 0);
            assert!(!block.is_empty());
            for r in block.entries() {
                assert_eq!(r.rva & !0xfff, block.page_rva);
                if r.kind == reloc_kind::ABSOLUTE {
                    padding += 1;
                } else {
                    found.push((r.rva, r.kind));
                }
            }
        }
        let mut expected = s.relocs.clone();
        expected.sort();
        found.sort();
        assert_eq!(found, expected);
        assert!(padding > 0);
    }
}

#[test]
fn relocate_matches_a_reference() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let pe = Pe::parse(&s.file).unwrap();
        let info = *pe.info();
        let original = map(&s.file);
        let bases: &[u64] = match bits {
            Bits::Pe32Plus => &[0x7ff6_1230_0000, 0x1_0000_0000, 0x10000, 0xffff_ffff_ffff_0000],
            Bits::Pe32 => &[0x1000_0000, 0x10000, 0xfff0_0000],
        };
        for &new_base in bases {
            let delta = new_base.wrapping_sub(s.base);
            // The reference: every listed pointer moved by delta, and the
            // header's ImageBase replaced; nothing else changes.
            let mut reference = original.clone();
            for &(rva, _) in &s.relocs {
                let v = read_ptr(&original, rva, bits).wrapping_add(delta);
                let at = rva as usize;
                match bits {
                    Bits::Pe32Plus => reference[at..at + 8].copy_from_slice(&v.to_le_bytes()),
                    Bits::Pe32 => reference[at..at + 4].copy_from_slice(&(v as u32).to_le_bytes()),
                }
            }
            let base_at = OPT + if bits == Bits::Pe32Plus { 24 } else { 28 };
            match bits {
                Bits::Pe32Plus => reference[base_at..base_at + 8].copy_from_slice(&new_base.to_le_bytes()),
                Bits::Pe32 => reference[base_at..base_at + 4].copy_from_slice(&(new_base as u32).to_le_bytes()),
            }

            let mut image = original.clone();
            let stats = relocate(&mut image, &info, new_base).unwrap();
            assert_eq!(stats.applied, s.relocs.len());
            assert!(stats.skipped > 0);
            assert!(image == reference, "{bits:?} at {new_base:#x}");

            // The relocated image reads back with its new base, and its VAs
            // still lead to the same RVAs.
            let moved_info = {
                let moved = Pe::parse_mapped(&image).unwrap();
                assert_eq!(moved.info().image_base(), new_base);
                assert_eq!(
                    read_ptr(&image, s.data + DATA_PTR_TEXT, bits),
                    new_base.wrapping_add(s.text.into()) & mask(bits)
                );
                assert_eq!(read_ptr(&image, s.data + DATA_PLAIN, Bits::Pe32Plus), 0x1122_3344_5566_7788);
                let tls = moved.tls().unwrap().unwrap();
                assert_eq!(moved.tls_index_rva(&tls).unwrap(), s.data + DATA_TLS_INDEX);
                let callbacks: Vec<u32> =
                    moved.tls_callbacks(&tls).unwrap().map(|va| moved.info().va_to_rva(va.unwrap()).unwrap()).collect();
                assert_eq!(callbacks, [s.text + 0x10, s.text + 0x20]);
                *moved.info()
            };

            // Moving back restores the original exactly.
            relocate(&mut image, &moved_info, s.base).unwrap();
            assert!(image == original);
        }
    }
}

fn mask(bits: Bits) -> u64 {
    if bits == Bits::Pe32Plus { u64::MAX } else { u64::from(u32::MAX) }
}

/// A one-section image with a relocation table of the given entries.
fn image_with_relocs(bits: Bits, entries: &[(u32, u8)], machine: u16) -> (Vec<u8>, ImageInfo) {
    let mut b = PeBuilder::new(bits);
    b.machine = machine;
    let mut d = Vec::new();
    for i in 0..16 {
        put32(&mut d, i * 4, 0x0001_0000 * (i as u32 + 1) + 0x1234);
    }
    b.section(".data", DATA, d, None);
    let table = relocs(entries);
    let len = table.len() as u32;
    let r = b.section(".reloc", RDATA, table, None);
    b.dir(dir::BASERELOC, r, len);
    let f = b.build();
    let info = *Pe::parse(&f).unwrap().info();
    (map(&f), info)
}

#[test]
fn relocate_x86_high_low_highlow() {
    use reloc_kind::*;
    let (mut image, info) =
        image_with_relocs(Bits::Pe32, &[(0x1000, HIGHLOW), (0x1004, HIGH), (0x1008, LOW)], machine::I386);
    let before = image.clone();
    let delta = 0x0123_4567u32;
    let stats = relocate(&mut image, &info, info.image_base() + u64::from(delta)).unwrap();
    assert_eq!(stats.applied, 3);
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes(b[at..at + 2].try_into().unwrap());
    assert_eq!(u32_at(&image, 0x1000), u32_at(&before, 0x1000).wrapping_add(delta));
    assert_eq!(u16_at(&image, 0x1004), u16_at(&before, 0x1004).wrapping_add((delta >> 16) as u16));
    assert_eq!(u16_at(&image, 0x1006), u16_at(&before, 0x1006));
    assert_eq!(u16_at(&image, 0x1008), u16_at(&before, 0x1008).wrapping_add(delta as u16));
    assert_eq!(&image[0x100c..0x2000], &before[0x100c..0x2000]);

    // Bases past 4 GiB cannot hold a PE32 image.
    assert_eq!(relocate(&mut image, &info, 0x1_0000_0000).unwrap_err(), Error::BadVa(0x1_0000_0000));
    // DIR64 is not an x86 relocation.
    let (mut image, info) = image_with_relocs(Bits::Pe32, &[(0x1000, DIR64)], machine::I386);
    assert_eq!(
        relocate(&mut image, &info, 0x1000_0000).unwrap_err(),
        Error::UnsupportedRelocation { kind: DIR64, rva: 0x1000 }
    );
}

#[test]
fn relocate_x86_highadj_uses_its_second_slot() {
    use reloc_kind::*;
    // One block: HIGHADJ at 0x1000 with low half 0xF000, then a HIGHLOW.
    let mut table = Vec::new();
    put32(&mut table, 0, 0x1000);
    put32(&mut table, 4, 16);
    put16(&mut table, 8, u16::from(HIGHADJ) << 12);
    put16(&mut table, 10, 0xF000);
    put16(&mut table, 12, (u16::from(HIGHLOW) << 12) | 0x008);
    put16(&mut table, 14, 0);
    let mut b = PeBuilder::new(Bits::Pe32);
    b.machine = machine::I386;
    let mut d = Vec::new();
    put16(&mut d, 0, 0x1234);
    put32(&mut d, 8, 0x5000_0000);
    b.section(".data", DATA, d, None);
    let len = table.len() as u32;
    let r = b.section(".reloc", RDATA, table.clone(), None);
    b.dir(dir::BASERELOC, r, len);
    let f = b.build();
    let info = *Pe::parse(&f).unwrap().info();
    let mut image = map(&f);
    let delta = 0x0001_2000u32; // low half 0xF000 + 0x2000 carries into the high half
    let stats = relocate(&mut image, &info, info.image_base() + u64::from(delta)).unwrap();
    assert_eq!(stats.applied, 2);
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes(b[at..at + 2].try_into().unwrap());
    assert_eq!(u16_at(&image, 0x1000), 0x1234 + 0x0001 + 1);
    assert_eq!(u32::from_le_bytes(image[0x1008..0x100c].try_into().unwrap()), 0x5000_0000 + delta);

    // A HIGHADJ in the block's last slot has no low half: nothing is written.
    let mut b = PeBuilder::new(Bits::Pe32);
    b.machine = machine::I386;
    b.section(".data", DATA, vec![0; 16], None);
    let table = relocs(&[(0x1000, HIGHLOW), (0x1004, HIGHADJ)]);
    let len = table.len() as u32;
    let r = b.section(".reloc", RDATA, table, None);
    b.dir(dir::BASERELOC, r, len);
    let f = b.build();
    let info = *Pe::parse(&f).unwrap().info();
    let mut image = map(&f);
    let before = image.clone();
    assert_eq!(
        relocate(&mut image, &info, info.image_base() + 0x1_0000).unwrap_err(),
        Error::Malformed("HIGHADJ without its low half")
    );
    assert!(image == before);
}

#[test]
fn relocate_rejects_unknown_types_for_x64_before_writing() {
    use reloc_kind::*;
    for kind in [HIGH, LOW, HIGHADJ, 5, 6, 7, 8, 9, 11, 15] {
        let (mut image, info) = image_with_relocs(Bits::Pe32Plus, &[(0x1000, DIR64), (0x1010, kind)], machine::AMD64);
        let before = image.clone();
        let err = relocate(&mut image, &info, 0x7ff0_0000_0000).unwrap_err();
        assert_eq!(err, Error::UnsupportedRelocation { kind, rva: 0x1010 });
        assert!(image == before, "type {kind}: nothing may be written");
    }
    let (mut image, info) = image_with_relocs(Bits::Pe32Plus, &[(0x1000, DIR64)], machine::ARMNT);
    assert_eq!(relocate(&mut image, &info, 0x7ff0_0000_0000).unwrap_err(), Error::UnsupportedMachine(machine::ARMNT));
    let (mut image, info) = image_with_relocs(Bits::Pe32Plus, &[(0x1000, DIR64), (0x1008, HIGHLOW)], machine::ARM64);
    assert_eq!(relocate(&mut image, &info, 0x7ff0_0000_0000).unwrap().applied, 2);
}

#[test]
fn relocate_without_a_table_only_moves_the_base() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.section(".text", CODE, vec![0xc3], None);
    let f = b.build();
    let info = *Pe::parse(&f).unwrap().info();
    let mut image = map(&f);
    assert_eq!(relocate(&mut image, &info, 0x2_0000_0000).unwrap(), RelocStats::default());
    assert_eq!(Pe::parse_mapped(&image).unwrap().info().image_base(), 0x2_0000_0000);
    assert!(Pe::parse(&f).unwrap().relocations().unwrap().is_none());
}

#[test]
fn malformed_relocation_tables() {
    let (image, info) = image_with_relocs(Bits::Pe32Plus, &[(0x1000, reloc_kind::DIR64)], machine::AMD64);
    let table = info.directory(dir::BASERELOC).unwrap().virtual_address as usize;
    let new_base = 0x3_0000_0000;

    // Block size below the header size would loop forever.
    let mut x = image.clone();
    x[table + 4..table + 8].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(relocate(&mut x, &info, new_base), Err(Error::Malformed(_))));
    let blocks = RelocBlocks::new(&x[table..table + 12]).collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    assert!(blocks[0].is_err());

    // Block size past the table.
    let mut x = image.clone();
    x[table + 4..table + 8].copy_from_slice(&0x100u32.to_le_bytes());
    assert_eq!(relocate(&mut x, &info, new_base).unwrap_err(), Error::Truncated);

    // A page RVA that leaves no room for the offset.
    let mut x = image.clone();
    x[table..table + 4].copy_from_slice(&0xffff_f800u32.to_le_bytes());
    assert!(matches!(relocate(&mut x, &info, new_base), Err(Error::Malformed(_))));

    // A target past the image.
    let mut x = image.clone();
    x[table..table + 4].copy_from_slice(&0x7000_0000u32.to_le_bytes());
    assert_eq!(relocate(&mut x, &info, new_base).unwrap_err(), Error::BadRva(0x7000_0000));

    // A target that straddles the end of the image.
    let end = image.len() as u32;
    let (mut x, info2) = image_with_relocs(Bits::Pe32Plus, &[(end - 4, reloc_kind::DIR64)], machine::AMD64);
    assert_eq!(relocate(&mut x, &info2, new_base).unwrap_err(), Error::BadRva(end - 4));

    // A directory outside the image.
    let mut bad = info;
    let mut dirs = [DataDirectory::default(); 16];
    dirs[dir::BASERELOC] = DataDirectory { virtual_address: 0xffff_fff0, size: 0x20 };
    bad.directories = directories(dirs);
    let mut x = image.clone();
    assert_eq!(relocate(&mut x, &bad, new_base).unwrap_err(), Error::BadRva(0xffff_fff0));

    // Trailing bytes too short for a block header are ignored.
    let blocks = RelocBlocks::new(&image[table..table + 12 + 6]).count();
    assert_eq!(blocks, 1);
}

/// Builds a `Directories` through a header, the only public way to make one.
fn directories(dirs: [DataDirectory; 16]) -> Directories {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.section(".text", CODE, vec![0xc3], None);
    for (i, d) in dirs.iter().enumerate() {
        b.dir(i, d.virtual_address, d.size);
    }
    *Pe::parse(&b.build()).unwrap().directories()
}
