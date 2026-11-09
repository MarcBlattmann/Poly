//! Classic dyld information (`LC_DYLD_INFO`, `LC_DYLD_INFO_ONLY`): the rebase
//! and bind opcode streams, decoded into plain records.
//!
//! The streams are byte codes (the opcode names and the table of tuples they
//! build are in `<mach-o/loader.h>`). The decoders here are iterators: each
//! `next` runs opcodes until one record is complete.
//!
//! What every record is checked for: the segment exists, and the pointer
//! slot (8 bytes) lies inside the part of the segment that comes from the
//! file (a pointer to fix has to be stored there; zero fill has nothing to
//! rebase or bind). A stream that emits more records than the file-backed
//! segments have pointer slots ends with [`Error::TooManyFixups`], so no
//! stream runs for long, whatever its bytes.
//!
//! Offset arithmetic follows the format: `ADD_ADDR` opcodes take an
//! unsigned LEB128 that wraps around 2^64, which is how a linker encodes a
//! negative skip. Every record is checked after the addition, so a wrap
//! that lands outside the segment is an error, not a wild address.

use crate::bytes::{Cursor, slice};
use crate::command::DyldInfoCommand;
use crate::consts::*;
use crate::error::{Error, Result};
use crate::image::SegmentInfo;
use crate::symtab::MAX_NAME_LEN;

/// The size of a pointer in a 64-bit image.
const PTR: u64 = 8;

/// The classic dyld streams of an image, borrowed from the file.
#[derive(Clone, Copy, Debug)]
pub struct DyldInfo<'s> {
    command: DyldInfoCommand,
    rebase: &'s [u8],
    bind: &'s [u8],
    weak_bind: &'s [u8],
    lazy_bind: &'s [u8],
    segments: &'s [SegmentInfo],
}

impl<'s> DyldInfo<'s> {
    pub(crate) fn new(file: &'s [u8], cmd: &DyldInfoCommand, segments: &'s [SegmentInfo]) -> Result<Self> {
        let stream = |off: u32, size: u32| -> Result<&'s [u8]> {
            if size == 0 {
                return Ok(&[]);
            }
            slice(file, u64::from(off), u64::from(size)).ok_or(Error::TableOutsideFile)
        };
        // The export range is checked too, though it is read by `Exports`.
        stream(cmd.export_off, cmd.export_size)?;
        Ok(DyldInfo {
            command: *cmd,
            rebase: stream(cmd.rebase_off, cmd.rebase_size)?,
            bind: stream(cmd.bind_off, cmd.bind_size)?,
            weak_bind: stream(cmd.weak_bind_off, cmd.weak_bind_size)?,
            lazy_bind: stream(cmd.lazy_bind_off, cmd.lazy_bind_size)?,
            segments,
        })
    }

    /// The command's fields.
    pub const fn command(&self) -> &DyldInfoCommand {
        &self.command
    }

    /// The raw rebase opcodes.
    pub const fn rebase_stream(&self) -> &'s [u8] {
        self.rebase
    }

    /// The raw bind opcodes.
    pub const fn bind_stream(&self) -> &'s [u8] {
        self.bind
    }

    /// The raw weak bind opcodes.
    pub const fn weak_bind_stream(&self) -> &'s [u8] {
        self.weak_bind
    }

    /// The raw lazy bind opcodes.
    pub const fn lazy_bind_stream(&self) -> &'s [u8] {
        self.lazy_bind
    }

    /// The rebases: pointers inside the image that move with its load
    /// address.
    pub fn rebases(&self) -> RebaseIter<'s> {
        RebaseIter { cursor: Cursor::new(self.rebase), walker: Walker::new(self.segments), kind: 0, done: false }
    }

    /// The binds: pointers filled with the address of a symbol from
    /// another image, at load time.
    pub fn binds(&self) -> BindIter<'s> {
        BindIter(self.events(self.bind, StreamKind::Regular))
    }

    /// The weak binds: pointers that may be rebound to a symbol another
    /// image defines too. Strong-definition markers are skipped; see
    /// [`weak_bind_events`](Self::weak_bind_events).
    pub fn weak_binds(&self) -> BindIter<'s> {
        BindIter(self.events(self.weak_bind, StreamKind::Weak))
    }

    /// The weak bind stream with its markers of non-weak definitions.
    pub fn weak_bind_events(&self) -> BindEvents<'s> {
        self.events(self.weak_bind, StreamKind::Weak)
    }

    /// The lazy binds: pointers the stubs fill at first call. Each record's
    /// [`stream_offset`](BindRecord::stream_offset) is the value a stub pushes
    /// to ask for it.
    pub fn lazy_binds(&self) -> BindIter<'s> {
        BindIter(self.events(self.lazy_bind, StreamKind::Lazy))
    }

    fn events(&self, stream: &'s [u8], kind: StreamKind) -> BindEvents<'s> {
        BindEvents {
            cursor: Cursor::new(stream),
            walker: Walker::new(self.segments),
            kind,
            bind_type: 0,
            ordinal: 0,
            symbol: &[],
            flags: 0,
            addend: 0,
            entry_start: 0,
            done: false,
        }
    }
}

/// The location state both decoders share: a segment and an offset in it, a
/// run of pointers still to emit, and the budget of records.
#[derive(Clone, Debug)]
struct Walker<'s> {
    segments: &'s [SegmentInfo],
    segment: Option<u8>,
    offset: u64,
    /// Pointers left in the current run.
    remaining: u64,
    /// Added to `offset` after each pointer of the run.
    advance: u64,
    /// Whether that addition may wrap (a single pointer) or not (a run).
    wrap: bool,
    /// The opcode that started the run, for error reports.
    op: u8,
    /// Records the stream may still emit.
    budget: u64,
}

impl<'s> Walker<'s> {
    fn new(segments: &'s [SegmentInfo]) -> Self {
        // Every legitimate record needs a pointer slot of its own in the
        // file contents of a segment.
        let slots: u64 = segments.iter().map(|s| s.filesize / PTR).fold(0u64, |a, b| a.saturating_add(b));
        Walker {
            segments,
            segment: None,
            offset: 0,
            remaining: 0,
            advance: 0,
            wrap: false,
            op: 0,
            budget: slots.saturating_add(16),
        }
    }

    fn set_segment(&mut self, index: u8, offset: u64) -> Result<()> {
        if usize::from(index) >= self.segments.len() {
            return Err(Error::BadSegmentIndex);
        }
        self.segment = Some(index);
        self.offset = offset;
        Ok(())
    }

    fn add(&mut self, delta: u64) {
        self.offset = self.offset.wrapping_add(delta);
    }

    /// Starts a run of `count` pointers, each followed by `advance` bytes.
    fn run(&mut self, count: u64, advance: u64, wrap: bool, op: u8) {
        self.remaining = count;
        self.advance = advance;
        self.wrap = wrap;
        self.op = op;
    }

    /// Takes the next pointer of the run: its segment, offset and address.
    fn take(&mut self) -> Result<(u8, u64, u64)> {
        self.remaining -= 1;
        let index = self.segment.ok_or(Error::BadOpcode(self.op))?;
        let seg = self.segments.get(usize::from(index)).ok_or(Error::BadSegmentIndex)?;
        let end = self.offset.checked_add(PTR).ok_or(Error::FixupOutsideSegment)?;
        if end > seg.filesize {
            return Err(Error::FixupOutsideSegment);
        }
        if self.budget == 0 {
            return Err(Error::TooManyFixups);
        }
        self.budget -= 1;
        let address = seg.vmaddr.checked_add(self.offset).ok_or(Error::FixupOutsideSegment)?;
        let at = self.offset;
        self.offset = if self.wrap {
            self.offset.wrapping_add(self.advance)
        } else {
            self.offset.checked_add(self.advance).ok_or(Error::FixupOutsideSegment)?
        };
        Ok((index, at, address))
    }
}

/// A pointer to move when the image is loaded at another address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RebaseRecord {
    /// The index of the segment (counting `LC_SEGMENT_64` commands).
    pub segment: u8,
    /// The offset of the pointer from the start of the segment.
    pub segment_offset: u64,
    /// The pointer's address at the preferred load address.
    pub address: u64,
    /// `REBASE_TYPE_*`.
    pub kind: u8,
}

/// Iterator over the rebase opcodes.
#[derive(Clone, Debug)]
pub struct RebaseIter<'s> {
    cursor: Cursor<'s>,
    walker: Walker<'s>,
    kind: u8,
    done: bool,
}

impl RebaseIter<'_> {
    fn step(&mut self) -> Result<Option<RebaseRecord>> {
        loop {
            if self.walker.remaining > 0 {
                let (segment, segment_offset, address) = self.walker.take()?;
                return Ok(Some(RebaseRecord { segment, segment_offset, address, kind: self.kind }));
            }
            let Some(byte) = self.cursor.u8() else {
                return Ok(None);
            };
            let imm = byte & REBASE_IMMEDIATE_MASK;
            match byte & REBASE_OPCODE_MASK {
                REBASE_OPCODE_DONE => return Ok(None),
                REBASE_OPCODE_SET_TYPE_IMM => {
                    if !(REBASE_TYPE_POINTER..=REBASE_TYPE_TEXT_PCREL32).contains(&imm) {
                        return Err(Error::BadOpcode(byte));
                    }
                    self.kind = imm;
                }
                REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                    let off = self.cursor.uleb()?;
                    self.walker.set_segment(imm, off)?;
                }
                REBASE_OPCODE_ADD_ADDR_ULEB => {
                    let d = self.cursor.uleb()?;
                    self.walker.add(d);
                }
                REBASE_OPCODE_ADD_ADDR_IMM_SCALED => self.walker.add(u64::from(imm) * PTR),
                REBASE_OPCODE_DO_REBASE_IMM_TIMES => self.walker.run(u64::from(imm), PTR, false, byte),
                REBASE_OPCODE_DO_REBASE_ULEB_TIMES => {
                    let n = self.cursor.uleb()?;
                    self.walker.run(n, PTR, false, byte);
                }
                REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB => {
                    let d = self.cursor.uleb()?;
                    self.walker.run(1, d.wrapping_add(PTR), true, byte);
                }
                REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB => {
                    let n = self.cursor.uleb()?;
                    let skip = self.cursor.uleb()?;
                    let advance = skip.checked_add(PTR).ok_or(Error::FixupOutsideSegment)?;
                    self.walker.run(n, advance, false, byte);
                }
                _ => return Err(Error::BadOpcode(byte)),
            }
        }
    }
}

impl Iterator for RebaseIter<'_> {
    type Item = Result<RebaseRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(r)) => Some(Ok(r)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StreamKind {
    Regular,
    Weak,
    Lazy,
}

/// A pointer to fill with the address of a symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindRecord<'s> {
    /// The index of the segment (counting `LC_SEGMENT_64` commands).
    pub segment: u8,
    /// The offset of the pointer from the start of the segment.
    pub segment_offset: u64,
    /// The pointer's address at the preferred load address.
    pub address: u64,
    /// `BIND_TYPE_*`.
    pub kind: u8,
    /// The library the symbol comes from: a 1-based ordinal into the
    /// dependency list ([`MachO::dylibs`](crate::MachO::dylibs)), or one of
    /// `BIND_SPECIAL_DYLIB_*` (0 self, -1 main executable, -2 flat lookup,
    /// -3 weak lookup).
    pub ordinal: i32,
    /// The symbol name.
    pub symbol: &'s [u8],
    /// `BIND_SYMBOL_FLAGS_*`.
    pub flags: u8,
    /// A value added to the symbol's address.
    pub addend: i64,
    /// Where in the stream the entry started (after the previous `DONE`).
    /// For lazy binds this is the offset a stub pushes.
    pub stream_offset: u32,
}

impl BindRecord<'_> {
    /// Whether the symbol may be missing (`BIND_SYMBOL_FLAGS_WEAK_IMPORT`).
    pub const fn is_weak_import(&self) -> bool {
        self.flags & BIND_SYMBOL_FLAGS_WEAK_IMPORT != 0
    }
}

/// An event of a bind stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindEvent<'s> {
    /// A pointer to bind.
    Bind(BindRecord<'s>),
    /// A symbol this image defines itself, not weakly
    /// (`BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION`): it takes part in weak
    /// symbol coalescing, but has no pointer to fill.
    NonWeakDefinition {
        /// The symbol name.
        symbol: &'s [u8],
    },
}

/// Iterator over the events of a bind, weak bind or lazy bind stream.
#[derive(Clone, Debug)]
pub struct BindEvents<'s> {
    cursor: Cursor<'s>,
    walker: Walker<'s>,
    kind: StreamKind,
    bind_type: u8,
    ordinal: i32,
    symbol: &'s [u8],
    flags: u8,
    addend: i64,
    entry_start: usize,
    done: bool,
}

impl<'s> BindEvents<'s> {
    fn step(&mut self) -> Result<Option<BindEvent<'s>>> {
        loop {
            if self.walker.remaining > 0 {
                let (segment, segment_offset, address) = self.walker.take()?;
                return Ok(Some(BindEvent::Bind(BindRecord {
                    segment,
                    segment_offset,
                    address,
                    kind: self.bind_type,
                    ordinal: self.ordinal,
                    symbol: self.symbol,
                    flags: self.flags,
                    addend: self.addend,
                    stream_offset: u32::try_from(self.entry_start).unwrap_or(u32::MAX),
                })));
            }
            let Some(byte) = self.cursor.u8() else {
                return Ok(None);
            };
            let imm = byte & BIND_IMMEDIATE_MASK;
            match byte & BIND_OPCODE_MASK {
                BIND_OPCODE_DONE => {
                    // A lazy stream is a series of entries, each ended by
                    // DONE; the other streams end at the first one.
                    if self.kind != StreamKind::Lazy {
                        return Ok(None);
                    }
                    self.entry_start = self.cursor.pos();
                }
                BIND_OPCODE_SET_DYLIB_ORDINAL_IMM => self.ordinal = i32::from(imm),
                BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB => {
                    let n = self.cursor.uleb()?;
                    self.ordinal = i32::try_from(n).map_err(|_| Error::BadOpcode(byte))?;
                }
                BIND_OPCODE_SET_DYLIB_SPECIAL_IMM => {
                    // 0 is "self"; 1..=15 are the negative values -15..=-1
                    // (the immediate with its high nibble set, as an i8).
                    self.ordinal = if imm == 0 { 0 } else { i32::from((BIND_OPCODE_MASK | imm) as i8) };
                }
                BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM => {
                    self.flags = imm;
                    self.symbol = self.cursor.cstr(MAX_NAME_LEN).ok_or(Error::BadString)?;
                    if imm & BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION != 0 {
                        return Ok(Some(BindEvent::NonWeakDefinition { symbol: self.symbol }));
                    }
                }
                BIND_OPCODE_SET_TYPE_IMM => {
                    if !(BIND_TYPE_POINTER..=BIND_TYPE_TEXT_PCREL32).contains(&imm) {
                        return Err(Error::BadOpcode(byte));
                    }
                    self.bind_type = imm;
                }
                BIND_OPCODE_SET_ADDEND_SLEB => self.addend = self.cursor.sleb()?,
                BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB => {
                    let off = self.cursor.uleb()?;
                    self.walker.set_segment(imm, off)?;
                }
                BIND_OPCODE_ADD_ADDR_ULEB => {
                    let d = self.cursor.uleb()?;
                    self.walker.add(d);
                }
                BIND_OPCODE_DO_BIND => self.walker.run(1, PTR, true, byte),
                BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB => {
                    let d = self.cursor.uleb()?;
                    self.walker.run(1, d.wrapping_add(PTR), true, byte);
                }
                BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED => {
                    self.walker.run(1, u64::from(imm) * PTR + PTR, true, byte);
                }
                BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB => {
                    let n = self.cursor.uleb()?;
                    let skip = self.cursor.uleb()?;
                    let advance = skip.checked_add(PTR).ok_or(Error::FixupOutsideSegment)?;
                    self.walker.run(n, advance, false, byte);
                }
                // BIND_OPCODE_THREADED and the unassigned values.
                _ => return Err(Error::BadOpcode(byte)),
            }
        }
    }
}

impl<'s> Iterator for BindEvents<'s> {
    type Item = Result<BindEvent<'s>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.step() {
            Ok(Some(e)) => Some(Ok(e)),
            Ok(None) => {
                self.done = true;
                None
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Iterator over the pointers a bind stream fills: [`BindEvents`] without
/// the markers of non-weak definitions.
#[derive(Clone, Debug)]
pub struct BindIter<'s>(BindEvents<'s>);

impl<'s> Iterator for BindIter<'s> {
    type Item = Result<BindRecord<'s>>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.0.next()? {
                Ok(BindEvent::Bind(b)) => return Some(Ok(b)),
                Ok(BindEvent::NonWeakDefinition { .. }) => continue,
                Err(e) => return Some(Err(e)),
            }
        }
    }
}
