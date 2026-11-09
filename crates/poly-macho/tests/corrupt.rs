//! Hand-made corrupt files: each test takes a real file, changes a few bytes
//! the way a broken or hostile file would, and checks the exact error.
//! Nothing here may panic or hang.

mod support;

use poly_macho::*;
use support::*;

/// The offset of the segment command with this name.
fn seg(data: &[u8], name: &str) -> usize {
    let m = MachO::parse(data).unwrap();
    for lc in m.raw_commands() {
        let lc = lc.unwrap();
        if lc.cmd() == LC_SEGMENT_64 && lc.data()[8..8 + name.len()] == *name.as_bytes() {
            return lc.data().as_ptr() as usize - data.as_ptr() as usize;
        }
    }
    panic!("no segment {name}");
}

fn base() -> Vec<u8> {
    fixture("arm64_classic.exe")
}

fn parse_err(data: &[u8]) -> Error {
    MachO::parse(data).unwrap_err()
}

// segment fields, relative to the command
const VMADDR: usize = 24;
const VMSIZE: usize = 32;
const FILEOFF: usize = 40;
const FILESIZE: usize = 48;
const NSECTS: usize = 64;
const SECT0: usize = 72; // first section; addr +32, size +40, offset +48

#[test]
fn truncated_headers_and_commands() {
    let data = base();
    let h = MachO::parse(&data).unwrap().header().sizeofcmds as usize;
    // Every length up to the end of the load commands fails, none panics.
    for len in 0..32 + h {
        let e = parse_err(&data[..len]);
        assert!(matches!(e, Error::Truncated | Error::NotMachO), "len {len}: {e:?}");
    }
    // With the commands whole but the segments' contents cut off, the first
    // segment that reaches past the end is reported.
    assert_eq!(parse_err(&data[..32 + h]), Error::SegmentOutsideFile);
    assert_eq!(parse_err(&data[..data.len() - 1]), Error::SegmentOutsideFile);
    assert!(MachO::parse(&data).is_ok());
}

#[test]
fn command_sizes_and_counts() {
    let data = base();
    for bad in [0u32, 1, 7, 9, 20] {
        let mut f = data.clone();
        w32(&mut f, 32 + 4, bad);
        assert_eq!(parse_err(&f), Error::BadCommandSize, "cmdsize {bad}");
    }
    let mut f = data.clone();
    w32(&mut f, 32 + 4, 0x7fff_fff8);
    assert_eq!(parse_err(&f), Error::CommandOverrun);
    let mut f = data.clone();
    w32(&mut f, 32 + 4, u32::MAX - 7);
    assert_eq!(parse_err(&f), Error::CommandOverrun);
    // The last command ends exactly at sizeofcmds; one more byte is a gap.
    let mut f = data.clone();
    let n = r32(&f, 20);
    w32(&mut f, 20, n + 8);
    assert_eq!(parse_err(&f), Error::CommandsSizeMismatch);
    let mut f = data.clone();
    w32(&mut f, 20, n - 8);
    assert!(matches!(parse_err(&f), Error::CommandsSizeMismatch | Error::CommandOverrun));
    // ncmds beyond what fits.
    let mut f = data.clone();
    w32(&mut f, 16, 0x1000_0000);
    assert_eq!(parse_err(&f), Error::CommandsSizeMismatch);
}

#[test]
fn bad_segments() {
    let data = base();
    let text = seg(&data, "__TEXT");
    let datas = seg(&data, "__DATA");
    let linkedit = seg(&data, "__LINKEDIT");

    // __DATA moved onto __TEXT in memory, and in the file.
    let mut f = data.clone();
    w64(&mut f, datas + VMADDR, 0x1_0000_1000);
    assert_eq!(parse_err(&f), Error::SegmentsOverlapInMemory);
    let mut f = data.clone();
    w64(&mut f, datas + FILEOFF, 0x1000);
    // (its sections now start before the segment, which is reported first)
    assert!(matches!(parse_err(&f), Error::SegmentsOverlapInFile | Error::SectionOutsideSegment));
    // Oversized: more file bytes than the file has, more than the vm size.
    let mut f = data.clone();
    w64(&mut f, linkedit + FILESIZE, 0x10_0000);
    assert_eq!(parse_err(&f), Error::SegmentFileSizeExceedsVmSize);
    let mut f = data.clone();
    w64(&mut f, linkedit + VMSIZE, 0x10_0000);
    w64(&mut f, linkedit + FILESIZE, 0x10_0000);
    assert_eq!(parse_err(&f), Error::SegmentOutsideFile);
    // Wrapping ranges.
    let mut f = data.clone();
    w64(&mut f, text + VMADDR, u64::MAX - 0x100);
    assert_eq!(parse_err(&f), Error::SegmentOverflow);
    let mut f = data.clone();
    w64(&mut f, linkedit + FILEOFF, u64::MAX - 0x100);
    assert_eq!(parse_err(&f), Error::SegmentOverflow);
    // A section count that does not match cmdsize, either way.
    for n in [0u32, 1, 100, u32::MAX, 0x1000_0000] {
        let mut f = data.clone();
        w32(&mut f, text + NSECTS, n);
        assert_eq!(parse_err(&f), Error::BadSectionCount, "nsects {n}");
    }
    // A section outside its segment: in memory, in the file, wrapping.
    let mut f = data.clone();
    w64(&mut f, text + SECT0 + 32, 0x1_0010_0000);
    assert_eq!(parse_err(&f), Error::SectionOutsideSegment);
    let mut f = data.clone();
    w64(&mut f, text + SECT0 + 40, u64::MAX);
    assert_eq!(parse_err(&f), Error::SectionOutsideSegment);
    let mut f = data.clone();
    w32(&mut f, text + SECT0 + 48, 0xffff_0000);
    assert_eq!(parse_err(&f), Error::SectionOutsideSegment);
    // A segment past the limit.
    let mut img = Image::arm64();
    for i in 0..=MAX_SEGMENTS as u64 {
        img = img.segment("__S", i * 0x1000, 0x1000, 0, 0, 3, &[]);
    }
    assert_eq!(parse_err(&img.build(0x10000)), Error::TooManySegments);
    // Exactly at the limit is fine.
    let mut img = Image::arm64();
    for i in 0..MAX_SEGMENTS as u64 {
        img = img.segment("__S", i * 0x1000, 0x1000, 0, 0, 3, &[]);
    }
    assert_eq!(MachO::parse(&img.build(0x10000)).unwrap().segment_table().len(), MAX_SEGMENTS);
}

#[test]
fn bad_symbol_tables() {
    let data = base();
    let st = cmd_offset(&data, LC_SYMTAB);
    let good = MachO::parse(&data).unwrap();
    let table = good.symtab().unwrap().unwrap();
    let symoff = r32(&data, st + 8) as usize;
    let stroff = r32(&data, st + 16) as usize;

    // Huge symbol counts and tables that leave the file.
    for n in [0x1000_0000u32, u32::MAX] {
        let mut f = data.clone();
        w32(&mut f, st + 12, n);
        assert_eq!(MachO::parse(&f).unwrap().symtab().unwrap_err(), Error::TableOutsideFile);
    }
    let mut f = data.clone();
    w32(&mut f, st + 8, u32::MAX);
    assert_eq!(MachO::parse(&f).unwrap().symtab().unwrap_err(), Error::TableOutsideFile);
    let mut f = data.clone();
    w32(&mut f, st + 16, u32::MAX - 3);
    assert_eq!(MachO::parse(&f).unwrap().symtab().unwrap_err(), Error::TableOutsideFile);
    let mut f = data.clone();
    w32(&mut f, st + 20, u32::MAX);
    assert_eq!(MachO::parse(&f).unwrap().symtab().unwrap_err(), Error::TableOutsideFile);

    // A string index past the table; at its last byte; a table that ends
    // inside a name.
    let mut f = data.clone();
    w32(&mut f, symoff, 0x7fff_ffff);
    let m = MachO::parse(&f).unwrap();
    let t = m.symtab().unwrap().unwrap();
    let sym = t.get(0).unwrap();
    assert_eq!(t.name(&sym), Err(Error::StringOutOfRange));
    assert_eq!(t.iter().count(), 15, "the symbols themselves are still readable");
    let mut f = data.clone();
    let strx = table.get(14).unwrap().n_strx;
    assert_eq!(table.name_at(strx).unwrap(), b"_weakdep_fn");
    w32(&mut f, st + 20, strx + 3);
    let m = MachO::parse(&f).unwrap();
    let t = m.symtab().unwrap().unwrap();
    assert_eq!(t.name_at(strx), Err(Error::BadString));
    assert_eq!(t.name_at(strx + 3), Err(Error::StringOutOfRange));
    // No NUL at all after the index.
    let mut f = data.clone();
    let strsize = r32(&f, st + 20) as usize;
    for b in &mut f[stroff..stroff + strsize] {
        *b = b'x';
    }
    let m = MachO::parse(&f).unwrap();
    let t = m.symtab().unwrap().unwrap();
    assert_eq!(t.name_at(1), Err(Error::BadString));
    // A name longer than MAX_NAME_LEN (a table with no NULs beyond the limit).
    let big = vec![b'a'; MAX_NAME_LEN + 100];
    let img = Image::arm64().symtab(0x100, 1, 0x200, big.len() as u32 + 8);
    let mut f = img.build(0x200 + big.len() + 8);
    f[0x200 + 1..0x200 + 1 + big.len()].copy_from_slice(&big);
    w32(&mut f, 0x100, 1);
    let m = MachO::parse(&f).unwrap();
    let t = m.symtab().unwrap().unwrap();
    assert_eq!(t.name(&t.get(0).unwrap()), Err(Error::BadString));

    // The table is intact in the original.
    assert_eq!(table.len(), 15);

    // Dysymtab: ranges beyond the symbols, tables outside the file.
    let dy = cmd_offset(&data, LC_DYSYMTAB);
    for (field, value, want) in [
        (12usize, 100u32, Error::BadSymbolRange),   // nlocalsym
        (8, u32::MAX, Error::BadSymbolRange),       // ilocalsym
        (20, 0xffff_ffff, Error::BadSymbolRange),   // nextdefsym
        (28, 50, Error::BadSymbolRange),            // nundefsym
        (56, u32::MAX, Error::TableOutsideFile),    // indirectsymoff
        (60, 0x4000_0000, Error::TableOutsideFile), // nindirectsyms
    ] {
        let mut f = data.clone();
        w32(&mut f, dy + field, value);
        assert_eq!(MachO::parse(&f).unwrap().dysymtab().unwrap_err(), want, "field {field}");
    }
    // A legacy table (relocations) outside the file.
    let mut f = data.clone();
    w32(&mut f, dy + 8 + 14 * 4, 0xffff_0000); // extreloff
    w32(&mut f, dy + 8 + 15 * 4, 4); // nextrel
    assert_eq!(MachO::parse(&f).unwrap().dysymtab().unwrap_err(), Error::TableOutsideFile);
    // A symbol index out of range in the indirect table is data, not an error:
    let mut f = data.clone();
    let ind = r32(&f, dy + 56) as usize;
    w32(&mut f, ind, 0x3fff_0000);
    let m = MachO::parse(&f).unwrap();
    let first = m.dysymtab().unwrap().unwrap().indirect_symbols().next().unwrap();
    assert_eq!(first.symbol_index(), Some(0x3fff_0000));
    assert_eq!(m.symtab().unwrap().unwrap().get(0x3fff_0000), Err(Error::SymbolOutOfRange));
}

#[test]
fn bad_load_command_contents() {
    let data = base();
    // A dylib name offset outside the command, or before its fixed part.
    let dy = cmd_offset(&data, LC_LOAD_DYLIB);
    for off in [0u32, 23, 0xffff, u32::MAX] {
        let mut f = data.clone();
        w32(&mut f, dy + 8, off);
        assert_eq!(parse_err(&f), Error::BadCommandString, "offset {off}");
    }
    // LC_MAIN beyond the image.
    let main = cmd_offset(&data, LC_MAIN);
    let mut f = data.clone();
    w64(&mut f, main + 8, 0x1000_0000);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.entry(), Err(Error::BadEntry));
    assert_eq!(m.summary().unwrap_err(), Error::BadEntry);
    let mut f = data.clone();
    w64(&mut f, main + 8, u64::MAX);
    assert_eq!(MachO::parse(&f).unwrap().entry(), Err(Error::BadEntry));
    // A command with a size smaller than its structure.
    let mut f = data.clone();
    w32(&mut f, main + 4, 8);
    assert!(MachO::parse(&f).is_err());
    // Linkedit blobs outside the file.
    let fs = cmd_offset(&data, LC_FUNCTION_STARTS);
    let mut f = data.clone();
    w32(&mut f, fs + 8, u32::MAX);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.function_starts().unwrap_err(), Error::TableOutsideFile);
    let mut f = data.clone();
    w32(&mut f, fs + 12, 0x1000_0000);
    assert_eq!(
        MachO::parse(&f).unwrap().linkedit_data(LinkeditKind::FunctionStarts).unwrap_err(),
        Error::TableOutsideFile
    );
    // A function starts table with a bad LEB128 in it.
    let off = r32(&data, fs + 8) as usize;
    let mut f = data.clone();
    for b in &mut f[off..off + 8] {
        *b = 0xff;
    }
    let m = MachO::parse(&f).unwrap();
    let items: Vec<_> = m.function_starts().unwrap().unwrap().collect();
    assert_eq!(items, [Err(Error::BadLeb128)]);
}

#[test]
fn bad_dyld_info_streams() {
    let data = base();
    let di = cmd_offset(&data, LC_DYLD_INFO_ONLY);
    let rebase = r32(&data, di + 8) as usize;
    let bind = r32(&data, di + 16) as usize;
    let lazy = r32(&data, di + 32) as usize;

    // A rebase stream of continuation bytes: a ULEB128 that never ends.
    let mut f = data.clone();
    f[rebase] = REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 3;
    for b in &mut f[rebase + 1..rebase + 16] {
        *b = 0x80;
    }
    let m = MachO::parse(&f).unwrap();
    let r: Vec<_> = m.dyld_info().unwrap().unwrap().rebases().collect();
    assert_eq!(r, [Err(Error::BadLeb128)]);
    // ... and one that is too long for 64 bits although it ends.
    let mut f = data.clone();
    f[rebase] = REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 3;
    for b in &mut f[rebase + 1..rebase + 11] {
        *b = 0xff;
    }
    f[rebase + 11] = 0x7f;
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.dyld_info().unwrap().unwrap().rebases().next(), Some(Err(Error::BadLeb128)));

    // A segment index that does not exist (the stream's first opcode).
    let mut f = data.clone();
    f[rebase] = REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 9;
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.dyld_info().unwrap().unwrap().rebases().next(), Some(Err(Error::BadSegmentIndex)));

    // A bind stream that is all DO_BIND: no segment, then no more.
    let mut f = data.clone();
    for b in &mut f[bind..bind + 8] {
        *b = BIND_OPCODE_DO_BIND;
    }
    let m = MachO::parse(&f).unwrap();
    let r: Vec<_> = m.dyld_info().unwrap().unwrap().binds().collect();
    assert_eq!(r, [Err(Error::BadOpcode(BIND_OPCODE_DO_BIND))]);

    // A threaded bind opcode.
    let mut f = data.clone();
    f[bind] = BIND_OPCODE_THREADED;
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.dyld_info().unwrap().unwrap().binds().next(), Some(Err(Error::BadOpcode(BIND_OPCODE_THREADED))));

    // A lazy stream without any DONE: its one entry is read to the end.
    let mut f = data.clone();
    let lazy_size = r32(&f, di + 36) as usize;
    for b in &mut f[lazy..lazy + lazy_size] {
        if *b == 0 {
            *b = 0x51; // SET_TYPE_IMM | 1
        }
    }
    let m = MachO::parse(&f).unwrap();
    let r: Vec<_> = m.dyld_info().unwrap().unwrap().lazy_binds().collect();
    assert!(r.len() <= 2);

    // Ranges that leave the file; every one of the five.
    for field in [8usize, 16, 24, 32, 40] {
        let mut f = data.clone();
        w32(&mut f, di + field, 0xffff_0000);
        w32(&mut f, di + field + 4, 0x1_0000);
        let m = MachO::parse(&f).unwrap();
        assert_eq!(m.dyld_info().unwrap_err(), Error::TableOutsideFile, "field {field}");
    }
}

#[test]
fn bad_chained_fixups() {
    let data = fixture("arm64_chained.exe");
    let cf = cmd_offset(&data, LC_DYLD_CHAINED_FIXUPS);
    let payload = r32(&data, cf + 8) as usize;
    let m = MachO::parse(&data).unwrap();
    assert_eq!(m.chained_fixups().unwrap().unwrap().fixups().count(), 10);

    // The payload is cut short, or outside the file.
    let mut f = data.clone();
    w32(&mut f, cf + 12, 8);
    assert_eq!(MachO::parse(&f).unwrap().chained_fixups().unwrap_err(), Error::Truncated);
    let mut f = data.clone();
    w32(&mut f, cf + 12, 0);
    assert_eq!(MachO::parse(&f).unwrap().chained_fixups().unwrap_err(), Error::Truncated);
    let mut f = data.clone();
    w32(&mut f, cf + 8, u32::MAX);
    assert_eq!(MachO::parse(&f).unwrap().chained_fixups().unwrap_err(), Error::TableOutsideFile);

    // A chain whose next pointer runs off the page (12 bits of 4-byte
    // strides: 0xfff * 4 = 0x3ffc, then 8 bytes do not fit the 16 KiB page
    // from offset 0x18 + 0x3ffc).
    let dc = seg(&data, "__DATA_CONST");
    let slot0 = r64(&data, dc_file_offset(&data, dc));
    let mut f = data.clone();
    w64(&mut f, dc_file_offset(&data, dc), slot0 | 0xfff << 51);
    let m = MachO::parse(&f).unwrap();
    let r: Vec<_> = m.chained_fixups().unwrap().unwrap().fixups().collect();
    assert_eq!(*r.last().unwrap(), Err(Error::ChainEscapesPage));

    // A chain linked slot after slot through the whole page (2048 slots of
    // 8 bytes, "next" 2 strides of 4 bytes): the last one points past the
    // page, so the walk ends with an error after the page is full.
    let mut f = data.clone();
    for i in 0..0x800usize {
        w64(&mut f, 0x8000 + i * 8, (2u64 << 51) | 0x1000_0000);
    }
    let m = MachO::parse(&f).unwrap();
    let r: Vec<_> = m.chained_fixups().unwrap().unwrap().fixups().collect();
    // 4 fixups in __DATA_CONST, then the full page.
    assert_eq!(r.len(), 4 + 0x800);
    assert_eq!(*r.last().unwrap(), Err(Error::ChainEscapesPage));

    // A starts_in_segment offset past the payload, and a page count that
    // claims more starts than the record has.
    let starts = payload + r32(&data, payload + 4) as usize;
    let seg2 = starts + r32(&data, starts + 4 + 4 * 2) as usize;
    let mut f = data.clone();
    w32(&mut f, starts + 4 + 4 * 2, 0x00ff_ffff);
    let m = MachO::parse(&f).unwrap();
    let e = m.chained_fixups().unwrap().unwrap().fixups().find_map(|x| x.err());
    assert_eq!(e, Some(Error::BadStartsInSegment));
    let mut f = data.clone();
    w16(&mut f, seg2 + 20, 0xffff);
    let m = MachO::parse(&f).unwrap();
    let e = m.chained_fixups().unwrap().unwrap().fixups().find_map(|x| x.err());
    assert_eq!(e, Some(Error::BadStartsInSegment));
    // A page start beyond the page.
    let mut f = data.clone();
    w16(&mut f, seg2 + 22, 0x3fff);
    let m = MachO::parse(&f).unwrap();
    let e = m.chained_fixups().unwrap().unwrap().fixups().find_map(|x| x.err());
    assert_eq!(e, Some(Error::BadPageStart));
    // An import whose name offset is outside the strings.
    let imports = payload + r32(&data, payload + 8) as usize;
    let mut f = data.clone();
    w32(&mut f, imports, 0xffff_fe00 | 1);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.chained_fixups().unwrap().unwrap().import(0), Err(Error::BadImport));
}

fn r64(b: &[u8], off: usize) -> u64 {
    u64::from_le_bytes(b[off..off + 8].try_into().unwrap())
}

/// The file offset of a segment's contents, from its command at `cmd`.
fn dc_file_offset(data: &[u8], cmd: usize) -> usize {
    r64(data, cmd + FILEOFF) as usize
}

#[test]
fn bad_export_tries() {
    let data = fixture("libtiny.dylib");
    let m = MachO::parse(&data).unwrap();
    let di = cmd_offset(&data, LC_DYLD_INFO_ONLY);
    let off = r32(&data, di + 40) as usize;
    let size = r32(&data, di + 44) as usize;
    assert_eq!(m.exports().unwrap().unwrap().count(), Ok(6));
    // The root has no export of its own and one edge: "_lib_" to a child.
    assert_eq!(&data[off..off + 2], &[0, 1]);
    let label_end = off + 2 + data[off + 2..].iter().position(|&b| b == 0).unwrap();
    let child_at = label_end + 1;

    // The child offset points back at the root: a cycle.
    let mut f = data.clone();
    f[child_at] = 0;
    let m = MachO::parse(&f).unwrap();
    let ex = m.exports().unwrap().unwrap();
    assert_eq!(ex.count(), Err(Error::TrieTooComplex));
    // ... which a lookup of a long enough name also notices, or ends.
    let long = vec![b'_'; 5000];
    assert!(ex.lookup(&long).is_err() || ex.lookup(&long) == Ok(None));
    // Out of range.
    let mut f = data.clone();
    f[child_at] = 0x7f;
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.exports().unwrap().unwrap().count(), Err(Error::TrieOutOfRange));
    // The trie range itself outside the file.
    let mut f = data.clone();
    w32(&mut f, di + 44, 0x1000_0000);
    assert_eq!(MachO::parse(&f).unwrap().exports().unwrap_err(), Error::TableOutsideFile);
    let _ = size;
}

#[test]
fn bad_fat_files() {
    let data = fixture("universal.bin");
    assert!(Fat::parse(&data).is_ok());
    // arch 1 at offset 8 + 20: offset +8, size +12
    let a1 = 8 + 20;
    // The second slice starts inside the first.
    let mut f = data.clone();
    be32(&mut f, a1 + 8, 4096 + 100);
    assert_eq!(Fat::parse(&f).unwrap_err(), Error::SlicesOverlap);
    // The second slice runs past the end of the file.
    let mut f = data.clone();
    be32(&mut f, a1 + 12, 0x7000_0000);
    assert_eq!(Fat::parse(&f).unwrap_err(), Error::Truncated);
    // The first slice grows over the second.
    let mut f = data.clone();
    be32(&mut f, 8 + 12, 40_000);
    assert_eq!(Fat::parse(&f).unwrap_err(), Error::SlicesOverlap);
    // A slice that starts inside the table.
    let mut f = data.clone();
    be32(&mut f, 8 + 8, 16);
    assert_eq!(Fat::parse(&f).unwrap_err(), Error::SlicesOverlap);
    // Counts: more slices than the table holds, absurd counts.
    let mut f = data.clone();
    be32(&mut f, 4, 3);
    assert!(Fat::parse(&f).is_err());
    let mut f = data.clone();
    be32(&mut f, 4, u32::MAX);
    assert_eq!(Fat::parse(&f).unwrap_err(), Error::TooManyArches);
    let mut f = data.clone();
    be32(&mut f, 4, 0);
    assert!(Fat::parse(&f).unwrap().is_empty());
    // A slice that is not a Mach-O file.
    let mut f = data.clone();
    f[4096] ^= 0xff;
    assert_eq!(select_slice(&f, CpuKind::X86_64).ok().map(|s| MachO::parse(s).is_err()), Some(true));
    // Truncated tables.
    for len in 0..8 + 2 * 20 {
        assert!(Fat::parse(&data[..len]).is_err(), "len {len}");
    }
}
