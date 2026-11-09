//! `LC_SYMTAB` with its `nlist_64` entries and string table, and
//! `LC_DYSYMTAB` with the indirect symbol table.

use crate::bytes::{cstr, slice, u16_le, u32_le, u64_le};
use crate::consts::*;
use crate::error::{Error, Result};

/// Longest symbol name accepted, terminating NUL excluded.
pub const MAX_NAME_LEN: usize = 0x1_0000;

/// A `symtab_command`: where the symbols and their names are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SymtabCommand {
    /// File offset of the symbol table.
    pub symoff: u32,
    /// The number of `nlist_64` entries.
    pub nsyms: u32,
    /// File offset of the string table.
    pub stroff: u32,
    /// Size of the string table in bytes.
    pub strsize: u32,
}

/// What an `nlist_64` entry is, from its `n_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    /// A debugging entry (`N_STAB` bits set).
    Stab,
    /// Undefined (`N_UNDF`): defined in another image.
    Undefined,
    /// Absolute (`N_ABS`): `n_value` is not an address.
    Absolute,
    /// Defined in section `n_sect` (`N_SECT`).
    Section,
    /// Indirect (`N_INDR`): an alias, `n_value` indexes the string table.
    Indirect,
    /// Prebound undefined (`N_PBUD`).
    PreboundUndefined,
    /// Any other `n_type`.
    Unknown,
}

/// A `struct nlist_64`: one symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Nlist {
    /// Index of the name in the string table.
    pub n_strx: u32,
    /// `N_*` type bits.
    pub n_type: u8,
    /// The section number (1-based) for `N_SECT`, `NO_SECT` otherwise.
    pub n_sect: u8,
    /// `n_desc`: library ordinal, weak flags and more (`<mach-o/nlist.h>`).
    pub n_desc: u16,
    /// The value: an address for defined symbols.
    pub n_value: u64,
}

impl Nlist {
    /// What kind of symbol this is.
    pub const fn kind(&self) -> SymbolKind {
        if self.n_type & N_STAB != 0 {
            return SymbolKind::Stab;
        }
        match self.n_type & N_TYPE {
            N_UNDF => SymbolKind::Undefined,
            N_ABS => SymbolKind::Absolute,
            N_SECT => SymbolKind::Section,
            N_INDR => SymbolKind::Indirect,
            N_PBUD => SymbolKind::PreboundUndefined,
            _ => SymbolKind::Unknown,
        }
    }

    /// Whether the symbol is external (`N_EXT`).
    pub const fn is_external(&self) -> bool {
        self.n_type & N_EXT != 0
    }

    /// Whether the symbol is private external (`N_PEXT`).
    pub const fn is_private_external(&self) -> bool {
        self.n_type & N_PEXT != 0
    }

    /// Whether the symbol is undefined.
    pub const fn is_undefined(&self) -> bool {
        self.n_type & N_STAB == 0 && self.n_type & N_TYPE == N_UNDF
    }

    /// The library ordinal an undefined symbol was bound to
    /// (`GET_LIBRARY_ORDINAL`): 1-based, with the special values
    /// [`SELF_LIBRARY_ORDINAL`], [`DYNAMIC_LOOKUP_ORDINAL`] and
    /// [`EXECUTABLE_ORDINAL`].
    pub const fn library_ordinal(&self) -> u8 {
        ((self.n_desc >> 8) & 0xff) as u8
    }

    /// Whether a defined symbol is a weak definition (`N_WEAK_DEF`).
    pub const fn is_weak_def(&self) -> bool {
        self.n_desc & N_WEAK_DEF != 0
    }

    /// Whether an undefined symbol is a weak reference (`N_WEAK_REF`).
    pub const fn is_weak_ref(&self) -> bool {
        self.n_desc & N_WEAK_REF != 0
    }
}

/// The symbol table and string table of an image, borrowed from the file.
#[derive(Clone, Copy, Debug)]
pub struct Symtab<'a> {
    symbols: &'a [u8],
    strings: &'a [u8],
    count: u32,
}

impl<'a> Symtab<'a> {
    /// Locates both tables in `file`; fails if either lies outside it.
    pub(crate) fn new(file: &'a [u8], cmd: &SymtabCommand) -> Result<Symtab<'a>> {
        let bytes = u64::from(cmd.nsyms) * NLIST_64_SIZE as u64;
        let symbols = slice(file, u64::from(cmd.symoff), bytes).ok_or(Error::TableOutsideFile)?;
        let strings = slice(file, u64::from(cmd.stroff), u64::from(cmd.strsize)).ok_or(Error::TableOutsideFile)?;
        Ok(Symtab { symbols, strings, count: cmd.nsyms })
    }

    /// The number of symbols.
    pub const fn len(&self) -> u32 {
        self.count
    }

    /// Whether there are no symbols.
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The string table's bytes.
    pub const fn strings(&self) -> &'a [u8] {
        self.strings
    }

    /// The symbol at `index`.
    pub fn get(&self, index: u32) -> Result<Nlist> {
        if index >= self.count {
            return Err(Error::SymbolOutOfRange);
        }
        let base = index as usize * NLIST_64_SIZE;
        let d = self.symbols;
        let truncated = Error::TableOutsideFile;
        Ok(Nlist {
            n_strx: u32_le(d, base).ok_or(truncated)?,
            n_type: *d.get(base + 4).ok_or(truncated)?,
            n_sect: *d.get(base + 5).ok_or(truncated)?,
            n_desc: u16_le(d, base + 6).ok_or(truncated)?,
            n_value: u64_le(d, base + 8).ok_or(truncated)?,
        })
    }

    /// The symbols in table order.
    pub fn iter(&self) -> Symbols<'a> {
        Symbols { table: *self, next: 0 }
    }

    /// The name at `strx` in the string table. Index 0 is the empty name.
    pub fn name_at(&self, strx: u32) -> Result<&'a [u8]> {
        if strx == 0 {
            return Ok(&[]);
        }
        if strx as usize >= self.strings.len() {
            return Err(Error::StringOutOfRange);
        }
        cstr(self.strings, strx as usize, MAX_NAME_LEN).ok_or(Error::BadString)
    }

    /// The name of a symbol.
    pub fn name(&self, sym: &Nlist) -> Result<&'a [u8]> {
        self.name_at(sym.n_strx)
    }
}

/// Iterator over the symbols of a [`Symtab`].
#[derive(Clone, Debug)]
pub struct Symbols<'a> {
    table: Symtab<'a>,
    next: u32,
}

impl Iterator for Symbols<'_> {
    type Item = Nlist;

    fn next(&mut self) -> Option<Nlist> {
        let s = self.table.get(self.next).ok()?;
        self.next += 1;
        Some(s)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.table.count.saturating_sub(self.next) as usize;
        (n, Some(n))
    }
}

/// A `dysymtab_command`: how the symbol table is grouped, and where the
/// indirect symbol table is. Field names are those of the header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct DysymtabCommand {
    pub ilocalsym: u32,
    pub nlocalsym: u32,
    pub iextdefsym: u32,
    pub nextdefsym: u32,
    pub iundefsym: u32,
    pub nundefsym: u32,
    pub tocoff: u32,
    pub ntoc: u32,
    pub modtaboff: u32,
    pub nmodtab: u32,
    pub extrefsymoff: u32,
    pub nextrefsyms: u32,
    pub indirectsymoff: u32,
    pub nindirectsyms: u32,
    pub extreloff: u32,
    pub nextrel: u32,
    pub locreloff: u32,
    pub nlocrel: u32,
}

impl DysymtabCommand {
    pub(crate) fn from_words(w: [u32; 18]) -> Self {
        DysymtabCommand {
            ilocalsym: w[0],
            nlocalsym: w[1],
            iextdefsym: w[2],
            nextdefsym: w[3],
            iundefsym: w[4],
            nundefsym: w[5],
            tocoff: w[6],
            ntoc: w[7],
            modtaboff: w[8],
            nmodtab: w[9],
            extrefsymoff: w[10],
            nextrefsyms: w[11],
            indirectsymoff: w[12],
            nindirectsyms: w[13],
            extreloff: w[14],
            nextrel: w[15],
            locreloff: w[16],
            nlocrel: w[17],
        }
    }
}

/// The dynamic symbol table: ranges into the symbol table and the indirect
/// symbol table, all checked against the file and the symbol count.
#[derive(Clone, Copy, Debug)]
pub struct Dysymtab<'a> {
    cmd: DysymtabCommand,
    indirect: &'a [u8],
}

impl<'a> Dysymtab<'a> {
    pub(crate) fn new(file: &'a [u8], cmd: &DysymtabCommand, nsyms: u32) -> Result<Self> {
        let in_symtab = |first: u32, n: u32| {
            if u64::from(first) + u64::from(n) > u64::from(nsyms) { Err(Error::BadSymbolRange) } else { Ok(()) }
        };
        in_symtab(cmd.ilocalsym, cmd.nlocalsym)?;
        in_symtab(cmd.iextdefsym, cmd.nextdefsym)?;
        in_symtab(cmd.iundefsym, cmd.nundefsym)?;
        // The legacy tables (contents, modules, references, relocations):
        // only their bounds are checked, they are not read.
        for (off, n, size) in [
            (cmd.tocoff, cmd.ntoc, 8u64),
            (cmd.modtaboff, cmd.nmodtab, 56),
            (cmd.extrefsymoff, cmd.nextrefsyms, 4),
            (cmd.extreloff, cmd.nextrel, 8),
            (cmd.locreloff, cmd.nlocrel, 8),
        ] {
            if n != 0 {
                slice(file, u64::from(off), u64::from(n) * size).ok_or(Error::TableOutsideFile)?;
            }
        }
        let indirect = slice(file, u64::from(cmd.indirectsymoff), u64::from(cmd.nindirectsyms) * 4)
            .ok_or(Error::TableOutsideFile)?;
        Ok(Dysymtab { cmd: *cmd, indirect })
    }

    /// The command's fields.
    pub const fn command(&self) -> &DysymtabCommand {
        &self.cmd
    }

    /// Indexes of the local symbols.
    pub fn local_range(&self) -> core::ops::Range<u32> {
        self.cmd.ilocalsym..self.cmd.ilocalsym + self.cmd.nlocalsym
    }

    /// Indexes of the externally defined symbols.
    pub fn extdef_range(&self) -> core::ops::Range<u32> {
        self.cmd.iextdefsym..self.cmd.iextdefsym + self.cmd.nextdefsym
    }

    /// Indexes of the undefined symbols.
    pub fn undef_range(&self) -> core::ops::Range<u32> {
        self.cmd.iundefsym..self.cmd.iundefsym + self.cmd.nundefsym
    }

    /// The indirect symbol table: one entry per stub or symbol pointer.
    pub fn indirect_symbols(&self) -> IndirectSymbols<'a> {
        IndirectSymbols { rest: self.indirect }
    }
}

/// An entry of the indirect symbol table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndirectSymbol(pub u32);

impl IndirectSymbol {
    /// Whether the entry is `INDIRECT_SYMBOL_LOCAL`: a defined symbol that
    /// was stripped.
    pub const fn is_local(&self) -> bool {
        self.0 & INDIRECT_SYMBOL_LOCAL != 0
    }

    /// Whether the entry is also `INDIRECT_SYMBOL_ABS`.
    pub const fn is_abs(&self) -> bool {
        self.0 & INDIRECT_SYMBOL_ABS != 0
    }

    /// The symbol table index, or `None` for a local or absolute marker.
    pub const fn symbol_index(&self) -> Option<u32> {
        if self.0 & (INDIRECT_SYMBOL_LOCAL | INDIRECT_SYMBOL_ABS) != 0 { None } else { Some(self.0) }
    }
}

/// Iterator over the indirect symbol table.
#[derive(Clone, Debug)]
pub struct IndirectSymbols<'a> {
    rest: &'a [u8],
}

impl Iterator for IndirectSymbols<'_> {
    type Item = IndirectSymbol;

    fn next(&mut self) -> Option<IndirectSymbol> {
        let v = u32_le(self.rest, 0)?;
        self.rest = &self.rest[4..];
        Some(IndirectSymbol(v))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.rest.len() / 4;
        (n, Some(n))
    }
}

impl ExactSizeIterator for Symbols<'_> {}
impl ExactSizeIterator for IndirectSymbols<'_> {}
