//! The runtime on real compiled metadata: the programs of `tests/data`
//! (our own `classes.m`, linked once by Apple's ld with relative method
//! lists and once by rust-lld with plain ones) are loaded by Poly's dyld
//! into a simulated memory, and the runtime maps them: classes, superclass
//! links, method lists, categories, selector uniquing and the `+load`
//! order of the `_objc_load_image` sheet.

use poly_dyld::{FileSystem, Options, Program, link};
use poly_objc::Mem;
use poly_objc::runtime::Runtime;
use std::collections::BTreeMap;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

struct Files(BTreeMap<String, Vec<u8>>);

impl FileSystem for Files {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.0.get(path).cloned()
    }
}

/// The program's memory after dyld: every segment, every fixup.
struct Sim {
    bytes: BTreeMap<u64, u8>,
    next: u64,
}

impl Sim {
    fn load(p: &Program) -> Sim {
        let mut bytes = BTreeMap::new();
        for im in &p.images {
            for s in &im.segments {
                for k in 0..s.mem_size {
                    let b = if k < s.file_size { im.file[(s.file_offset + k) as usize] } else { 0 };
                    bytes.insert(s.addr + k, b);
                }
            }
        }
        let mut sim = Sim { bytes, next: 0x40_0000_0000 };
        p.fixups(&mut |addr, value| sim.write_u64(addr, value)).unwrap();
        sim
    }
}

impl Mem for Sim {
    fn u8(&self, addr: u64) -> u8 {
        self.bytes.get(&addr).copied().unwrap_or(0)
    }
    fn write_u64(&mut self, addr: u64, value: u64) {
        for (i, b) in value.to_le_bytes().into_iter().enumerate() {
            self.bytes.insert(addr + i as u64, b);
        }
    }
    fn alloc(&mut self, bytes: &[u8]) -> u64 {
        let at = self.next;
        for (i, &b) in bytes.iter().enumerate() {
            self.bytes.insert(at + i as u64, b);
        }
        self.next += (bytes.len() as u64).next_multiple_of(16).max(16);
        at
    }
}

struct Loaded {
    rt: Runtime,
    mem: Sim,
    /// Implementation address -> its symbol ("+[Zeta load]").
    names: BTreeMap<u64, String>,
    loads: Vec<String>,
    linker: &'static str,
}

fn load(file: &str, linker: &'static str) -> Loaded {
    let mut fs = Files(BTreeMap::new());
    fs.0.insert("/bin/classes".into(), data(file));
    fs.0.insert("/usr/lib/libobjc.A.dylib".into(), data("libobjc_stub.dylib"));
    fs.0.insert(
        "/usr/lib/libSystem.B.dylib".into(),
        std::fs::read(format!("{}/../poly-dyld/tests/data/libSystem_x64.dylib", env!("CARGO_MANIFEST_DIR"))).unwrap(),
    );
    let p = link(&mut fs, "/bin/classes", &Options::X86_64).unwrap();
    let mut mem = Sim::load(&p);
    // The function symbols of the program, by address.
    let exe = &p.images[0];
    let m = exe.macho().unwrap();
    let st = m.symtab().unwrap().unwrap();
    let mut names = BTreeMap::new();
    for sym in st.iter() {
        let name = String::from_utf8_lossy(st.name(&sym).unwrap()).into_owned();
        if name.starts_with("+[") || name.starts_with("-[") {
            names.insert(sym.n_value.wrapping_add(exe.slide), name);
        }
    }
    let mut rt = Runtime::new();
    let mut loads = Vec::new();
    // The library first (its classes are the program's superclasses).
    let lib = p.image_named("/usr/lib/libobjc.A.dylib").unwrap();
    for i in [lib, 0] {
        let path = mem.alloc(format!("{}\0", p.images[i].path).as_bytes());
        let (plan, _) = rt.map_image(&mut mem, p.images[i].base, path);
        loads.extend(plan.iter().map(|&(_, imp)| names.get(&imp).cloned().unwrap_or_else(|| format!("{imp:#x}"))));
    }
    Loaded { rt, mem, names, loads, linker }
}

fn both() -> [Loaded; 2] {
    [load("classes_ld64", "ld64"), load("classes_lld", "lld")]
}

/// The +load method names in the order the runtime calls them, by owner:
/// "+[Zeta load]" -> "Zeta", "+[Zeta(CatZ) load]" -> "Zeta(CatZ)".
fn owners(l: &Loaded) -> Vec<String> {
    l.loads.iter().map(|n| n.trim_start_matches("+[").trim_end_matches(" load]").to_string()).collect()
}

#[test]
fn load_order_follows_the_sheet() {
    for l in both() {
        // Example 1 and example 3 of the _objc_load_image sheet, in one
        // image: the classes first (superclasses before subclasses, a
        // superclass without +load running its category in the chain),
        // then the remaining categories in category-list order.
        assert_eq!(
            owners(&l),
            [
                "Zeta",
                "Mid",
                "Alpha",
                "Q",
                "P",
                "R(C2)",
                "S",
                "Zeta(CatZ)",
                "Alpha(CatB)",
                "Alpha(CatA)",
                "Q(C4)",
                "S(C3)",
                "P(C1)"
            ],
            "{}",
            l.linker
        );
    }
}

#[test]
fn classes_superclasses_and_metaclasses() {
    for l in both() {
        let rt = &l.rt;
        let ns = rt.class_named(b"NSObject").unwrap();
        let zeta = rt.class_named(b"Zeta").unwrap();
        let mid = rt.class_named(b"Mid").unwrap();
        assert_eq!(rt.class(mid).unwrap().superclass, zeta, "{}", l.linker);
        assert_eq!(rt.class(zeta).unwrap().superclass, ns);
        let (mm, zm) = (rt.class(mid).unwrap().isa, rt.class(zeta).unwrap().isa);
        assert!(rt.class(mm).unwrap().meta && !rt.class(mid).unwrap().meta);
        assert_eq!(rt.class(mm).unwrap().superclass, zm, "a metaclass's superclass is the superclass's metaclass");
        assert_eq!(l.mem.cstr(rt.class(mm).unwrap().name), b"Mid");
        assert_eq!(rt.class(mid).unwrap().instance_size, 8);
        assert!(rt.is_subclass(mid, ns));
        assert_eq!(rt.class(mid).unwrap().image, Some(1));
        // Every class of the source, and NSObject.
        let mut names: Vec<String> =
            rt.class_list().iter().map(|&c| String::from_utf8(rt.class_name(&l.mem, c).unwrap()).unwrap()).collect();
        names.sort();
        assert_eq!(names, ["Alpha", "Lazy", "Leaf", "Mid", "NSObject", "P", "Q", "Quiet", "R", "S", "Shadow", "Zeta"]);
    }
}

#[test]
fn categories_win_dispatch_and_come_first() {
    for mut l in both() {
        let shadow = l.rt.class_named(b"Shadow").unwrap();
        let imp_of = |l: &mut Loaded, sel: &[u8]| {
            let s = l.rt.sel_lookup(sel).unwrap();
            let imp = l.rt.lookup(shadow, s).unwrap();
            l.names[&imp].clone()
        };
        assert_eq!(imp_of(&mut l, b"shadowed"), "-[Shadow(One) shadowed]", "{}", l.linker);
        assert_eq!(imp_of(&mut l, b"twice"), "-[Shadow(Two) twice]", "the later category wins");
        assert_eq!(imp_of(&mut l, b"own"), "-[Shadow own]");
        assert_eq!(imp_of(&mut l, b"fromOne"), "-[Shadow(One) fromOne]");
        // Both -shadowed stay in the list, the category's first; 6 methods.
        let names: Vec<String> =
            l.rt.class(shadow)
                .unwrap()
                .methods
                .iter()
                .map(|&h| String::from_utf8(l.mem.cstr(l.rt.method(h).unwrap().sel)).unwrap())
                .collect();
        assert_eq!(names.len(), 6, "{}: {names:?}", l.linker);
        let first = names.iter().position(|n| n == "shadowed").unwrap();
        let h = l.rt.class(shadow).unwrap().methods[first];
        assert_eq!(l.names[&l.rt.method(h).unwrap().imp], "-[Shadow(One) shadowed]");
        assert_eq!(names.iter().filter(|n| *n == "shadowed").count(), 2);
        assert_eq!(names.iter().filter(|n| *n == "twice").count(), 2);
        // Class methods of a class and of its categories are on the
        // metaclass; a category without +load still attached.
        let quiet = l.rt.class_named(b"Quiet").unwrap();
        let q = l.rt.sel_lookup(b"q").unwrap();
        assert_eq!(l.names[&l.rt.lookup(quiet, q).unwrap()], "-[Quiet(CatQ) q]");
        let lazy = l.rt.class_named(b"Lazy").unwrap();
        let lazy_meta = l.rt.class(lazy).unwrap().isa;
        let ls = l.rt.sel_lookup(b"lazy").unwrap();
        assert_eq!(l.names[&l.rt.lookup(lazy_meta, ls).unwrap()], "+[Lazy lazy]");
        assert_eq!(l.rt.lookup(lazy, ls), None, "a class method is not an instance method");
    }
}

#[test]
fn selector_references_are_unique() {
    for mut l in both() {
        // Methods' selectors are the canonical ones the table gives back.
        let s = l.rt.sel(&mut l.mem, b"own");
        let shadow = l.rt.class_named(b"Shadow").unwrap();
        assert!(l.rt.own_method(shadow, s).is_some(), "{}", l.linker);
        assert_eq!(l.mem.cstr(s), b"own");
    }
}
