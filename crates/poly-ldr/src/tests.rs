//! The loader against a fake process, with images built by [`crate::builder`].

extern crate std;

use crate::builder::{Export, Import, PeBuilder, Section, Sym, TEXT, Target, Tls};
use crate::ldrdata::*;
use crate::path::{Dirs, wide};
use crate::status::*;
use crate::*;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;
use std::collections::{BTreeMap, HashMap};

// ----- the fake process -----------------------------------------------------

struct Region {
    data: Vec<u8>,
    prot: Vec<u32>,
}

#[derive(Default)]
struct Fake {
    files: HashMap<String, Vec<u8>>,
    regions: BTreeMap<u64, Region>,
    next_any: u64,
    next_heap: u64,
    tried: Vec<String>,
}

fn s(v: &[u16]) -> String {
    String::from_utf16(v).unwrap()
}

impl Fake {
    fn new() -> Fake {
        Fake { next_any: 0x7_0000_0000, next_heap: 0x10_0000_0000, ..Fake::default() }
    }

    fn file(&mut self, dos_path: &str, bytes: Vec<u8>) {
        self.files.insert(std::format!("\\??\\{dos_path}").to_lowercase(), bytes);
    }

    fn overlaps(&self, base: u64, size: u64) -> bool {
        self.regions.iter().any(|(&b, r)| base < b + r.data.len() as u64 && b < base + size)
    }

    fn insert(&mut self, base: u64, size: u64) {
        let pages = size.div_ceil(PAGE_SIZE) as usize;
        self.regions.insert(base, Region { data: vec![0; size as usize], prot: vec![page::READWRITE; pages] });
    }

    /// Map an image file the way the kernel does (for the executable and
    /// ntdll): laid out at its preferred base.
    fn map_image(&mut self, file: &[u8]) -> u64 {
        let pe = poly_pe::Pe::parse(file).unwrap();
        let plan = pe.load_plan().unwrap();
        let size = u64::from(plan.size_of_image).div_ceil(PAGE_SIZE) * PAGE_SIZE;
        let base = plan.image_base;
        assert!(!self.overlaps(base, size));
        self.insert(base, size);
        plan.map_into(&mut self.regions.get_mut(&base).unwrap().data).unwrap();
        base
    }

    fn prot_at(&self, addr: u64) -> u32 {
        let (&b, r) = self.regions.range(..=addr).next_back().unwrap();
        r.prot[((addr - b) / PAGE_SIZE) as usize]
    }

    fn images(&self) -> usize {
        self.regions.keys().filter(|&&b| b < 0x10_0000_0000).count()
    }

    fn heap_blocks(&self) -> usize {
        self.regions.keys().filter(|&&b| b >= 0x10_0000_0000).count()
    }
}

impl Process for Fake {
    fn read_file(&mut self, nt_path: &[u16]) -> Result<Vec<u8>, NtStatus> {
        let key = s(nt_path).to_lowercase();
        self.tried.push(key.clone());
        self.files.get(&key).cloned().ok_or(STATUS_OBJECT_NAME_NOT_FOUND)
    }

    fn allocate(&mut self, base: Option<u64>, size: u64) -> Result<u64, NtStatus> {
        let base = match base {
            Some(b) => {
                if self.overlaps(b, size) {
                    return Err(STATUS_CONFLICTING_ADDRESSES);
                }
                b
            }
            None => loop {
                let b = self.next_any;
                self.next_any += size.div_ceil(0x1_0000) * 0x1_0000;
                if !self.overlaps(b, size) {
                    break b;
                }
            },
        };
        self.insert(base, size);
        Ok(base)
    }

    fn free(&mut self, base: u64) {
        assert!(self.regions.remove(&base).is_some(), "freed {base:#x}, which is not an allocation");
    }

    fn protect(&mut self, addr: u64, size: u64, protect: u32) -> Result<(), NtStatus> {
        let (&b, r) = self.regions.range_mut(..=addr).next_back().ok_or(STATUS_ACCESS_VIOLATION)?;
        let end = addr + size;
        if end > b + r.data.len() as u64 {
            return Err(STATUS_ACCESS_VIOLATION);
        }
        for pg in (addr - b) / PAGE_SIZE..(end - b).div_ceil(PAGE_SIZE) {
            r.prot[pg as usize] = protect;
        }
        Ok(())
    }

    fn memory(&mut self, addr: u64, len: usize) -> Result<&mut [u8], NtStatus> {
        let (&b, r) = self.regions.range_mut(..=addr).next_back().ok_or(STATUS_ACCESS_VIOLATION)?;
        let off = (addr - b) as usize;
        r.data.get_mut(off..off.checked_add(len).ok_or(STATUS_ACCESS_VIOLATION)?).ok_or(STATUS_ACCESS_VIOLATION)
    }

    fn alloc(&mut self, size: usize) -> Result<u64, NtStatus> {
        let b = self.next_heap;
        self.next_heap += (size as u64).div_ceil(16) * 16 + 16;
        self.insert(b, size.max(1) as u64);
        Ok(b)
    }

    fn free_alloc(&mut self, addr: u64) {
        assert!(self.regions.remove(&addr).is_some(), "freed {addr:#x} twice or never allocated");
    }
}

// ----- images ---------------------------------------------------------------

const EXE_BASE: u64 = 0x1_4000_0000;
const NTDLL_BASE: u64 = 0x7FF0_0000_0000;
const PEB: u64 = 0x5000_0000;
const TEB: u64 = 0x5001_0000;
const TEB2: u64 = 0x5002_0000;

/// A text section of `n` bytes of `ret`s: every RVA from 0x1000 is "code".
fn text(n: usize) -> Section {
    Section::new(".text", vec![0xC3; n], TEXT)
}

fn named(names: &[(&str, u32)]) -> Vec<Export> {
    names
        .iter()
        .enumerate()
        .map(|(i, &(n, rva))| Export { name: Some(n.into()), ordinal: i as u32 + 1, target: Target::Rva(rva) })
        .collect()
}

fn ntdll_image() -> Vec<u8> {
    PeBuilder {
        image_base: NTDLL_BASE,
        export_name: "ntdll.dll".into(),
        sections: vec![text(0x100)],
        exports: named(&[("RtlAllocateHeap", 0x1010), ("RtlFreeHeap", 0x1020), ("RtlUpcaseUnicodeChar", 0x1030)]),
        ..PeBuilder::default()
    }
    .write()
}

fn imports(dll: &str, syms: &[Sym]) -> Import {
    Import { dll: dll.into(), symbols: syms.to_vec(), ..Import::default() }
}

fn dll(name: &str, base: u64, exports: Vec<Export>, imps: Vec<Import>) -> PeBuilder {
    PeBuilder {
        image_base: base,
        export_name: name.into(),
        entry: 0x1000,
        sections: vec![text(0x200)],
        exports,
        imports: imps,
        ..PeBuilder::default()
    }
}

fn exe(imps: Vec<Import>) -> PeBuilder {
    PeBuilder {
        image_base: EXE_BASE,
        dll: false,
        entry: 0x1000,
        sections: vec![text(0x100)],
        imports: imps,
        ..PeBuilder::default()
    }
}

fn dirs() -> Dirs {
    Dirs {
        app: wide("C:\\app"),
        system32: wide("C:\\Windows\\System32"),
        system16: wide("C:\\Windows\\System"),
        windows: wide("C:\\Windows"),
        current: wide("C:\\work"),
        path: vec![wide("C:\\tools")],
    }
}

/// A process with ntdll and `exe` mapped by "the kernel" and registered.
struct World {
    p: Fake,
    l: Loader,
    exe: ModuleId,
    ntdll: ModuleId,
}

fn world(exe_image: &[u8]) -> World {
    let mut p = Fake::new();
    p.insert(PEB, 0x1000);
    p.insert(TEB, 0x2000);
    p.insert(TEB2, 0x2000);
    let mut l = Loader::new(Config { dirs: dirs() });
    l.init_ldr_data(&mut p, PEB).unwrap();
    let eb = p.map_image(exe_image);
    let nb = p.map_image(&ntdll_image());
    let exe = l.add_mapped(&mut p, eb, &wide("C:\\app\\test.exe"), false).unwrap();
    let ntdll = l.add_mapped(&mut p, nb, &wide("C:\\Windows\\System32\\ntdll.dll"), true).unwrap();
    World { p, l, exe, ntdll }
}

fn base_of(w: &World, name: &str) -> u64 {
    w.l.get_handle(&wide(name)).unwrap()
}

fn iat(w: &mut World, b: &PeBuilder, i: usize, j: usize) -> u64 {
    read_u64(&mut w.p, b.image_base + u64::from(b.layout().iat[i][j])).unwrap()
}

fn kinds(c: &[Callout], w: &World) -> Vec<(String, CalloutKind, u32)> {
    c.iter().map(|c| (s(&w.l.module(c.module).unwrap().name), c.kind, c.reason)).collect()
}

fn names(list: &[u64], w: &mut World) -> Vec<String> {
    list.iter()
        .map(|&e| {
            let len = u16::from_le_bytes(w.p.memory(e + ENTRY_BASE_NAME, 2).unwrap().try_into().unwrap()) as usize;
            let buf = read_u64(&mut w.p, e + ENTRY_BASE_NAME + 8).unwrap();
            let bytes = w.p.memory(buf, len).unwrap().to_vec();
            let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            s(&units)
        })
        .collect()
}

// ----- tests ----------------------------------------------------------------

#[test]
fn process_start_binds_imports_in_dependency_order() {
    // exe -> a.dll (by name and ordinal), ntdll; a -> b.dll; b has TLS.
    let b = PeBuilder {
        tls: Some(Tls { template: vec![1, 2, 3, 4], zero_fill: 12, callbacks: vec![0x1100], align_flag: 0 }),
        ..dll("b.dll", 0x1_8100_0000, named(&[("BFunc", 0x1040)]), vec![])
    };
    let a = dll(
        "a.dll",
        0x1_8200_0000,
        vec![
            Export { name: Some("AFunc".into()), ordinal: 1, target: Target::Rva(0x1010) },
            Export { name: None, ordinal: 5, target: Target::Rva(0x1050) },
        ],
        vec![imports("b.dll", &[Sym::name("BFunc")])],
    );
    let e = exe(vec![
        imports("A.DLL", &[Sym::name("AFunc"), Sym::Ordinal(5)]),
        imports("ntdll.dll", &[Sym::Name { name: "RtlFreeHeap".into(), hint: 1 }]),
    ]);
    let mut w = world(&e.write());
    w.p.file("C:\\app\\a.dll", a.write());
    w.p.file("C:\\Windows\\System32\\b.dll", b.write());
    let c = w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    assert_eq!(
        kinds(&c, &w),
        [
            ("b.dll".to_string(), CalloutKind::Tls, reason::PROCESS_ATTACH),
            ("b.dll".to_string(), CalloutKind::Entry, reason::PROCESS_ATTACH),
            ("A.DLL".to_string(), CalloutKind::Entry, reason::PROCESS_ATTACH),
        ]
    );
    assert!(c.iter().all(|c| c.reserved == 1), "static loads have a non-NULL lpReserved");
    let (ab, bb) = (base_of(&w, "a.dll"), base_of(&w, "b"));
    assert_eq!((ab, bb), (0x1_8200_0000, 0x1_8100_0000), "preferred bases were free");
    assert_eq!(c[0].routine, bb + 0x1100);
    assert_eq!(c[1].routine, bb + 0x1000);
    assert_eq!(iat(&mut w, &e, 0, 0), ab + 0x1010);
    assert_eq!(iat(&mut w, &e, 0, 1), ab + 0x1050);
    assert_eq!(iat(&mut w, &e, 1, 0), NTDLL_BASE + 0x1020);
    assert_eq!(iat(&mut w, &a, 0, 0), bb + 0x1040);
    // Section protections: code read/execute, the headers read-only.
    assert_eq!(w.p.prot_at(ab), page::READONLY);
    assert_eq!(w.p.prot_at(ab + 0x1000), page::EXECUTE_READ);
    assert_eq!(w.p.prot_at(EXE_BASE + 0x1000), page::EXECUTE_READ);
    // The search: a.dll from the app directory, b.dll after it was not there.
    assert!(w.p.tried.contains(&"\\??\\c:\\app\\b.dll".into()));
    // TLS: b got index 0, the first thread a block with the template.
    let index = read_u32(&mut w.p, bb + u64::from(b.layout().tls_index.unwrap())).unwrap();
    assert_eq!(index, 0);
    let array = read_u64(&mut w.p, TEB + TEB_TLS_POINTER).unwrap();
    let block = read_u64(&mut w.p, array).unwrap();
    assert_eq!(w.p.memory(block, 16).unwrap(), &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    // The PEB's lists.
    let ldr = read_u64(&mut w.p, PEB + PEB_LDR).unwrap();
    let load = walk(&mut w.p, ldr + LDR_IN_LOAD_ORDER, ENTRY_IN_LOAD_ORDER, 100).unwrap();
    let mem = walk(&mut w.p, ldr + LDR_IN_MEMORY_ORDER, ENTRY_IN_MEMORY_ORDER, 100).unwrap();
    let init = walk(&mut w.p, ldr + LDR_IN_INIT_ORDER, ENTRY_IN_INIT_ORDER, 100).unwrap();
    assert_eq!(names(&load, &mut w), ["test.exe", "ntdll.dll", "A.DLL", "b.dll"], "names as the importer spelled them");
    assert_eq!(load, mem);
    assert_eq!(names(&init, &mut w), ["ntdll.dll", "b.dll", "A.DLL"]);
    assert_eq!(read_u64(&mut w.p, load[2] + ENTRY_DLL_BASE).unwrap(), ab);
    assert_eq!(read_u64(&mut w.p, load[2] + ENTRY_ENTRY_POINT).unwrap(), ab + 0x1000);
    assert!(w.l.modules().iter().all(|m| m.pinned), "static imports are pinned");
    assert_eq!(w.l.find(ab + 0x1234).unwrap().base, ab);
    assert!(w.l.find(0x1234).is_none());
}

#[test]
fn a_dll_whose_base_is_taken_is_relocated() {
    let mut d = dll("r.dll", EXE_BASE, named(&[("Get", 0x1010)]), vec![]);
    // A data pointer to its own export, and TLS whose directory holds VAs.
    let mut data = vec![0u8; 16];
    data[..8].copy_from_slice(&(EXE_BASE + 0x1010).to_le_bytes());
    d.sections.push(Section::new(".data", data, crate::builder::DATA));
    d.relocations.push(0x2000);
    d.tls = Some(Tls { template: vec![9; 8], zero_fill: 0, callbacks: vec![0x1020, 0x1030], align_flag: 0 });
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    w.p.file("C:\\app\\r.dll", d.write());
    let (id, base, op) = w.l.load(&mut w.p, &wide("r"), None).unwrap();
    assert_ne!(base, EXE_BASE);
    assert_eq!(base % 0x1_0000, 0);
    assert_eq!(read_u64(&mut w.p, base + 0x2000).unwrap(), base + 0x1010, "the pointer moved with the image");
    let routines: Vec<u64> = op.callouts.iter().map(|c| c.routine).collect();
    assert_eq!(routines, [base + 0x1020, base + 0x1030, base + 0x1000]);
    assert!(op.callouts.iter().all(|c| c.reserved == 0 && c.base == base));
    // Its TLS index and a block for the running thread, from the moved template.
    let index = w.l.module(id).unwrap().tls_index.unwrap();
    let block = w.l.tls_block(TEB, index).unwrap();
    assert_eq!(w.p.memory(block, 8).unwrap(), &[9; 8]);
    let (pa, _) = w.l.get_procedure(&mut w.p, base, Some(b"Get"), 0).unwrap();
    assert_eq!(pa, base + 0x1010);
    assert_eq!(w.l.get_procedure(&mut w.p, base, None, 1).unwrap().0, base + 0x1010);
    assert_eq!(w.l.get_procedure(&mut w.p, base, Some(b"Nope"), 0).unwrap_err(), STATUS_PROCEDURE_NOT_FOUND);
    assert_eq!(w.l.get_procedure(&mut w.p, base, None, 9).unwrap_err(), STATUS_ORDINAL_NOT_FOUND);
}

#[test]
fn an_image_without_relocations_needs_its_base() {
    let mut d = dll("fixed.dll", EXE_BASE, named(&[("F", 0x1010)]), vec![]);
    d.fixed = true;
    let mut w = world(&exe(vec![]).write());
    w.p.file("C:\\app\\fixed.dll", d.write());
    assert_eq!(w.l.load(&mut w.p, &wide("fixed.dll"), None).unwrap_err(), STATUS_CONFLICTING_ADDRESSES);
    let mut other = d.clone();
    other.image_base = 0x2_0000_0000;
    w.p.file("C:\\app\\fixed.dll", other.write());
    assert_eq!(w.l.load(&mut w.p, &wide("fixed.dll"), None).unwrap().1, 0x2_0000_0000);
}

#[test]
fn forwarders_follow_to_their_target() {
    let t = dll("t.dll", 0x1_8300_0000, named(&[("Real", 0x1080), ("Other", 0x1090)]), vec![]);
    let f = dll(
        "f.dll",
        0x1_8400_0000,
        vec![
            Export { name: Some("ByName".into()), ordinal: 1, target: Target::Forward("t.Real".into()) },
            Export { name: Some("ByOrdinal".into()), ordinal: 2, target: Target::Forward("t.#2".into()) },
            Export {
                name: Some("ToApiSet".into()),
                ordinal: 3,
                target: Target::Forward("api-ms-win-core-rtlsupport-l1-1-0.RtlUpcaseUnicodeChar".into()),
            },
            Export { name: Some("Loop".into()), ordinal: 4, target: Target::Forward("f.Loop".into()) },
        ],
        vec![],
    );
    let e = exe(vec![imports("f.dll", &[Sym::name("ByName"), Sym::name("ByOrdinal"), Sym::name("ToApiSet")])]);
    let mut w = world(&e.write());
    w.p.file("C:\\app\\f.dll", f.write());
    w.p.file("C:\\app\\t.dll", t.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let tb = base_of(&w, "t.dll");
    assert_eq!(iat(&mut w, &e, 0, 0), tb + 0x1080);
    assert_eq!(iat(&mut w, &e, 0, 1), tb + 0x1090);
    assert_eq!(iat(&mut w, &e, 0, 2), NTDLL_BASE + 0x1030);
    let fb = base_of(&w, "f.dll");
    assert_eq!(w.l.get_procedure(&mut w.p, fb, Some(b"Loop"), 0).unwrap_err(), STATUS_ENTRYPOINT_NOT_FOUND);
}

#[test]
fn missing_dlls_and_symbols_fail_and_leave_nothing() {
    let t = dll("t.dll", 0x1_8300_0000, named(&[("Real", 0x1080)]), vec![]);
    let needs = |s: Sym| dll("n.dll", 0x1_8500_0000, vec![], vec![imports("t.dll", &[s])]);
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let (images, heap) = (w.p.images(), w.p.heap_blocks());
    assert_eq!(w.l.load(&mut w.p, &wide("nothere.dll"), None).unwrap_err(), STATUS_DLL_NOT_FOUND);
    w.p.file("C:\\app\\n.dll", needs(Sym::name("Real")).write());
    assert_eq!(w.l.load(&mut w.p, &wide("n.dll"), None).unwrap_err(), STATUS_DLL_NOT_FOUND, "t.dll is missing");
    assert_eq!((w.p.images(), w.p.heap_blocks()), (images, heap), "n.dll was taken back");
    w.p.file("C:\\app\\t.dll", t.write());
    w.p.file("C:\\app\\n.dll", needs(Sym::name("Unreal")).write());
    assert_eq!(w.l.load(&mut w.p, &wide("n.dll"), None).unwrap_err(), STATUS_ENTRYPOINT_NOT_FOUND);
    w.p.file("C:\\app\\n.dll", needs(Sym::Ordinal(77)).write());
    assert_eq!(w.l.load(&mut w.p, &wide("n.dll"), None).unwrap_err(), STATUS_ORDINAL_NOT_FOUND);
    assert_eq!((w.p.images(), w.p.heap_blocks()), (images, heap), "t.dll and n.dll were both taken back");
    assert_eq!(w.l.modules().len(), 2);
    let ldr = w.l.ldr_data();
    assert_eq!(walk(&mut w.p, ldr + LDR_IN_LOAD_ORDER, 0, 100).unwrap().len(), 2);
    // A process whose static import is missing cannot start.
    let mut w = world(&exe(vec![imports("gone.dll", &[Sym::name("X")])]).write());
    assert_eq!(w.l.process_init(&mut w.p, w.exe, TEB).unwrap_err(), STATUS_DLL_NOT_FOUND);
}

#[test]
fn reference_counts_and_unloading() {
    let lib = dll("lib.dll", 0x1_8600_0000, named(&[("L", 0x1010)]), vec![]);
    let top = dll("top.dll", 0x1_8700_0000, named(&[("T", 0x1010)]), vec![imports("lib.dll", &[Sym::name("L")])]);
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    w.p.file("C:\\app\\lib.dll", lib.write());
    w.p.file("C:\\app\\top.dll", top.write());
    let images = w.p.images();
    let (top_id, tb, op) = w.l.load(&mut w.p, &wide("TOP.DLL"), None).unwrap();
    assert_eq!(kinds(&op.callouts, &w).iter().map(|k| k.0.as_str()).collect::<Vec<_>>(), ["lib.dll", "TOP.DLL"]);
    let lib_id = op.new_modules()[0];
    assert_eq!(w.l.module(top_id).unwrap().load_count, 1);
    assert_eq!(w.l.module(lib_id).unwrap().load_count, 1, "held by top.dll");
    // Loading again only counts.
    let (_, tb2, op2) = w.l.load(&mut w.p, &wide("top"), None).unwrap();
    assert_eq!((tb2, op2.callouts.len()), (tb, 0));
    let (_, lb, _) = w.l.load(&mut w.p, &wide("lib.dll"), None).unwrap();
    assert_eq!(w.l.module(lib_id).unwrap().load_count, 2);
    assert!(w.l.unload(tb).unwrap().callouts.is_empty());
    let u = w.l.unload(tb).unwrap();
    assert_eq!(kinds(&u.callouts, &w), [("TOP.DLL".to_string(), CalloutKind::Entry, reason::PROCESS_DETACH)]);
    assert_eq!(u.callouts[0].reserved, 0);
    w.l.finish_unload(&mut w.p, u);
    assert_eq!(w.p.images(), images + 1, "lib.dll stays: the caller still holds it");
    assert_eq!(w.l.module(lib_id).unwrap().load_count, 1);
    let u = w.l.unload(lb).unwrap();
    assert_eq!(u.callouts.len(), 1);
    w.l.finish_unload(&mut w.p, u);
    assert_eq!(w.p.images(), images);
    assert_eq!(w.l.unload(lb).unwrap_err(), STATUS_DLL_NOT_FOUND);
    // The executable and ntdll are pinned; so is a module pinned by AddRef.
    assert!(w.l.unload(NTDLL_BASE).unwrap().callouts.is_empty());
    let (_, lb, _) = w.l.load(&mut w.p, &wide("lib.dll"), None).unwrap();
    w.l.add_ref(lb, 0).unwrap();
    assert_eq!(w.l.get_handle(&wide("lib")).unwrap(), lb);
    w.l.add_ref(lb, ADDREF_PIN).unwrap();
    for _ in 0..3 {
        assert!(w.l.unload(lb).unwrap().callouts.is_empty());
    }
    assert!(w.l.get_handle(&wide("lib.dll")).is_ok());
}

#[test]
fn a_failing_dllmain_unloads_its_load() {
    let lib = dll("lib.dll", 0x1_8600_0000, named(&[("L", 0x1010)]), vec![]);
    let bad = dll("bad.dll", 0x1_8700_0000, vec![], vec![imports("lib.dll", &[Sym::name("L")])]);
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    w.p.file("C:\\app\\lib.dll", lib.write());
    w.p.file("C:\\app\\bad.dll", bad.write());
    let (images, heap) = (w.p.images(), w.p.heap_blocks());
    let (bad_id, _, op) = w.l.load(&mut w.p, &wide("bad.dll"), None).unwrap();
    // lib.dll's DllMain ran, bad.dll's returned FALSE.
    let u = w.l.fail_load(&op, bad_id);
    assert_eq!(
        kinds(&u.callouts, &w),
        [
            ("bad.dll".to_string(), CalloutKind::Entry, reason::PROCESS_DETACH),
            ("lib.dll".to_string(), CalloutKind::Entry, reason::PROCESS_DETACH),
        ]
    );
    w.l.finish_unload(&mut w.p, u);
    assert_eq!((w.p.images(), w.p.heap_blocks()), (images, heap));
    assert!(w.l.get_handle(&wide("lib.dll")).is_err());
    assert_eq!(w.l.init_order(), [w.ntdll, w.exe]);
}

#[test]
fn bound_imports_are_kept_only_while_valid() {
    let t =
        PeBuilder { time_date_stamp: 0x1234_5678, ..dll("t.dll", 0x1_8300_0000, named(&[("Real", 0x1080)]), vec![]) };
    let bound = |stamp: u32| {
        exe(vec![Import {
            dll: "t.dll".into(),
            symbols: vec![Sym::name("Real")],
            bound: Some((stamp, vec![0xDEAD_BEEF])),
            delay_thunks: vec![],
        }])
    };
    for (stamp, expect) in [(0x1234_5678u32, 0xDEAD_BEEFu64), (0x1111_1111, 0x1_8300_1080)] {
        let e = bound(stamp);
        let mut w = world(&e.write());
        w.p.file("C:\\app\\t.dll", t.write());
        w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
        assert_eq!(iat(&mut w, &e, 0, 0), expect, "stamp {stamp:#x}");
    }
    // Valid stamp, but t.dll had to move: rebound.
    let e = bound(0x1234_5678);
    let mut w = world(&e.write());
    w.p.insert(0x1_8300_0000, 0x1000);
    w.p.file("C:\\app\\t.dll", t.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let tb = base_of(&w, "t.dll");
    assert_ne!(tb, 0x1_8300_0000);
    assert_eq!(iat(&mut w, &e, 0, 0), tb + 0x1080);
}

#[test]
fn delay_load_imports_resolve_on_first_use() {
    let d = dll("late.dll", 0x1_8800_0000, named(&[("Mul", 0x1010), ("Add", 0x1020)]), vec![]);
    let e = PeBuilder {
        delay_imports: vec![Import {
            dll: "late.dll".into(),
            symbols: vec![Sym::name("Mul"), Sym::Ordinal(2), Sym::name("Gone")],
            bound: None,
            delay_thunks: vec![0x1040, 0x1048, 0x1050],
        }],
        ..exe(vec![])
    };
    let lay = e.layout();
    let mut w = world(&e.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    assert!(w.l.get_handle(&wide("late.dll")).is_err(), "not loaded at start");
    // The slots start at their thunks (relocated VAs).
    let slot0 = EXE_BASE + u64::from(lay.delay_iat[0][0]);
    assert_eq!(read_u64(&mut w.p, slot0).unwrap(), EXE_BASE + 0x1040);
    let desc = EXE_BASE + u64::from(lay.delay_descriptors[0]);
    // The DLL is missing: the failure says so.
    let f = w.l.resolve_delay(&mut w.p, EXE_BASE, desc, slot0).unwrap_err();
    assert!(f.dll_failed);
    assert_eq!(
        (f.status, f.dll.as_slice(), f.name.as_deref()),
        (STATUS_DLL_NOT_FOUND, &b"late.dll"[..], Some(&b"Mul"[..]))
    );
    w.p.file("C:\\app\\late.dll", d.write());
    let (a, op) = w.l.resolve_delay(&mut w.p, EXE_BASE, desc, slot0).unwrap();
    let lb = base_of(&w, "late.dll");
    assert_eq!(a, lb + 0x1010);
    assert_eq!(kinds(&op.callouts, &w), [("late.dll".to_string(), CalloutKind::Entry, reason::PROCESS_ATTACH)]);
    assert_eq!(read_u64(&mut w.p, slot0).unwrap(), a, "the slot now holds the address");
    assert_eq!(read_u64(&mut w.p, EXE_BASE + u64::from(lay.delay_handles[0])).unwrap(), lb, "and the handle is kept");
    let slot1 = EXE_BASE + u64::from(lay.delay_iat[0][1]);
    let (a1, op1) = w.l.resolve_delay(&mut w.p, EXE_BASE, desc, slot1).unwrap();
    assert_eq!((a1, op1.callouts.len()), (lb + 0x1020, 0));
    let slot2 = EXE_BASE + u64::from(lay.delay_iat[0][2]);
    let f = w.l.resolve_delay(&mut w.p, EXE_BASE, desc, slot2).unwrap_err();
    assert!(!f.dll_failed);
    assert_eq!(f.status, STATUS_PROCEDURE_NOT_FOUND);
    // A thunk address that is not a slot.
    assert!(w.l.resolve_delay(&mut w.p, EXE_BASE, desc, slot0 + 3).is_err());
    assert!(w.l.resolve_delay(&mut w.p, EXE_BASE, EXE_BASE + 0x100_0000, slot0).is_err());
}

#[test]
fn threads_get_tls_and_attach_callouts() {
    let tls = |cb: u32| Some(Tls { template: vec![0xAB; 4], zero_fill: 4, callbacks: vec![cb], align_flag: 0 });
    let a = PeBuilder { tls: tls(0x1100), ..dll("a.dll", 0x1_8200_0000, named(&[("A", 0x1010)]), vec![]) };
    let quiet = dll("quiet.dll", 0x1_8300_0000, named(&[("Q", 0x1010)]), vec![]);
    let e = PeBuilder {
        tls: tls(0x1080),
        ..exe(vec![imports("a.dll", &[Sym::name("A")]), imports("quiet.dll", &[Sym::name("Q")])])
    };
    let mut w = world(&e.write());
    w.p.file("C:\\app\\a.dll", a.write());
    w.p.file("C:\\app\\quiet.dll", quiet.write());
    let start = w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    assert_eq!(
        start.last().map(|c| (c.kind, c.routine)),
        Some((CalloutKind::Tls, EXE_BASE + 0x1080)),
        "the exe's callback last"
    );
    w.l.disable_thread_calls(base_of(&w, "quiet.dll")).unwrap();
    let c = w.l.thread_attach(&mut w.p, TEB2).unwrap();
    assert_eq!(
        kinds(&c, &w),
        [
            ("a.dll".to_string(), CalloutKind::Tls, reason::THREAD_ATTACH),
            ("a.dll".to_string(), CalloutKind::Entry, reason::THREAD_ATTACH),
            ("test.exe".to_string(), CalloutKind::Tls, reason::THREAD_ATTACH),
        ]
    );
    // Two TLS indices, a block for each in the new thread.
    let array = read_u64(&mut w.p, TEB2 + TEB_TLS_POINTER).unwrap();
    for i in 0..2 {
        let b = read_u64(&mut w.p, array + 8 * i).unwrap();
        assert_eq!(w.p.memory(b, 8).unwrap(), &[0xAB, 0xAB, 0xAB, 0xAB, 0, 0, 0, 0]);
    }
    // A TLS DLL loaded later gets blocks in both threads (and grows arrays).
    let mut later = Vec::new();
    for k in 0..4u64 {
        let name = std::format!("t{k}.dll");
        let d =
            PeBuilder { tls: tls(0x1100), ..dll(&name, 0x1_9000_0000 + (k << 24), named(&[("X", 0x1010)]), vec![]) };
        w.p.file(&std::format!("C:\\app\\{name}"), d.write());
        let (id, base, _) = w.l.load(&mut w.p, &wide(&name), None).unwrap();
        later.push((id, base));
    }
    for teb in [TEB, TEB2] {
        let array = read_u64(&mut w.p, teb + TEB_TLS_POINTER).unwrap();
        for &(id, _) in &later {
            let idx = w.l.module(id).unwrap().tls_index.unwrap();
            let b = read_u64(&mut w.p, array + 8 * u64::from(idx)).unwrap();
            assert_eq!(b, w.l.tls_block(teb, idx).unwrap());
            assert_eq!(w.p.memory(b, 4).unwrap(), &[0xAB; 4]);
        }
    }
    // Unloading frees the index, which the next TLS DLL reuses.
    let (id0, base0) = later[0];
    let idx0 = w.l.module(id0).unwrap().tls_index.unwrap();
    let u = w.l.unload(base0).unwrap();
    w.l.finish_unload(&mut w.p, u);
    assert!(w.l.tls_block(TEB, idx0).is_none());
    let d = PeBuilder { tls: tls(0x1100), ..dll("again.dll", 0x1_9800_0000, named(&[("X", 0x1010)]), vec![]) };
    w.p.file("C:\\app\\again.dll", d.write());
    let (id, _, _) = w.l.load(&mut w.p, &wide("again"), None).unwrap();
    assert_eq!(w.l.module(id).unwrap().tls_index, Some(idx0));
    // Detach goes the other way; then the thread's TLS is freed.
    let det = w.l.thread_detach();
    assert_eq!(det.first().map(|c| c.module), Some(id));
    assert!(det.iter().all(|c| c.reason == reason::THREAD_DETACH));
    let heap = w.p.heap_blocks();
    w.l.thread_exit(&mut w.p, TEB2);
    assert!(w.p.heap_blocks() < heap);
    assert_eq!(read_u64(&mut w.p, TEB2 + TEB_TLS_POINTER).unwrap(), 0);
    // Process exit: everything detaches in reverse, the exe first.
    let exit = w.l.process_detach();
    assert_eq!(exit.first().map(|c| c.module), Some(id));
    assert!(exit.iter().all(|c| c.reason == reason::PROCESS_DETACH && c.reserved == 1));
    assert!(exit.iter().any(|c| c.module == w.exe));
}

#[test]
fn api_sets_and_the_search_order() {
    let e = exe(vec![imports("api-ms-win-core-rtlsupport-l1-1-0.dll", &[Sym::name("RtlFreeHeap")])]);
    let mut w = world(&e.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    assert_eq!(iat(&mut w, &e, 0, 0), NTDLL_BASE + 0x1020);
    assert_eq!(w.l.get_handle(&wide("api-ms-win-core-rtlsupport-l1-2-0")).unwrap(), NTDLL_BASE);
    assert_eq!(w.l.load(&mut w.p, &wide("ext-ms-win-something-l1-1-0.dll"), None).unwrap_err(), STATUS_DLL_NOT_FOUND);
    // An API set hosted by a DLL that is not there yet.
    w.p.tried.clear();
    assert_eq!(w.l.load(&mut w.p, &wide("api-ms-win-crt-runtime-l1-1-0"), None).unwrap_err(), STATUS_DLL_NOT_FOUND);
    assert_eq!(w.p.tried, ["\\??\\c:\\windows\\system32\\ucrtbase.dll"], "a known DLL: System32 only");
    // The app directory wins over System32; the rest are tried in order.
    let one = dll("one.dll", 0x1_8a00_0000, named(&[("X", 0x1010)]), vec![]);
    w.p.file("C:\\Windows\\System32\\one.dll", one.write());
    w.p.file("C:\\app\\one.dll", PeBuilder { image_base: 0x1_8b00_0000, ..one.clone() }.write());
    assert_eq!(w.l.load(&mut w.p, &wide("one"), None).unwrap().1, 0x1_8b00_0000);
    w.p.tried.clear();
    assert!(w.l.load(&mut w.p, &wide("two.dll"), None).is_err());
    assert_eq!(
        w.p.tried,
        [
            "\\??\\c:\\app\\two.dll",
            "\\??\\c:\\windows\\system32\\two.dll",
            "\\??\\c:\\windows\\system\\two.dll",
            "\\??\\c:\\windows\\two.dll",
            "\\??\\c:\\work\\two.dll",
            "\\??\\c:\\tools\\two.dll",
        ]
    );
    // A search path replaces the directories; a full path is tried alone.
    w.p.tried.clear();
    let sp = wide("D:\\x;D:\\y");
    assert!(w.l.load(&mut w.p, &wide("three"), Some(&sp)).is_err());
    assert_eq!(w.p.tried, ["\\??\\d:\\x\\three.dll", "\\??\\d:\\y\\three.dll"]);
    w.p.tried.clear();
    w.p.file("D:\\lib\\four.drv", dll("four.drv", 0x1_8c00_0000, vec![], vec![]).write());
    let (id, _, _) = w.l.load(&mut w.p, &wide("D:\\lib\\four.drv"), None).unwrap();
    assert_eq!(w.p.tried, ["\\??\\d:\\lib\\four.drv"]);
    assert_eq!(s(&w.l.module(id).unwrap().path), "D:\\lib\\four.drv");
    assert!(w.l.get_handle(&wide("four.drv")).is_ok());
    // Loading by a full path finds the already-loaded module.
    let (id2, _, op) = w.l.load(&mut w.p, &wide("d:\\LIB\\four.drv"), None).unwrap();
    assert_eq!((id2, op.callouts.len()), (id, 0));
}

#[test]
fn import_cycles_load_once() {
    let x = dll("x.dll", 0x1_8d00_0000, named(&[("X", 0x1010)]), vec![imports("y.dll", &[Sym::name("Y")])]);
    let y = dll("y.dll", 0x1_8e00_0000, named(&[("Y", 0x1020)]), vec![imports("x.dll", &[Sym::name("X")])]);
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    w.p.file("C:\\app\\x.dll", x.write());
    w.p.file("C:\\app\\y.dll", y.write());
    let (_, xb, op) = w.l.load(&mut w.p, &wide("x.dll"), None).unwrap();
    assert_eq!(kinds(&op.callouts, &w).iter().map(|k| k.0.clone()).collect::<Vec<_>>(), ["y.dll", "x.dll"]);
    let yb = base_of(&w, "y.dll");
    assert_eq!(read_u64(&mut w.p, yb + u64::from(y.layout().iat[0][0])).unwrap(), xb + 0x1010);
    assert_eq!(read_u64(&mut w.p, xb + u64::from(x.layout().iat[0][0])).unwrap(), yb + 0x1020);
    let u = w.l.unload(xb).unwrap();
    assert_eq!(u.modules().len(), 2, "the cycle goes as a whole");
    w.l.finish_unload(&mut w.p, u);
    assert_eq!(w.l.modules().len(), 2);
}

/// A small deterministic generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn malformed_images_never_panic() {
    let t = dll("t.dll", 0x1_8300_0000, named(&[("Real", 0x1080)]), vec![]);
    let good = PeBuilder {
        tls: Some(Tls { template: vec![1; 8], zero_fill: 8, callbacks: vec![0x1100], align_flag: 0 }),
        delay_imports: vec![Import {
            dll: "t.dll".into(),
            symbols: vec![Sym::name("Real")],
            bound: None,
            delay_thunks: vec![0x1010],
        }],
        ..dll(
            "m.dll",
            0x1_8f00_0000,
            vec![
                Export { name: Some("A".into()), ordinal: 1, target: Target::Rva(0x1010) },
                Export { name: Some("F".into()), ordinal: 2, target: Target::Forward("t.Real".into()) },
            ],
            vec![imports("t.dll", &[Sym::name("Real"), Sym::Ordinal(1)])],
        )
    }
    .write();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut loaded = 0;
    for round in 0..600 {
        let mut img = good.clone();
        match round % 3 {
            0 => img.truncate((rng.next() % good.len() as u64) as usize),
            _ => {
                for _ in 0..1 + rng.next() % 8 {
                    let at = (rng.next() % img.len() as u64) as usize;
                    img[at] = rng.next() as u8;
                }
            }
        }
        let mut w = world(&exe(vec![]).write());
        w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
        w.p.file("C:\\app\\t.dll", t.write());
        w.p.file("C:\\app\\m.dll", img);
        if let Ok((_, base, _)) = w.l.load(&mut w.p, &wide("m.dll"), None) {
            loaded += 1;
            let _ = w.l.get_procedure(&mut w.p, base, Some(b"F"), 0);
            let _ = w.l.thread_attach(&mut w.p, TEB2);
            let u = w.l.unload(base).unwrap();
            w.l.finish_unload(&mut w.p, u);
        }
        assert_eq!(w.l.modules().len(), 2, "round {round}: nothing of the image stays");
    }
    assert!(loaded > 0, "some damaged images still load");
}

#[test]
fn ldr_entries_carry_names_and_headers() {
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let ldr = read_u64(&mut w.p, PEB + PEB_LDR).unwrap();
    let load = walk(&mut w.p, ldr + LDR_IN_LOAD_ORDER, ENTRY_IN_LOAD_ORDER, 10).unwrap();
    let e = load[1];
    let full_len = u16::from_le_bytes(w.p.memory(e + ENTRY_FULL_NAME, 2).unwrap().try_into().unwrap());
    assert_eq!(usize::from(full_len), "C:\\Windows\\System32\\ntdll.dll".len() * 2);
    assert_eq!(read_u64(&mut w.p, e + ENTRY_SIZE_OF_IMAGE).unwrap(), w.l.module(w.ntdll).unwrap().size);
    assert_eq!(read_u32(&mut w.p, e + ENTRY_TIME_DATE_STAMP).unwrap(), 0x5000_0000);
    assert_eq!(w.l.module(w.ntdll).unwrap().record, e);
}
