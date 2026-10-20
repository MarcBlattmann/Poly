//! A small PE writer for the tests: headers, sections and the directory
//! contents (imports, exports, relocations, TLS, resources, debug, pdata),
//! all built byte by byte from the spec's layouts.

extern crate std;

use std::string::String;
use std::vec::Vec;

use crate::{Bits, dir, machine, section_flags as scn};

pub const NT_OFFSET: usize = 0x80;
pub const SECTION_ALIGN: u32 = 0x1000;
pub const FILE_ALIGN: u32 = 0x200;
/// Every built image has this much header space, so the first section is
/// always at RVA 0x1000.
pub const HEADERS: u32 = 0x400;

pub const CODE: u32 = scn::CNT_CODE | scn::MEM_EXECUTE | scn::MEM_READ;
pub const RDATA: u32 = scn::CNT_INITIALIZED_DATA | scn::MEM_READ;
pub const DATA: u32 = scn::CNT_INITIALIZED_DATA | scn::MEM_READ | scn::MEM_WRITE;
pub const BSS: u32 = scn::CNT_UNINITIALIZED_DATA | scn::MEM_READ | scn::MEM_WRITE;

pub fn put16(b: &mut Vec<u8>, at: usize, v: u16) {
    grow(b, at + 2);
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn put32(b: &mut Vec<u8>, at: usize, v: u32) {
    grow(b, at + 4);
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn put64(b: &mut Vec<u8>, at: usize, v: u64) {
    grow(b, at + 8);
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

pub fn put_ptr(b: &mut Vec<u8>, at: usize, v: u64, bits: Bits) {
    match bits {
        Bits::Pe32Plus => put64(b, at, v),
        Bits::Pe32 => put32(b, at, v as u32),
    }
}

pub fn put_bytes(b: &mut Vec<u8>, at: usize, v: &[u8]) {
    grow(b, at + v.len());
    b[at..at + v.len()].copy_from_slice(v);
}

pub fn grow(b: &mut Vec<u8>, len: usize) {
    if b.len() < len {
        b.resize(len, 0);
    }
}

fn align(v: usize, a: usize) -> usize {
    v.div_ceil(a) * a
}

pub struct Section {
    pub name: [u8; 8],
    pub rva: u32,
    pub data: Vec<u8>,
    pub virtual_size: u32,
    pub characteristics: u32,
}

pub struct PeBuilder {
    pub bits: Bits,
    pub machine: u16,
    pub image_base: u64,
    pub characteristics: u16,
    pub dll_characteristics: u16,
    pub subsystem: u16,
    pub entry: u32,
    pub sections: Vec<Section>,
    pub dirs: [(u32, u32); dir::COUNT],
    next_rva: u32,
}

impl PeBuilder {
    pub fn new(bits: Bits) -> Self {
        let (machine, image_base) = match bits {
            Bits::Pe32Plus => (machine::AMD64, 0x1_4000_0000),
            Bits::Pe32 => (machine::I386, 0x40_0000),
        };
        PeBuilder {
            bits,
            machine,
            image_base,
            characteristics: crate::file_flags::EXECUTABLE_IMAGE | crate::file_flags::LARGE_ADDRESS_AWARE,
            dll_characteristics: crate::dll_flags::DYNAMIC_BASE | crate::dll_flags::NX_COMPAT,
            subsystem: crate::subsystem::WINDOWS_CUI,
            entry: 0,
            sections: Vec::new(),
            dirs: [(0, 0); dir::COUNT],
            next_rva: SECTION_ALIGN,
        }
    }

    pub fn ptr(&self) -> usize {
        self.bits.pointer_size()
    }

    /// RVA the next section will get.
    pub fn next_rva(&self) -> u32 {
        self.next_rva
    }

    /// Adds a section; its memory size is `virtual_size` or the data size.
    pub fn section(&mut self, name: &str, characteristics: u32, data: Vec<u8>, virtual_size: Option<u32>) -> u32 {
        let rva = self.next_rva;
        let mut n = [0u8; 8];
        n[..name.len()].copy_from_slice(name.as_bytes());
        let vsize = virtual_size.unwrap_or(data.len().max(1) as u32);
        self.next_rva = rva + align(vsize as usize, SECTION_ALIGN as usize) as u32;
        self.sections.push(Section { name: n, rva, data, virtual_size: vsize, characteristics });
        rva
    }

    pub fn dir(&mut self, index: usize, rva: u32, size: u32) {
        self.dirs[index] = (rva, size);
    }

    pub fn opt_size(&self) -> usize {
        match self.bits {
            Bits::Pe32Plus => 112 + 128,
            Bits::Pe32 => 96 + 128,
        }
    }

    pub fn build(&self) -> Vec<u8> {
        let mut f = Vec::new();
        put_bytes(&mut f, 0, b"MZ");
        put32(&mut f, 0x3c, NT_OFFSET as u32);
        put_bytes(&mut f, NT_OFFSET, b"PE\0\0");
        let fh = NT_OFFSET + 4;
        put16(&mut f, fh, self.machine);
        put16(&mut f, fh + 2, self.sections.len() as u16);
        put32(&mut f, fh + 4, 0x5eed_0001);
        put16(&mut f, fh + 16, self.opt_size() as u16);
        put16(&mut f, fh + 18, self.characteristics);

        let oh = fh + 20;
        let wide = self.bits == Bits::Pe32Plus;
        put16(&mut f, oh, if wide { 0x20b } else { 0x10b });
        grow(&mut f, oh + 4);
        f[oh + 2] = 14;
        let code: u32 =
            self.sections.iter().filter(|s| s.characteristics & scn::CNT_CODE != 0).map(|s| s.data.len() as u32).sum();
        put32(&mut f, oh + 4, code);
        put32(&mut f, oh + 16, self.entry);
        put32(&mut f, oh + 20, SECTION_ALIGN);
        if wide {
            put64(&mut f, oh + 24, self.image_base);
        } else {
            put32(&mut f, oh + 24, 0x2000);
            put32(&mut f, oh + 28, self.image_base as u32);
        }
        put32(&mut f, oh + 32, SECTION_ALIGN);
        put32(&mut f, oh + 36, FILE_ALIGN);
        put16(&mut f, oh + 40, 6);
        put16(&mut f, oh + 48, 6);
        put32(&mut f, oh + 56, self.next_rva);
        put32(&mut f, oh + 60, HEADERS);
        put16(&mut f, oh + 68, self.subsystem);
        put16(&mut f, oh + 70, self.dll_characteristics);
        let p = self.ptr();
        put_ptr(&mut f, oh + 72, 0x10_0000, self.bits);
        put_ptr(&mut f, oh + 72 + p, 0x1000, self.bits);
        put_ptr(&mut f, oh + 72 + 2 * p, 0x10_0000, self.bits);
        put_ptr(&mut f, oh + 72 + 3 * p, 0x1000, self.bits);
        let rest = if wide { 104 } else { 88 };
        put32(&mut f, oh + rest + 4, dir::COUNT as u32);
        for (i, (rva, size)) in self.dirs.iter().enumerate() {
            put32(&mut f, oh + rest + 8 + i * 8, *rva);
            put32(&mut f, oh + rest + 12 + i * 8, *size);
        }

        let table = oh + self.opt_size();
        assert!(table + 40 * self.sections.len() <= HEADERS as usize, "too many sections for the test headers");
        let mut raw = HEADERS as usize;
        grow(&mut f, raw);
        for (i, s) in self.sections.iter().enumerate() {
            let h = table + i * 40;
            put_bytes(&mut f, h, &s.name);
            put32(&mut f, h + 8, s.virtual_size);
            put32(&mut f, h + 12, s.rva);
            let raw_size = align(s.data.len(), FILE_ALIGN as usize);
            put32(&mut f, h + 16, raw_size as u32);
            put32(&mut f, h + 20, if raw_size == 0 { 0 } else { raw as u32 });
            put32(&mut f, h + 36, s.characteristics);
            if raw_size != 0 {
                put_bytes(&mut f, raw, &s.data);
                raw += raw_size;
                grow(&mut f, raw);
            }
        }
        f
    }
}

/// An import for [`imports`].
#[derive(Clone, Copy, Debug)]
pub enum Sym<'a> {
    Name(u16, &'a str),
    Ordinal(u16),
}

/// Builds an import section at `base`: descriptors, lookup tables, address
/// tables (holding the lookup values, as on disk), hint/name entries and
/// DLL names. Returns the bytes, the directory size and each DLL's IAT RVA.
pub fn imports(base: u32, bits: Bits, dlls: &[(&str, &[Sym<'_>])]) -> (Vec<u8>, u32, Vec<u32>) {
    let p = bits.pointer_size();
    let mut b = Vec::new();
    let desc_size = (dlls.len() + 1) * 20;
    let mut cursor = desc_size;
    let mut tables = Vec::new();
    for (_, syms) in dlls {
        let ilt = cursor;
        cursor += (syms.len() + 1) * p;
        let iat = cursor;
        cursor += (syms.len() + 1) * p;
        tables.push((ilt, iat));
    }
    let mut iats = Vec::new();
    for (i, (name, syms)) in dlls.iter().enumerate() {
        let (ilt, iat) = tables[i];
        for (j, s) in syms.iter().enumerate() {
            let value = match s {
                Sym::Ordinal(n) => (1u64 << (p * 8 - 1)) | u64::from(*n),
                Sym::Name(hint, n) => {
                    let at = cursor;
                    put16(&mut b, at, *hint);
                    put_bytes(&mut b, at + 2, n.as_bytes());
                    cursor = align(at + 2 + n.len() + 1, 2);
                    u64::from(base) + at as u64
                }
            };
            put_ptr(&mut b, ilt + j * p, value, bits);
            put_ptr(&mut b, iat + j * p, value, bits);
        }
        let name_at = cursor;
        put_bytes(&mut b, name_at, name.as_bytes());
        cursor += name.len() + 1;
        put32(&mut b, i * 20, base + ilt as u32);
        put32(&mut b, i * 20 + 12, base + name_at as u32);
        put32(&mut b, i * 20 + 16, base + iat as u32);
        iats.push(base + iat as u32);
    }
    grow(&mut b, cursor);
    (b, desc_size as u32, iats)
}

/// An export for [`exports`].
#[derive(Clone, Copy, Debug)]
pub enum Target<'a> {
    Rva(u32),
    Forward(&'a str),
    Empty,
}

/// Builds an export section at `base`. `functions[i]` gets ordinal
/// `ordinal_base + i`; `names` pairs a name with a function index. The
/// directory covers the whole section, so forwarder strings are inside it.
pub fn exports(base: u32, dll: &str, ordinal_base: u32, functions: &[Target<'_>], names: &[(&str, u16)]) -> Vec<u8> {
    let mut names: Vec<(&str, u16)> = names.to_vec();
    names.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut b = Vec::new();
    let eat = 40;
    let npt = eat + functions.len() * 4;
    let ords = npt + names.len() * 4;
    let mut cursor = ords + names.len() * 2;
    let dll_at = cursor;
    put_bytes(&mut b, dll_at, dll.as_bytes());
    cursor += dll.len() + 1;
    for (i, f) in functions.iter().enumerate() {
        let v = match f {
            Target::Rva(r) => *r,
            Target::Empty => 0,
            Target::Forward(s) => {
                let at = cursor;
                put_bytes(&mut b, at, s.as_bytes());
                cursor += s.len() + 1;
                base + at as u32
            }
        };
        put32(&mut b, eat + i * 4, v);
    }
    for (i, (name, index)) in names.iter().enumerate() {
        let at = cursor;
        put_bytes(&mut b, at, name.as_bytes());
        cursor += name.len() + 1;
        put32(&mut b, npt + i * 4, base + at as u32);
        put16(&mut b, ords + i * 2, *index);
    }
    grow(&mut b, cursor + 1);
    put32(&mut b, 12, base + dll_at as u32);
    put32(&mut b, 16, ordinal_base);
    put32(&mut b, 20, functions.len() as u32);
    put32(&mut b, 24, names.len() as u32);
    put32(&mut b, 28, base + eat as u32);
    put32(&mut b, 32, base + npt as u32);
    put32(&mut b, 36, base + ords as u32);
    b
}

/// Builds a base relocation table from `(rva, kind)` pairs, one block per
/// 4 KiB page, each padded to 4 bytes with an ABSOLUTE entry.
pub fn relocs(entries: &[(u32, u8)]) -> Vec<u8> {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|e| e.0);
    let mut b = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let page = sorted[i].0 & !0xfff;
        let start = b.len();
        put32(&mut b, start, page);
        let mut n = 0;
        while i < sorted.len() && sorted[i].0 & !0xfff == page {
            let (rva, kind) = sorted[i];
            put16(&mut b, start + 8 + n * 2, (u16::from(kind) << 12) | (rva & 0xfff) as u16);
            n += 1;
            i += 1;
        }
        if n % 2 == 1 {
            put16(&mut b, start + 8 + n * 2, 0);
            n += 1;
        }
        put32(&mut b, start + 4, (8 + n * 2) as u32);
    }
    b
}

/// A resource leaf for [`resources`]: type, name, language, data.
pub struct Res<'a> {
    pub kind: ResId<'a>,
    pub name: ResId<'a>,
    pub lang: u32,
    pub data: &'a [u8],
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResId<'a> {
    Name(&'a str),
    Id(u32),
}

/// Builds a three-level resource tree at `base` (types, names, languages).
pub fn resources(base: u32, leaves: &[Res<'_>]) -> Vec<u8> {
    fn table(b: &mut Vec<u8>, at: usize, ids: &[ResId<'_>]) {
        let named = ids.iter().filter(|i| matches!(i, ResId::Name(_))).count();
        put16(b, at + 12, named as u16);
        put16(b, at + 14, (ids.len() - named) as u16);
    }
    fn uniq<'a>(mut v: Vec<ResId<'a>>) -> Vec<ResId<'a>> {
        v.sort();
        v.dedup();
        v
    }
    let mut b = Vec::new();
    let types = uniq(leaves.iter().map(|l| l.kind).collect());
    // Layout: all tables first (computed sizes), then strings, then data
    // entries and data. Allocate as we go with a cursor.
    let mut cursor = 16 + types.len() * 8;
    table(&mut b, 0, &types);
    let mut strings: Vec<(usize, String)> = Vec::new();
    let mut leaf_entries: Vec<(usize, &Res<'_>)> = Vec::new();
    let name_key = |b: &mut Vec<u8>, at: usize, id: ResId<'_>, strings: &mut Vec<(usize, String)>| match id {
        ResId::Id(n) => put32(b, at, n),
        ResId::Name(s) => strings.push((at, String::from(s))),
    };
    for (ti, t) in types.iter().enumerate() {
        name_key(&mut b, 16 + ti * 8, *t, &mut strings);
        let names = uniq(leaves.iter().filter(|l| l.kind == *t).map(|l| l.name).collect());
        let nt = cursor;
        put32(&mut b, 16 + ti * 8 + 4, 0x8000_0000 | nt as u32);
        table(&mut b, nt, &names);
        cursor += 16 + names.len() * 8;
        for (ni, n) in names.iter().enumerate() {
            name_key(&mut b, nt + 16 + ni * 8, *n, &mut strings);
            let mut langs: Vec<&Res<'_>> = leaves.iter().filter(|l| l.kind == *t && l.name == *n).collect();
            langs.sort_by_key(|l| l.lang);
            let lt = cursor;
            put32(&mut b, nt + 16 + ni * 8 + 4, 0x8000_0000 | lt as u32);
            put16(&mut b, lt + 14, langs.len() as u16);
            cursor += 16 + langs.len() * 8;
            for (li, l) in langs.iter().enumerate() {
                put32(&mut b, lt + 16 + li * 8, l.lang);
                leaf_entries.push((lt + 16 + li * 8 + 4, l));
            }
        }
    }
    for (at, s) in strings {
        put32(&mut b, at, 0x8000_0000 | cursor as u32);
        let units: Vec<u16> = s.encode_utf16().collect();
        put16(&mut b, cursor, units.len() as u16);
        for (i, u) in units.iter().enumerate() {
            put16(&mut b, cursor + 2 + i * 2, *u);
        }
        cursor = align(cursor + 2 + units.len() * 2, 4);
    }
    for (at, l) in leaf_entries {
        let entry = cursor;
        put32(&mut b, at, entry as u32);
        let data_at = entry + 16;
        put32(&mut b, entry, base + data_at as u32);
        put32(&mut b, entry + 4, l.data.len() as u32);
        put_bytes(&mut b, data_at, l.data);
        cursor = align(data_at + l.data.len(), 4);
    }
    grow(&mut b, cursor);
    b
}
