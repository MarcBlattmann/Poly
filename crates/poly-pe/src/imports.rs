//! Imports (spec "The .idata Section"), delay-load imports (spec
//! "Delay-Load Import Tables (Image Only)") and bound imports, plus filling
//! import address tables through a resolver.

use crate::headers::{Bits, ImageInfo, Pe};
use crate::read::{cstr, put_u32, put_u64, u16_at, u32_at};
use crate::view::{View, rva_add};
use crate::{Error, MAX_BIND_ENTRIES, dir};

/// Size of an import directory entry.
const IMPORT_DESCRIPTOR: usize = 20;
/// Size of a delay-load directory entry.
const DELAY_DESCRIPTOR: usize = 32;
/// Size of a bound import descriptor and of a forwarder reference.
const BOUND_DESCRIPTOR: usize = 8;

/// What one import names: an export by name (with a hint, the index in the
/// exporting DLL's name pointer table worth trying first) or by ordinal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSymbol<'a> {
    Name { hint: u16, name: &'a [u8] },
    Ordinal(u16),
}

impl<'a> ImportSymbol<'a> {
    /// Decodes an import lookup table entry (spec "Import Lookup Table"):
    /// the top bit chooses ordinal (low 16 bits) or hint/name table RVA (low
    /// 31 bits). The bits the spec says must be zero are checked.
    fn decode(view: &View<'a>, raw: u64, bits: Bits) -> Result<Self, Error> {
        let flag = match bits {
            Bits::Pe32Plus => 1 << 63,
            Bits::Pe32 => 1 << 31,
        };
        if raw & flag != 0 {
            if raw & !flag & !0xffff != 0 {
                return Err(Error::Malformed("ordinal import with reserved bits set"));
            }
            return Ok(ImportSymbol::Ordinal(raw as u16));
        }
        let rva = u32::try_from(raw).ok().filter(|&r| r <= 0x7fff_ffff);
        let rva = rva.ok_or(Error::Malformed("name import with reserved bits set"))?;
        let hint = view.u16(rva)?;
        let name = view.cstr(rva.checked_add(2).ok_or(Error::BadRva(rva))?)?;
        Ok(ImportSymbol::Name { hint, name })
    }
}

/// One import: its symbol and the address table slot it fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportEntry<'a> {
    /// Position in the module's lookup table.
    pub index: usize,
    /// RVA of the import address table slot for this import.
    pub iat_rva: u32,
    pub symbol: ImportSymbol<'a>,
}

/// Iterator over a lookup table, up to its zero entry. It stops after the
/// first error.
#[derive(Clone, Debug)]
pub struct ImportEntries<'a> {
    view: View<'a>,
    bits: Bits,
    lookup_rva: u32,
    iat_rva: u32,
    index: usize,
    done: bool,
}

impl<'a> ImportEntries<'a> {
    fn entry(&self) -> Result<Option<ImportEntry<'a>>, Error> {
        let size = self.bits.pointer_size();
        let raw = self.view.ptr(rva_add(self.lookup_rva, self.index, size)?, self.bits.wide())?;
        if raw == 0 {
            return Ok(None);
        }
        let symbol = ImportSymbol::decode(&self.view, raw, self.bits)?;
        Ok(Some(ImportEntry { index: self.index, iat_rva: rva_add(self.iat_rva, self.index, size)?, symbol }))
    }
}

impl<'a> Iterator for ImportEntries<'a> {
    type Item = Result<ImportEntry<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.entry() {
            Ok(Some(e)) => {
                self.index += 1;
                Some(Ok(e))
            }
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

/// An import directory entry: one DLL (spec "Import Directory Table").
#[derive(Clone, Copy, Debug)]
pub struct ImportDescriptor<'a> {
    /// RVA of the import lookup table (`OriginalFirstThunk`); zero in some
    /// old images, which then keep the names in the address table only.
    pub original_first_thunk: u32,
    /// Zero unless the image is bound: -1 (0xffffffff) for the new style
    /// with a bound import directory, else the bound DLL's time stamp.
    pub time_date_stamp: u32,
    pub forwarder_chain: u32,
    pub name_rva: u32,
    /// RVA of the import address table (`FirstThunk`).
    pub first_thunk: u32,
    view: View<'a>,
    bits: Bits,
}

impl<'a> ImportDescriptor<'a> {
    fn parse(view: View<'a>, bits: Bits, rva: u32) -> Result<Option<Self>, Error> {
        let b = view.slice(rva, IMPORT_DESCRIPTOR)?;
        let u = |o| u32_at(b, o).unwrap_or(0);
        let d = ImportDescriptor {
            original_first_thunk: u(0),
            time_date_stamp: u(4),
            forwarder_chain: u(8),
            name_rva: u(12),
            first_thunk: u(16),
            view,
            bits,
        };
        if b.iter().all(|&c| c == 0) {
            return Ok(None);
        }
        if d.name_rva == 0 || d.first_thunk == 0 {
            return Err(Error::Malformed("import descriptor without a name or address table"));
        }
        Ok(Some(d))
    }

    pub fn dll_name(&self) -> Result<&'a [u8], Error> {
        self.view.cstr(self.name_rva)
    }

    /// The image was bound to this DLL by a binder.
    pub fn is_bound(&self) -> bool {
        self.time_date_stamp != 0
    }

    /// RVA of the table to read names from: the lookup table, or the
    /// address table when there is none (only valid while unbound).
    fn lookup_rva(&self) -> Result<u32, Error> {
        match (self.original_first_thunk, self.is_bound()) {
            (0, true) => Err(Error::Malformed("bound import without a lookup table")),
            (0, false) => Ok(self.first_thunk),
            (ilt, _) => Ok(ilt),
        }
    }

    pub fn entries(&self) -> Result<ImportEntries<'a>, Error> {
        Ok(ImportEntries {
            view: self.view,
            bits: self.bits,
            lookup_rva: self.lookup_rva()?,
            iat_rva: self.first_thunk,
            index: 0,
            done: false,
        })
    }
}

/// Iterator over the import directory, up to its all-zero entry. It stops
/// after the first error.
#[derive(Clone, Debug)]
pub struct ImportDescriptors<'a> {
    view: View<'a>,
    bits: Bits,
    rva: u32,
    index: usize,
    done: bool,
}

impl<'a> Iterator for ImportDescriptors<'a> {
    type Item = Result<ImportDescriptor<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let r = rva_add(self.rva, self.index, IMPORT_DESCRIPTOR)
            .and_then(|rva| ImportDescriptor::parse(self.view, self.bits, rva));
        self.index += 1;
        match r {
            Ok(Some(d)) => Some(Ok(d)),
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

/// A delay-load directory entry (spec "Delay-Load Directory Table"). The
/// table's fields are RVAs.
#[derive(Clone, Copy, Debug)]
pub struct DelayImport<'a> {
    /// Zero per the spec; linkers set bit 0 to say "RVAs".
    pub attributes: u32,
    pub name_rva: u32,
    /// RVA of the slot the delay-load helper stores the module handle in.
    pub module_handle_rva: u32,
    /// RVA of the delay import address table.
    pub iat_rva: u32,
    /// RVA of the delay import name table (same format as a lookup table).
    pub name_table_rva: u32,
    pub bound_iat_rva: u32,
    pub unload_iat_rva: u32,
    pub time_date_stamp: u32,
    view: View<'a>,
    bits: Bits,
}

impl<'a> DelayImport<'a> {
    fn parse(view: View<'a>, bits: Bits, rva: u32) -> Result<Option<Self>, Error> {
        let b = view.slice(rva, DELAY_DESCRIPTOR)?;
        if b.iter().all(|&c| c == 0) {
            return Ok(None);
        }
        let u = |o| u32_at(b, o).unwrap_or(0);
        let d = DelayImport {
            attributes: u(0),
            name_rva: u(4),
            module_handle_rva: u(8),
            iat_rva: u(12),
            name_table_rva: u(16),
            bound_iat_rva: u(20),
            unload_iat_rva: u(24),
            time_date_stamp: u(28),
            view,
            bits,
        };
        if d.name_rva == 0 || d.iat_rva == 0 || d.name_table_rva == 0 {
            return Err(Error::Malformed("delay import without a name or tables"));
        }
        Ok(Some(d))
    }

    pub fn dll_name(&self) -> Result<&'a [u8], Error> {
        self.view.cstr(self.name_rva)
    }

    pub fn entries(&self) -> ImportEntries<'a> {
        ImportEntries {
            view: self.view,
            bits: self.bits,
            lookup_rva: self.name_table_rva,
            iat_rva: self.iat_rva,
            index: 0,
            done: false,
        }
    }
}

/// Iterator over the delay-load directory, up to its all-zero entry.
#[derive(Clone, Debug)]
pub struct DelayImports<'a> {
    view: View<'a>,
    bits: Bits,
    rva: u32,
    index: usize,
    done: bool,
}

impl<'a> Iterator for DelayImports<'a> {
    type Item = Result<DelayImport<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let r = rva_add(self.rva, self.index, DELAY_DESCRIPTOR)
            .and_then(|rva| DelayImport::parse(self.view, self.bits, rva));
        self.index += 1;
        match r {
            Ok(Some(d)) => Some(Ok(d)),
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

/// A bound import descriptor (`IMAGE_BOUND_IMPORT_DESCRIPTOR` in
/// `winnt.h`): the DLL and time stamp an image was bound against.
#[derive(Clone, Copy, Debug)]
pub struct BoundImport<'a> {
    pub time_date_stamp: u32,
    pub module_name: &'a [u8],
    forwarders: BoundForwarders<'a>,
}

impl<'a> BoundImport<'a> {
    /// The DLLs this one forwarded to when the image was bound.
    pub fn forwarders(&self) -> BoundForwarders<'a> {
        self.forwarders
    }
}

/// A bound forwarder reference (`IMAGE_BOUND_FORWARDER_REF`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoundForwarder<'a> {
    pub time_date_stamp: u32,
    pub module_name: &'a [u8],
}

/// Iterator over a bound import's forwarder references.
#[derive(Clone, Copy, Debug)]
pub struct BoundForwarders<'a> {
    table: &'a [u8],
    pos: usize,
    left: usize,
}

impl<'a> Iterator for BoundForwarders<'a> {
    type Item = Result<BoundForwarder<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.left == 0 {
            return None;
        }
        self.left -= 1;
        let r = bound_record(self.table, self.pos)
            .map(|(time_date_stamp, module_name, _)| BoundForwarder { time_date_stamp, module_name });
        self.pos += BOUND_DESCRIPTOR;
        if r.is_err() {
            self.left = 0;
        }
        Some(r)
    }
}

/// Reads an 8-byte bound import record: time stamp, name offset (from the
/// start of the table), and the third field.
fn bound_record(table: &[u8], pos: usize) -> Result<(u32, &[u8], u16), Error> {
    let ts = u32_at(table, pos).ok_or(Error::Truncated)?;
    let name = u16_at(table, pos + 4).ok_or(Error::Truncated)?;
    let third = u16_at(table, pos + 6).ok_or(Error::Truncated)?;
    Ok((ts, cstr(table, usize::from(name))?, third))
}

/// Iterator over the bound import directory, up to its all-zero entry.
#[derive(Clone, Debug)]
pub struct BoundImports<'a> {
    table: &'a [u8],
    pos: usize,
    done: bool,
}

impl<'a> Iterator for BoundImports<'a> {
    type Item = Result<BoundImport<'a>, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        match self.table.get(self.pos..self.pos.saturating_add(BOUND_DESCRIPTOR)) {
            Some(b) if b.iter().all(|&c| c == 0) => {
                self.done = true;
                return None;
            }
            None => {
                self.done = true;
                return Some(Err(Error::Truncated));
            }
            Some(_) => {}
        }
        let r = bound_record(self.table, self.pos).map(|(time_date_stamp, module_name, count)| {
            let forwarders =
                BoundForwarders { table: self.table, pos: self.pos + BOUND_DESCRIPTOR, left: count.into() };
            (BoundImport { time_date_stamp, module_name, forwarders }, count)
        });
        match r {
            Ok((b, count)) => {
                self.pos += BOUND_DESCRIPTOR * (1 + usize::from(count));
                Some(Ok(b))
            }
            Err(e) => {
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

impl<'a> Pe<'a> {
    /// The import directory, if the image has one.
    pub fn imports(&self) -> Result<Option<ImportDescriptors<'a>>, Error> {
        let Some(d) = self.info().directory(dir::IMPORT) else { return Ok(None) };
        let bits = self.info().bits();
        Ok(Some(ImportDescriptors { view: self.view, bits, rva: d.virtual_address, index: 0, done: false }))
    }

    /// The delay-load directory, if the image has one.
    pub fn delay_imports(&self) -> Result<Option<DelayImports<'a>>, Error> {
        let Some(d) = self.info().directory(dir::DELAY_IMPORT) else { return Ok(None) };
        let bits = self.info().bits();
        Ok(Some(DelayImports { view: self.view, bits, rva: d.virtual_address, index: 0, done: false }))
    }

    /// The bound import directory, if the image was bound (new style).
    /// Loaders may ignore it: [`bind_imports`] always resolves every import.
    pub fn bound_imports(&self) -> Result<Option<BoundImports<'a>>, Error> {
        let Some(d) = self.info().directory(dir::BOUND_IMPORT).filter(|d| d.size != 0) else { return Ok(None) };
        Ok(Some(BoundImports { table: self.view.slice(d.virtual_address, d.size as usize)?, pos: 0, done: false }))
    }

    /// Whether any import descriptor says the image was bound.
    pub fn is_bound(&self) -> Result<bool, Error> {
        if self.info().directory(dir::BOUND_IMPORT).is_some() {
            return Ok(true);
        }
        let Some(imports) = self.imports()? else { return Ok(false) };
        for d in imports {
            if d?.is_bound() {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// One import for a resolver to look up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportRequest<'a> {
    /// Position of the DLL in the import (or delay-load) directory, so a
    /// resolver can load each DLL once and cache it by index.
    pub module_index: usize,
    pub dll: &'a [u8],
    /// Position of the import in the DLL's lookup table.
    pub index: usize,
    pub symbol: ImportSymbol<'a>,
    /// The address table slot that receives the address.
    pub iat_rva: u32,
}

/// Why binding stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindError<E> {
    /// The image's tables are malformed.
    Image(Error),
    /// The resolver refused an import.
    Resolve { module_index: usize, index: usize, iat_rva: u32, error: E },
}

impl<E> From<Error> for BindError<E> {
    fn from(e: Error) -> Self {
        BindError::Image(e)
    }
}

#[derive(Clone, Copy)]
enum Directory {
    Import,
    Delay,
}

/// Fills every import address table slot of a mapped image with the
/// address `resolve` returns for it, DLL by DLL in directory order, and
/// returns how many slots were written.
///
/// Names are read from the lookup tables (`OriginalFirstThunk`), so a
/// bound image is simply rebound. The first error stops binding: slots
/// already written keep their new values. Addresses are full virtual
/// addresses; for a PE32 image they must fit in 32 bits.
pub fn bind_imports<E, F>(image: &mut [u8], info: &ImageInfo, resolve: F) -> Result<usize, BindError<E>>
where
    F: FnMut(&ImportRequest<'_>) -> Result<u64, E>,
{
    bind(image, info, Directory::Import, resolve)
}

/// Like [`bind_imports`], for the delay-load directory: resolves every
/// delay-loaded import now instead of on first call. The module handle slot
/// is left alone.
pub fn bind_delay_imports<E, F>(image: &mut [u8], info: &ImageInfo, resolve: F) -> Result<usize, BindError<E>>
where
    F: FnMut(&ImportRequest<'_>) -> Result<u64, E>,
{
    bind(image, info, Directory::Delay, resolve)
}

fn bind<E, F>(image: &mut [u8], info: &ImageInfo, which: Directory, mut resolve: F) -> Result<usize, BindError<E>>
where
    F: FnMut(&ImportRequest<'_>) -> Result<u64, E>,
{
    let bits = info.bits();
    let size = bits.pointer_size();
    let index = match which {
        Directory::Import => dir::IMPORT,
        Directory::Delay => dir::DELAY_IMPORT,
    };
    let Some(d) = info.directory(index) else { return Ok(0) };
    let mut written = 0usize;
    for module_index in 0.. {
        // Each step re-reads the image: the borrows must end before the
        // slot is written.
        let (name_rva, lookup_rva, iat_rva) = {
            let view = View::mapped(image);
            match which {
                Directory::Import => {
                    let rva = rva_add(d.virtual_address, module_index, IMPORT_DESCRIPTOR)?;
                    let Some(desc) = ImportDescriptor::parse(view, bits, rva)? else { break };
                    (desc.name_rva, desc.lookup_rva()?, desc.first_thunk)
                }
                Directory::Delay => {
                    let rva = rva_add(d.virtual_address, module_index, DELAY_DESCRIPTOR)?;
                    let Some(desc) = DelayImport::parse(view, bits, rva)? else { break };
                    (desc.name_rva, desc.name_table_rva, desc.iat_rva)
                }
            }
        };
        for i in 0.. {
            let slot = rva_add(iat_rva, i, size)?;
            let address = {
                let view = View::mapped(image);
                let raw = view.ptr(rva_add(lookup_rva, i, size)?, bits.wide())?;
                if raw == 0 {
                    break;
                }
                if written >= MAX_BIND_ENTRIES {
                    return Err(BindError::Image(Error::TooMany));
                }
                let symbol = ImportSymbol::decode(&view, raw, bits)?;
                let dll = view.cstr(name_rva)?;
                let req = ImportRequest { module_index, dll, index: i, symbol, iat_rva: slot };
                resolve(&req).map_err(|error| BindError::Resolve { module_index, index: i, iat_rva: slot, error })?
            };
            let at = slot as usize;
            let stored = match bits {
                Bits::Pe32Plus => put_u64(image, at, address),
                Bits::Pe32 => {
                    let a = u32::try_from(address).map_err(|_| Error::BadVa(address))?;
                    put_u32(image, at, a)
                }
            };
            stored.ok_or(Error::BadRva(slot))?;
            written += 1;
        }
    }
    Ok(written)
}
