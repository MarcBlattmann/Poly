//! Exports (spec "The .edata Section (Image Only)"): the export address
//! table, the sorted name pointer table with its ordinal table, and
//! forwarders (export RVAs that point inside the export directory at a
//! "DLL.Name" or "DLL.#ordinal" string).

use core::cmp::Ordering;

use crate::headers::{DataDirectory, Pe};
use crate::read::{u16_at, u32_at};
use crate::view::View;
use crate::{Error, dir};

/// Size of the export directory table.
const EXPORT_DIRECTORY: usize = 40;

/// What a forwarder names in the other DLL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForwardTarget<'a> {
    Name(&'a [u8]),
    Ordinal(u16),
}

/// A forwarded export: "NTDLL.RtlAllocateHeap" means `RtlAllocateHeap` in
/// `NTDLL` (the loader adds ".dll"); "MYDLL.#12" means ordinal 12.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Forwarder<'a> {
    /// The whole string.
    pub raw: &'a [u8],
    /// The part before the last '.'.
    pub dll: &'a [u8],
    pub target: ForwardTarget<'a>,
}

impl<'a> Forwarder<'a> {
    /// Splits a forwarder string at its last '.'.
    pub fn parse(raw: &'a [u8]) -> Result<Self, Error> {
        let dot = raw.iter().rposition(|&c| c == b'.').ok_or(Error::Malformed("forwarder without a '.'"))?;
        let (dll, name) = (&raw[..dot], &raw[dot + 1..]);
        if dll.is_empty() || name.is_empty() {
            return Err(Error::Malformed("forwarder with an empty part"));
        }
        let target = match name.strip_prefix(b"#") {
            Some(digits) => ForwardTarget::Ordinal(parse_u16(digits).ok_or(Error::Malformed("forwarder ordinal"))?),
            None => ForwardTarget::Name(name),
        };
        Ok(Forwarder { raw, dll, target })
    }
}

fn parse_u16(digits: &[u8]) -> Option<u16> {
    if digits.is_empty() {
        return None;
    }
    digits.iter().try_fold(0u16, |v, &d| {
        if !d.is_ascii_digit() {
            return None;
        }
        v.checked_mul(10)?.checked_add(u16::from(d - b'0'))
    })
}

/// Where an export leads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportTarget<'a> {
    /// Code or data at this RVA in the exporting image.
    Rva(u32),
    /// Another DLL's export.
    Forwarder(Forwarder<'a>),
}

/// One export address table entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Export<'a> {
    /// The (biased) ordinal: `OrdinalBase` + index.
    pub ordinal: u32,
    pub target: ExportTarget<'a>,
}

/// The export directory (spec "Export Directory Table").
#[derive(Clone, Copy, Debug)]
pub struct Exports<'a> {
    pub characteristics: u32,
    pub time_date_stamp: u32,
    pub major_version: u16,
    pub minor_version: u16,
    pub name_rva: u32,
    pub ordinal_base: u32,
    pub number_of_functions: u32,
    pub number_of_names: u32,
    pub address_of_functions: u32,
    pub address_of_names: u32,
    pub address_of_name_ordinals: u32,
    view: View<'a>,
    range: DataDirectory,
    functions: &'a [u8],
    names: &'a [u8],
    ordinals: &'a [u8],
}

fn table<'a>(view: &View<'a>, rva: u32, count: u32, size: usize) -> Result<&'a [u8], Error> {
    if count == 0 {
        return Ok(&[]);
    }
    let len = (count as usize).checked_mul(size).ok_or(Error::Truncated)?;
    view.slice(rva, len)
}

impl<'a> Exports<'a> {
    fn parse(view: View<'a>, range: DataDirectory) -> Result<Self, Error> {
        let b = view.slice(range.virtual_address, EXPORT_DIRECTORY)?;
        let u32 = |o| u32_at(b, o).unwrap_or(0);
        let u16 = |o| u16_at(b, o).unwrap_or(0);
        let (nfuncs, nnames) = (u32(20), u32(24));
        let (afuncs, anames, aords) = (u32(28), u32(32), u32(36));
        Ok(Exports {
            characteristics: u32(0),
            time_date_stamp: u32(4),
            major_version: u16(8),
            minor_version: u16(10),
            name_rva: u32(12),
            ordinal_base: u32(16),
            number_of_functions: nfuncs,
            number_of_names: nnames,
            address_of_functions: afuncs,
            address_of_names: anames,
            address_of_name_ordinals: aords,
            view,
            range,
            functions: table(&view, afuncs, nfuncs, 4)?,
            names: table(&view, anames, nnames, 4)?,
            ordinals: table(&view, aords, nnames, 2)?,
        })
    }

    /// The DLL's own name as the linker recorded it.
    pub fn dll_name(&self) -> Result<&'a [u8], Error> {
        self.view.cstr(self.name_rva)
    }

    fn target(&self, rva: u32) -> Result<ExportTarget<'a>, Error> {
        if self.range.contains(rva) {
            return Ok(ExportTarget::Forwarder(Forwarder::parse(self.view.cstr(rva)?)?));
        }
        Ok(ExportTarget::Rva(rva))
    }

    fn by_index(&self, index: usize) -> Result<Option<ExportTarget<'a>>, Error> {
        match u32_at(self.functions, index.saturating_mul(4)) {
            None | Some(0) => Ok(None),
            Some(rva) => self.target(rva).map(Some),
        }
    }

    /// The export with this (biased) ordinal. `None` if out of range or the
    /// slot is empty.
    pub fn by_ordinal(&self, ordinal: u32) -> Result<Option<ExportTarget<'a>>, Error> {
        match ordinal.checked_sub(self.ordinal_base) {
            Some(index) => self.by_index(index as usize),
            None => Ok(None),
        }
    }

    /// The name at position `index` of the name pointer table.
    pub fn name(&self, index: usize) -> Result<&'a [u8], Error> {
        let rva = u32_at(self.names, index.checked_mul(4).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?;
        self.view.cstr(rva)
    }

    /// Index in the address table for name number `index`.
    fn name_ordinal(&self, index: usize) -> Result<usize, Error> {
        Ok(usize::from(u16_at(self.ordinals, index.checked_mul(2).ok_or(Error::Truncated)?).ok_or(Error::Truncated)?))
    }

    /// Binary search of the (lexically sorted) name pointer table.
    fn find_name(&self, name: &[u8]) -> Result<Option<usize>, Error> {
        let (mut lo, mut hi) = (0usize, self.number_of_names as usize);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            match self.name(mid)?.cmp(name) {
                Ordering::Equal => return Ok(Some(mid)),
                Ordering::Less => lo = mid + 1,
                Ordering::Greater => hi = mid,
            }
        }
        Ok(None)
    }

    /// The (biased) ordinal of an export name.
    pub fn ordinal_of(&self, name: &[u8]) -> Result<Option<u32>, Error> {
        let Some(i) = self.find_name(name)? else { return Ok(None) };
        let index = self.name_ordinal(i)? as u32;
        Ok(Some(self.ordinal_base.wrapping_add(index)))
    }

    /// The export with this name (binary search; names compare as bytes).
    pub fn by_name(&self, name: &[u8]) -> Result<Option<ExportTarget<'a>>, Error> {
        match self.find_name(name)? {
            Some(i) => self.by_index(self.name_ordinal(i)?),
            None => Ok(None),
        }
    }

    /// Like [`by_name`](Self::by_name), trying the import's hint (an index
    /// in the name pointer table) first.
    pub fn by_name_with_hint(&self, name: &[u8], hint: u16) -> Result<Option<ExportTarget<'a>>, Error> {
        let hint = usize::from(hint);
        if hint < self.number_of_names as usize && self.name(hint).is_ok_and(|n| n == name) {
            return self.by_index(self.name_ordinal(hint)?);
        }
        self.by_name(name)
    }

    /// Every non-empty export address table entry, in ordinal order.
    pub fn functions(&self) -> impl Iterator<Item = Result<Export<'a>, Error>> + 'a {
        let this = *self;
        (0..this.functions.len() / 4).filter_map(move |i| match this.by_index(i) {
            Ok(Some(target)) => Some(Ok(Export { ordinal: this.ordinal_base.wrapping_add(i as u32), target })),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        })
    }

    /// Every exported name with its (biased) ordinal, in table order.
    pub fn names(&self) -> ExportNames<'a> {
        ExportNames { exports: *self, next: 0 }
    }
}

/// Iterator over `(name, ordinal)` pairs of an export directory.
#[derive(Clone, Debug)]
pub struct ExportNames<'a> {
    exports: Exports<'a>,
    next: usize,
}

impl<'a> Iterator for ExportNames<'a> {
    type Item = Result<(&'a [u8], u32), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        let i = self.next;
        if i >= self.exports.names.len() / 4 {
            return None;
        }
        self.next += 1;
        let e = &self.exports;
        Some(e.name(i).and_then(|n| Ok((n, e.ordinal_base.wrapping_add(e.name_ordinal(i)? as u32)))))
    }
}

impl<'a> Pe<'a> {
    /// The export directory, if the image has one.
    pub fn exports(&self) -> Result<Option<Exports<'a>>, Error> {
        match self.info().directory(dir::EXPORT) {
            Some(d) => Exports::parse(self.view, d).map(Some),
            None => Ok(None),
        }
    }
}
