//! MS-DOS stub, PE signature, COFF file header, optional header, data
//! directories, section table and COFF string table (spec "Overview",
//! "MS-DOS Stub", "Signature", "COFF File Header", "Optional Header",
//! "Section Table", "COFF String Table").

use crate::read::{bytes, ptr_at, u8_at, u16_at, u32_at, u64_at};
use crate::view::{Layout, View};
use crate::{Error, MAX_IMAGE_SECTIONS, dir, dll_flags, file_flags, section_flags};

/// Offset of `e_lfanew` (file offset of the PE signature) in the MS-DOS header.
const E_LFANEW: usize = 0x3c;
/// Optional header magic numbers.
const MAGIC_PE32: u16 = 0x10b;
const MAGIC_PE32_PLUS: u16 = 0x20b;
/// Size of the optional header's fixed fields, before the data directories.
const OPT_FIXED_PE32: usize = 96;
const OPT_FIXED_PE32_PLUS: usize = 112;
/// Size of one COFF symbol table record; the string table follows the table.
const SYMBOL_SIZE: usize = 18;

/// PE32 (32-bit addresses) or PE32+ (64-bit addresses).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bits {
    Pe32,
    Pe32Plus,
}

impl Bits {
    /// Size of a pointer (and of an import/TLS table slot) in bytes.
    pub const fn pointer_size(self) -> usize {
        match self {
            Bits::Pe32 => 4,
            Bits::Pe32Plus => 8,
        }
    }

    pub(crate) const fn wide(self) -> bool {
        matches!(self, Bits::Pe32Plus)
    }
}

/// The COFF file header (spec "COFF File Header (Object and Image)").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FileHeader {
    pub machine: u16,
    pub number_of_sections: u16,
    pub time_date_stamp: u32,
    pub pointer_to_symbol_table: u32,
    pub number_of_symbols: u32,
    pub size_of_optional_header: u16,
    /// `IMAGE_FILE_*` flags, see [`crate::file_flags`].
    pub characteristics: u16,
}

impl FileHeader {
    pub const SIZE: usize = 20;

    fn parse(b: &[u8], off: usize) -> Result<Self, Error> {
        let h = bytes(b, off, Self::SIZE).ok_or(Error::Truncated)?;
        let u16 = |o| u16_at(h, o).unwrap_or(0);
        let u32 = |o| u32_at(h, o).unwrap_or(0);
        Ok(FileHeader {
            machine: u16(0),
            number_of_sections: u16(2),
            time_date_stamp: u32(4),
            pointer_to_symbol_table: u32(8),
            number_of_symbols: u32(12),
            size_of_optional_header: u16(16),
            characteristics: u16(18),
        })
    }
}

/// The optional header's standard and Windows-specific fields (spec
/// "Optional Header Standard Fields" and "Optional Header Windows-Specific
/// Fields"). PE32 fields that are 32 bits wide are zero-extended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OptionalHeader {
    pub bits: Bits,
    pub major_linker_version: u8,
    pub minor_linker_version: u8,
    pub size_of_code: u32,
    pub size_of_initialized_data: u32,
    pub size_of_uninitialized_data: u32,
    pub address_of_entry_point: u32,
    pub base_of_code: u32,
    /// PE32 only; zero for PE32+.
    pub base_of_data: u32,
    pub image_base: u64,
    pub section_alignment: u32,
    pub file_alignment: u32,
    pub major_operating_system_version: u16,
    pub minor_operating_system_version: u16,
    pub major_image_version: u16,
    pub minor_image_version: u16,
    pub major_subsystem_version: u16,
    pub minor_subsystem_version: u16,
    pub win32_version_value: u32,
    pub size_of_image: u32,
    pub size_of_headers: u32,
    pub check_sum: u32,
    /// `IMAGE_SUBSYSTEM_*`, see [`crate::subsystem`].
    pub subsystem: u16,
    /// `IMAGE_DLLCHARACTERISTICS_*`, see [`crate::dll_flags`].
    pub dll_characteristics: u16,
    pub size_of_stack_reserve: u64,
    pub size_of_stack_commit: u64,
    pub size_of_heap_reserve: u64,
    pub size_of_heap_commit: u64,
    pub loader_flags: u32,
    /// As stored; [`Directories`] holds at most 16 and only those that fit
    /// in `SizeOfOptionalHeader`.
    pub number_of_rva_and_sizes: u32,
}

impl OptionalHeader {
    /// Parses the optional header in `h` (exactly `SizeOfOptionalHeader`
    /// bytes) and returns it with the offset of its data directories.
    fn parse(h: &[u8]) -> Result<(Self, usize), Error> {
        let bits = match u16_at(h, 0).ok_or(Error::BadOptionalHeader)? {
            MAGIC_PE32 => Bits::Pe32,
            MAGIC_PE32_PLUS => Bits::Pe32Plus,
            _ => return Err(Error::BadOptionalHeader),
        };
        let wide = bits.wide();
        let fixed = if wide { OPT_FIXED_PE32_PLUS } else { OPT_FIXED_PE32 };
        if h.len() < fixed {
            return Err(Error::BadOptionalHeader);
        }
        // All reads below are inside `fixed` bytes, checked above.
        let u8 = |o| u8_at(h, o).unwrap_or(0);
        let u16 = |o| u16_at(h, o).unwrap_or(0);
        let u32 = |o| u32_at(h, o).unwrap_or(0);
        let ptr = |o| ptr_at(h, o, wide).unwrap_or(0);
        // From SizeOfStackReserve on, PE32+ fields are 8 bytes and PE32's 4.
        let (p, rest) = if wide { (8, 104) } else { (4, 88) };
        let opt = OptionalHeader {
            bits,
            major_linker_version: u8(2),
            minor_linker_version: u8(3),
            size_of_code: u32(4),
            size_of_initialized_data: u32(8),
            size_of_uninitialized_data: u32(12),
            address_of_entry_point: u32(16),
            base_of_code: u32(20),
            base_of_data: if wide { 0 } else { u32(24) },
            image_base: if wide { u64_at(h, 24).unwrap_or(0) } else { u64::from(u32(28)) },
            section_alignment: u32(32),
            file_alignment: u32(36),
            major_operating_system_version: u16(40),
            minor_operating_system_version: u16(42),
            major_image_version: u16(44),
            minor_image_version: u16(46),
            major_subsystem_version: u16(48),
            minor_subsystem_version: u16(50),
            win32_version_value: u32(52),
            size_of_image: u32(56),
            size_of_headers: u32(60),
            check_sum: u32(64),
            subsystem: u16(68),
            dll_characteristics: u16(70),
            size_of_stack_reserve: ptr(72),
            size_of_stack_commit: ptr(72 + p),
            size_of_heap_reserve: ptr(72 + 2 * p),
            size_of_heap_commit: ptr(72 + 3 * p),
            loader_flags: u32(rest),
            number_of_rva_and_sizes: u32(rest + 4),
        };
        Ok((opt, fixed))
    }

    /// Offset of the `ImageBase` field from the start of the optional header.
    pub(crate) const fn image_base_offset(bits: Bits) -> usize {
        match bits {
            Bits::Pe32 => 28,
            Bits::Pe32Plus => 24,
        }
    }
}

/// One data directory: an RVA and a size (spec "Optional Header Data
/// Directories (Image Only)").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct DataDirectory {
    pub virtual_address: u32,
    pub size: u32,
}

impl DataDirectory {
    /// Whether `rva` lies in `[virtual_address, virtual_address + size)`.
    pub fn contains(&self, rva: u32) -> bool {
        rva >= self.virtual_address && u64::from(rva) < u64::from(self.virtual_address) + u64::from(self.size)
    }
}

/// The data directories an image has (at most 16).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Directories {
    entries: [DataDirectory; dir::COUNT],
    count: usize,
}

impl Directories {
    /// Number of directory slots the header has.
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The directory at `index` (see [`crate::dir`]) as stored, or all zeros
    /// if the header has no such slot.
    pub fn raw(&self, index: usize) -> DataDirectory {
        if index < self.count { self.entries[index] } else { DataDirectory::default() }
    }

    /// The directory at `index` if the image has it: a slot exists and its
    /// address is not zero.
    pub fn get(&self, index: usize) -> Option<DataDirectory> {
        let d = self.raw(index);
        (d.virtual_address != 0).then_some(d)
    }

    fn parse(b: &[u8], count: usize) -> Self {
        let mut d = Directories::default();
        for i in 0..count.min(dir::COUNT) {
            let (Some(rva), Some(size)) = (u32_at(b, i * 8), u32_at(b, i * 8 + 4)) else { break };
            d.entries[i] = DataDirectory { virtual_address: rva, size };
            d.count = i + 1;
        }
        d
    }
}

/// Memory protection of a section, from its `IMAGE_SCN_MEM_*` flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub struct Protection {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl Protection {
    /// What the headers get: read-only.
    pub const READ_ONLY: Protection = Protection { read: true, write: false, execute: false };

    pub const fn from_characteristics(c: u32) -> Self {
        Protection {
            read: c & section_flags::MEM_READ != 0,
            write: c & section_flags::MEM_WRITE != 0,
            execute: c & section_flags::MEM_EXECUTE != 0,
        }
    }
}

/// One section table entry (spec "Section Table (Section Headers)").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SectionHeader {
    /// The raw 8-byte name field. See [`SectionTable::name`] for long names.
    pub name: [u8; 8],
    pub virtual_size: u32,
    pub virtual_address: u32,
    pub size_of_raw_data: u32,
    pub pointer_to_raw_data: u32,
    pub pointer_to_relocations: u32,
    pub pointer_to_linenumbers: u32,
    pub number_of_relocations: u16,
    pub number_of_linenumbers: u16,
    /// `IMAGE_SCN_*` flags, see [`crate::section_flags`].
    pub characteristics: u32,
}

impl SectionHeader {
    pub const SIZE: usize = 40;

    fn parse(h: &[u8]) -> Self {
        let mut name = [0; 8];
        if let Some(n) = bytes(h, 0, 8) {
            name.copy_from_slice(n);
        }
        let u16 = |o| u16_at(h, o).unwrap_or(0);
        let u32 = |o| u32_at(h, o).unwrap_or(0);
        SectionHeader {
            name,
            virtual_size: u32(8),
            virtual_address: u32(12),
            size_of_raw_data: u32(16),
            pointer_to_raw_data: u32(20),
            pointer_to_relocations: u32(24),
            pointer_to_linenumbers: u32(28),
            number_of_relocations: u16(32),
            number_of_linenumbers: u16(34),
            characteristics: u32(36),
        }
    }

    /// The name field up to its first NUL (all 8 bytes if there is none).
    pub fn short_name(&self) -> &[u8] {
        let n = self.name.iter().position(|&c| c == 0).unwrap_or(8);
        &self.name[..n]
    }

    /// For a name of the form "/123", the decimal offset into the string
    /// table where the real (long) name is.
    pub fn long_name_offset(&self) -> Option<u32> {
        let digits = self.short_name().strip_prefix(b"/")?;
        if digits.is_empty() {
            return None;
        }
        let mut v: u32 = 0;
        for &d in digits {
            if !d.is_ascii_digit() {
                return None;
            }
            v = v.checked_mul(10)?.checked_add(u32::from(d - b'0'))?;
        }
        Some(v)
    }

    pub fn protection(&self) -> Protection {
        Protection::from_characteristics(self.characteristics)
    }

    /// Size of the section in memory. A `VirtualSize` of zero (as some
    /// linkers leave it) means the raw size.
    pub fn mapped_size(&self) -> u32 {
        if self.virtual_size == 0 { self.size_of_raw_data } else { self.virtual_size }
    }

    /// Bytes of the section that come from the file; the rest of
    /// [`mapped_size`](Self::mapped_size) is zero-filled.
    pub fn file_backed_size(&self) -> u32 {
        self.size_of_raw_data.min(self.mapped_size())
    }

    /// Whether `rva` falls in `[VirtualAddress, VirtualAddress + mapped_size)`.
    pub fn contains_rva(&self, rva: u32) -> bool {
        rva >= self.virtual_address && u64::from(rva) < u64::from(self.virtual_address) + u64::from(self.mapped_size())
    }

    /// Alignment from the `IMAGE_SCN_ALIGN_*` field (object files only).
    pub fn alignment(&self) -> Option<u32> {
        alignment_field(self.characteristics)
    }
}

/// Decodes an `IMAGE_SCN_ALIGN_*` field (bits 20-23: `n` means `1 << (n-1)`).
pub(crate) fn alignment_field(characteristics: u32) -> Option<u32> {
    match (characteristics & section_flags::ALIGN_MASK) >> 20 {
        0 | 15 => None,
        n => Some(1 << (n - 1)),
    }
}

/// The section table: an array of [`SectionHeader`]s in the file.
#[derive(Clone, Copy, Debug, Default)]
pub struct SectionTable<'a> {
    bytes: &'a [u8],
}

impl<'a> SectionTable<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        SectionTable { bytes }
    }

    pub fn len(&self) -> usize {
        self.bytes.len() / SectionHeader::SIZE
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<SectionHeader> {
        let h = bytes(self.bytes, index.checked_mul(SectionHeader::SIZE)?, SectionHeader::SIZE)?;
        Some(SectionHeader::parse(h))
    }

    pub fn iter(&self) -> impl Iterator<Item = SectionHeader> + 'a {
        let table = *self;
        (0..table.len()).filter_map(move |i| table.get(i))
    }

    /// The name of section `index`: the 8-byte field up to its first NUL,
    /// or for "/nnn" names the string at offset nnn of `strings` (object
    /// files, and images that keep a COFF string table). Without a string
    /// table a "/nnn" name is returned as is.
    pub fn name(&self, index: usize, strings: Option<&StringTable<'a>>) -> Result<&'a [u8], Error> {
        let raw = bytes(self.bytes, index.checked_mul(SectionHeader::SIZE).ok_or(Error::Truncated)?, 8)
            .ok_or(Error::Truncated)?;
        let header = self.get(index).ok_or(Error::Truncated)?;
        if let (Some(offset), Some(strings)) = (header.long_name_offset(), strings) {
            return strings.get(offset);
        }
        let n = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
        Ok(&raw[..n])
    }

    /// The first section whose memory range contains `rva`.
    pub fn find_rva(&self, rva: u32) -> Option<(usize, SectionHeader)> {
        self.iter().enumerate().find(|(_, s)| s.contains_rva(rva))
    }
}

/// The COFF string table that follows the symbol table (spec "COFF String
/// Table"). Its first 4 bytes hold its size, including those 4 bytes.
#[derive(Clone, Copy, Debug)]
pub struct StringTable<'a> {
    bytes: &'a [u8],
}

impl<'a> StringTable<'a> {
    fn locate(data: &'a [u8], header: &FileHeader) -> Option<Self> {
        if header.pointer_to_symbol_table == 0 {
            return None;
        }
        let symbols = (header.number_of_symbols as usize).checked_mul(SYMBOL_SIZE)?;
        let start = (header.pointer_to_symbol_table as usize).checked_add(symbols)?;
        let size = u32_at(data, start)? as usize;
        if size < 4 {
            return None;
        }
        Some(StringTable { bytes: bytes(data, start, size)? })
    }

    /// The NUL-terminated string at `offset` (counted from the size field).
    pub fn get(&self, offset: u32) -> Result<&'a [u8], Error> {
        let offset = offset as usize;
        if offset < 4 {
            return Err(Error::Malformed("string table offset inside its size field"));
        }
        crate::read::cstr(self.bytes, offset)
    }

    /// Size in bytes, including the 4-byte size field.
    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

/// A COFF object file (`.obj`): a file header, sections and a string table,
/// no optional header.
#[derive(Clone, Copy, Debug)]
pub struct CoffObject<'a> {
    pub header: FileHeader,
    sections: SectionTable<'a>,
    strings: Option<StringTable<'a>>,
    data: &'a [u8],
}

impl<'a> CoffObject<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        let header = FileHeader::parse(data, 0)?;
        // Import-library members start with machine 0 and 0xffff.
        if header.machine == crate::machine::UNKNOWN && header.number_of_sections == 0xffff {
            return Err(Error::UnsupportedMachine(header.machine));
        }
        let table_off = FileHeader::SIZE + usize::from(header.size_of_optional_header);
        let table_len = usize::from(header.number_of_sections) * SectionHeader::SIZE;
        let sections = SectionTable::new(bytes(data, table_off, table_len).ok_or(Error::Truncated)?);
        Ok(CoffObject { header, sections, strings: StringTable::locate(data, &header), data })
    }

    pub fn sections(&self) -> SectionTable<'a> {
        self.sections
    }

    pub fn string_table(&self) -> Option<StringTable<'a>> {
        self.strings
    }

    /// Section `index`'s name, resolving "/nnn" long names.
    pub fn section_name(&self, index: usize) -> Result<&'a [u8], Error> {
        self.sections.name(index, self.strings.as_ref())
    }

    /// Section `index`'s raw data in the file.
    pub fn section_data(&self, index: usize) -> Result<&'a [u8], Error> {
        let s = self.sections.get(index).ok_or(Error::Truncated)?;
        if s.characteristics & section_flags::CNT_UNINITIALIZED_DATA != 0 || s.size_of_raw_data == 0 {
            return Ok(&[]);
        }
        bytes(self.data, s.pointer_to_raw_data as usize, s.size_of_raw_data as usize).ok_or(Error::Truncated)
    }
}

/// A copy of everything in an image's headers that loading needs. It holds
/// no borrow, so it can be kept while the image buffer is changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// File offset (and RVA) of the "PE\0\0" signature: `e_lfanew`.
    pub nt_offset: u32,
    pub file_header: FileHeader,
    pub optional: OptionalHeader,
    pub directories: Directories,
}

impl ImageInfo {
    pub fn bits(&self) -> Bits {
        self.optional.bits
    }

    pub fn machine(&self) -> u16 {
        self.file_header.machine
    }

    /// The preferred load address.
    pub fn image_base(&self) -> u64 {
        self.optional.image_base
    }

    pub fn size_of_image(&self) -> u32 {
        self.optional.size_of_image
    }

    /// RVA of the entry point; zero if there is none (allowed for DLLs).
    pub fn entry_point(&self) -> u32 {
        self.optional.address_of_entry_point
    }

    pub fn is_dll(&self) -> bool {
        self.file_header.characteristics & file_flags::DLL != 0
    }

    pub fn is_executable(&self) -> bool {
        self.file_header.characteristics & file_flags::EXECUTABLE_IMAGE != 0
    }

    pub fn large_address_aware(&self) -> bool {
        self.file_header.characteristics & file_flags::LARGE_ADDRESS_AWARE != 0
    }

    /// `IMAGE_FILE_RELOCS_STRIPPED`: the image must load at its preferred base.
    pub fn relocs_stripped(&self) -> bool {
        self.file_header.characteristics & file_flags::RELOCS_STRIPPED != 0
    }

    pub fn dynamic_base(&self) -> bool {
        self.optional.dll_characteristics & dll_flags::DYNAMIC_BASE != 0
    }

    pub fn nx_compat(&self) -> bool {
        self.optional.dll_characteristics & dll_flags::NX_COMPAT != 0
    }

    pub fn high_entropy_va(&self) -> bool {
        self.optional.dll_characteristics & dll_flags::HIGH_ENTROPY_VA != 0
    }

    pub fn guard_cf(&self) -> bool {
        self.optional.dll_characteristics & dll_flags::GUARD_CF != 0
    }

    /// The directory at `index` (see [`crate::dir`]) if present.
    pub fn directory(&self, index: usize) -> Option<DataDirectory> {
        self.directories.get(index)
    }

    /// Whether the image can be moved away from its preferred base: its
    /// relocations were not stripped. (An image with no relocation
    /// directory and the flag clear has nothing to fix up.)
    pub fn relocatable(&self) -> bool {
        !self.relocs_stripped()
    }

    /// The RVA of virtual address `va` with the image at `base`, if inside
    /// the image.
    pub fn va_to_rva_at(&self, va: u64, base: u64) -> Option<u32> {
        let rva = u32::try_from(va.checked_sub(base)?).ok()?;
        (rva < self.optional.size_of_image).then_some(rva)
    }

    /// The RVA of `va` with the image at its header's `ImageBase` (which
    /// [`crate::relocate`] updates when it moves an image).
    pub fn va_to_rva(&self, va: u64) -> Option<u32> {
        self.va_to_rva_at(va, self.optional.image_base)
    }
}

/// A parsed PE image. It borrows the bytes and reads directories on demand.
///
/// The bytes are either the file as stored ([`Pe::parse`]: RVAs are
/// translated through the section table) or an image already laid out in
/// memory ([`Pe::parse_mapped`]: an RVA is an offset).
#[derive(Clone, Copy, Debug)]
pub struct Pe<'a> {
    pub(crate) view: View<'a>,
    info: ImageInfo,
    sections: SectionTable<'a>,
    strings: Option<StringTable<'a>>,
}

impl<'a> Pe<'a> {
    /// Parses a PE file as stored on disk.
    pub fn parse(data: &'a [u8]) -> Result<Self, Error> {
        Self::parse_as(data, Layout::File)
    }

    /// Parses an image that is already mapped: headers at offset 0 and each
    /// section at its RVA (as [`crate::LoadPlan::map_into`] lays it out).
    pub fn parse_mapped(image: &'a [u8]) -> Result<Self, Error> {
        Self::parse_as(image, Layout::Mapped)
    }

    fn parse_as(data: &'a [u8], layout: Layout) -> Result<Self, Error> {
        if data.get(..2) != Some(b"MZ") {
            return Err(Error::NotMz);
        }
        let nt = u32_at(data, E_LFANEW).ok_or(Error::Truncated)?;
        let nt_off = nt as usize;
        let sig = bytes(data, nt_off, 4).ok_or(Error::NoPeSignature)?;
        if sig != b"PE\0\0" {
            return Err(Error::NoPeSignature);
        }
        let file_header = FileHeader::parse(data, nt_off + 4)?;
        let opt_off = nt_off + 4 + FileHeader::SIZE;
        let opt_len = usize::from(file_header.size_of_optional_header);
        let opt_bytes = bytes(data, opt_off, opt_len).ok_or(Error::Truncated)?;
        let (optional, dirs_off) = OptionalHeader::parse(opt_bytes)?;
        let dir_room = (opt_len - dirs_off) / 8;
        let dir_count = (optional.number_of_rva_and_sizes as usize).min(dir_room);
        let directories = Directories::parse(&opt_bytes[dirs_off..], dir_count);

        let nsections = usize::from(file_header.number_of_sections);
        if nsections > MAX_IMAGE_SECTIONS {
            return Err(Error::TooMany);
        }
        let table_off = opt_off + opt_len;
        let sections =
            SectionTable::new(bytes(data, table_off, nsections * SectionHeader::SIZE).ok_or(Error::Truncated)?);
        let strings = match layout {
            Layout::File => StringTable::locate(data, &file_header),
            Layout::Mapped => None,
        };
        let info = ImageInfo { nt_offset: nt, file_header, optional, directories };
        let view = View::new(data, layout, optional.size_of_headers, sections);
        Ok(Pe { view, info, sections, strings })
    }

    /// The bytes this was parsed from.
    pub fn data(&self) -> &'a [u8] {
        self.view.data()
    }

    /// Whether this was parsed with [`Pe::parse_mapped`].
    pub fn is_mapped(&self) -> bool {
        self.view.layout() == Layout::Mapped
    }

    pub fn info(&self) -> &ImageInfo {
        &self.info
    }

    pub fn file_header(&self) -> &FileHeader {
        &self.info.file_header
    }

    pub fn optional_header(&self) -> &OptionalHeader {
        &self.info.optional
    }

    pub fn directories(&self) -> &Directories {
        &self.info.directories
    }

    pub fn sections(&self) -> SectionTable<'a> {
        self.sections
    }

    pub fn string_table(&self) -> Option<StringTable<'a>> {
        self.strings
    }

    /// Section `index`'s name, resolving "/nnn" long names through the COFF
    /// string table when the file has one (MinGW images keep one).
    pub fn section_name(&self, index: usize) -> Result<&'a [u8], Error> {
        self.sections.name(index, self.strings.as_ref())
    }

    /// The first section with this name (short or long).
    pub fn section_by_name(&self, name: &[u8]) -> Option<(usize, SectionHeader)> {
        (0..self.sections.len())
            .find(|&i| self.section_name(i).is_ok_and(|n| n == name))
            .and_then(|i| Some((i, self.sections.get(i)?)))
    }

    /// `len` bytes at `rva`.
    pub fn bytes_at_rva(&self, rva: u32, len: usize) -> Result<&'a [u8], Error> {
        self.view.slice(rva, len)
    }

    /// The NUL-terminated string at `rva`.
    pub fn str_at_rva(&self, rva: u32) -> Result<&'a [u8], Error> {
        self.view.cstr(rva)
    }

    /// Where the byte at `rva` is in [`data`](Self::data).
    pub fn rva_to_offset(&self, rva: u32) -> Result<usize, Error> {
        self.view.offset(rva).map(|(off, _)| off)
    }
}
