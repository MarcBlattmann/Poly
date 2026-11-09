//! [`MachO`]: a validated 64-bit little-endian Mach-O file, and the summary a
//! loader wants of it.

use core::ops::Range;

use crate::bytes::{slice, u32_le};
use crate::chained::ChainedFixups;
use crate::command::*;
use crate::consts::*;
use crate::dyld_info::DyldInfo;
use crate::error::{Error, Result};
use crate::exports::Exports;
use crate::fat::select_slice;
use crate::header::{Cpu, CpuKind, FileType, Header};
use crate::segment::{Prot, Segment};
use crate::symtab::{Dysymtab, DysymtabCommand, Symtab, SymtabCommand};

/// Most `LC_SEGMENT_64` commands accepted. Linkers emit three to twelve.
/// The limit keeps [`MachO`] a fixed-size value, so parsing needs no
/// allocator.
pub const MAX_SEGMENTS: usize = 32;

/// What [`MachO`] remembers of a segment: everything the fixup walkers and a
/// loader need, without borrowing the load commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentInfo {
    /// The segment name, NUL padded.
    pub segname: [u8; 16],
    /// The memory address.
    pub vmaddr: u64,
    /// The size in memory.
    pub vmsize: u64,
    /// The file offset of the contents.
    pub fileoff: u64,
    /// The number of file bytes mapped.
    pub filesize: u64,
    /// The maximum protection.
    pub maxprot: Prot,
    /// The initial protection.
    pub initprot: Prot,
    /// `SG_*` flags.
    pub flags: u32,
    /// The number of sections.
    pub nsects: u32,
}

impl SegmentInfo {
    const EMPTY: SegmentInfo = SegmentInfo {
        segname: [0; 16],
        vmaddr: 0,
        vmsize: 0,
        fileoff: 0,
        filesize: 0,
        maxprot: Prot(0),
        initprot: Prot(0),
        flags: 0,
        nsects: 0,
    };

    /// The segment name without its NUL padding.
    pub fn name(&self) -> &[u8] {
        let n = self.segname.iter().position(|&b| b == 0).unwrap_or(16);
        &self.segname[..n]
    }

    /// The end of the segment in memory (`vmaddr + vmsize`, checked at
    /// parse time not to wrap).
    pub const fn vm_end(&self) -> u64 {
        self.vmaddr.wrapping_add(self.vmsize)
    }

    /// The memory range of the segment.
    pub const fn vm_range(&self) -> Range<u64> {
        self.vmaddr..self.vm_end()
    }

    /// The file range of the segment's contents.
    pub const fn file_range(&self) -> Range<u64> {
        self.fileoff..self.fileoff.wrapping_add(self.filesize)
    }

    /// Whether this is the `__PAGEZERO` segment.
    pub fn is_pagezero(&self) -> bool {
        self.name() == SEG_PAGEZERO
    }
}

/// Where a program starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Entry {
    /// `LC_MAIN`: `main` is called by dyld's start code.
    Main {
        /// Offset of `main` from the start of `__TEXT`.
        entryoff: u64,
        /// The address of `main` at the preferred load address.
        vmaddr: u64,
        /// The initial stack size, 0 for the default.
        stacksize: u64,
    },
    /// `LC_UNIXTHREAD`: the kernel starts the main thread with this state.
    UnixThread {
        /// The initial program counter, a preferred-load-address value.
        pc: u64,
        /// The initial stack pointer, 0 if the kernel chooses.
        sp: u64,
    },
}

/// The platform and versions an image was built for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    /// `PLATFORM_*`.
    pub platform: u32,
    /// The minimum OS version.
    pub minos: Version,
    /// The SDK version.
    pub sdk: Version,
}

/// The facts a loader asks of an image first.
#[derive(Clone, Debug)]
pub struct Summary<'a> {
    /// The CPU the image is for.
    pub cpu: Cpu,
    /// The kind of file.
    pub file_type: FileType,
    /// The header flags (`MH_*`).
    pub flags: u32,
    /// Whether `MH_PIE` is set: the main executable may be loaded at any
    /// address.
    pub is_pie: bool,
    /// Whether the image can be loaded at an address other than its
    /// preferred one: PIE executables, and dylibs, bundles and dynamic
    /// linkers, which always can.
    pub is_slidable: bool,
    /// Where the program starts, if it is a program.
    pub entry: Option<Entry>,
    /// The preferred load address: where the Mach-O header sits in memory.
    pub base_address: u64,
    /// The memory the image occupies, without `__PAGEZERO`.
    pub vm_range: Range<u64>,
    /// The platform, if the file says.
    pub platform: Option<Platform>,
    /// The UUID, if the file has one.
    pub uuid: Option<[u8; 16]>,
    /// The install name of a dylib (`LC_ID_DYLIB`).
    pub install_name: Option<&'a str>,
    /// The dynamic linker the file asks for (`LC_LOAD_DYLINKER`).
    pub dylinker: Option<&'a str>,
    /// Whether the image uses chained fixups (rather than classic opcodes).
    pub has_chained_fixups: bool,
}

/// A parsed 64-bit little-endian Mach-O file.
///
/// [`MachO::parse`] checks the header, every load command and every
/// segment once; the accessors afterwards check only the tables they open.
/// Nothing is copied except a small table of segments.
#[derive(Clone)]
pub struct MachO<'a> {
    data: &'a [u8],
    header: Header,
    area: &'a [u8],
    segments: [SegmentInfo; MAX_SEGMENTS],
    nsegments: usize,
    /// The first command of each kind that is looked up often, found while
    /// parsing so the accessors need not walk the commands again.
    index: Index<'a>,
}

#[derive(Clone, Copy, Default)]
struct Index<'a> {
    symtab: Option<SymtabCommand>,
    dysymtab: Option<DysymtabCommand>,
    dyld_info: Option<DyldInfoCommand>,
    linkedit: [Option<LinkeditData>; LinkeditKind::COUNT],
    main: Option<EntryPoint>,
    unix_thread: Option<Thread<'a>>,
    uuid: Option<[u8; 16]>,
    dylib_id: Option<Dylib<'a>>,
    dylinker: Option<&'a str>,
    source_version: Option<SourceVersion>,
    build_version: Option<Platform>,
    version_min: Option<Platform>,
}

impl<'a> Index<'a> {
    /// Remembers `c` if it is the first of its kind.
    fn note(&mut self, c: &Command<'a>) {
        match *c {
            Command::Symtab(x) => {
                self.symtab.get_or_insert(x);
            }
            Command::Dysymtab(x) => {
                self.dysymtab.get_or_insert(x);
            }
            Command::DyldInfo(x) => {
                self.dyld_info.get_or_insert(x);
            }
            Command::LinkeditData(l) => {
                self.linkedit[l.kind.index()].get_or_insert(l);
            }
            Command::Main(m) => {
                self.main.get_or_insert(m);
            }
            Command::Thread(t) if t.is_unix_thread => {
                self.unix_thread.get_or_insert(t);
            }
            Command::Uuid(u) => {
                self.uuid.get_or_insert(u);
            }
            Command::Dylib(d) if d.kind == DylibKind::Id => {
                self.dylib_id.get_or_insert(d);
            }
            Command::Dylinker(d) if d.kind == DylinkerKind::Load => {
                self.dylinker.get_or_insert(d.name);
            }
            Command::SourceVersion(v) => {
                self.source_version.get_or_insert(v);
            }
            Command::BuildVersion(b) => {
                self.build_version.get_or_insert(Platform { platform: b.platform, minos: b.minos, sdk: b.sdk });
            }
            Command::VersionMin(v) => {
                self.version_min.get_or_insert(Platform { platform: v.platform, minos: v.version, sdk: v.sdk });
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for MachO<'_> {
    /// Shows the header and the segments, not the file's bytes.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MachO")
            .field("len", &self.data.len())
            .field("header", &self.header)
            .field("segments", &self.segment_table())
            .finish()
    }
}

impl<'a> MachO<'a> {
    /// Parses a thin 64-bit little-endian Mach-O file.
    ///
    /// For a universal file use [`MachO::parse_universal`] or pick the
    /// slice with [`Fat`](crate::Fat).
    ///
    /// Checks: the header; that `ncmds` commands of valid size fill
    /// `sizeofcmds` bytes right after it; every command's structure; that
    /// every segment fits the file and `filesize <= vmsize`; that sections
    /// lie inside their segments; and that no two segments overlap, in
    /// memory or in the file. If several commands of a kind exist (two
    /// `LC_SYMTAB`), the accessors use the first.
    pub fn parse(data: &'a [u8]) -> Result<MachO<'a>> {
        let header = Header::parse(data)?;
        let end = (MACH_HEADER_64_SIZE as u64).checked_add(u64::from(header.sizeofcmds)).ok_or(Error::Truncated)?;
        let area = slice(data, MACH_HEADER_64_SIZE as u64, u64::from(header.sizeofcmds)).ok_or(Error::Truncated)?;
        debug_assert!(end <= data.len() as u64);
        let mut macho = MachO {
            data,
            header,
            area,
            segments: [SegmentInfo::EMPTY; MAX_SEGMENTS],
            nsegments: 0,
            index: Index::default(),
        };
        for lc in LoadCommands::new(area, header.ncmds) {
            let cmd = Command::parse(lc?)?;
            macho.index.note(&cmd);
            if let Command::Segment(seg) = cmd {
                seg.validate(data.len() as u64)?;
                if macho.nsegments == MAX_SEGMENTS {
                    return Err(Error::TooManySegments);
                }
                macho.segments[macho.nsegments] = SegmentInfo {
                    segname: seg.segname,
                    vmaddr: seg.vmaddr,
                    vmsize: seg.vmsize,
                    fileoff: seg.fileoff,
                    filesize: seg.filesize,
                    maxprot: seg.maxprot,
                    initprot: seg.initprot,
                    flags: seg.flags,
                    nsects: seg.nsects,
                };
                macho.nsegments += 1;
            }
        }
        macho.check_segment_overlaps()?;
        Ok(macho)
    }

    /// Parses the slice of `data` for `kind` if `data` is universal, or
    /// `data` itself if it is a thin file for that CPU.
    pub fn parse_universal(data: &'a [u8], kind: CpuKind) -> Result<MachO<'a>> {
        MachO::parse(select_slice(data, kind)?)
    }

    fn check_segment_overlaps(&self) -> Result<()> {
        let segs = self.segment_table();
        for (i, a) in segs.iter().enumerate() {
            for b in &segs[i + 1..] {
                // Ends were checked not to wrap in `Segment::validate`.
                if a.vmsize != 0 && b.vmsize != 0 && a.vmaddr < b.vm_end() && b.vmaddr < a.vm_end() {
                    return Err(Error::SegmentsOverlapInMemory);
                }
                if a.filesize != 0
                    && b.filesize != 0
                    && a.fileoff < b.file_range().end
                    && b.fileoff < a.file_range().end
                {
                    return Err(Error::SegmentsOverlapInFile);
                }
            }
        }
        Ok(())
    }

    /// The bytes of the file (the slice, for a universal file). Every
    /// offset in the file is relative to this.
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The header.
    pub const fn header(&self) -> &Header {
        &self.header
    }

    /// The CPU the file is for.
    pub const fn cpu(&self) -> Cpu {
        self.header.cpu
    }

    /// The raw load commands.
    pub fn raw_commands(&self) -> LoadCommands<'a> {
        LoadCommands::new(self.area, self.header.ncmds)
    }

    /// The load commands, typed. [`MachO::parse`] has checked them all, so
    /// the iterator never stops early.
    pub fn commands(&self) -> Commands<'a> {
        Commands(self.raw_commands())
    }

    /// The segments, with their sections.
    pub fn segments(&self) -> impl Iterator<Item = Segment<'a>> + use<'a> {
        self.commands().filter_map(|c| match c {
            Command::Segment(s) => Some(s),
            _ => None,
        })
    }

    /// The segments as remembered at parse time, in file order. A fixup's
    /// "segment index" counts into this table.
    pub fn segment_table(&self) -> &[SegmentInfo] {
        &self.segments[..self.nsegments]
    }

    /// The segment with this name, first in file order.
    pub fn find_segment(&self, name: &[u8]) -> Option<(usize, &SegmentInfo)> {
        self.segment_table().iter().enumerate().find(|(_, s)| s.name() == name)
    }

    /// The segment at the start of the file that holds the header: the first
    /// one with `fileoff == 0` and file contents. Its address is where the
    /// image's Mach-O header sits in memory.
    pub fn header_segment(&self) -> Option<&SegmentInfo> {
        self.segment_table().iter().find(|s| s.fileoff == 0 && s.filesize != 0)
    }

    /// The symbol table and string table, if the file has `LC_SYMTAB`.
    pub fn symtab(&self) -> Result<Option<Symtab<'a>>> {
        match self.index.symtab {
            Some(cmd) => Symtab::new(self.data, &cmd).map(Some),
            None => Ok(None),
        }
    }

    /// The dynamic symbol table, if the file has `LC_DYSYMTAB`. Its ranges
    /// are checked against the symbol table.
    pub fn dysymtab(&self) -> Result<Option<Dysymtab<'a>>> {
        let Some(cmd) = self.index.dysymtab else {
            return Ok(None);
        };
        let nsyms = self.index.symtab.map_or(0, |s| s.nsyms);
        Dysymtab::new(self.data, &cmd, nsyms).map(Some)
    }

    /// The libraries this image depends on, in load command order. Their
    /// 1-based position in this list is the library ordinal of binds and
    /// symbols. `LC_ID_DYLIB` is not a dependency and is left out.
    pub fn dylibs(&self) -> impl Iterator<Item = Dylib<'a>> + use<'a> {
        self.commands().filter_map(|c| match c {
            Command::Dylib(d) if d.kind != DylibKind::Id => Some(d),
            _ => None,
        })
    }

    /// The library for a 1-based library ordinal.
    pub fn dylib_by_ordinal(&self, ordinal: u32) -> Option<Dylib<'a>> {
        let index = ordinal.checked_sub(1)? as usize;
        self.dylibs().nth(index)
    }

    /// The `LC_ID_DYLIB` of a dylib: its install name and versions.
    pub fn dylib_id(&self) -> Option<Dylib<'a>> {
        self.index.dylib_id
    }

    /// The `LC_RPATH` entries, in order.
    pub fn rpaths(&self) -> impl Iterator<Item = &'a str> + use<'a> {
        self.commands().filter_map(|c| match c {
            Command::Rpath(p) => Some(p),
            _ => None,
        })
    }

    /// The dynamic linker path (`LC_LOAD_DYLINKER`).
    pub fn dylinker(&self) -> Option<&'a str> {
        self.index.dylinker
    }

    /// The UUID (`LC_UUID`).
    pub fn uuid(&self) -> Option<[u8; 16]> {
        self.index.uuid
    }

    /// The source version (`LC_SOURCE_VERSION`).
    pub fn source_version(&self) -> Option<SourceVersion> {
        self.index.source_version
    }

    /// The platform and versions, from `LC_BUILD_VERSION`, or else from an
    /// `LC_VERSION_MIN_*` command.
    pub fn platform(&self) -> Option<Platform> {
        self.index.build_version.or(self.index.version_min)
    }

    /// The bytes of a `linkedit_data_command` blob, if the file has one of
    /// that kind.
    pub fn linkedit_data(&self, kind: LinkeditKind) -> Result<Option<&'a [u8]>> {
        let Some(l) = self.index.linkedit[kind.index()] else {
            return Ok(None);
        };
        slice(self.data, u64::from(l.dataoff), u64::from(l.datasize)).map(Some).ok_or(Error::TableOutsideFile)
    }

    /// The export trie, from `LC_DYLD_EXPORTS_TRIE`, or else from the
    /// export range of `LC_DYLD_INFO(_ONLY)`.
    pub fn exports(&self) -> Result<Option<Exports<'a>>> {
        if let Some(d) = self.linkedit_data(LinkeditKind::ExportsTrie)? {
            return Ok(Some(Exports::new(d)));
        }
        match self.dyld_info_command() {
            Some(i) if i.export_size != 0 => {
                let d = slice(self.data, u64::from(i.export_off), u64::from(i.export_size))
                    .ok_or(Error::TableOutsideFile)?;
                Ok(Some(Exports::new(d)))
            }
            _ => Ok(None),
        }
    }

    fn dyld_info_command(&self) -> Option<DyldInfoCommand> {
        self.index.dyld_info
    }

    /// The classic rebase and bind streams of `LC_DYLD_INFO(_ONLY)`.
    pub fn dyld_info(&self) -> Result<Option<DyldInfo<'_>>> {
        match self.dyld_info_command() {
            Some(cmd) => DyldInfo::new(self.data, &cmd, self.segment_table()).map(Some),
            None => Ok(None),
        }
    }

    /// The chained fixups of `LC_DYLD_CHAINED_FIXUPS`.
    pub fn chained_fixups(&self) -> Result<Option<ChainedFixups<'_>>> {
        match self.linkedit_data(LinkeditKind::ChainedFixups)? {
            Some(payload) => ChainedFixups::new(self.data, payload, self.segment_table()).map(Some),
            None => Ok(None),
        }
    }

    /// The function start offsets (`LC_FUNCTION_STARTS`): offsets from the
    /// start of `__TEXT`, decoded from their delta encoding.
    pub fn function_starts(&self) -> Result<Option<FunctionStarts<'a>>> {
        Ok(self.linkedit_data(LinkeditKind::FunctionStarts)?.map(|d| FunctionStarts {
            cursor: crate::bytes::Cursor::new(d),
            addr: 0,
            done: false,
        }))
    }

    /// The ranges of data in code (`LC_DATA_IN_CODE`).
    pub fn data_in_code(&self) -> Result<Option<DataInCode<'a>>> {
        Ok(self.linkedit_data(LinkeditKind::DataInCode)?.map(|d| DataInCode { rest: d }))
    }

    /// Where the program starts, or `None` if the file has neither
    /// `LC_MAIN` nor `LC_UNIXTHREAD`. The entry point must lie inside a
    /// segment.
    pub fn entry(&self) -> Result<Option<Entry>> {
        if let Some(m) = self.index.main {
            let text = self.header_segment().ok_or(Error::BadEntry)?;
            if m.entryoff >= text.filesize {
                return Err(Error::BadEntry);
            }
            let vmaddr = text.vmaddr.checked_add(m.entryoff).ok_or(Error::BadEntry)?;
            return Ok(Some(Entry::Main { entryoff: m.entryoff, vmaddr, stacksize: m.stacksize }));
        }
        if let Some(t) = self.index.unix_thread {
            let kind = self.header.cpu.kind();
            let pc = t.entry_pc(kind).ok_or(Error::BadEntry)?;
            if !self.segment_table().iter().any(|s| s.vm_range().contains(&pc)) {
                return Err(Error::BadEntry);
            }
            return Ok(Some(Entry::UnixThread { pc, sp: t.stack_pointer(kind).unwrap_or(0) }));
        }
        Ok(None)
    }

    /// The memory the image occupies: from its lowest to its highest
    /// segment address, `__PAGEZERO` excluded. `None` if no segment has
    /// memory.
    pub fn vm_range(&self) -> Option<Range<u64>> {
        let mut r: Option<Range<u64>> = None;
        for s in self.segment_table() {
            if s.is_pagezero() || s.vmsize == 0 {
                continue;
            }
            r = Some(match r {
                None => s.vm_range(),
                Some(r) => r.start.min(s.vmaddr)..r.end.max(s.vm_end()),
            });
        }
        r
    }

    /// The facts a loader asks first: see [`Summary`].
    pub fn summary(&self) -> Result<Summary<'a>> {
        let file_type = self.header.file_type();
        Ok(Summary {
            cpu: self.header.cpu,
            file_type,
            flags: self.header.flags,
            is_pie: self.header.is_pie(),
            is_slidable: self.header.is_pie()
                || matches!(file_type, FileType::Dylib | FileType::Bundle | FileType::Dylinker),
            entry: self.entry()?,
            base_address: self.header_segment().map_or(0, |s| s.vmaddr),
            vm_range: self.vm_range().unwrap_or(0..0),
            platform: self.platform(),
            uuid: self.uuid(),
            install_name: self.dylib_id().map(|d| d.name),
            dylinker: self.dylinker(),
            has_chained_fixups: self.index.linkedit[LinkeditKind::ChainedFixups.index()].is_some(),
        })
    }
}

/// Iterator over the typed load commands of a [`MachO`].
#[derive(Clone, Debug)]
pub struct Commands<'a>(LoadCommands<'a>);

impl<'a> Iterator for Commands<'a> {
    type Item = Command<'a>;

    fn next(&mut self) -> Option<Command<'a>> {
        // `MachO::parse` has checked every command, so `Err` cannot happen
        // on a parsed file; it ends the iteration if it ever did.
        match self.0.next()? {
            Ok(lc) => Command::parse(lc).ok(),
            Err(_) => None,
        }
    }
}

/// Iterator over the function start offsets of `LC_FUNCTION_STARTS`.
///
/// The table is a list of ULEB128 distances from the previous function,
/// the first from the start of `__TEXT`, ended by a zero.
#[derive(Clone, Debug)]
pub struct FunctionStarts<'a> {
    cursor: crate::bytes::Cursor<'a>,
    addr: u64,
    done: bool,
}

impl Iterator for FunctionStarts<'_> {
    type Item = Result<u64>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.cursor.is_empty() {
            return None;
        }
        let delta = match self.cursor.uleb() {
            Ok(d) => d,
            Err(e) => {
                self.done = true;
                return Some(Err(e));
            }
        };
        if delta == 0 {
            self.done = true;
            return None;
        }
        match self.addr.checked_add(delta) {
            Some(a) => {
                self.addr = a;
                Some(Ok(a))
            }
            None => {
                self.done = true;
                Some(Err(Error::BadLeb128))
            }
        }
    }
}

/// A range of data inside a code section (`struct data_in_code_entry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataInCodeEntry {
    /// Offset from the start of the file's Mach-O header.
    pub offset: u32,
    /// Length of the range in bytes.
    pub length: u16,
    /// `DICE_KIND_*`.
    pub kind: u16,
}

/// Iterator over `LC_DATA_IN_CODE` entries.
#[derive(Clone, Debug)]
pub struct DataInCode<'a> {
    rest: &'a [u8],
}

impl Iterator for DataInCode<'_> {
    type Item = DataInCodeEntry;

    fn next(&mut self) -> Option<DataInCodeEntry> {
        if self.rest.len() < DATA_IN_CODE_ENTRY_SIZE {
            return None;
        }
        let e = DataInCodeEntry {
            offset: u32_le(self.rest, 0)?,
            length: crate::bytes::u16_le(self.rest, 4)?,
            kind: crate::bytes::u16_le(self.rest, 6)?,
        };
        self.rest = &self.rest[DATA_IN_CODE_ENTRY_SIZE..];
        Some(e)
    }
}
