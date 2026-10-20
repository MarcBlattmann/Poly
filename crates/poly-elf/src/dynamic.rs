//! The dynamic section of a file (`PT_DYNAMIC`): which shared objects an
//! image needs and where it asks to find them.
//!
//! The kernel never reads it (the dynamic linker does); the build tools
//! use it to find the shared libraries a test program needs, the way
//! ld.so(8) describes the search: `DT_NEEDED` names, searched in
//! `DT_RPATH` (if there is no `DT_RUNPATH`), `LD_LIBRARY_PATH`,
//! `DT_RUNPATH`, the cache and the default directories.
//!
//! Layout (gABI, "Dynamic Section"; `<elf.h>` `Elf64_Dyn`): an array of
//! 16-byte entries, a signed 64-bit tag and a 64-bit value, ending with
//! `DT_NULL`. String-valued entries are offsets into the string table that
//! `DT_STRTAB` (an address) and `DT_STRSZ` describe. Addresses are link-time
//! virtual addresses; they are turned into file offsets through the
//! `PT_LOAD` segment whose file bytes contain them.

use crate::Elf;
use crate::header::{Error, u64_at};

/// End of the dynamic array (`<elf.h>` `DT_NULL`).
pub const DT_NULL: i64 = 0;
/// Name of a needed shared object (`<elf.h>` `DT_NEEDED`).
pub const DT_NEEDED: i64 = 1;
/// Address of the string table (`<elf.h>` `DT_STRTAB`).
pub const DT_STRTAB: i64 = 5;
/// Size of the string table (`<elf.h>` `DT_STRSZ`).
pub const DT_STRSZ: i64 = 10;
/// The object's own name (`<elf.h>` `DT_SONAME`).
pub const DT_SONAME: i64 = 14;
/// Library search path, the older form (`<elf.h>` `DT_RPATH`).
pub const DT_RPATH: i64 = 15;
/// Library search path (`<elf.h>` `DT_RUNPATH`).
pub const DT_RUNPATH: i64 = 29;

/// Size of one `Elf64_Dyn`.
const DYN_SIZE: usize = 16;

/// A validated dynamic section.
#[derive(Clone, Copy, Debug)]
pub struct Dynamic<'a> {
    /// The entries before `DT_NULL`.
    entries: &'a [u8],
    strtab: &'a [u8],
}

impl<'a> Dynamic<'a> {
    /// The dynamic section of `elf`; `Ok(None)` if it has no `PT_DYNAMIC`.
    /// Fails with [`Error::BadDynamic`] if the array is not in the file or
    /// not terminated, the string table is missing or outside the file, or
    /// a string entry does not point at a NUL-terminated string in it.
    pub fn parse(elf: &Elf<'a>) -> Result<Option<Dynamic<'a>>, Error> {
        let Some(ph) = elf.dynamic() else { return Ok(None) };
        let data = elf.data();
        let start = usize::try_from(ph.offset).map_err(|_| Error::BadDynamic)?;
        let len = usize::try_from(ph.filesz).map_err(|_| Error::BadDynamic)?;
        let array = data.get(start..start.checked_add(len).ok_or(Error::BadDynamic)?).ok_or(Error::BadDynamic)?;
        let count = (0..array.len() / DYN_SIZE)
            .find(|&i| u64_at(array, i * DYN_SIZE) == Some(DT_NULL as u64))
            .ok_or(Error::BadDynamic)?;
        let entries = &array[..count * DYN_SIZE];

        let value = |tag: i64| {
            entries.chunks_exact(DYN_SIZE).find(|e| u64_at(e, 0) == Some(tag as u64)).and_then(|e| u64_at(e, 8))
        };
        let strtab = match (value(DT_STRTAB), value(DT_STRSZ)) {
            (Some(addr), Some(size)) => {
                let off = file_offset(elf, addr, size).ok_or(Error::BadDynamic)?;
                let size = usize::try_from(size).map_err(|_| Error::BadDynamic)?;
                data.get(off..off.checked_add(size).ok_or(Error::BadDynamic)?).ok_or(Error::BadDynamic)?
            }
            (None, None) => &[],
            _ => return Err(Error::BadDynamic),
        };
        let dynamic = Dynamic { entries, strtab };
        for (tag, value) in dynamic.entries() {
            if matches!(tag, DT_NEEDED | DT_SONAME | DT_RPATH | DT_RUNPATH) && dynamic.string(value).is_none() {
                return Err(Error::BadDynamic);
            }
        }
        Ok(Some(dynamic))
    }

    /// The `(tag, value)` entries before `DT_NULL`, in file order.
    pub fn entries(&self) -> impl Iterator<Item = (i64, u64)> + use<'a> {
        self.entries.chunks_exact(DYN_SIZE).filter_map(|e| Some((u64_at(e, 0)? as i64, u64_at(e, 8)?)))
    }

    /// The string at `offset` in the string table, without its NUL.
    pub fn string(&self, offset: u64) -> Option<&'a [u8]> {
        let rest = self.strtab.get(usize::try_from(offset).ok()?..)?;
        rest.get(..rest.iter().position(|&b| b == 0)?)
    }

    fn strings(&self, tag: i64) -> impl Iterator<Item = &'a [u8]> + use<'a> {
        let copy = *self;
        self.entries().filter(move |&(t, _)| t == tag).filter_map(move |(_, v)| copy.string(v))
    }

    /// The `DT_NEEDED` names, in order (the order the dynamic linker loads
    /// them in).
    pub fn needed(&self) -> impl Iterator<Item = &'a [u8]> + use<'a> {
        self.strings(DT_NEEDED)
    }

    /// `DT_SONAME`, the object's own name.
    pub fn soname(&self) -> Option<&'a [u8]> {
        self.strings(DT_SONAME).next()
    }

    /// `DT_RPATH`: colon-separated directories.
    pub fn rpath(&self) -> Option<&'a [u8]> {
        self.strings(DT_RPATH).next()
    }

    /// `DT_RUNPATH`: colon-separated directories.
    pub fn runpath(&self) -> Option<&'a [u8]> {
        self.strings(DT_RUNPATH).next()
    }
}

/// The file offset of `[addr, addr + len)`, a link-time address range
/// inside one `PT_LOAD` segment's file bytes.
fn file_offset(elf: &Elf<'_>, addr: u64, len: u64) -> Option<usize> {
    let end = addr.checked_add(len)?;
    let ph = elf.load_segments().find(|ph| addr >= ph.vaddr && end <= ph.vaddr.saturating_add(ph.filesz))?;
    usize::try_from(ph.offset.checked_add(addr - ph.vaddr)?).ok()
}
