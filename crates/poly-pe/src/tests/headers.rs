use std::vec;

use super::builder::*;
use super::*;

#[test]
fn pe32_plus_headers() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let info = pe.info();
    assert!(!pe.is_mapped());
    assert_eq!(info.nt_offset, NT_OFFSET as u32);
    assert_eq!(info.machine(), machine::AMD64);
    assert_eq!(info.bits(), Bits::Pe32Plus);
    assert_eq!(info.bits().pointer_size(), 8);
    assert_eq!(info.image_base(), 0x1_4000_0000);
    assert_eq!(info.entry_point(), 0x1000);
    assert!(info.is_executable() && !info.is_dll() && info.large_address_aware());
    assert!(info.dynamic_base() && info.nx_compat() && !info.high_entropy_va() && !info.guard_cf());
    assert!(info.relocatable() && !info.relocs_stripped());

    let opt = pe.optional_header();
    assert_eq!(opt.section_alignment, SECTION_ALIGN);
    assert_eq!(opt.file_alignment, FILE_ALIGN);
    assert_eq!(opt.size_of_headers, HEADERS);
    assert_eq!(opt.subsystem, subsystem::WINDOWS_CUI);
    assert_eq!(opt.major_linker_version, 14);
    assert_eq!(opt.major_operating_system_version, 6);
    assert_eq!(opt.major_subsystem_version, 6);
    assert_eq!(opt.base_of_code, SECTION_ALIGN);
    assert_eq!(opt.base_of_data, 0);
    assert_eq!(opt.size_of_stack_reserve, 0x10_0000);
    assert_eq!(opt.size_of_stack_commit, 0x1000);
    assert_eq!(opt.size_of_heap_reserve, 0x10_0000);
    assert_eq!(opt.size_of_heap_commit, 0x1000);
    assert_eq!(opt.number_of_rva_and_sizes, 16);
    assert_eq!(pe.directories().len(), 16);
    assert_eq!(pe.file_header().number_of_sections as usize, pe.sections().len());
    assert_eq!(pe.file_header().time_date_stamp, 0x5eed_0001);

    let names: Vec<&[u8]> = (0..pe.sections().len()).map(|i| pe.section_name(i).unwrap()).collect();
    assert_eq!(
        names,
        [
            &b".text"[..],
            b".idata",
            b".data",
            b".tls",
            b".rdata",
            b".xdata",
            b".pdata",
            b".didat",
            b".rsrc",
            b".bss",
            b".reloc"
        ]
    );
    let (i, text) = pe.section_by_name(b".text").unwrap();
    assert_eq!(i, 0);
    assert_eq!(text.protection(), Protection { read: true, write: false, execute: true });
    let (_, data) = pe.section_by_name(b".data").unwrap();
    assert_eq!(data.protection(), Protection { read: true, write: true, execute: false });
    let (_, reloc) = pe.section_by_name(b".reloc").unwrap();
    assert!(reloc.characteristics & section_flags::MEM_DISCARDABLE != 0);
    assert_eq!(reloc.protection(), Protection::READ_ONLY);
    assert!(pe.section_by_name(b".nothere").is_none());
    let (_, bss) = pe.section_by_name(b".bss").unwrap();
    assert_eq!((bss.size_of_raw_data, bss.virtual_size, bss.file_backed_size()), (0, 0x2000, 0));
}

#[test]
fn pe32_headers() {
    let s = sample_exe(Bits::Pe32);
    let pe = Pe::parse(&s.file).unwrap();
    let opt = pe.optional_header();
    assert_eq!(opt.bits, Bits::Pe32);
    assert_eq!(pe.info().machine(), machine::I386);
    assert_eq!(opt.image_base, 0x40_0000);
    assert_eq!(opt.base_of_data, 0x2000);
    assert_eq!(opt.size_of_stack_reserve, 0x10_0000);
    assert_eq!(opt.size_of_stack_commit, 0x1000);
    assert_eq!(opt.size_of_heap_reserve, 0x10_0000);
    assert_eq!(opt.size_of_heap_commit, 0x1000);
    assert_eq!(opt.number_of_rva_and_sizes, 16);
    assert_eq!(pe.info().bits().pointer_size(), 4);
    assert_eq!(pe.info().directory(dir::IMPORT).unwrap().virtual_address, s.idata);
    assert!(pe.info().directory(dir::EXCEPTION).is_none());
}

#[test]
fn dll_flags_and_characteristics() {
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.characteristics = file_flags::EXECUTABLE_IMAGE | file_flags::DLL | file_flags::RELOCS_STRIPPED;
    b.dll_characteristics = dll_flags::HIGH_ENTROPY_VA | dll_flags::GUARD_CF | dll_flags::TERMINAL_SERVER_AWARE;
    b.subsystem = subsystem::WINDOWS_GUI;
    b.section(".text", CODE, vec![0xc3], None);
    let f = b.build();
    let pe = Pe::parse(&f).unwrap();
    let info = pe.info();
    assert!(info.is_dll() && info.relocs_stripped() && !info.relocatable());
    assert!(info.high_entropy_va() && info.guard_cf() && !info.dynamic_base() && !info.nx_compat());
    assert_eq!(pe.optional_header().subsystem, subsystem::WINDOWS_GUI);
}

#[test]
fn signatures_and_magic() {
    let f = sample_exe(Bits::Pe32Plus).file;
    assert_eq!(Pe::parse(b"").unwrap_err(), Error::NotMz);
    assert_eq!(Pe::parse(b"ZM\0\0").unwrap_err(), Error::NotMz);
    assert_eq!(Pe::parse(&f[..0x3e]).unwrap_err(), Error::Truncated);

    let mut bad = f.clone();
    bad[NT_OFFSET] = b'X';
    assert_eq!(Pe::parse(&bad).unwrap_err(), Error::NoPeSignature);

    let mut bad = f.clone();
    put32(&mut bad, 0x3c, 0xffff_fff0);
    assert_eq!(Pe::parse(&bad).unwrap_err(), Error::NoPeSignature);

    let mut bad = f.clone();
    put16(&mut bad, NT_OFFSET + 24, 0x107);
    assert_eq!(Pe::parse(&bad).unwrap_err(), Error::BadOptionalHeader);

    // A PE32+ magic in a header too short for PE32+ fields.
    let mut bad = f.clone();
    put16(&mut bad, NT_OFFSET + 20, 96);
    assert_eq!(Pe::parse(&bad).unwrap_err(), Error::BadOptionalHeader);

    // SizeOfOptionalHeader past the end of the data.
    let mut bad = f[..0x200].to_vec();
    put16(&mut bad, NT_OFFSET + 20, 0xffff);
    assert_eq!(Pe::parse(&bad).unwrap_err(), Error::Truncated);
}

#[test]
fn data_directory_count_is_clipped() {
    let f = sample_exe(Bits::Pe32Plus).file;
    let count_at = NT_OFFSET + 24 + 108;

    let mut huge = f.clone();
    put32(&mut huge, count_at, u32::MAX);
    let pe = Pe::parse(&huge).unwrap();
    assert_eq!(pe.directories().len(), 16);
    assert_eq!(pe.optional_header().number_of_rva_and_sizes, u32::MAX);

    // Only two slots: imports exist, everything after does not.
    let mut few = f.clone();
    put32(&mut few, count_at, 2);
    let pe = Pe::parse(&few).unwrap();
    assert_eq!(pe.directories().len(), 2);
    assert!(pe.info().directory(dir::IMPORT).is_some());
    assert!(pe.info().directory(dir::BASERELOC).is_none());
    assert_eq!(pe.directories().raw(dir::TLS), DataDirectory::default());
    assert!(pe.tls().unwrap().is_none());

    // Slots must also fit in SizeOfOptionalHeader: shrink it to 3 slots
    // (the section table then starts inside the old directories, which
    // only changes what the section table reads).
    let mut short = f.clone();
    put16(&mut short, NT_OFFSET + 20, 112 + 3 * 8);
    let pe = Pe::parse(&short).unwrap();
    assert_eq!(pe.directories().len(), 3);
}

#[test]
fn section_count_limit() {
    let mut f = sample_exe(Bits::Pe32Plus).file;
    put16(&mut f, NT_OFFSET + 6, 97);
    assert_eq!(Pe::parse(&f).unwrap_err(), Error::TooMany);
    put16(&mut f, NT_OFFSET + 6, 0xffff);
    assert_eq!(Pe::parse(&f).unwrap_err(), Error::TooMany);
    // 96 sections fit the limit but not the data.
    put16(&mut f, NT_OFFSET + 6, 96);
    let short = &f[..0x400];
    assert_eq!(Pe::parse(short).unwrap_err(), Error::Truncated);
}

#[test]
fn rva_translation() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    // Headers map one to one.
    assert_eq!(pe.rva_to_offset(0x3c).unwrap(), 0x3c);
    // .text is the first section in the file, at SizeOfHeaders.
    assert_eq!(pe.rva_to_offset(s.text + 5).unwrap(), HEADERS as usize + 5);
    assert_eq!(pe.bytes_at_rva(s.text, 1).unwrap(), &[0xc3]);
    // Between the headers and the first section: nothing.
    assert_eq!(pe.rva_to_offset(0x800).unwrap_err(), Error::BadRva(0x800));
    // A zero-filled section has no bytes in the file.
    assert_eq!(pe.rva_to_offset(s.bss).unwrap_err(), Error::BadRva(s.bss));
    // Past the image.
    assert!(pe.rva_to_offset(0x7fff_0000).is_err());
    // A read cannot run off the end of a section's raw data.
    assert_eq!(pe.bytes_at_rva(s.text + 0x7f, 2).unwrap_err(), Error::Truncated);
    // Raw data past VirtualSize (file alignment padding) is not part of the image.
    assert_eq!(pe.rva_to_offset(s.text + 0x80).unwrap_err(), Error::BadRva(s.text + 0x80));
    assert_eq!(pe.str_at_rva(s.didat + 0x100).unwrap(), b"delayed.dll");

    let image = map(&s.file);
    let m = Pe::parse_mapped(&image).unwrap();
    assert!(m.is_mapped());
    assert_eq!(m.rva_to_offset(s.text + 5).unwrap(), s.text as usize + 5);
    // Mapped, the zero fill is readable.
    assert_eq!(m.bytes_at_rva(s.bss, 4).unwrap(), &[0, 0, 0, 0]);
}

/// A COFF object with long section names in its string table.
fn object_with_long_names() -> Vec<u8> {
    let mut o = Vec::new();
    put16(&mut o, 0, machine::AMD64);
    put16(&mut o, 2, 3);
    let table = 20;
    put_bytes(&mut o, table, b".text\0\0\0");
    put32(&mut o, table + 16, 4);
    put32(&mut o, table + 20, 0x100);
    put32(&mut o, table + 36, CODE | 0x0050_0000);
    put_bytes(&mut o, table + 40, b"/4\0\0\0\0\0\0");
    put32(&mut o, table + 40 + 16, 2);
    put32(&mut o, table + 40 + 20, 0x104);
    put32(&mut o, table + 40 + 36, DATA);
    put_bytes(&mut o, table + 80, b"/23\0\0\0\0\0");
    put32(&mut o, table + 80 + 36, BSS);
    put_bytes(&mut o, 0x100, &[0xc3, 0x90, 0x90, 0x90, 0xab, 0xcd]);
    // One symbol record (18 bytes), then the string table.
    let symtab = 0x108;
    put32(&mut o, 8, symtab);
    put32(&mut o, 12, 1);
    let strings = symtab as usize + 18;
    put_bytes(&mut o, strings + 4, b".data$long_section\0.bss$another\0");
    put32(&mut o, strings, 4 + 32);
    o
}

#[test]
fn coff_object_long_names() {
    let o = object_with_long_names();
    let obj = CoffObject::parse(&o).unwrap();
    assert_eq!(obj.header.machine, machine::AMD64);
    assert_eq!(obj.sections().len(), 3);
    assert_eq!(obj.section_name(0).unwrap(), b".text");
    assert_eq!(obj.section_name(1).unwrap(), b".data$long_section");
    assert_eq!(obj.section_name(2).unwrap(), b".bss$another");
    assert_eq!(obj.sections().get(1).unwrap().long_name_offset(), Some(4));
    assert_eq!(obj.sections().get(0).unwrap().alignment(), Some(16));
    assert_eq!(obj.section_data(0).unwrap(), &[0xc3, 0x90, 0x90, 0x90]);
    assert_eq!(obj.section_data(1).unwrap(), &[0xab, 0xcd]);
    assert_eq!(obj.section_data(2).unwrap(), &[] as &[u8]);
    assert_eq!(obj.string_table().unwrap().size(), 36);

    // An offset inside the size field, or past the table.
    let mut bad = o.clone();
    put_bytes(&mut bad, 20 + 40, b"/2\0\0\0\0\0\0");
    assert!(CoffObject::parse(&bad).unwrap().section_name(1).is_err());
    let mut bad = o.clone();
    put_bytes(&mut bad, 20 + 40, b"/9999999");
    assert!(CoffObject::parse(&bad).unwrap().section_name(1).is_err());
    // A "/" name that is not a number stays as it is.
    let mut odd = o.clone();
    put_bytes(&mut odd, 20 + 40, b"/x\0\0\0\0\0\0");
    assert_eq!(CoffObject::parse(&odd).unwrap().section_name(1).unwrap(), b"/x");
    // No string table: the raw name.
    let mut none = o.clone();
    put32(&mut none, 8, 0);
    assert_eq!(CoffObject::parse(&none).unwrap().section_name(1).unwrap(), b"/4");
    // An import library member header is not an object.
    let mut lib = vec![0u8; 20];
    put16(&mut lib, 2, 0xffff);
    assert_eq!(CoffObject::parse(&lib).unwrap_err(), Error::UnsupportedMachine(0));
    assert_eq!(CoffObject::parse(&o[..19]).unwrap_err(), Error::Truncated);
    assert_eq!(CoffObject::parse(&o[..80]).unwrap_err(), Error::Truncated);
}

#[test]
fn image_section_long_names() {
    // MinGW-style: an image that keeps a COFF string table for long names.
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.section(".text", CODE, vec![0xc3], None);
    b.section("/4", RDATA | section_flags::MEM_DISCARDABLE, vec![1, 2, 3], None);
    let mut f = b.build();
    let symtab = f.len();
    put32(&mut f, NT_OFFSET + 4 + 8, symtab as u32);
    put32(&mut f, NT_OFFSET + 4 + 12, 0);
    put32(&mut f, symtab, 4 + 12);
    put_bytes(&mut f, symtab + 4, b".debug_info\0");
    let pe = Pe::parse(&f).unwrap();
    assert_eq!(pe.section_name(1).unwrap(), b".debug_info");
    assert_eq!(pe.section_by_name(b".debug_info").unwrap().0, 1);
    // A mapped image has no string table.
    let image = map(&f);
    assert_eq!(Pe::parse_mapped(&image).unwrap().section_name(1).unwrap(), b"/4");
}

#[test]
fn protections_from_characteristics() {
    use section_flags::*;
    let p = Protection::from_characteristics;
    assert_eq!(p(0), Protection::default());
    assert_eq!(p(MEM_READ | MEM_WRITE | MEM_EXECUTE), Protection { read: true, write: true, execute: true });
    assert_eq!(p(MEM_EXECUTE), Protection { read: false, write: false, execute: true });
    // CNT_CODE alone does not make a section executable.
    assert_eq!(p(CNT_CODE | MEM_READ), Protection::READ_ONLY);
}
