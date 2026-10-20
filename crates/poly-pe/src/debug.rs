//! The debug directory (spec "The .debug Section") and CodeView PDB 7.0
//! records ("RSDS", `CV_INFO_PDB70` in Microsoft's public `cvinfo.h`), which
//! name the PDB file and the GUID and age a symbol server indexes it by.

use crate::headers::Pe;
use crate::read::{bytes, cstr, u16_at, u32_at};
use crate::view::Layout;
use crate::{Error, debug_type, dir};

/// Size of a debug directory entry.
const DEBUG_ENTRY: usize = 28;

/// A debug directory entry (`IMAGE_DEBUG_DIRECTORY`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugEntry {
    pub characteristics: u32,
    pub time_date_stamp: u32,
    pub major_version: u16,
    pub minor_version: u16,
    /// `IMAGE_DEBUG_TYPE_*`, see [`crate::debug_type`].
    pub kind: u32,
    pub size_of_data: u32,
    /// RVA of the data when it is mapped, else zero.
    pub address_of_raw_data: u32,
    /// File offset of the data.
    pub pointer_to_raw_data: u32,
}

/// Iterator over the debug directory's entries.
#[derive(Clone, Debug)]
pub struct DebugEntries<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Iterator for DebugEntries<'_> {
    type Item = DebugEntry;

    fn next(&mut self) -> Option<DebugEntry> {
        let b = bytes(self.bytes, self.pos, DEBUG_ENTRY)?;
        self.pos += DEBUG_ENTRY;
        let u16 = |o| u16_at(b, o).unwrap_or(0);
        let u32 = |o| u32_at(b, o).unwrap_or(0);
        Some(DebugEntry {
            characteristics: u32(0),
            time_date_stamp: u32(4),
            major_version: u16(8),
            minor_version: u16(10),
            kind: u32(12),
            size_of_data: u32(16),
            address_of_raw_data: u32(20),
            pointer_to_raw_data: u32(24),
        })
    }
}

/// A CodeView PDB 7.0 record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CodeView<'a> {
    /// The PDB's GUID as stored (the first three fields little-endian).
    pub guid: [u8; 16],
    pub age: u32,
    /// The PDB path the linker recorded (UTF-8, no NUL).
    pub path: &'a [u8],
}

impl<'a> Pe<'a> {
    /// The debug directory's entries, if the image has one.
    pub fn debug_entries(&self) -> Result<Option<DebugEntries<'a>>, Error> {
        let Some(d) = self.info().directory(dir::DEBUG).filter(|d| d.size != 0) else { return Ok(None) };
        let len = d.size as usize / DEBUG_ENTRY * DEBUG_ENTRY;
        Ok(Some(DebugEntries { bytes: self.view.slice(d.virtual_address, len)?, pos: 0 }))
    }

    /// An entry's data: from its file offset in a file, from its RVA in a
    /// mapped image. `None` if it has no data where this image keeps it.
    pub fn debug_data(&self, entry: &DebugEntry) -> Result<Option<&'a [u8]>, Error> {
        let len = entry.size_of_data as usize;
        if len == 0 {
            return Ok(None);
        }
        match (self.view.layout(), entry.address_of_raw_data, entry.pointer_to_raw_data) {
            (Layout::File, _, off) if off != 0 => {
                bytes(self.data(), off as usize, len).ok_or(Error::Truncated).map(Some)
            }
            (_, 0, _) => Ok(None),
            (_, rva, _) => self.view.slice(rva, len).map(Some),
        }
    }

    /// The entry's CodeView PDB 7.0 record, if it is one.
    pub fn codeview(&self, entry: &DebugEntry) -> Result<Option<CodeView<'a>>, Error> {
        if entry.kind != debug_type::CODEVIEW {
            return Ok(None);
        }
        let Some(b) = self.debug_data(entry)? else { return Ok(None) };
        if b.get(..4) != Some(b"RSDS") {
            return Ok(None);
        }
        let guid = bytes(b, 4, 16).ok_or(Error::Truncated)?.try_into().map_err(|_| Error::Truncated)?;
        let age = u32_at(b, 20).ok_or(Error::Truncated)?;
        Ok(Some(CodeView { guid, age, path: cstr(b, 24)? }))
    }

    /// The first CodeView PDB 7.0 record in the debug directory.
    pub fn pdb_info(&self) -> Result<Option<CodeView<'a>>, Error> {
        let Some(entries) = self.debug_entries()? else { return Ok(None) };
        for e in entries {
            if let Some(cv) = self.codeview(&e)? {
                return Ok(Some(cv));
            }
        }
        Ok(None)
    }
}
