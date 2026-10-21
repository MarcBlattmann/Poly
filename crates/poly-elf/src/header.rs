//! The ELF file header and program header table.
//!
//! Layouts and constants are those of the System V gABI, chapters "ELF
//! Header" and "Program Header", with the numeric values from the public
//! `<elf.h>` header. `PT_GNU_STACK` is the GNU extension listed in the same
//! header.

use crate::{PAGE_SIZE, align_down};

/// `EI_CLASS` value of a 64-bit object (`<elf.h>` `ELFCLASS64`).
pub const ELFCLASS64: u8 = 2;
/// `EI_DATA` value of a little-endian object (`<elf.h>` `ELFDATA2LSB`).
pub const ELFDATA2LSB: u8 = 1;
/// The only ELF version (`<elf.h>` `EV_CURRENT`), in `EI_VERSION` and `e_version`.
pub const EV_CURRENT: u8 = 1;
/// `EI_OSABI` of a plain System V object (`<elf.h>` `ELFOSABI_SYSV`).
pub const ELFOSABI_SYSV: u8 = 0;
/// `EI_OSABI` of an object using GNU extensions such as `STT_GNU_IFUNC`
/// (`<elf.h>` `ELFOSABI_GNU`, alias `ELFOSABI_LINUX`); static glibc binaries carry it.
pub const ELFOSABI_GNU: u8 = 3;
/// `e_type` of an executable (`<elf.h>` `ET_EXEC`).
pub const ET_EXEC: u16 = 2;
/// `e_type` of a shared object, which includes position-independent executables (`<elf.h>` `ET_DYN`).
pub const ET_DYN: u16 = 3;
/// `e_machine` of x86-64 (`<elf.h>` `EM_X86_64`).
pub const EM_X86_64: u16 = 62;
/// `e_phnum` value meaning "the real count is in section header 0" (`<elf.h>` `PN_XNUM`).
pub const PN_XNUM: u16 = 0xffff;

/// Unused program header (`<elf.h>` `PT_NULL`).
pub const PT_NULL: u32 = 0;
/// Loadable segment (`<elf.h>` `PT_LOAD`).
pub const PT_LOAD: u32 = 1;
/// Dynamic linking information (`<elf.h>` `PT_DYNAMIC`).
pub const PT_DYNAMIC: u32 = 2;
/// Path of the program interpreter (`<elf.h>` `PT_INTERP`).
pub const PT_INTERP: u32 = 3;
/// Auxiliary notes (`<elf.h>` `PT_NOTE`).
pub const PT_NOTE: u32 = 4;
/// The program header table itself (`<elf.h>` `PT_PHDR`).
pub const PT_PHDR: u32 = 6;
/// Thread-local storage template (`<elf.h>` `PT_TLS`).
pub const PT_TLS: u32 = 7;
/// Stack executability (`<elf.h>` `PT_GNU_STACK`).
pub const PT_GNU_STACK: u32 = 0x6474_e551;
/// Read-only after relocation (`<elf.h>` `PT_GNU_RELRO`).
pub const PT_GNU_RELRO: u32 = 0x6474_e552;

/// Segment is executable (`<elf.h>` `PF_X`).
pub const PF_X: u32 = 1 << 0;
/// Segment is writable (`<elf.h>` `PF_W`).
pub const PF_W: u32 = 1 << 1;
/// Segment is readable (`<elf.h>` `PF_R`).
pub const PF_R: u32 = 1 << 2;

/// Size of the ELF64 file header (`Elf64_Ehdr`).
pub const EHDR_SIZE: usize = 64;
/// Size of one ELF64 program header (`Elf64_Phdr`); also the value of `AT_PHENT`.
pub const PHDR_SIZE: usize = 56;
/// Longest interpreter path accepted, terminating NUL included (`PATH_MAX`
/// from `<limits.h>`).
pub const MAX_INTERP_LEN: usize = 4096;
/// Most `PT_LOAD` segments an image may have. Linkers emit two to five; the
/// limit keeps [`LoadPlan`](crate::LoadPlan) a fixed-size value.
pub const MAX_LOAD_SEGMENTS: usize = 16;

/// Why a file is not a loadable x86-64 executable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The file does not start with `\x7fELF`.
    NotElf,
    /// The file ends before the header, the program header table or a
    /// segment's contents.
    Truncated,
    /// Not a 64-bit object (`EI_CLASS`).
    UnsupportedClass,
    /// Not little-endian (`EI_DATA`).
    UnsupportedEncoding,
    /// `EI_VERSION` or `e_version` is not `EV_CURRENT`.
    UnsupportedVersion,
    /// `EI_OSABI` is neither System V nor GNU/Linux.
    UnsupportedOsAbi,
    /// `e_machine` is not x86-64.
    UnsupportedMachine,
    /// `e_type` is neither `ET_EXEC` nor `ET_DYN` (relocatable objects and
    /// core files do not run).
    UnsupportedType,
    /// `e_ehsize` is smaller than the ELF64 header.
    BadHeaderSize,
    /// `e_phentsize` is not the size of an ELF64 program header.
    BadPhentsize,
    /// `e_phnum` is `PN_XNUM` (the count would be in section header 0; no
    /// executable needs that many program headers).
    TooManyProgramHeaders,
    /// No `PT_LOAD` segment.
    NoLoadableSegments,
    /// More than [`MAX_LOAD_SEGMENTS`] `PT_LOAD` segments.
    TooManyLoadSegments,
    /// A segment's `p_filesz` is larger than its `p_memsz`.
    FileSizeExceedsMemSize,
    /// `p_align` is neither 0, 1 nor a power of two.
    BadAlignment,
    /// A `PT_LOAD` segment's `p_vaddr` and `p_offset` differ modulo the page
    /// size or modulo `p_align`, so it cannot be mapped page by page.
    MisalignedSegment,
    /// An address or offset computation wraps around 2^64.
    AddressOverflow,
    /// Two `PT_LOAD` segments share a page.
    SegmentsOverlap,
    /// `PT_LOAD` segments are not sorted by `p_vaddr`, as the gABI requires.
    SegmentsUnordered,
    /// A mapping would lie below the lowest user address or above the end of
    /// user space.
    OutsideUserSpace,
    /// `PT_INTERP` is empty, too long, not NUL-terminated or contains a NUL.
    BadInterp,
    /// A segment type that may occur only once occurs twice (`PT_INTERP`,
    /// `PT_PHDR`, `PT_TLS`, `PT_DYNAMIC`, `PT_GNU_STACK`).
    DuplicateSegment,
    /// `PT_TLS` has a bad alignment or its initialised part is not inside a
    /// loaded segment.
    BadTls,
    /// `PT_PHDR` does not describe the whole table inside a loaded segment.
    BadPhdr,
    /// `e_entry` is not inside a loaded segment.
    BadEntry,
    /// A program interpreter names an interpreter itself.
    NestedInterpreter,
    /// `PT_DYNAMIC` or a table it points to is malformed (`dynamic`).
    BadDynamic,
}

/// `e_type` of an image that can run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ElfType {
    /// `ET_EXEC`: linked at fixed addresses.
    Exec,
    /// `ET_DYN`: position independent; loaded at a chosen base (the load bias).
    Dyn,
}

/// One program header (`Elf64_Phdr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ProgramHeader {
    pub p_type: u32,
    pub flags: u32,
    pub offset: u64,
    pub vaddr: u64,
    pub paddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
}

impl ProgramHeader {
    /// Reads the program header at `off`; `None` if it does not fit.
    fn read(data: &[u8], off: usize) -> Option<ProgramHeader> {
        Some(ProgramHeader {
            p_type: u32_at(data, off)?,
            flags: u32_at(data, off.checked_add(4)?)?,
            offset: u64_at(data, off.checked_add(8)?)?,
            vaddr: u64_at(data, off.checked_add(16)?)?,
            paddr: u64_at(data, off.checked_add(24)?)?,
            filesz: u64_at(data, off.checked_add(32)?)?,
            memsz: u64_at(data, off.checked_add(40)?)?,
            align: u64_at(data, off.checked_add(48)?)?,
        })
    }

    /// End of the segment in memory (`p_vaddr + p_memsz`); validated not to wrap.
    pub const fn mem_end(&self) -> u64 {
        self.vaddr.wrapping_add(self.memsz)
    }

    /// Whether the memory image of this segment contains `[addr, addr + len)`.
    pub const fn contains(&self, addr: u64, len: u64) -> bool {
        match addr.checked_add(len) {
            Some(end) => addr >= self.vaddr && end <= self.mem_end(),
            None => false,
        }
    }
}

/// The thread-local storage template (`PT_TLS`), at its link-time address.
///
/// The first `filesz` bytes at `vaddr` initialise each thread's block; the
/// rest up to `memsz` is zero. A static binary's C library reads this through
/// `AT_PHDR` itself, so the kernel only needs it for diagnostics today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tls {
    pub vaddr: u64,
    pub filesz: u64,
    pub memsz: u64,
    pub align: u64,
}

/// A validated ELF64 x86-64 executable.
///
/// Everything [`Elf::parse`] accepted is safe to use without further checks:
/// the program header table and the contents of every `PT_LOAD` and
/// `PT_INTERP` segment lie inside the file, sizes and addresses do not wrap,
/// and the `PT_LOAD` segments are sorted and do not share pages.
#[derive(Clone, Copy, Debug)]
pub struct Elf<'a> {
    data: &'a [u8],
    elf_type: ElfType,
    os_abi: u8,
    entry: u64,
    phoff: usize,
    phnum: u16,
    load_count: usize,
    interp: Option<&'a [u8]>,
    phdr: Option<ProgramHeader>,
    dynamic: Option<ProgramHeader>,
    tls: Option<Tls>,
    gnu_stack: Option<u32>,
}

impl<'a> Elf<'a> {
    /// Validates `data`, the whole executable file.
    pub fn parse(data: &'a [u8]) -> Result<Elf<'a>, Error> {
        Elf::parse_prefix(data, data.len() as u64)
    }

    /// Validates an executable file of `file_len` bytes from `head`, its
    /// first bytes: the header, the program header table and the
    /// `PT_INTERP` path must lie in `head` ([`Error::Truncated`] if not:
    /// read more), the `PT_LOAD` segments' contents only inside the file.
    /// What a loader needs, without reading the whole file;
    /// [`data`](Self::data) is then `head`.
    pub fn parse_prefix(data: &'a [u8], file_len: u64) -> Result<Elf<'a>, Error> {
        if data.len() as u64 > file_len {
            return Err(Error::Truncated);
        }
        if data.get(..4) != Some(b"\x7fELF") {
            return Err(Error::NotElf);
        }
        if data.len() < EHDR_SIZE {
            return Err(Error::Truncated);
        }
        // e_ident (gABI "ELF Identification").
        if data[4] != ELFCLASS64 {
            return Err(Error::UnsupportedClass);
        }
        if data[5] != ELFDATA2LSB {
            return Err(Error::UnsupportedEncoding);
        }
        if data[6] != EV_CURRENT {
            return Err(Error::UnsupportedVersion);
        }
        let os_abi = data[7];
        if os_abi != ELFOSABI_SYSV && os_abi != ELFOSABI_GNU {
            return Err(Error::UnsupportedOsAbi);
        }
        // The fixed-size fields; the header is known to be 64 bytes long.
        let field16 = |off| u16_at(data, off).ok_or(Error::Truncated);
        let field32 = |off| u32_at(data, off).ok_or(Error::Truncated);
        let field64 = |off| u64_at(data, off).ok_or(Error::Truncated);
        let elf_type = match field16(16)? {
            ET_EXEC => ElfType::Exec,
            ET_DYN => ElfType::Dyn,
            _ => return Err(Error::UnsupportedType),
        };
        if field16(18)? != EM_X86_64 {
            return Err(Error::UnsupportedMachine);
        }
        if field32(20)? != u32::from(EV_CURRENT) {
            return Err(Error::UnsupportedVersion);
        }
        let entry = field64(24)?;
        let phoff = field64(32)?;
        if usize::from(field16(52)?) < EHDR_SIZE {
            return Err(Error::BadHeaderSize);
        }
        if usize::from(field16(54)?) != PHDR_SIZE {
            return Err(Error::BadPhentsize);
        }
        let phnum = field16(56)?;
        if phnum == PN_XNUM {
            return Err(Error::TooManyProgramHeaders);
        }
        // The table must lie inside the file.
        let phoff = usize::try_from(phoff).map_err(|_| Error::Truncated)?;
        let table_len = usize::from(phnum) * PHDR_SIZE;
        let table_end = phoff.checked_add(table_len).ok_or(Error::Truncated)?;
        if table_end > data.len() {
            return Err(Error::Truncated);
        }

        let mut elf = Elf {
            data,
            elf_type,
            os_abi,
            entry,
            phoff,
            phnum,
            load_count: 0,
            interp: None,
            phdr: None,
            dynamic: None,
            tls: None,
            gnu_stack: None,
        };
        let mut prev_load: Option<ProgramHeader> = None;
        for ph in elf.program_headers() {
            match ph.p_type {
                PT_LOAD => {
                    check_load(file_len, &ph, prev_load.as_ref())?;
                    elf.load_count += 1;
                    if elf.load_count > MAX_LOAD_SEGMENTS {
                        return Err(Error::TooManyLoadSegments);
                    }
                    if ph.memsz != 0 {
                        prev_load = Some(ph);
                    }
                }
                PT_INTERP => set_once(&mut elf.interp, interp_path(data, &ph)?)?,
                PT_PHDR => set_once(&mut elf.phdr, ph)?,
                PT_DYNAMIC => set_once(&mut elf.dynamic, ph)?,
                PT_TLS => {
                    if !valid_align(ph.align) {
                        return Err(Error::BadTls);
                    }
                    if ph.filesz > ph.memsz {
                        return Err(Error::FileSizeExceedsMemSize);
                    }
                    let tls = Tls { vaddr: ph.vaddr, filesz: ph.filesz, memsz: ph.memsz, align: ph.align };
                    set_once(&mut elf.tls, tls)?;
                }
                PT_GNU_STACK => set_once(&mut elf.gnu_stack, ph.flags)?,
                _ => {}
            }
        }
        if prev_load.is_none() {
            return Err(Error::NoLoadableSegments);
        }

        // Checks that need every PT_LOAD segment.
        if !elf.in_memory(entry, 1) {
            return Err(Error::BadEntry);
        }
        if let Some(phdr) = elf.phdr
            && (phdr.memsz < table_len as u64 || !elf.in_memory(phdr.vaddr, table_len as u64))
        {
            return Err(Error::BadPhdr);
        }
        if let Some(tls) = elf.tls
            && tls.filesz != 0
            && !elf.in_file_image(tls.vaddr, tls.filesz)
        {
            return Err(Error::BadTls);
        }
        Ok(elf)
    }

    /// The whole file.
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// `ET_EXEC` or `ET_DYN`.
    pub const fn elf_type(&self) -> ElfType {
        self.elf_type
    }

    /// `EI_OSABI`: [`ELFOSABI_SYSV`] or [`ELFOSABI_GNU`].
    pub const fn os_abi(&self) -> u8 {
        self.os_abi
    }

    /// `e_entry`, at its link-time address (add the load bias for `ET_DYN`).
    pub const fn entry(&self) -> u64 {
        self.entry
    }

    /// Number of program headers (`e_phnum`, the value of `AT_PHNUM`).
    pub const fn phnum(&self) -> u16 {
        self.phnum
    }

    /// Number of `PT_LOAD` program headers.
    pub const fn load_count(&self) -> usize {
        self.load_count
    }

    /// The raw program header table, for a loader that has to place a copy
    /// of it in memory (see [`LoadPlan::phdr`](crate::LoadPlan::phdr)).
    pub fn program_header_table(&self) -> &'a [u8] {
        let len = usize::from(self.phnum) * PHDR_SIZE;
        self.data.get(self.phoff..self.phoff + len).unwrap_or(&[])
    }

    /// All program headers, in file order.
    pub fn program_headers(&self) -> ProgramHeaders<'a> {
        ProgramHeaders { table: self.program_header_table(), next: 0 }
    }

    /// The `PT_LOAD` program headers, in file (and address) order.
    pub fn load_segments(&self) -> impl Iterator<Item = ProgramHeader> + use<'a> {
        self.program_headers().filter(|ph| ph.p_type == PT_LOAD)
    }

    /// The interpreter path from `PT_INTERP`, without its NUL; `None` for a
    /// static executable.
    pub const fn interp(&self) -> Option<&'a [u8]> {
        self.interp
    }

    /// Whether the image runs without a program interpreter.
    pub const fn is_static(&self) -> bool {
        self.interp.is_none()
    }

    /// The `PT_PHDR` program header, if any.
    pub const fn phdr_segment(&self) -> Option<ProgramHeader> {
        self.phdr
    }

    /// The `PT_DYNAMIC` program header, if any. A static-PIE has one (its
    /// startup code relocates itself from it); a classic static executable
    /// does not.
    pub const fn dynamic(&self) -> Option<ProgramHeader> {
        self.dynamic
    }

    /// The `PT_TLS` template, if any.
    pub const fn tls(&self) -> Option<Tls> {
        self.tls
    }

    /// `p_flags` of `PT_GNU_STACK`, if present.
    pub const fn gnu_stack(&self) -> Option<u32> {
        self.gnu_stack
    }

    /// Link-time address of the program header table in the memory image:
    /// `PT_PHDR`'s address if present, else found through the `PT_LOAD`
    /// segment whose file contents include the table (static executables
    /// usually have no `PT_PHDR`, but their first segment starts at file
    /// offset 0 and so maps the table). `None` if no segment maps it.
    pub fn phdr_vaddr(&self) -> Option<u64> {
        if let Some(phdr) = self.phdr {
            return Some(phdr.vaddr);
        }
        let start = self.phoff as u64;
        let len = (usize::from(self.phnum) * PHDR_SIZE) as u64;
        let end = start.checked_add(len)?;
        self.load_segments()
            .find(|ph| start >= ph.offset && end <= ph.offset.saturating_add(ph.filesz))
            .map(|ph| ph.vaddr + (start - ph.offset))
    }

    /// Whether `[addr, addr + len)` lies inside one segment's memory image.
    fn in_memory(&self, addr: u64, len: u64) -> bool {
        self.load_segments().any(|ph| ph.contains(addr, len))
    }

    /// Whether `[addr, addr + len)` lies inside one segment's file-backed part.
    fn in_file_image(&self, addr: u64, len: u64) -> bool {
        self.load_segments().any(|ph| {
            let file_part = ProgramHeader { memsz: ph.filesz, ..ph };
            file_part.contains(addr, len)
        })
    }
}

/// Iterator over a validated program header table.
#[derive(Clone, Debug)]
pub struct ProgramHeaders<'a> {
    table: &'a [u8],
    next: usize,
}

impl Iterator for ProgramHeaders<'_> {
    type Item = ProgramHeader;

    fn next(&mut self) -> Option<ProgramHeader> {
        let ph = ProgramHeader::read(self.table, self.next.checked_mul(PHDR_SIZE)?)?;
        self.next += 1;
        Some(ph)
    }
}

/// Records a segment that may occur only once.
fn set_once<T>(slot: &mut Option<T>, value: T) -> Result<(), Error> {
    match slot.replace(value) {
        Some(_) => Err(Error::DuplicateSegment),
        None => Ok(()),
    }
}

/// gABI: "Values 0 and 1 mean no alignment is required. Otherwise, p_align
/// should be a positive, integral power of 2."
const fn valid_align(align: u64) -> bool {
    align <= 1 || align.is_power_of_two()
}

/// Validates one `PT_LOAD` segment against the file and the previous one.
fn check_load(file_len: u64, ph: &ProgramHeader, prev: Option<&ProgramHeader>) -> Result<(), Error> {
    if ph.filesz > ph.memsz {
        return Err(Error::FileSizeExceedsMemSize);
    }
    if !valid_align(ph.align) {
        return Err(Error::BadAlignment);
    }
    // gABI: "p_vaddr should equal p_offset, modulo p_align". Mapping by
    // pages needs the same congruence modulo the page size.
    if ph.vaddr % PAGE_SIZE != ph.offset % PAGE_SIZE || (ph.align > 1 && ph.vaddr % ph.align != ph.offset % ph.align) {
        return Err(Error::MisalignedSegment);
    }
    let file_end = ph.offset.checked_add(ph.filesz).ok_or(Error::AddressOverflow)?;
    if file_end > file_len {
        return Err(Error::Truncated);
    }
    // The end must leave room for rounding up to a page.
    let mem_end = ph.vaddr.checked_add(ph.memsz).ok_or(Error::AddressOverflow)?;
    crate::align_up(mem_end, PAGE_SIZE).ok_or(Error::AddressOverflow)?;
    if ph.memsz == 0 {
        return Ok(());
    }
    if let Some(prev) = prev {
        // gABI: "Loadable segment entries in the program header table appear
        // in ascending order, sorted on the p_vaddr member."
        if ph.vaddr < prev.vaddr {
            return Err(Error::SegmentsUnordered);
        }
        // Pages carry one protection each, so segments must not share one.
        // The previous end was checked to round up without overflow.
        if align_down(ph.vaddr, PAGE_SIZE) < crate::align_up(prev.mem_end(), PAGE_SIZE).unwrap_or(u64::MAX) {
            return Err(Error::SegmentsOverlap);
        }
    }
    Ok(())
}

/// Extracts and validates the interpreter path of `PT_INTERP`.
fn interp_path<'a>(data: &'a [u8], ph: &ProgramHeader) -> Result<&'a [u8], Error> {
    let start = usize::try_from(ph.offset).map_err(|_| Error::Truncated)?;
    let len = usize::try_from(ph.filesz).map_err(|_| Error::BadInterp)?;
    if !(2..=MAX_INTERP_LEN).contains(&len) {
        return Err(Error::BadInterp);
    }
    let bytes = data.get(start..start.checked_add(len).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
    let (last, path) = bytes.split_last().ok_or(Error::BadInterp)?;
    if *last != 0 || path.contains(&0) {
        return Err(Error::BadInterp);
    }
    Ok(path)
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off.checked_add(4)?)?.try_into().ok()?))
}

pub(crate) fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off.checked_add(8)?)?.try_into().ok()?))
}
