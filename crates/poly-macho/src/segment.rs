//! `LC_SEGMENT_64` and its `section_64` records.

use core::fmt;

use crate::bytes::{u32_le, u64_le};
use crate::command::LoadCommand;
use crate::consts::*;
use crate::error::{Error, Result};

/// The part of a fixed 16-byte name before the first NUL.
fn trim_nul(name: &[u8; 16]) -> &[u8] {
    let n = name.iter().position(|&b| b == 0).unwrap_or(16);
    &name[..n]
}

/// Memory protection bits (`VM_PROT_*` of `<mach/vm_prot.h>`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Prot(pub u32);

impl Prot {
    /// Whether the memory may be read.
    pub const fn read(&self) -> bool {
        self.0 & VM_PROT_READ != 0
    }
    /// Whether the memory may be written.
    pub const fn write(&self) -> bool {
        self.0 & VM_PROT_WRITE != 0
    }
    /// Whether the memory may be executed.
    pub const fn execute(&self) -> bool {
        self.0 & VM_PROT_EXECUTE != 0
    }
}

impl fmt::Display for Prot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let c = |on: bool, ch: char| if on { ch } else { '-' };
        write!(f, "{}{}{}", c(self.read(), 'r'), c(self.write(), 'w'), c(self.execute(), 'x'))
    }
}

impl fmt::Debug for Prot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Prot({self})")
    }
}

/// A `segment_command_64`: a part of the file to map into memory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment<'a> {
    cmd: &'a [u8],
    /// The segment name, NUL padded (`__TEXT`, `__DATA`, ...).
    pub segname: [u8; 16],
    /// The memory address of the segment.
    pub vmaddr: u64,
    /// The size in memory.
    pub vmsize: u64,
    /// The file offset of the segment's contents.
    pub fileoff: u64,
    /// How many bytes of the file are mapped; the rest up to `vmsize` is
    /// zero filled.
    pub filesize: u64,
    /// The maximum protection.
    pub maxprot: Prot,
    /// The initial protection.
    pub initprot: Prot,
    /// The number of sections.
    pub nsects: u32,
    /// `SG_*` flags.
    pub flags: u32,
}

impl<'a> Segment<'a> {
    pub(crate) fn parse(lc: &LoadCommand<'a>) -> Result<Segment<'a>> {
        let d = lc.data();
        if d.len() < SEGMENT_COMMAND_64_SIZE {
            return Err(Error::CommandOverrun);
        }
        let mut segname = [0u8; 16];
        segname.copy_from_slice(&d[8..24]);
        let w = |o: usize| u32_le(d, o).ok_or(Error::CommandOverrun);
        let q = |o: usize| u64_le(d, o).ok_or(Error::CommandOverrun);
        let nsects = w(64)?;
        let expected = (nsects as usize)
            .checked_mul(SECTION_64_SIZE)
            .and_then(|n| n.checked_add(SEGMENT_COMMAND_64_SIZE))
            .ok_or(Error::BadSectionCount)?;
        if expected != d.len() {
            return Err(Error::BadSectionCount);
        }
        Ok(Segment {
            cmd: d,
            segname,
            vmaddr: q(24)?,
            vmsize: q(32)?,
            fileoff: q(40)?,
            filesize: q(48)?,
            maxprot: Prot(w(56)?),
            initprot: Prot(w(60)?),
            nsects,
            flags: w(68)?,
        })
    }

    /// The segment name without its NUL padding.
    pub fn name(&self) -> &[u8] {
        trim_nul(&self.segname)
    }

    /// Whether this is the `__PAGEZERO` segment.
    pub fn is_pagezero(&self) -> bool {
        self.name() == SEG_PAGEZERO
    }

    /// The sections of the segment.
    pub fn sections(&self) -> Sections<'a> {
        Sections { rest: &self.cmd[SEGMENT_COMMAND_64_SIZE..] }
    }

    /// Checks the segment against the file and its sections against the
    /// segment. `file_len` is the length of the Mach-O file (the slice).
    pub(crate) fn validate(&self, file_len: u64) -> Result<()> {
        let vm_end = self.vmaddr.checked_add(self.vmsize).ok_or(Error::SegmentOverflow)?;
        let file_end = self.fileoff.checked_add(self.filesize).ok_or(Error::SegmentOverflow)?;
        if self.filesize > self.vmsize {
            return Err(Error::SegmentFileSizeExceedsVmSize);
        }
        if file_end > file_len {
            return Err(Error::SegmentOutsideFile);
        }
        for s in self.sections() {
            let s_vm_end = s.addr.checked_add(s.size).ok_or(Error::SectionOutsideSegment)?;
            if s.addr < self.vmaddr || s_vm_end > vm_end {
                return Err(Error::SectionOutsideSegment);
            }
            if s.has_file_contents() && s.size != 0 {
                let s_file_end = u64::from(s.offset).checked_add(s.size).ok_or(Error::SectionOutsideSegment)?;
                if u64::from(s.offset) < self.fileoff || s_file_end > file_end {
                    return Err(Error::SectionOutsideSegment);
                }
            }
        }
        Ok(())
    }
}

/// A `section_64`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Section {
    /// The section name, NUL padded (`__text`).
    pub sectname: [u8; 16],
    /// The name of the segment the section belongs to.
    pub segname: [u8; 16],
    /// The memory address.
    pub addr: u64,
    /// The size in bytes.
    pub size: u64,
    /// The file offset of the contents (0 for zero fill sections).
    pub offset: u32,
    /// The alignment as a power of 2.
    pub align: u32,
    /// The file offset of relocation entries (object files).
    pub reloff: u32,
    /// The number of relocation entries.
    pub nreloc: u32,
    /// The section type (low byte) and attributes.
    pub flags: u32,
    /// Reserved: an index into the indirect symbol table for symbol pointer
    /// and stub sections.
    pub reserved1: u32,
    /// Reserved: the size of a stub for stub sections.
    pub reserved2: u32,
    /// Reserved.
    pub reserved3: u32,
}

impl Section {
    /// The section name without its NUL padding.
    pub fn name(&self) -> &[u8] {
        trim_nul(&self.sectname)
    }

    /// The segment name without its NUL padding.
    pub fn segment_name(&self) -> &[u8] {
        trim_nul(&self.segname)
    }

    /// The section type, `S_*` ([`SECTION_TYPE`] of `flags`).
    pub const fn section_type(&self) -> u32 {
        self.flags & SECTION_TYPE
    }

    /// Whether the section has bytes in the file (all but the zero fill
    /// types).
    pub const fn has_file_contents(&self) -> bool {
        !matches!(self.section_type(), S_ZEROFILL | S_GB_ZEROFILL | S_THREAD_LOCAL_ZEROFILL)
    }
}

/// Iterator over the sections of a [`Segment`].
#[derive(Clone, Debug)]
pub struct Sections<'a> {
    rest: &'a [u8],
}

impl Iterator for Sections<'_> {
    type Item = Section;

    fn next(&mut self) -> Option<Section> {
        if self.rest.len() < SECTION_64_SIZE {
            return None;
        }
        let (d, rest) = self.rest.split_at(SECTION_64_SIZE);
        self.rest = rest;
        let mut sectname = [0u8; 16];
        sectname.copy_from_slice(&d[0..16]);
        let mut segname = [0u8; 16];
        segname.copy_from_slice(&d[16..32]);
        Some(Section {
            sectname,
            segname,
            addr: u64_le(d, 32)?,
            size: u64_le(d, 40)?,
            offset: u32_le(d, 48)?,
            align: u32_le(d, 52)?,
            reloff: u32_le(d, 56)?,
            nreloc: u32_le(d, 60)?,
            flags: u32_le(d, 64)?,
            reserved1: u32_le(d, 68)?,
            reserved2: u32_le(d, 72)?,
            reserved3: u32_le(d, 76)?,
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.rest.len() / SECTION_64_SIZE;
        (n, Some(n))
    }
}

impl ExactSizeIterator for Sections<'_> {}
