//! Tests. Every image is built byte by byte by [`builder`].

extern crate std;

mod builder;
mod dirs;
mod exports;
mod headers;
mod hostile;
mod imports;
mod load;

use std::vec;
use std::vec::Vec;

use builder::*;

use crate::*;

/// Where things are in a [`sample_exe`].
pub struct SampleExe {
    pub file: Vec<u8>,
    pub base: u64,
    pub text: u32,
    pub idata: u32,
    pub iats: Vec<u32>,
    pub data: u32,
    pub tls: u32,
    pub rdata: u32,
    pub didat: u32,
    pub bss: u32,
    /// Every location the relocation table names, with its type.
    pub relocs: Vec<(u32, u8)>,
}

/// Offsets inside `.data` of a sample.
pub const DATA_PTR_TEXT: u32 = 0x00;
pub const DATA_PTR_SELF: u32 = 0x08;
pub const DATA_PLAIN: u32 = 0x10;
pub const DATA_TLS_INDEX: u32 = 0x20;
pub const DATA_CALLBACKS: u32 = 0x28;
pub const DATA_COOKIE: u32 = 0x48;
pub const DATA_MODULE: u32 = 0x50;
/// Offsets inside `.rdata`.
pub const RDATA_LOAD_CONFIG: u32 = 0x40;
pub const RDATA_DEBUG: u32 = 0x200;
pub const RDATA_RSDS: u32 = 0x240;
pub const PDB_GUID: [u8; 16] = *b"poly-pe-guid-001";

pub fn load_config_size(bits: Bits) -> u32 {
    match bits {
        Bits::Pe32Plus => 148,
        Bits::Pe32 => 92,
    }
}

/// An executable shaped like a real one: code, imports from two DLLs (by
/// name and by ordinal), delay-loaded imports, writable data with absolute
/// pointers, TLS with two callbacks, a load config, debug directory with a
/// CodeView record, (x64) exception data, resources, a zero-filled section
/// and a relocation table covering every absolute pointer.
pub fn sample_exe(bits: Bits) -> SampleExe {
    let mut pe = PeBuilder::new(bits);
    let p = pe.ptr();
    let base = pe.image_base;
    let reloc_kind = if bits == Bits::Pe32Plus { reloc_kind::DIR64 } else { reloc_kind::HIGHLOW };
    let mut relocs = Vec::new();

    let mut code = vec![0xcc; 0x80];
    code[0] = 0xc3;
    let text = pe.section(".text", CODE, code, None);
    pe.entry = text;

    let idata = pe.next_rva();
    let (bytes, size, iats) = imports(
        idata,
        bits,
        &[
            ("KERNEL32.dll", &[Sym::Name(0x10, "GetProcAddress"), Sym::Name(0x20, "LoadLibraryA"), Sym::Ordinal(42)]),
            ("ntdll.dll", &[Sym::Name(0, "RtlAllocateHeap"), Sym::Ordinal(7)]),
        ],
    );
    pe.section(".idata", RDATA | section_flags::MEM_WRITE, bytes, None);
    pe.dir(dir::IMPORT, idata, size);
    pe.dir(dir::IAT, iats[0], (5 * p) as u32);

    let data = pe.next_rva();
    let mut d = vec![0; 0x80];
    put_ptr(&mut d, DATA_PTR_TEXT as usize, base + u64::from(text), bits);
    put_ptr(&mut d, DATA_PTR_SELF as usize, base + u64::from(data + 0x10), bits);
    put64(&mut d, DATA_PLAIN as usize, 0x1122_3344_5566_7788);
    put_ptr(&mut d, DATA_CALLBACKS as usize, base + u64::from(text + 0x10), bits);
    put_ptr(&mut d, DATA_CALLBACKS as usize + p, base + u64::from(text + 0x20), bits);
    for off in [DATA_PTR_TEXT, DATA_PTR_SELF, DATA_CALLBACKS, DATA_CALLBACKS + p as u32] {
        relocs.push((data + off, reloc_kind));
    }
    pe.section(".data", DATA, d, None);

    let tls = pe.section(".tls", DATA, b"TLSDATA!".to_vec(), None);

    let rdata = pe.next_rva();
    let mut r = Vec::new();
    put_ptr(&mut r, 0, base + u64::from(tls), bits);
    put_ptr(&mut r, p, base + u64::from(tls) + 8, bits);
    put_ptr(&mut r, 2 * p, base + u64::from(data + DATA_TLS_INDEX), bits);
    put_ptr(&mut r, 3 * p, base + u64::from(data + DATA_CALLBACKS), bits);
    put32(&mut r, 4 * p, 0x18);
    put32(&mut r, 4 * p + 4, 0x0030_0000);
    for i in 0..4 {
        relocs.push((rdata + (i * p) as u32, reloc_kind));
    }
    let lc = RDATA_LOAD_CONFIG as usize;
    put32(&mut r, lc, load_config_size(bits));
    put32(&mut r, lc + 4, 0x1234_5678);
    put16(&mut r, lc + 8, 1);
    let (cookie_off, flags_off) = if bits == Bits::Pe32Plus { (88, 144) } else { (60, 88) };
    put_ptr(&mut r, lc + cookie_off, base + u64::from(data + DATA_COOKIE), bits);
    relocs.push((rdata + RDATA_LOAD_CONFIG + cookie_off as u32, reloc_kind));
    put32(&mut r, lc + flags_off, guard_flags::CF_INSTRUMENTED | guard_flags::CF_FUNCTION_TABLE_PRESENT);
    let dbg = RDATA_DEBUG as usize;
    let path = b"C:\\build\\sample.pdb\0";
    put32(&mut r, dbg + 4, 0x5eed_0001);
    put32(&mut r, dbg + 12, debug_type::CODEVIEW);
    put32(&mut r, dbg + 16, (24 + path.len()) as u32);
    put32(&mut r, dbg + 20, rdata + RDATA_RSDS);
    // PointerToRawData is patched below, once the file offset is known.
    put32(&mut r, dbg + 28 + 12, debug_type::REPRO);
    let rsds = RDATA_RSDS as usize;
    put_bytes(&mut r, rsds, b"RSDS");
    put_bytes(&mut r, rsds + 4, &PDB_GUID);
    put32(&mut r, rsds + 20, 3);
    put_bytes(&mut r, rsds + 24, path);
    pe.section(".rdata", RDATA, r, None);
    pe.dir(dir::TLS, rdata, (4 * p + 8) as u32);
    pe.dir(dir::LOAD_CONFIG, rdata + RDATA_LOAD_CONFIG, load_config_size(bits));
    pe.dir(dir::DEBUG, rdata + RDATA_DEBUG, 2 * 28);

    if bits == Bits::Pe32Plus {
        let xdata = pe.next_rva();
        let mut x = Vec::new();
        // Function 0: sub rsp, 0x28 (ALLOC_SMALL 4), push rbx (PUSH_NONVOL 3).
        put_bytes(&mut x, 0, &[0x01, 0x06, 0x02, 0x00, 0x06, 0x42, 0x02, 0x30]);
        // Function 1: frame register RBP at offset 0x20, a handler. Codes:
        // ALLOC_LARGE(0) 0x100 * 8, SAVE_NONVOL(R12) at 0x40, SET_FPREG.
        put_bytes(&mut x, 0x20, &[0x01 | (unwind_flags::EHANDLER << 3), 0x10, 0x05, 0x25]);
        put_bytes(&mut x, 0x24, &[0x10, 0x03, 0x0c, 0xc4, 0x08, 0x00, 0x04, 0x01, 0x00, 0x01]);
        put32(&mut x, 0x30, text + 0x70);
        put32(&mut x, 0x34, 0xdead_beef);
        // Function 2: chained to function 0.
        put_bytes(&mut x, 0x40, &[0x01 | (unwind_flags::CHAININFO << 3), 0x00, 0x00, 0x00]);
        put32(&mut x, 0x44, text);
        put32(&mut x, 0x48, text + 0x10);
        put32(&mut x, 0x4c, xdata);
        pe.section(".xdata", RDATA, x, None);

        let pdata = pe.next_rva();
        let mut f = Vec::new();
        for (i, (b, e, u)) in [(0, 0x10, 0), (0x10, 0x30, 0x20), (0x30, 0x40, 0x40)].into_iter().enumerate() {
            put32(&mut f, i * 12, text + b);
            put32(&mut f, i * 12 + 4, text + e);
            put32(&mut f, i * 12 + 8, xdata + u);
        }
        pe.section(".pdata", RDATA, f, None);
        pe.dir(dir::EXCEPTION, pdata, 36);
    }

    let didat = pe.next_rva();
    let mut dl = Vec::new();
    put32(&mut dl, 0, 1);
    put32(&mut dl, 4, didat + 0x100);
    put32(&mut dl, 8, data + DATA_MODULE);
    put32(&mut dl, 12, didat + 0x60);
    put32(&mut dl, 16, didat + 0x40);
    put_ptr(&mut dl, 0x40, u64::from(didat + 0x80), bits);
    put_ptr(&mut dl, 0x40 + p, (1 << (8 * p - 1)) | 3, bits);
    for i in 0..2 {
        put_ptr(&mut dl, 0x60 + i * p, base + u64::from(text + 0x40), bits);
        relocs.push((didat + 0x60 + (i * p) as u32, reloc_kind));
    }
    put16(&mut dl, 0x80, 5);
    put_bytes(&mut dl, 0x82, b"DelayedFunction\0");
    put_bytes(&mut dl, 0x100, b"delayed.dll\0");
    pe.section(".didat", DATA, dl, None);
    pe.dir(dir::DELAY_IMPORT, didat, 64);

    let rsrc = pe.next_rva();
    let rs = resources(
        rsrc,
        &[
            Res { kind: ResId::Id(resource_type::RCDATA), name: ResId::Id(1), lang: 1033, data: b"poly" },
            Res { kind: ResId::Id(resource_type::RCDATA), name: ResId::Name("FIXTURE"), lang: 1033, data: b"named-en" },
            Res { kind: ResId::Id(resource_type::RCDATA), name: ResId::Name("FIXTURE"), lang: 1031, data: b"named-de" },
            Res { kind: ResId::Name("MYTYPE"), name: ResId::Id(7), lang: 0, data: b"custom" },
        ],
    );
    let rs_len = rs.len() as u32;
    pe.section(".rsrc", RDATA, rs, None);
    pe.dir(dir::RESOURCE, rsrc, rs_len);

    let bss = pe.section(".bss", BSS, Vec::new(), Some(0x2000));

    let table = builder::relocs(&relocs);
    let reloc = pe.next_rva();
    let len = table.len() as u32;
    pe.section(".reloc", RDATA | section_flags::MEM_DISCARDABLE, table, None);
    pe.dir(dir::BASERELOC, reloc, len);

    let mut file = pe.build();
    let rsds_off = Pe::parse(&file).unwrap().rva_to_offset(rdata + RDATA_RSDS).unwrap();
    let dbg_off = Pe::parse(&file).unwrap().rva_to_offset(rdata + RDATA_DEBUG).unwrap();
    put32(&mut file, dbg_off + 24, rsds_off as u32);
    SampleExe { file, base, text, idata, iats, data, tls, rdata, didat, bss, relocs }
}

/// Where things are in a [`sample_dll`].
pub struct SampleDll {
    pub file: Vec<u8>,
    pub text: u32,
    /// Exported names in the order given (not sorted) with their function index.
    pub names: Vec<(std::string::String, u16)>,
}

pub const DLL_ORDINAL_BASE: u32 = 5;

/// A DLL exporting 200 generated functions by name, one ordinal-only
/// function, an empty slot, and forwarders by name and by ordinal.
pub fn sample_dll(bits: Bits) -> SampleDll {
    let mut pe = PeBuilder::new(bits);
    pe.characteristics |= file_flags::DLL;
    pe.image_base = if bits == Bits::Pe32Plus { 0x1_8000_0000 } else { 0x1000_0000 };
    let text = pe.section(".text", CODE, vec![0xc3; 0x400], None);
    let mut functions = Vec::new();
    let mut names = Vec::new();
    for i in 0..200u16 {
        functions.push(builder::Target::Rva(text + u32::from(i) * 4));
        names.push((std::format!("Func{:03}_{}", 199 - i, if i % 2 == 0 { "A" } else { "W" }), i));
    }
    functions.push(builder::Target::Rva(text + 0x3f0)); // 200: ordinal only
    functions.push(builder::Target::Empty); // 201
    functions.push(builder::Target::Forward("NTDLL.RtlAllocateHeap")); // 202
    functions.push(builder::Target::Forward("api-ms-win-core-synch-l1-2-0.#12")); // 203
    names.push(("HeapAlloc".into(), 202));
    names.push(("Sleepy".into(), 203));
    let edata = pe.next_rva();
    let name_refs: Vec<(&str, u16)> = names.iter().map(|(n, i)| (n.as_str(), *i)).collect();
    let e = exports(edata, "sample.dll", DLL_ORDINAL_BASE, &functions, &name_refs);
    let len = e.len() as u32;
    pe.section(".edata", RDATA, e, None);
    pe.dir(dir::EXPORT, edata, len);
    SampleDll { file: pe.build(), text, names }
}

/// Lays an image out in memory with its own plan.
pub fn map(file: &[u8]) -> Vec<u8> {
    let pe = Pe::parse(file).unwrap();
    let plan = pe.load_plan().unwrap();
    let mut image = vec![0xaa; plan.size_of_image as usize];
    plan.map_into(&mut image).unwrap();
    image
}

/// Runs every parser over `data` (as a file and as a mapped image) and
/// every loading step over a copy, ignoring results. Used by the hostile
/// input tests: it must return, without panicking, whatever `data` is.
pub fn parse_everything(data: &[u8]) {
    for pe in [Pe::parse(data), Pe::parse_mapped(data)].into_iter().flatten() {
        exercise(&pe);
        if let Ok(plan) = pe.load_plan() {
            let _ = plan.regions().count();
            if plan.size_of_image <= 1 << 20 {
                let mut image = vec![0; plan.size_of_image as usize];
                if plan.map_into(&mut image).is_ok() {
                    let info = *pe.info();
                    let _ = relocate(&mut image, &info, info.image_base().wrapping_add(0x7_0000));
                    let _ = bind_imports(&mut image, &info, |r| Ok::<u64, ()>(r.iat_rva.into()));
                    let _ = bind_delay_imports(&mut image, &info, |_| Err::<u64, ()>(()));
                    let _ = bind_delay_imports(&mut image, &info, |r| Ok::<u64, ()>(r.index as u64));
                    if let Ok(mapped) = Pe::parse_mapped(&image) {
                        exercise(&mapped);
                    }
                }
            }
        }
    }
    if let Ok(obj) = CoffObject::parse(data) {
        for i in 0..obj.sections().len() {
            let _ = obj.section_name(i);
            let _ = obj.section_data(i);
        }
    }
}

fn exercise(pe: &Pe<'_>) {
    for i in 0..pe.sections().len() {
        let _ = pe.section_name(i);
    }
    let _ = pe.section_by_name(b".text");
    if let Ok(Some(blocks)) = pe.relocations() {
        for b in blocks.flatten() {
            let _ = b.entries().count();
        }
    }
    if let Ok(Some(descs)) = pe.imports() {
        for d in descs.take(10_000).flatten() {
            let _ = d.dll_name();
            if let Ok(entries) = d.entries() {
                let _ = entries.take(10_000).count();
            }
        }
    }
    if let Ok(Some(descs)) = pe.delay_imports() {
        for d in descs.take(10_000).flatten() {
            let _ = d.dll_name();
            let _ = d.entries().take(10_000).count();
        }
    }
    if let Ok(Some(bound)) = pe.bound_imports() {
        for b in bound.take(10_000).flatten() {
            let _ = b.forwarders().count();
        }
    }
    let _ = pe.is_bound();
    if let Ok(Some(e)) = pe.exports() {
        let _ = e.dll_name();
        let _ = e.functions().take(10_000).count();
        for n in e.names().take(10_000).flatten() {
            let _ = e.by_name(n.0);
        }
        let _ = e.by_name(b"HeapAlloc");
        let _ = e.by_name_with_hint(b"HeapAlloc", 3);
        let _ = e.by_ordinal(7);
        let _ = e.ordinal_of(b"Func000_A");
    }
    if let Ok(Some(tls)) = pe.tls() {
        let _ = pe.tls_template(&tls);
        let _ = pe.tls_index_rva(&tls);
        let _ = tls.total_size();
        if let Ok(cbs) = pe.tls_callbacks(&tls) {
            let _ = cbs.take(10_000).count();
        }
    }
    let _ = pe.load_config();
    if let Ok(Some(funcs)) = pe.runtime_functions() {
        for f in funcs.iter().take(10_000) {
            if let Ok(u) = pe.unwind_info(&f) {
                let _ = u.codes().count();
            }
        }
        let _ = funcs.lookup(0x1010);
    }
    if let Ok(Some(root)) = pe.resources() {
        let _ = root.walk(|_, d| {
            let _ = pe.resource_bytes(&d);
        });
        let _ = pe.find_resource(ResourceKey::Id(10), ResourceKey::Name("FIXTURE"), None);
    }
    if let Ok(Some(entries)) = pe.debug_entries() {
        for e in entries {
            let _ = pe.debug_data(&e);
            let _ = pe.codeview(&e);
        }
    }
    let _ = pe.pdb_info();
}

/// A tiny deterministic PRNG (xorshift64*) for the mutation tests.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}
