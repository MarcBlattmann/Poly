extern crate std;

use super::*;
use std::vec;
use std::vec::Vec;

const R: u32 = PF_R;
const RW: u32 = PF_R | PF_W;
const RX: u32 = PF_R | PF_X;

fn ph(p_type: u32, flags: u32, offset: u64, vaddr: u64, filesz: u64, memsz: u64, align: u64) -> ProgramHeader {
    ProgramHeader { p_type, flags, offset, vaddr, paddr: vaddr, filesz, memsz, align }
}

/// A file image built field by field, as the gABI lays it out.
#[derive(Clone)]
struct Image {
    e_type: u16,
    entry: u64,
    phdrs: Vec<ProgramHeader>,
    len: usize,
    /// Extra bytes to write at an offset (interpreter paths).
    patches: Vec<(usize, Vec<u8>)>,
}

impl Image {
    fn bytes(&self) -> Vec<u8> {
        let table_end = EHDR_SIZE + PHDR_SIZE * self.phdrs.len();
        // Recognisable contents: byte i is a function of i.
        let mut b: Vec<u8> = (0..self.len.max(table_end)).map(|i| (i % 251) as u8 ^ 0x5a).collect();
        b[..EHDR_SIZE].fill(0);
        b[..4].copy_from_slice(b"\x7fELF");
        b[4] = ELFCLASS64;
        b[5] = ELFDATA2LSB;
        b[6] = EV_CURRENT;
        b[7] = ELFOSABI_SYSV;
        b[16..18].copy_from_slice(&self.e_type.to_le_bytes());
        b[18..20].copy_from_slice(&EM_X86_64.to_le_bytes());
        b[20..24].copy_from_slice(&1u32.to_le_bytes());
        b[24..32].copy_from_slice(&self.entry.to_le_bytes());
        b[32..40].copy_from_slice(&(EHDR_SIZE as u64).to_le_bytes());
        b[52..54].copy_from_slice(&(EHDR_SIZE as u16).to_le_bytes());
        b[54..56].copy_from_slice(&(PHDR_SIZE as u16).to_le_bytes());
        b[56..58].copy_from_slice(&(self.phdrs.len() as u16).to_le_bytes());
        for (i, p) in self.phdrs.iter().enumerate() {
            let o = EHDR_SIZE + i * PHDR_SIZE;
            b[o..o + 4].copy_from_slice(&p.p_type.to_le_bytes());
            b[o + 4..o + 8].copy_from_slice(&p.flags.to_le_bytes());
            for (j, v) in [p.offset, p.vaddr, p.paddr, p.filesz, p.memsz, p.align].iter().enumerate() {
                b[o + 8 + 8 * j..o + 16 + 8 * j].copy_from_slice(&v.to_le_bytes());
            }
        }
        for (off, bytes) in &self.patches {
            b[*off..*off + bytes.len()].copy_from_slice(bytes);
        }
        b
    }
}

/// A classic static executable: headers + read-only data, text, data + bss.
fn static_exec() -> Image {
    Image {
        e_type: ET_EXEC,
        entry: 0x401000,
        phdrs: vec![
            ph(PT_LOAD, R, 0, 0x400000, 0x200, 0x200, 0x1000),
            ph(PT_LOAD, RX, 0x1000, 0x401000, 0x123, 0x123, 0x1000),
            ph(PT_LOAD, RW, 0x2010, 0x403010, 0x100, 0x3000, 0x1000),
            ph(PT_TLS, R, 0x2010, 0x403010, 0x10, 0x20, 8),
            ph(PT_GNU_STACK, RW, 0, 0, 0, 0, 16),
        ],
        len: 0x2110,
        patches: vec![],
    }
}

/// A static-PIE: linked at 0 with PT_PHDR and PT_DYNAMIC.
fn static_pie() -> Image {
    Image {
        e_type: ET_DYN,
        entry: 0x1040,
        phdrs: vec![
            ph(PT_PHDR, R, 64, 64, 6 * 56, 6 * 56, 8),
            ph(PT_LOAD, R, 0, 0, 0x300, 0x300, 0x1000),
            ph(PT_LOAD, RX, 0x1000, 0x1000, 0x80, 0x80, 0x1000),
            ph(PT_LOAD, RW, 0x1e00, 0x2e00, 0x200, 0x400, 0x1000),
            ph(PT_DYNAMIC, RW, 0x1f00, 0x2f00, 0x100, 0x100, 8),
            ph(PT_GNU_STACK, RW, 0, 0, 0, 0, 16),
        ],
        len: 0x2000,
        patches: vec![],
    }
}

/// A dynamically linked PIE naming an interpreter.
fn dynamic_pie() -> Image {
    let interp = b"/lib64/ld-linux-x86-64.so.2\0".to_vec();
    let n = interp.len() as u64;
    let mut img = static_pie();
    img.phdrs[0].filesz = 7 * 56;
    img.phdrs[0].memsz = 7 * 56;
    img.phdrs.insert(1, ph(PT_INTERP, R, 0x200, 0x200, n, n, 1));
    img.patches.push((0x200, interp));
    img
}

fn parse(img: &Image) -> Result<(), Error> {
    let bytes = img.bytes();
    let elf = Elf::parse(&bytes)?;
    LoadPlan::new(&elf, &LoadOptions::default()).map(|_| ())
}

fn with<F: FnOnce(&mut Image)>(base: Image, f: F) -> Image {
    let mut img = base;
    f(&mut img);
    img
}

#[test]
fn parses_static_exec() {
    let bytes = static_exec().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    assert_eq!(elf.elf_type(), ElfType::Exec);
    assert_eq!(elf.entry(), 0x401000);
    assert_eq!(elf.phnum(), 5);
    assert_eq!(elf.load_count(), 3);
    assert!(elf.is_static());
    assert_eq!(elf.interp(), None);
    assert_eq!(elf.dynamic(), None);
    assert_eq!(elf.phdr_segment(), None);
    assert_eq!(elf.gnu_stack(), Some(RW));
    assert_eq!(elf.tls(), Some(Tls { vaddr: 0x403010, filesz: 0x10, memsz: 0x20, align: 8 }));
    // No PT_PHDR: found through the first segment, which maps file offset 0.
    assert_eq!(elf.phdr_vaddr(), Some(0x400040));
    assert_eq!(elf.program_header_table(), &bytes[64..64 + 5 * 56]);
    let loads: Vec<_> = elf.load_segments().map(|p| p.vaddr).collect();
    assert_eq!(loads, [0x400000, 0x401000, 0x403010]);
}

#[test]
fn plans_static_exec() {
    let bytes = static_exec().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap();
    let r = Prot::READ;
    let rw = Prot::READ.union(Prot::WRITE);
    let rx = Prot::READ.union(Prot::EXEC);
    let file = |offset, len| Some(FileRange { offset, len });
    assert_eq!(
        plan.mappings(),
        [
            Mapping { start: 0x400000, end: 0x401000, prot: r, file: file(0, 0x200) },
            Mapping { start: 0x401000, end: 0x402000, prot: rx, file: file(0x1000, 0x123) },
            // The data page starts 0x10 bytes before p_vaddr, at file offset 0x2000.
            Mapping { start: 0x403000, end: 0x404000, prot: rw, file: file(0x2000, 0x110) },
            Mapping { start: 0x404000, end: 0x407000, prot: rw, file: None },
        ]
    );
    assert_eq!(plan.bias(), 0);
    assert_eq!(plan.entry(), 0x401000);
    assert_eq!(plan.phdr(), Some(0x400040));
    assert_eq!((plan.phnum(), plan.phent()), (5, 56));
    assert_eq!((plan.start(), plan.end(), plan.brk()), (0x400000, 0x407000, 0x407000));
    assert!(!plan.stack_executable());
    assert!(plan.warnings().is_empty());
    assert_eq!(plan.tls().unwrap().vaddr, 0x403010);
    assert_eq!(plan.mapping_at(0x404123).unwrap().start, 0x404000);
    assert_eq!(plan.mapping_at(0x402000), None);
    assert_eq!(plan.mappings()[2].zero_start(), 0x403110);
    assert_eq!(plan.mappings()[3].zero_start(), 0x404000);
    assert_eq!(plan.mappings()[3].size(), 0x3000);
}

#[test]
fn page_contents_and_zero_tail() {
    let bytes = static_exec().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap();
    let mut page = vec![0xffu8; PAGE_SIZE as usize];

    let data = plan.mapping_at(0x403010).unwrap();
    assert!(data.fill_page(&bytes, 0x403000, &mut page));
    assert_eq!(&page[..0x110], &bytes[0x2000..0x2110]);
    assert!(page[0x110..].iter().all(|&b| b == 0));
    assert_eq!(data.file_bytes(&bytes), &bytes[0x2000..0x2110]);

    let bss = plan.mapping_at(0x405000).unwrap();
    page.fill(0xff);
    assert!(bss.fill_page(&bytes, 0x405000, &mut page));
    assert!(page.iter().all(|&b| b == 0));
    assert_eq!(bss.file_bytes(&bytes), b"");

    // The header page holds the ELF header itself, so AT_PHDR is readable.
    let hdr = plan.mapping_at(0x400000).unwrap();
    assert!(hdr.fill_page(&bytes, 0x400000, &mut page));
    assert_eq!(&page[..4], b"\x7fELF");
    assert_eq!(&page[0x40..0x40 + 56], &bytes[0x40..0x40 + 56]);

    // Wrong page, misaligned address, wrong buffer size, wrong file.
    page.fill(0xee);
    assert!(!hdr.fill_page(&bytes, 0x401000, &mut page));
    assert!(!hdr.fill_page(&bytes, 0x400010, &mut page));
    assert!(!hdr.fill_page(&bytes, 0x400000, &mut page[..100]));
    assert!(!data.fill_page(&bytes[..0x2100], 0x403000, &mut page));
    assert!(page.iter().all(|&b| b == 0xee));
    assert_eq!(data.file_bytes(&bytes[..0x2100]), b"");
}

#[test]
fn plans_static_pie_at_default_base() {
    let bytes = static_pie().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    assert_eq!(elf.elf_type(), ElfType::Dyn);
    assert!(elf.is_static());
    assert!(elf.dynamic().is_some());
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap();
    let base = DEFAULT_DYN_BASE;
    assert_eq!(plan.bias(), base);
    assert_eq!(plan.entry(), base + 0x1040);
    assert_eq!(plan.phdr(), Some(base + 64));
    assert_eq!(plan.start(), base);
    assert_eq!(plan.brk(), base + 0x4000);
    let starts: Vec<_> = plan.mappings().iter().map(|m| (m.start - base, m.end - base, m.file.is_some())).collect();
    assert_eq!(starts, [(0, 0x1000, true), (0x1000, 0x2000, true), (0x2000, 0x3000, true), (0x3000, 0x4000, false)]);
    // p_vaddr 0x2e00 at file offset 0x1e00: the page starts at file offset 0x1000.
    assert_eq!(plan.mappings()[2].file, Some(FileRange { offset: 0x1000, len: 0xe00 + 0x200 }));
}

#[test]
fn dyn_base_is_raised_to_the_largest_alignment() {
    let img = with(static_pie(), |i| {
        for p in i.phdrs.iter_mut().filter(|p| p.p_type == PT_LOAD) {
            p.align = 0x20_0000;
        }
        // Keep p_vaddr ≡ p_offset (mod 2 MiB): move the data segment up 2 MiB.
        i.phdrs[3].vaddr += 0x20_0000 - 0x1000;
        i.phdrs[4].vaddr += 0x20_0000 - 0x1000;
    });
    let bytes = img.bytes();
    let elf = Elf::parse(&bytes).unwrap();
    let plan = LoadPlan::new(&elf, &LoadOptions::default()).unwrap();
    assert_eq!(plan.bias() % 0x20_0000, 0);
    assert!(plan.bias() >= DEFAULT_DYN_BASE && plan.bias() - DEFAULT_DYN_BASE < 0x20_0000);
    for m in plan.mappings() {
        assert_eq!(m.start % PAGE_SIZE, 0);
        if let Some(f) = m.file {
            assert_eq!(m.start % 0x20_0000, f.offset % 0x20_0000);
        }
    }
}

#[test]
fn interpreter_gets_its_own_base_and_auxv() {
    let exe_bytes = dynamic_pie().bytes();
    let exe = Elf::parse(&exe_bytes).unwrap();
    assert_eq!(exe.interp(), Some(&b"/lib64/ld-linux-x86-64.so.2"[..]));
    assert!(!exe.is_static());
    let exe_plan = LoadPlan::new(&exe, &LoadOptions::default()).unwrap();

    // The interpreter is an ET_DYN image too, planned at another base.
    let ld_bytes = static_pie().bytes();
    let ld = Elf::parse(&ld_bytes).unwrap();
    let opts = LoadOptions { dyn_base: 0x7fff_f7fc_0000, ..LoadOptions::default() };
    let ld_plan = LoadPlan::new(&ld, &opts).unwrap();
    assert_eq!(ld_plan.bias(), 0x7fff_f7fc_0000);

    let aux = AuxInfo::new(&exe_plan, Some(&ld_plan), Credentials::default());
    assert_eq!(aux.base, 0x7fff_f7fc_0000);
    assert_eq!(aux.entry, exe_plan.entry());
    assert_eq!(aux.phdr, Some(DEFAULT_DYN_BASE + 64));
    assert_eq!(aux.phnum, 7);
    assert_eq!(start_address(&exe_plan, Some(&ld_plan)), ld_plan.entry());
    assert_eq!(start_address(&exe_plan, None), exe_plan.entry());
    assert_eq!(AuxInfo::new(&exe_plan, None, Credentials::default()).base, 0);
}

#[test]
fn osabi_gnu_is_accepted() {
    let mut bytes = static_exec().bytes();
    bytes[7] = ELFOSABI_GNU;
    assert_eq!(Elf::parse(&bytes).unwrap().os_abi(), ELFOSABI_GNU);
}

#[test]
fn warnings_are_reported_not_refused() {
    let rwx = with(static_exec(), |i| i.phdrs[2].flags = RW | PF_X);
    let bytes = rwx.bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    assert_eq!(plan.warnings(), Warnings::WRITABLE_AND_EXECUTABLE);
    assert!(plan.mappings()[2].prot.writable() && plan.mappings()[2].prot.executable());

    let exec_stack = with(static_exec(), |i| i.phdrs[4].flags = RW | PF_X);
    let bytes = exec_stack.bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    assert!(plan.stack_executable());
    assert_eq!(plan.warnings(), Warnings::EXECUTABLE_STACK);

    let no_stack = with(static_exec(), |i| {
        i.phdrs.pop();
    });
    let bytes = no_stack.bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    assert!(!plan.stack_executable());
    assert_eq!(plan.warnings(), Warnings::NO_GNU_STACK);

    let data_entry = with(static_exec(), |i| i.entry = 0x403010);
    let bytes = data_entry.bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    assert!(plan.warnings().contains(Warnings::ENTRY_NOT_EXECUTABLE));
}

#[test]
fn empty_load_segments_are_skipped() {
    let img = with(static_exec(), |i| i.phdrs.insert(2, ph(PT_LOAD, RW, 0, 0x402000, 0, 0, 0x1000)));
    let bytes = img.bytes();
    let elf = Elf::parse(&bytes).unwrap();
    assert_eq!(elf.load_count(), 4);
    assert_eq!(LoadPlan::new(&elf, &LoadOptions::default()).unwrap().mappings().len(), 4);
}

#[test]
fn bss_only_segment_is_anonymous() {
    let img = with(static_exec(), |i| {
        i.phdrs[2] = ph(PT_LOAD, RW, 0x2010, 0x403010, 0, 0x10, 0x1000);
        i.phdrs.remove(3); // the TLS template needs file-backed memory
    });
    let bytes = img.bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    let last = plan.mappings().last().unwrap();
    assert_eq!((last.start, last.end, last.file), (0x403000, 0x404000, None));
    assert_eq!(plan.mappings().len(), 3);
}

/// The fixture's bytes with `bytes` written at `off`.
fn patch(img: &Image, off: usize, bytes: &[u8]) -> Vec<u8> {
    let mut b = img.bytes();
    b[off..off + bytes.len()].copy_from_slice(bytes);
    b
}

fn parse_bytes(b: &[u8]) -> Result<(), Error> {
    let elf = Elf::parse(b)?;
    LoadPlan::new(&elf, &LoadOptions::default()).map(|_| ())
}

#[test]
fn bad_file_headers_are_refused() {
    let img = static_exec();
    let full = img.bytes();
    let cases: &[(Vec<u8>, Error)] = &[
        (vec![], Error::NotElf),
        (b"\x7fEL".to_vec(), Error::NotElf),
        (b"MZ\x90\0".to_vec(), Error::NotElf),
        (full[..63].to_vec(), Error::Truncated),
        (patch(&img, 4, &[1]), Error::UnsupportedClass),
        (patch(&img, 5, &[2]), Error::UnsupportedEncoding),
        (patch(&img, 6, &[0]), Error::UnsupportedVersion),
        (patch(&img, 20, &2u32.to_le_bytes()), Error::UnsupportedVersion),
        (patch(&img, 7, &[9]), Error::UnsupportedOsAbi),
        (patch(&img, 16, &1u16.to_le_bytes()), Error::UnsupportedType),
        (patch(&img, 16, &4u16.to_le_bytes()), Error::UnsupportedType),
        (patch(&img, 18, &3u16.to_le_bytes()), Error::UnsupportedMachine),
        (patch(&img, 18, &183u16.to_le_bytes()), Error::UnsupportedMachine),
        (patch(&img, 52, &32u16.to_le_bytes()), Error::BadHeaderSize),
        (patch(&img, 54, &32u16.to_le_bytes()), Error::BadPhentsize),
        (patch(&img, 54, &64u16.to_le_bytes()), Error::BadPhentsize),
        (patch(&img, 56, &PN_XNUM.to_le_bytes()), Error::TooManyProgramHeaders),
        (patch(&img, 56, &0u16.to_le_bytes()), Error::NoLoadableSegments),
        (patch(&img, 56, &200u16.to_le_bytes()), Error::Truncated),
        (patch(&img, 32, &u64::MAX.to_le_bytes()), Error::Truncated),
        (patch(&img, 32, &(u64::MAX - 100).to_le_bytes()), Error::Truncated),
        (patch(&img, 32, &(full.len() as u64 - 10).to_le_bytes()), Error::Truncated),
        (patch(&img, 24, &0u64.to_le_bytes()), Error::BadEntry),
        (patch(&img, 24, &0x402000u64.to_le_bytes()), Error::BadEntry),
        (patch(&img, 24, &u64::MAX.to_le_bytes()), Error::BadEntry),
    ];
    for (i, (bytes, want)) in cases.iter().enumerate() {
        assert_eq!(parse_bytes(bytes), Err(*want), "case {i}");
    }
}

#[test]
fn bad_segments_are_refused() {
    let e = static_exec;
    let cases: Vec<(Image, Error)> = vec![
        (with(e(), |i| i.phdrs.retain(|p| p.p_type != PT_LOAD)), Error::NoLoadableSegments),
        (
            with(e(), |i| {
                for k in 0..MAX_LOAD_SEGMENTS as u64 - 2 {
                    i.phdrs.push(ph(PT_LOAD, R, 0, 0x50_0000 + k * 0x1000, 0, 0x10, 0x1000));
                }
            }),
            Error::TooManyLoadSegments,
        ),
        (with(e(), |i| i.phdrs[1].filesz = 0x124), Error::FileSizeExceedsMemSize),
        (with(e(), |i| i.phdrs[1].align = 3), Error::BadAlignment),
        (with(e(), |i| i.phdrs[1].offset = 0x1001), Error::MisalignedSegment),
        (
            with(e(), |i| {
                i.phdrs[1].align = 0x1_0000;
                i.phdrs[1].vaddr = 0x402000;
            }),
            Error::MisalignedSegment,
        ),
        (with(e(), |i| i.phdrs[2].filesz = 0x3000), Error::Truncated),
        (with(e(), |i| i.phdrs[1].memsz = 0x2_0000), Error::SegmentsOverlap),
        (with(e(), |i| i.phdrs[1].offset = 0xffff_ffff_ffff_f000), Error::Truncated),
        (
            with(e(), |i| i.phdrs[1] = ph(PT_LOAD, RX, 0xffff_ffff_ffff_f000, 0x401000, 0x1123, 0x1123, 0)),
            Error::AddressOverflow,
        ),
        (
            with(e(), |i| {
                i.phdrs[2] = ph(PT_LOAD, RW, 0x10, 0xffff_ffff_ffff_f010, 0, 0x2000, 0x1000);
            }),
            Error::AddressOverflow,
        ),
        (
            with(e(), |i| {
                i.phdrs[2] = ph(PT_LOAD, RW, 0x10, 0xffff_ffff_ffff_f010, 0, 0xfe0, 0x1000);
            }),
            Error::AddressOverflow,
        ),
        (with(e(), |i| i.phdrs[2] = ph(PT_LOAD, RW, 0x10, 0x401010, 0x10, 0x10, 0x1000)), Error::SegmentsOverlap),
        // Sharing a page is overlap too, even without sharing bytes.
        (with(e(), |i| i.phdrs[2] = ph(PT_LOAD, RW, 0xff0, 0x401ff0, 0, 0x20, 0x1000)), Error::SegmentsOverlap),
        (with(e(), |i| i.phdrs.swap(0, 1)), Error::SegmentsUnordered),
        (with(e(), |i| i.phdrs.push(ph(PT_TLS, R, 0, 0x400000, 0, 0, 8))), Error::DuplicateSegment),
        (with(e(), |i| i.phdrs.push(ph(PT_GNU_STACK, RW, 0, 0, 0, 0, 16))), Error::DuplicateSegment),
        (with(e(), |i| i.phdrs[3].align = 24), Error::BadTls),
        (with(e(), |i| i.phdrs[3].filesz = 0x21), Error::FileSizeExceedsMemSize),
        (with(e(), |i| (i.phdrs[3].filesz, i.phdrs[3].memsz) = (0x101, 0x200)), Error::BadTls),
        (with(e(), |i| i.phdrs[3].vaddr = 0x402000), Error::BadTls),
        (with(e(), |i| i.phdrs.push(ph(PT_PHDR, R, 64, 0x500000, 6 * 56, 6 * 56, 8))), Error::BadPhdr),
        (with(e(), |i| i.phdrs.push(ph(PT_PHDR, R, 64, 0x400040, 56, 56, 8))), Error::BadPhdr),
        (with(e(), |i| i.phdrs.push(ph(PT_PHDR, R, 64, 0x4001f0, 6 * 56, 6 * 56, 8))), Error::BadPhdr),
    ];
    for (n, (img, want)) in cases.iter().enumerate() {
        assert_eq!(parse(img), Err(*want), "case {n}");
    }
}

#[test]
fn duplicate_dynamic_segments_are_refused() {
    let dynamic = with(static_pie(), |i| i.phdrs.push(ph(PT_DYNAMIC, RW, 0x1f00, 0x2f00, 0x10, 0x10, 8)));
    assert_eq!(parse(&dynamic), Err(Error::DuplicateSegment));
    let phdr = with(static_pie(), |i| i.phdrs.push(i.phdrs[0]));
    assert_eq!(parse(&phdr), Err(Error::DuplicateSegment));
    let interp = with(dynamic_pie(), |i| i.phdrs.push(i.phdrs[1]));
    assert_eq!(parse(&interp), Err(Error::DuplicateSegment));
}

#[test]
fn bad_interpreter_paths_are_refused() {
    let at = |path: &[u8], filesz: u64| {
        with(dynamic_pie(), |i| {
            i.patches[0].1 = path.to_vec();
            i.phdrs[1].filesz = filesz;
            i.phdrs[1].memsz = filesz;
        })
    };
    assert_eq!(parse(&at(b"/lib/ld.so\0", 11)), Ok(()));
    assert_eq!(parse(&at(b"/lib/ld.so!", 11)), Err(Error::BadInterp));
    assert_eq!(parse(&at(b"/lib\0ld.so\0", 11)), Err(Error::BadInterp));
    assert_eq!(parse(&at(b"\0", 1)), Err(Error::BadInterp));
    assert_eq!(parse(&at(b"", 0)), Err(Error::BadInterp));
    assert_eq!(parse(&at(b"", MAX_INTERP_LEN as u64 + 1)), Err(Error::BadInterp));
    let past_end = with(at(b"/x\0", 3), |i| {
        i.phdrs[1].offset = 0x1ff0;
        i.phdrs[1].filesz = 0x20;
    });
    assert_eq!(parse(&past_end), Err(Error::Truncated));
    let far = with(at(b"/x\0", 3), |i| i.phdrs[1].offset = u64::MAX - 1);
    assert_eq!(parse(&far), Err(Error::Truncated));
}

#[test]
fn mappings_must_stay_in_user_space() {
    let shift_all = |img: Image, by: i64| {
        with(img, |i| {
            for p in i.phdrs.iter_mut().filter(|p| p.p_type == PT_LOAD || p.p_type == PT_TLS) {
                p.vaddr = p.vaddr.wrapping_add_signed(by);
            }
            i.entry = i.entry.wrapping_add_signed(by);
        })
    };
    // 0x10000 is the lowest allowed page.
    assert_eq!(parse(&shift_all(static_exec(), -0x3f_0000)), Ok(()));
    assert_eq!(parse(&shift_all(static_exec(), -0x3f_1000)), Err(Error::OutsideUserSpace));

    let top = shift_all(static_exec(), (USER_END - 0x407000) as i64);
    assert_eq!(parse(&top), Ok(()));
    let over = with(top, |i| i.phdrs[2].memsz += 0x1000);
    assert_eq!(parse(&over), Err(Error::OutsideUserSpace));
    let kernel = shift_all(static_exec(), 0xffff_8000_0000_0000u64 as i64);
    assert_eq!(parse(&kernel), Err(Error::OutsideUserSpace));

    let bytes = static_pie().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    let high = LoadOptions { dyn_base: USER_END - 0x3000, ..LoadOptions::default() };
    assert_eq!(LoadPlan::new(&elf, &high).err(), Some(Error::OutsideUserSpace));
    let wrap = LoadOptions { dyn_base: u64::MAX - 0x10, ..LoadOptions::default() };
    assert_eq!(LoadPlan::new(&elf, &wrap).err(), Some(Error::AddressOverflow));
    let huge_align = with(static_pie(), |i| i.phdrs[1].align = 1 << 63);
    let bytes = huge_align.bytes();
    let elf = Elf::parse(&bytes).unwrap();
    assert_eq!(LoadPlan::new(&elf, &LoadOptions::default()).err(), Some(Error::OutsideUserSpace));
}

/// Every proper prefix of every fixture is refused, without panicking.
#[test]
fn truncation_never_panics() {
    for img in [static_exec(), static_pie(), dynamic_pie()] {
        let bytes = img.bytes();
        for n in 0..bytes.len() {
            assert!(parse_bytes(&bytes[..n]).is_err(), "prefix of {n} bytes was accepted");
        }
        assert_eq!(parse_bytes(&bytes), Ok(()));
    }
}

/// A small deterministic generator (xorshift64*), so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Values that tend to break arithmetic.
const NASTY: [u64; 12] = [
    0,
    1,
    7,
    0xfff,
    0x1000,
    0x1001,
    0x7fff_ffff_f000,
    0x8000_0000_0000,
    1 << 63,
    u64::MAX - 0xfff,
    u64::MAX - 1,
    u64::MAX,
];

/// Parses and plans `bytes`, checks the plan's invariants and reads the
/// first and last page of every mapping. Returns whether a plan was made.
fn exercise(bytes: &[u8]) -> bool {
    let Ok(elf) = Elf::parse(bytes) else { return false };
    let _ = elf.phdr_vaddr();
    let _ = elf.program_headers().count();
    let Ok(plan) = LoadPlan::new(&elf, &LoadOptions::default()) else { return false };
    let mut page = vec![0u8; PAGE_SIZE as usize];
    let mut prev_end = 0;
    for m in plan.mappings() {
        assert!(m.start < m.end && m.start % PAGE_SIZE == 0 && m.end % PAGE_SIZE == 0);
        assert!(m.start >= prev_end, "mappings overlap or are unsorted");
        assert!(m.start >= DEFAULT_MIN_ADDR && m.end <= USER_END);
        prev_end = m.end;
        let _ = m.file_bytes(bytes);
        assert!(m.fill_page(bytes, m.start, &mut page));
        assert!(m.fill_page(bytes, m.end - PAGE_SIZE, &mut page));
    }
    assert!(plan.mapping_at(plan.entry()).is_some());
    true
}

#[test]
fn mutated_headers_never_panic() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let fixtures = [static_exec().bytes(), static_pie().bytes(), dynamic_pie().bytes()];
    let mut planned = 0;
    for round in 0..30_000 {
        let mut b = fixtures[round % fixtures.len()].clone();
        let phnum = usize::from(u16::from_le_bytes([b[56], b[57]]));
        let headers = EHDR_SIZE + phnum * PHDR_SIZE;
        for _ in 0..1 + rng.below(4) {
            if rng.below(2) == 0 {
                // An 8-byte field of the header or a program header gets an edge-case value.
                let off = rng.below(headers / 8) * 8;
                let v = NASTY[rng.below(NASTY.len())].wrapping_add(rng.below(3) as u64);
                b[off..off + 8].copy_from_slice(&v.to_le_bytes());
            } else {
                let off = rng.below(headers);
                b[off] = rng.next() as u8;
            }
        }
        if rng.below(8) == 0 {
            b.truncate(rng.below(b.len()));
        }
        planned += usize::from(exercise(&b));
    }
    // Mutations of fields a loader ignores (p_paddr, PT_NOTE contents...)
    // are accepted, so a fair share of the rounds reach the planner.
    assert!(planned > 3000, "only {planned} mutated images were planned");
}

// ---- Initial stack ----

const TOP: u64 = 0x7fff_ffff_e000;
const RANDOM: [u8; 16] = *b"0123456789abcdef";

fn aux_for_test() -> AuxInfo {
    let bytes = static_exec().bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    let creds = Credentials { uid: 1000, euid: 1000, gid: 100, egid: 100 };
    AuxInfo { hwcap: 0x178b_fbff, ..AuxInfo::new(&plan, None, creds) }
}

fn params<'a>(argv: &'a [&'a [u8]], envp: &'a [&'a [u8]]) -> StackParams<'a> {
    StackParams {
        argv,
        envp,
        execfn: b"/bin/hello",
        platform: PLATFORM_X86_64,
        random: &RANDOM,
        aux: aux_for_test(),
        phdr_table: &[],
        extra_aux: &[],
    }
}

#[test]
fn stack_round_trip() {
    let argv: &[&[u8]] = &[b"/bin/hello", b"-v", b"", b"world"];
    let envp: &[&[u8]] = &[b"HOME=/root", b"PATH=/bin:/usr/bin", b"TERM=linux"];
    let mut buf = vec![0xaau8; 4096];
    let p = params(argv, envp);
    let l = build_initial_stack(&mut buf, TOP, &p).unwrap();
    assert_eq!(l.rsp % 16, 0);
    assert_eq!((l.argc, l.argv, l.envp), (4, l.rsp + 8, l.rsp + 8 + 5 * 8));
    assert_eq!(l.auxv, l.envp + 4 * 8);

    let s = InitialStack::parse(&buf, TOP, l.rsp).unwrap();
    assert_eq!((s.argc(), s.envc()), (4, 3));
    for (i, a) in argv.iter().enumerate() {
        assert_eq!(s.arg(i as u64), Some(*a));
    }
    for (i, e) in envp.iter().enumerate() {
        assert_eq!(s.env(i as u64), Some(*e));
    }
    assert_eq!((s.arg(4), s.env(3)), (None, None));
    assert_eq!((s.envp(), s.auxv_addr()), (l.envp, l.auxv));

    let want = [
        (AT_HWCAP, 0x178b_fbff),
        (AT_PAGESZ, 4096),
        (AT_CLKTCK, 100),
        (AT_PHDR, 0x400040),
        (AT_PHENT, 56),
        (AT_PHNUM, 5),
        (AT_BASE, 0),
        (AT_FLAGS, 0),
        (AT_ENTRY, 0x401000),
        (AT_UID, 1000),
        (AT_EUID, 1000),
        (AT_GID, 100),
        (AT_EGID, 100),
        (AT_SECURE, 0),
        (AT_RANDOM, l.random),
        (AT_EXECFN, l.execfn),
    ];
    let got: Vec<_> = s.auxv().collect();
    assert_eq!(&got[..want.len()], &want);
    assert_eq!(got[want.len()].0, AT_PLATFORM);
    assert_eq!(got.len(), want.len() + 1);
    assert_eq!(l.auxc as usize, got.len() + 1);
    assert_eq!((s.aux(AT_SYSINFO_EHDR), s.aux(AT_HWCAP2)), (None, None));
    assert_eq!(l.phdr, 0x400040);

    // What the entries point to.
    assert_eq!(l.random % 16, 0);
    let ri = (l.random - (TOP - 4096)) as usize;
    assert_eq!(&buf[ri..ri + 16], &RANDOM);
    assert_eq!(s.c_str(s.aux(AT_EXECFN).unwrap()), Some(&b"/bin/hello"[..]));
    assert_eq!(s.c_str(s.aux(AT_PLATFORM).unwrap()), Some(&b"x86_64"[..]));
    // Strings ascend: argv, then envp, then the file name, then 8 zero bytes.
    let ptrs: Vec<u64> = (0..4).map(|i| l.argv + 8 * i).chain((0..3).map(|i| l.envp + 8 * i)).collect();
    let strs: Vec<u64> = ptrs.iter().map(|&a| s.word(a).unwrap()).collect();
    assert!(strs.windows(2).all(|w| w[0] < w[1]));
    assert!(l.execfn > strs[6]);
    assert_eq!(&buf[4096 - 8..], &[0; 8]);
    // Nothing below rsp was touched.
    let used = l.used(TOP) as usize;
    assert!(buf[..4096 - used].iter().all(|&b| b == 0xaa));
}

#[test]
fn stack_is_aligned_for_every_parity() {
    let words: [&[u8]; 6] = [b"a", b"bb", b"ccc", b"dddd", b"eeeee", b"ffffffffffffffff"];
    let extras = [(AT_MINSIGSTKSZ, 2048), (AT_IGNORE, 0)];
    for top in [TOP, TOP - 8, TOP - 3, TOP + 1] {
        for argc in 0..=6 {
            for envc in 0..=6 {
                for extra in 0..3 {
                    let mut p = params(&words[..argc], &words[6 - envc..]);
                    p.extra_aux = &extras[..extra];
                    if extra == 2 {
                        p.aux.hwcap2 = Some(2);
                        p.aux.sysinfo_ehdr = Some(0x7fff_f7fc_1000);
                    }
                    let mut buf = vec![0u8; 2048];
                    let l = build_initial_stack(&mut buf, top, &p).unwrap();
                    assert_eq!(l.rsp % 16, 0, "argc {argc} envc {envc} top {top:#x}");
                    let s = InitialStack::parse(&buf, top, l.rsp).unwrap();
                    assert_eq!((s.argc(), s.envc()), (argc as u64, envc as u64));
                    for (i, w) in words[..argc].iter().enumerate() {
                        assert_eq!(s.arg(i as u64), Some(*w));
                    }
                    for (i, w) in words[6 - envc..].iter().enumerate() {
                        assert_eq!(s.env(i as u64), Some(*w));
                    }
                    assert_eq!(s.aux(AT_RANDOM), Some(l.random));
                    assert_eq!(s.aux(AT_MINSIGSTKSZ), (extra >= 1).then_some(2048));
                    if extra == 2 {
                        assert_eq!(s.aux(AT_HWCAP2), Some(2));
                        assert_eq!(s.auxv().next(), Some((AT_SYSINFO_EHDR, 0x7fff_f7fc_1000)));
                    }
                }
            }
        }
    }
}

#[test]
fn stack_too_small_is_an_error_at_every_size() {
    let argv: &[&[u8]] = &[b"prog", b"arg"];
    let envp: &[&[u8]] = &[b"A=1"];
    let p = params(argv, envp);
    let mut big = vec![0u8; 4096];
    let need = build_initial_stack(&mut big, TOP, &p).unwrap().used(TOP) as usize;
    for len in 0..need {
        let mut buf = vec![0u8; len];
        assert_eq!(build_initial_stack(&mut buf, TOP, &p), Err(StackError::TooSmall), "len {len}");
    }
    let mut exact = vec![0u8; need];
    let l = build_initial_stack(&mut exact, TOP, &p).unwrap();
    assert_eq!(l.rsp, TOP - need as u64);
    assert_eq!(&exact[..], &big[4096 - need..]);
    let mut buf = vec![0u8; 64];
    assert_eq!(build_initial_stack(&mut buf, 16, &p), Err(StackError::AddressOverflow));
}

#[test]
fn stack_rejects_bad_input() {
    let mut buf = vec![0u8; 4096];
    let nul: &[&[u8]] = &[b"a\0b"];
    assert_eq!(build_initial_stack(&mut buf, TOP, &params(nul, &[])), Err(StackError::InteriorNul));
    assert_eq!(build_initial_stack(&mut buf, TOP, &params(&[], nul)), Err(StackError::InteriorNul));
    let p = StackParams { execfn: b"/x\0", ..params(&[], &[]) };
    assert_eq!(build_initial_stack(&mut buf, TOP, &p), Err(StackError::InteriorNul));
    let p = StackParams { extra_aux: &[(AT_NULL, 1)], ..params(&[], &[]) };
    assert_eq!(build_initial_stack(&mut buf, TOP, &p), Err(StackError::BadAuxEntry));
    let mut p = params(&[], &[]);
    p.aux.phdr = None;
    assert_eq!(build_initial_stack(&mut buf, TOP, &p), Err(StackError::MissingPhdr));
    p.phdr_table = &[0; 57];
    assert_eq!(build_initial_stack(&mut buf, TOP, &p), Err(StackError::MissingPhdr));
}

#[test]
fn stack_copies_unmapped_program_headers() {
    let bytes = static_exec().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    let mut p = params(&[], &[]);
    p.aux.phdr = None;
    p.phdr_table = elf.program_header_table();
    p.platform = b"";
    let mut buf = vec![0u8; 4096];
    let l = build_initial_stack(&mut buf, TOP, &p).unwrap();
    let s = InitialStack::parse(&buf, TOP, l.rsp).unwrap();
    assert_eq!(s.argc(), 0);
    assert_eq!(s.aux(AT_PHDR), Some(l.phdr));
    assert_eq!(l.phdr % 8, 0);
    let at = (l.phdr - (TOP - 4096)) as usize;
    assert_eq!(&buf[at..at + 5 * 56], elf.program_header_table());
    assert_eq!(s.aux(AT_PLATFORM), None);
}

#[test]
fn secure_follows_credentials() {
    let bytes = static_exec().bytes();
    let plan = LoadPlan::new(&Elf::parse(&bytes).unwrap(), &LoadOptions::default()).unwrap();
    let setuid = Credentials { uid: 1000, euid: 0, gid: 100, egid: 100 };
    let aux = AuxInfo::new(&plan, None, setuid);
    assert!(aux.secure);
    let mut buf = vec![0u8; 1024];
    let l = build_initial_stack(&mut buf, TOP, &StackParams { aux, ..params(&[], &[]) }).unwrap();
    let s = InitialStack::parse(&buf, TOP, l.rsp).unwrap();
    assert_eq!((s.aux(AT_SECURE), s.aux(AT_EUID)), (Some(1), Some(0)));
    assert!(!AuxInfo::new(&plan, None, Credentials::default()).secure);
}

#[test]
fn stack_reader_survives_garbage() {
    let mut rng = Rng(42);
    for _ in 0..3000 {
        let len = rng.below(512);
        let mut buf: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // Zero words make the walk go further before it fails.
        for chunk in buf.chunks_mut(8) {
            if rng.below(3) == 0 {
                chunk.fill(0);
            }
        }
        let top = [TOP, u64::MAX, len as u64][rng.below(3)];
        let rsp = top.wrapping_sub(rng.below(len + 16) as u64);
        if let Some(s) = InitialStack::parse(&buf, top, rsp) {
            for i in 0..s.argc().min(64) {
                let _ = s.arg(i);
            }
            for i in 0..s.envc() {
                let _ = s.env(i);
            }
            let _ = s.auxv().count();
        }
    }
}

#[test]
fn initial_stack_size_is_a_tight_upper_bound() {
    let words: [&[u8]; 6] = [b"a", b"bb", b"", b"dddd", b"eeeee", b"ffffffffffffffffffffffff"];
    let extras = [(AT_MINSIGSTKSZ, 2048), (AT_IGNORE, 0)];
    let bytes = static_exec().bytes();
    let elf = Elf::parse(&bytes).unwrap();
    for top in [TOP, TOP - 8, TOP - 3, TOP + 1] {
        for argc in 0..=6 {
            for envc in 0..=6 {
                for variant in 0..4 {
                    let mut p = params(&words[..argc], &words[6 - envc..]);
                    p.extra_aux = &extras[..variant.min(2)];
                    if variant >= 2 {
                        p.aux.hwcap2 = Some(2);
                        p.aux.sysinfo_ehdr = Some(0x7fff_f7fc_1000);
                        p.platform = b"";
                    }
                    if variant == 3 {
                        p.aux.phdr = None;
                        p.phdr_table = elf.program_header_table();
                    }
                    let size = initial_stack_size(&p).unwrap();
                    let mut buf = vec![0u8; size];
                    let l = build_initial_stack(&mut buf, top, &p).unwrap();
                    let used = l.used(top) as usize;
                    assert!(used <= size && size - used <= 37, "used {used}, bound {size}");
                }
            }
        }
    }
    // The bound grows with every byte of every string.
    let p = params(&[b"x"], &[]);
    let longer = params(&[b"xy"], &[]);
    assert_eq!(initial_stack_size(&longer), initial_stack_size(&p).map(|n| n + 1));
}
