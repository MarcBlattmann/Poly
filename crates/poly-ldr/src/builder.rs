//! A PE32+ image builder for the tests: executables and DLLs with exports
//! (by name, by ordinal, forwarders), imports (by name with a hint, by
//! ordinal, new-style bound imports), delay-load imports, a TLS directory
//! with callbacks, base relocations and an exception directory. Poly's own
//! tool, from the PE Format specification (Learn: "PE Format"): no
//! Microsoft toolchain is involved.
//!
//! Layout: the headers in the first page; the caller's sections from RVA
//! 0x1000, each on its own page; then one generated section `.poly` (read
//! and write) with every table, at [`PeBuilder::generated_rva`] if given
//! (so hand-assembled code can address the tables), else on the next page;
//! then `.reloc`. File and section alignment are both 4 KiB, so the file is
//! laid out as the image is mapped. In `.poly`, in this order: the import
//! address tables (one per imported DLL, each ending in a zero slot), the
//! delay-load address tables, the delay-load module handle slots, the TLS
//! index slot, then everything else. [`Layout`] gives the addresses.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

const ALIGN: u32 = 0x1000;
const PE_OFFSET: usize = 0x40;
const OPTIONAL_HEADER_SIZE: usize = 240;

/// `IMAGE_SCN_*` combinations.
pub const TEXT: u32 = 0x6000_0020; // CNT_CODE | MEM_EXECUTE | MEM_READ
pub const DATA: u32 = 0xC000_0040; // CNT_INITIALIZED_DATA | MEM_READ | MEM_WRITE
pub const RDATA: u32 = 0x4000_0040; // CNT_INITIALIZED_DATA | MEM_READ
const RELOC: u32 = 0x4200_0040; // + MEM_DISCARDABLE

/// One of the caller's sections.
#[derive(Clone, Debug)]
pub struct Section {
    pub name: [u8; 8],
    pub data: Vec<u8>,
    /// At least `data.len()`.
    pub virtual_size: u32,
    pub characteristics: u32,
}

impl Section {
    pub fn new(name: &str, data: Vec<u8>, characteristics: u32) -> Section {
        let mut n = [0u8; 8];
        n[..name.len().min(8)].copy_from_slice(&name.as_bytes()[..name.len().min(8)]);
        let virtual_size = data.len().max(1) as u32;
        Section { name: n, data, virtual_size, characteristics }
    }
}

/// An imported symbol.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sym {
    Name { name: String, hint: u16 },
    Ordinal(u16),
}

impl Sym {
    pub fn name(n: &str) -> Sym {
        Sym::Name { name: n.into(), hint: 0 }
    }
}

/// One imported DLL.
#[derive(Clone, Debug, Default)]
pub struct Import {
    pub dll: String,
    pub symbols: Vec<Sym>,
    /// New-style binding: the bound DLL's time stamp and the addresses the
    /// address table holds.
    pub bound: Option<(u32, Vec<u64>)>,
    /// For delay-load imports: the RVA each slot starts out pointing at
    /// (the thunk that calls the helper). Relocated.
    pub delay_thunks: Vec<u32>,
}

/// Where an export leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Rva(u32),
    /// "DLL.Name" or "DLL.#ordinal".
    Forward(String),
}

/// One export.
#[derive(Clone, Debug)]
pub struct Export {
    pub name: Option<String>,
    pub ordinal: u32,
    pub target: Target,
}

/// The TLS directory.
#[derive(Clone, Debug, Default)]
pub struct Tls {
    pub template: Vec<u8>,
    pub zero_fill: u32,
    /// RVAs of the callbacks.
    pub callbacks: Vec<u32>,
    /// `IMAGE_SCN_ALIGN_*` value for `Characteristics` (0: none).
    pub align_flag: u32,
}

/// An image to write.
#[derive(Clone, Debug)]
pub struct PeBuilder {
    pub image_base: u64,
    pub dll: bool,
    /// RVA of the entry point (0: none).
    pub entry: u32,
    /// `IMAGE_FILE_RELOCS_STRIPPED`.
    pub fixed: bool,
    pub machine: u16,
    pub time_date_stamp: u32,
    pub checksum: u32,
    pub sections: Vec<Section>,
    /// The DLL name the export directory records.
    pub export_name: String,
    pub ordinal_base: u32,
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
    pub delay_imports: Vec<Import>,
    pub tls: Option<Tls>,
    /// RVAs of 64-bit absolute addresses to relocate.
    pub relocations: Vec<u32>,
    /// The exception directory (RVA, size), if the code has `.pdata`.
    pub exception: Option<(u32, u32)>,
    /// Where `.poly` goes (a page boundary after the caller's sections).
    pub generated_rva: Option<u32>,
    pub stack_reserve: u64,
    pub stack_commit: u64,
}

impl Default for PeBuilder {
    fn default() -> Self {
        PeBuilder {
            image_base: 0x1_8000_0000,
            dll: true,
            entry: 0,
            fixed: false,
            machine: 0x8664,
            time_date_stamp: 0x5000_0000,
            checksum: 0,
            sections: Vec::new(),
            export_name: String::new(),
            ordinal_base: 1,
            exports: Vec::new(),
            imports: Vec::new(),
            delay_imports: Vec::new(),
            tls: None,
            relocations: Vec::new(),
            exception: None,
            generated_rva: None,
            stack_reserve: 0x10_0000,
            stack_commit: 0x1000,
        }
    }
}

const fn align(v: u32) -> u32 {
    v.div_ceil(ALIGN) * ALIGN
}

/// Where the generated tables went.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// RVA of each caller section.
    pub sections: Vec<u32>,
    pub generated: u32,
    /// `iat[i][j]`: RVA of import `i`'s slot `j`.
    pub iat: Vec<Vec<u32>>,
    pub delay_iat: Vec<Vec<u32>>,
    pub delay_handles: Vec<u32>,
    /// RVA of each delay-load descriptor.
    pub delay_descriptors: Vec<u32>,
    pub tls_index: Option<u32>,
    pub tls_template: Option<u32>,
    pub size_of_image: u32,
}

/// Bytes being built, addressed by offset in `.poly`.
struct Gen {
    bytes: Vec<u8>,
}

impl Gen {
    fn at(&self) -> u32 {
        self.bytes.len() as u32
    }
    fn align(&mut self, a: usize) {
        while !self.bytes.len().is_multiple_of(a) {
            self.bytes.push(0);
        }
    }
    fn reserve(&mut self, n: usize) -> u32 {
        let at = self.at();
        self.bytes.resize(self.bytes.len() + n, 0);
        at
    }
    fn put16(&mut self, at: u32, v: u16) {
        self.bytes[at as usize..at as usize + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn put32(&mut self, at: u32, v: u32) {
        self.bytes[at as usize..at as usize + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn put64(&mut self, at: u32, v: u64) {
        self.bytes[at as usize..at as usize + 8].copy_from_slice(&v.to_le_bytes());
    }
    fn cstr(&mut self, s: &str) -> u32 {
        let at = self.at();
        self.bytes.extend_from_slice(s.as_bytes());
        self.bytes.push(0);
        at
    }
}

impl PeBuilder {
    fn section_rvas(&self) -> (Vec<u32>, u32) {
        let mut rva = ALIGN;
        let mut out = Vec::new();
        for s in &self.sections {
            out.push(rva);
            rva += align(s.virtual_size.max(s.data.len() as u32).max(1));
        }
        (out, rva)
    }

    /// The layout [`write`](Self::write) produces.
    pub fn layout(&self) -> Layout {
        self.build().1
    }

    /// The image file.
    pub fn write(&self) -> Vec<u8> {
        self.build().0
    }

    fn build(&self) -> (Vec<u8>, Layout) {
        let (sec_rvas, after) = self.section_rvas();
        let gen_rva = self.generated_rva.unwrap_or(after).max(after);
        let mut g = Gen { bytes: Vec::new() };
        let mut relocs: Vec<u32> = self.relocations.clone();
        let mut lay = Layout { sections: sec_rvas.clone(), generated: gen_rva, ..Layout::default() };
        let va = |off: u32| self.image_base + u64::from(gen_rva + off);

        // Address tables first, at fixed places.
        let mut iat_off = Vec::new();
        for imp in &self.imports {
            let at = g.reserve((imp.symbols.len() + 1) * 8);
            iat_off.push(at);
            lay.iat.push((0..imp.symbols.len()).map(|j| gen_rva + at + 8 * j as u32).collect());
        }
        let mut diat_off = Vec::new();
        for imp in &self.delay_imports {
            let at = g.reserve((imp.symbols.len() + 1) * 8);
            diat_off.push(at);
            lay.delay_iat.push((0..imp.symbols.len()).map(|j| gen_rva + at + 8 * j as u32).collect());
        }
        let mut handle_off = Vec::new();
        for _ in &self.delay_imports {
            let at = g.reserve(8);
            handle_off.push(at);
            lay.delay_handles.push(gen_rva + at);
        }
        let tls_index_off = self.tls.as_ref().map(|_| g.reserve(8));
        lay.tls_index = tls_index_off.map(|o| gen_rva + o);

        // TLS: template, callbacks, directory.
        let mut tls_dir = None;
        if let Some(t) = &self.tls {
            g.align(16);
            let tpl = g.at();
            g.bytes.extend_from_slice(&t.template);
            lay.tls_template = Some(gen_rva + tpl);
            g.align(8);
            let cbs = g.reserve((t.callbacks.len() + 1) * 8);
            for (i, &cb) in t.callbacks.iter().enumerate() {
                let at = cbs + 8 * i as u32;
                g.put64(at, self.image_base + u64::from(cb));
                relocs.push(gen_rva + at);
            }
            let dir = g.reserve(40);
            g.put64(dir, va(tpl));
            g.put64(dir + 8, va(tpl) + t.template.len() as u64);
            g.put64(dir + 16, va(tls_index_off.unwrap_or(0)));
            g.put64(dir + 24, if t.callbacks.is_empty() { 0 } else { va(cbs) });
            g.put32(dir + 32, t.zero_fill);
            g.put32(dir + 36, t.align_flag);
            for k in 0..3 {
                relocs.push(gen_rva + dir + 8 * k);
            }
            if !t.callbacks.is_empty() {
                relocs.push(gen_rva + dir + 24);
            }
            tls_dir = Some(dir);
        }

        // Lookup tables shared by imports and delay imports.
        let name_table = |g: &mut Gen, syms: &[Sym]| -> u32 {
            g.align(8);
            let ilt = g.reserve((syms.len() + 1) * 8);
            for (j, s) in syms.iter().enumerate() {
                let entry = match s {
                    Sym::Ordinal(o) => (1u64 << 63) | u64::from(*o),
                    Sym::Name { name, hint } => {
                        g.align(2);
                        let hn = g.at();
                        g.bytes.extend_from_slice(&hint.to_le_bytes());
                        g.cstr(name);
                        u64::from(gen_rva + hn)
                    }
                };
                g.put64(ilt + 8 * j as u32, entry);
            }
            ilt
        };

        // Imports.
        let mut import_dir = None;
        if !self.imports.is_empty() {
            g.align(4);
            let descs = g.reserve((self.imports.len() + 1) * 20);
            for (i, imp) in self.imports.iter().enumerate() {
                let ilt = name_table(&mut g, &imp.symbols);
                let name = g.cstr(&imp.dll);
                let d = descs + 20 * i as u32;
                g.put32(d, gen_rva + ilt);
                if imp.bound.is_some() {
                    g.put32(d + 4, 0xFFFF_FFFF);
                }
                g.put32(d + 12, gen_rva + name);
                g.put32(d + 16, gen_rva + iat_off[i]);
                // The address table starts as a copy of the lookup table,
                // or the bound addresses.
                for j in 0..imp.symbols.len() {
                    let v = match &imp.bound {
                        Some((_, addrs)) => addrs.get(j).copied().unwrap_or(0),
                        None => u64::from_le_bytes(
                            g.bytes[(ilt as usize + 8 * j)..(ilt as usize + 8 * j + 8)].try_into().unwrap(),
                        ),
                    };
                    g.put64(iat_off[i] + 8 * j as u32, v);
                }
            }
            import_dir = Some((descs, (self.imports.len() as u32 + 1) * 20));
        }

        // Delay-load imports (RVA-based descriptors, Attributes = 1).
        let mut delay_dir = None;
        if !self.delay_imports.is_empty() {
            g.align(4);
            let descs = g.reserve((self.delay_imports.len() + 1) * 32);
            for (i, imp) in self.delay_imports.iter().enumerate() {
                let int = name_table(&mut g, &imp.symbols);
                let name = g.cstr(&imp.dll);
                let d = descs + 32 * i as u32;
                lay.delay_descriptors.push(gen_rva + d);
                g.put32(d, 1);
                g.put32(d + 4, gen_rva + name);
                g.put32(d + 8, gen_rva + handle_off[i]);
                g.put32(d + 12, gen_rva + diat_off[i]);
                g.put32(d + 16, gen_rva + int);
                for (j, &thunk) in imp.delay_thunks.iter().enumerate().take(imp.symbols.len()) {
                    g.put64(diat_off[i] + 8 * j as u32, self.image_base + u64::from(thunk));
                    relocs.push(gen_rva + diat_off[i] + 8 * j as u32);
                }
            }
            delay_dir = Some((descs, (self.delay_imports.len() as u32 + 1) * 32));
        }

        // Exports.
        let mut export_dir = None;
        if !self.exports.is_empty() || !self.export_name.is_empty() {
            g.align(4);
            let start = g.at();
            let dir = g.reserve(40);
            let max = self.exports.iter().map(|e| e.ordinal).max().unwrap_or(self.ordinal_base);
            let count = max.saturating_sub(self.ordinal_base) + 1;
            let eat = g.reserve(4 * count as usize);
            let mut named: Vec<&Export> = self.exports.iter().filter(|e| e.name.is_some()).collect();
            named.sort_by(|a, b| a.name.cmp(&b.name));
            let names = g.reserve(4 * named.len());
            let ords = g.reserve(2 * named.len());
            let dll = g.cstr(&self.export_name);
            for (k, e) in named.iter().enumerate() {
                let n = g.cstr(e.name.as_deref().unwrap_or(""));
                g.put32(names + 4 * k as u32, gen_rva + n);
                g.put16(ords + 2 * k as u32, (e.ordinal - self.ordinal_base) as u16);
            }
            for e in &self.exports {
                let slot = eat + 4 * (e.ordinal - self.ordinal_base);
                let rva = match &e.target {
                    Target::Rva(r) => *r,
                    Target::Forward(f) => gen_rva + g.cstr(f),
                };
                g.put32(slot, rva);
            }
            g.put32(dir + 4, self.time_date_stamp);
            g.put32(dir + 12, gen_rva + dll);
            g.put32(dir + 16, self.ordinal_base);
            g.put32(dir + 20, count);
            g.put32(dir + 24, named.len() as u32);
            g.put32(dir + 28, gen_rva + eat);
            g.put32(dir + 32, gen_rva + names);
            g.put32(dir + 36, gen_rva + ords);
            export_dir = Some((start, g.at() - start));
        }

        // Bound import directory (new style), in the generated section.
        let mut bound_dir = None;
        let bound: Vec<&Import> = self.imports.iter().filter(|i| i.bound.is_some()).collect();
        if !bound.is_empty() {
            g.align(4);
            let start = g.reserve((bound.len() + 1) * 8);
            let mut name_offs = Vec::new();
            for imp in &bound {
                let at = g.cstr(&imp.dll);
                name_offs.push(at - start);
            }
            for (k, imp) in bound.iter().enumerate() {
                let d = start + 8 * k as u32;
                g.put32(d, imp.bound.as_ref().map_or(0, |b| b.0));
                g.put16(d + 4, name_offs[k] as u16);
            }
            bound_dir = Some((start, g.at() - start));
        }

        g.align(16);
        let gen_size = g.at().max(16);
        let reloc_rva = gen_rva + align(gen_size);
        let reloc = if self.fixed { Vec::new() } else { reloc_table(&relocs) };
        let size_of_image = reloc_rva + if reloc.is_empty() { 0 } else { align(reloc.len() as u32) };
        lay.size_of_image = size_of_image;

        // Headers.
        let mut out = vec![0u8; size_of_image as usize];
        let put16 = |o: &mut Vec<u8>, at: usize, v: u16| o[at..at + 2].copy_from_slice(&v.to_le_bytes());
        let put32 = |o: &mut Vec<u8>, at: usize, v: u32| o[at..at + 4].copy_from_slice(&v.to_le_bytes());
        let put64 = |o: &mut Vec<u8>, at: usize, v: u64| o[at..at + 8].copy_from_slice(&v.to_le_bytes());
        let mut all: Vec<Placed<'_>> = Vec::new();
        for (s, &rva) in self.sections.iter().zip(&sec_rvas) {
            all.push((&s.name, rva, s.virtual_size.max(s.data.len() as u32), s.characteristics, &s.data));
        }
        all.push((b".poly\0\0\0", gen_rva, gen_size, DATA, &g.bytes));
        if !reloc.is_empty() {
            all.push((b".reloc\0\0", reloc_rva, reloc.len() as u32, RELOC, &reloc));
        }
        out[0..2].copy_from_slice(b"MZ");
        put32(&mut out, 0x3C, PE_OFFSET as u32);
        out[PE_OFFSET..PE_OFFSET + 4].copy_from_slice(b"PE\0\0");
        let coff = PE_OFFSET + 4;
        put16(&mut out, coff, self.machine);
        put16(&mut out, coff + 2, all.len() as u16);
        put32(&mut out, coff + 4, self.time_date_stamp);
        put16(&mut out, coff + 16, OPTIONAL_HEADER_SIZE as u16);
        let mut ch: u16 = 0x0002 | 0x0020; // EXECUTABLE_IMAGE | LARGE_ADDRESS_AWARE
        if self.dll {
            ch |= 0x2000;
        }
        if self.fixed {
            ch |= 0x0001;
        }
        put16(&mut out, coff + 18, ch);
        let opt = coff + 20;
        put16(&mut out, opt, 0x20B);
        put32(&mut out, opt + 16, self.entry);
        put32(&mut out, opt + 20, ALIGN);
        put64(&mut out, opt + 24, self.image_base);
        put32(&mut out, opt + 32, ALIGN);
        put32(&mut out, opt + 36, ALIGN);
        put16(&mut out, opt + 40, 6);
        put16(&mut out, opt + 48, 6);
        put32(&mut out, opt + 56, size_of_image);
        put32(&mut out, opt + 60, ALIGN);
        put32(&mut out, opt + 64, self.checksum);
        put16(&mut out, opt + 68, if self.dll { 2 } else { 3 });
        let mut dllch: u16 = 0x0100; // NX_COMPAT
        if !self.fixed {
            dllch |= 0x0040 | 0x0020; // DYNAMIC_BASE | HIGH_ENTROPY_VA
        }
        put16(&mut out, opt + 70, dllch);
        put64(&mut out, opt + 72, self.stack_reserve);
        put64(&mut out, opt + 80, self.stack_commit);
        put64(&mut out, opt + 88, 0x10_0000);
        put64(&mut out, opt + 96, 0x1000);
        put32(&mut out, opt + 108, 16);
        let set_dir = |o: &mut Vec<u8>, idx: usize, rva: u32, size: u32| {
            let d = opt + 112 + idx * 8;
            put32(o, d, rva);
            put32(o, d + 4, size);
        };
        if let Some((o, s)) = export_dir {
            set_dir(&mut out, 0, gen_rva + o, s);
        }
        if let Some((o, s)) = import_dir {
            set_dir(&mut out, 1, gen_rva + o, s);
        }
        if let Some((rva, s)) = self.exception {
            set_dir(&mut out, 3, rva, s);
        }
        if !reloc.is_empty() {
            set_dir(&mut out, 5, reloc_rva, reloc.len() as u32);
        }
        if let Some(o) = tls_dir {
            set_dir(&mut out, 9, gen_rva + o, 40);
        }
        if let Some((o, s)) = bound_dir {
            set_dir(&mut out, 11, gen_rva + o, s);
        }
        if !self.imports.is_empty() {
            let first = iat_off[0];
            let end = *iat_off.last().unwrap() + 8 * (self.imports.last().unwrap().symbols.len() as u32 + 1);
            set_dir(&mut out, 12, gen_rva + first, end - first);
        }
        if let Some((o, s)) = delay_dir {
            set_dir(&mut out, 13, gen_rva + o, s);
        }
        let table = opt + OPTIONAL_HEADER_SIZE;
        for (i, (name, rva, vsize, chars, data)) in all.iter().enumerate() {
            let h = table + i * 40;
            out[h..h + 8].copy_from_slice(*name);
            put32(&mut out, h + 8, *vsize);
            put32(&mut out, h + 12, *rva);
            put32(&mut out, h + 16, align(data.len() as u32));
            put32(&mut out, h + 20, if data.is_empty() { 0 } else { *rva });
            put32(&mut out, h + 36, *chars);
            let at = *rva as usize;
            out[at..at + data.len()].copy_from_slice(data);
        }
        (out, lay)
    }
}

/// A section as written: name, RVA, virtual size, characteristics, bytes.
type Placed<'a> = (&'a [u8; 8], u32, u32, u32, &'a [u8]);

/// A `.reloc` table of `IMAGE_REL_BASED_DIR64` entries.
fn reloc_table(rvas: &[u32]) -> Vec<u8> {
    let mut rvas = rvas.to_vec();
    rvas.sort_unstable();
    rvas.dedup();
    let mut data = Vec::new();
    let mut i = 0;
    while i < rvas.len() {
        let page = rvas[i] & !(ALIGN - 1);
        let mut entries: Vec<u16> = Vec::new();
        while i < rvas.len() && rvas[i] & !(ALIGN - 1) == page {
            entries.push((10 << 12) | (rvas[i] & (ALIGN - 1)) as u16);
            i += 1;
        }
        if !entries.len().is_multiple_of(2) {
            entries.push(0);
        }
        data.extend_from_slice(&page.to_le_bytes());
        data.extend_from_slice(&(8 + 2 * entries.len() as u32).to_le_bytes());
        for e in entries {
            data.extend_from_slice(&e.to_le_bytes());
        }
    }
    data
}
