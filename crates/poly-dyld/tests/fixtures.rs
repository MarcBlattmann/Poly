//! Linking the test programs of `tests/data` (built by `generate.sh` from
//! our own sources) into a simulated memory, and checking every pointer
//! the loader writes. Expected export offsets and slot addresses were
//! recorded with `llvm-objdump --macho --dyld-info --exports-trie` and
//! `otool -lv` on those files.

use poly_dyld::{Entry, Error, FileSystem, Options, Program, link};
use poly_macho::CpuKind;
use std::collections::BTreeMap;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/data/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// Files by path.
#[derive(Default)]
struct Files(BTreeMap<String, Vec<u8>>);

impl Files {
    fn with(mut self, path: &str, file: &str) -> Self {
        self.0.insert(path.into(), data(file));
        self
    }
}

impl FileSystem for Files {
    fn read(&mut self, path: &str) -> Option<Vec<u8>> {
        self.0.get(path).cloned()
    }
}

const LIBDIR: &str = "/usr/lib/poly-dyld-test";

fn x64_files() -> Files {
    Files::default()
        .with("/bin/hello", "hello_x64")
        .with(&format!("{LIBDIR}/libgreet.dylib"), "libgreet_x64.dylib")
        .with("/usr/lib/libSystem.B.dylib", "libSystem_x64.dylib")
}

/// A byte-addressed memory holding the mapped segments, with the fixups
/// written into it.
struct Memory(BTreeMap<u64, u8>);

impl Memory {
    fn load(p: &Program) -> Memory {
        let mut m = BTreeMap::new();
        for im in &p.images {
            for s in &im.segments {
                for k in 0..s.mem_size {
                    let b = if k < s.file_size { im.file[(s.file_offset + k) as usize] } else { 0 };
                    m.insert(s.addr + k, b);
                }
            }
        }
        let mut mem = Memory(m);
        p.fixups(&mut |addr, value| mem.write(addr, value)).unwrap();
        mem
    }

    fn write(&mut self, addr: u64, value: u64) {
        for (k, b) in value.to_le_bytes().into_iter().enumerate() {
            let slot = self.0.get_mut(&(addr + k as u64)).unwrap_or_else(|| panic!("write to unmapped {addr:#x}"));
            *slot = b;
        }
    }

    fn u64(&self, addr: u64) -> u64 {
        let mut b = [0u8; 8];
        for (k, x) in b.iter_mut().enumerate() {
            *x = self.0[&(addr + k as u64)];
        }
        u64::from_le_bytes(b)
    }

    fn u32(&self, addr: u64) -> u32 {
        self.u64(addr) as u32
    }
}

#[test]
fn chained_x86_64_program_and_libraries() {
    let p = link(&mut x64_files(), "/bin/hello", &Options::X86_64).unwrap();
    let paths: Vec<&str> = p.images.iter().map(|i| i.path.as_str()).collect();
    // Breadth first, from @rpath (`@executable_path/../usr/lib/poly-dyld-test`).
    assert_eq!(paths, ["/bin/hello", "/usr/lib/poly-dyld-test/libgreet.dylib", "/usr/lib/libSystem.B.dylib"]);
    assert_eq!(p.images[1].install_name.as_deref(), Some("@rpath/libgreet.dylib"));
    // The program is not moved; libraries go from 32 GiB on, apart.
    let (exe, greet, sys) = (&p.images[0], &p.images[1], &p.images[2]);
    assert_eq!(exe.base, 0x1_0000_0000);
    assert_eq!(greet.base, 0x8_0000_0000);
    assert!(sys.base >= greet.range().end + 4096);
    assert_eq!(exe.deps, [Some(1), Some(2)]);
    assert_eq!(greet.deps, [Some(2)]);
    // Segments: __TEXT r-x, __DATA_CONST read-only after fixups, no __PAGEZERO.
    let names: Vec<&str> = exe.segments.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["__TEXT", "__DATA_CONST", "__DATA", "__LINKEDIT"]);
    assert!(exe.segments[0].prot.execute && !exe.segments[0].prot.write);
    assert!(exe.segments[1].read_only_after_fixups);

    let mem = Memory::load(&p);
    // __got of the program: each import bound across images.
    assert_eq!(mem.u64(0x1_0000_1000), sys.base + 0x270, "_poly_write");
    assert_eq!(mem.u64(0x1_0000_1008), greet.base + 0x202c, "_greet_inits");
    assert_eq!(mem.u64(0x1_0000_1010), 0, "_poly_missing: a missing weak import is null");
    assert_eq!(mem.u64(0x1_0000_1018), greet.base + 0x660, "_greet_word");
    assert_eq!(mem.u64(0x1_0000_1020), greet.base + 0x5b0, "_greet");
    // The library's pointer table, rebased.
    assert_eq!(mem.u64(greet.base + 0x2000), greet.base + 0x680);
    assert_eq!(mem.u64(greet.base + 0x2008), greet.base + 0x686);
    assert_eq!(mem.u64(greet.base + 0x1000), sys.base + 0x270, "the library's _poly_write");

    // Thread-local descriptors: the thunk words are left for tlv_writes.
    let tlv = exe.tlv.as_ref().unwrap();
    assert_eq!(tlv.descriptors, [0x1_0000_2000, 0x1_0000_2018]);
    assert_eq!(mem.u64(0x1_0000_2000), 0x8030_0000_0000_0005, "untouched");
    assert_eq!(tlv.init_size, 4);
    assert!(tlv.size >= 16);
    let mut w = Vec::new();
    p.tlv_writes(0, 0x5000, 0x6000, &mut |a, v| w.push((a, v)));
    assert_eq!(w, [(0x1_0000_2000, 0x6000), (0x1_0000_2008, 0x5000), (0x1_0000_2018, 0x6000), (0x1_0000_2020, 0x5000)]);
    assert!(greet.tlv.is_some() && sys.tlv.is_none());

    // libSystem, then libgreet, then the program; one initializer each.
    assert_eq!(p.init_order, [2, 1, 0]);
    assert_eq!(greet.initializers.len(), 1);
    assert_eq!(exe.initializers.len(), 1);
    assert_eq!(p.initializers(), [greet.initializers[0], exe.initializers[0]]);
    assert!(exe.segments[0].addr < exe.initializers[0] && exe.initializers[0] < exe.segments[0].addr + 0x1000);

    // main, exit, __mh_execute_header.
    assert!(p.dynamic);
    assert_eq!(p.entry, Entry::Main { addr: p.lookup(0, b"_main").unwrap().unwrap(), stack_size: 0 });
    assert_eq!(p.exit_function().unwrap(), Some(sys.base + 0x240));
    assert_eq!(p.lookup(0, b"__mh_execute_header").unwrap(), Some(exe.base));
    assert_eq!(mem.u32(exe.base), 0xfeed_facf);
}

#[test]
fn a_moved_program_is_rebased() {
    let opts = Options { program_base: Some(0x2_0000_0000), ..Options::X86_64 };
    let p = link(&mut x64_files(), "/bin/hello", &opts).unwrap();
    assert_eq!(p.images[0].base, 0x2_0000_0000);
    assert_eq!(p.images[0].slide, 0x1_0000_0000);
    let mem = Memory::load(&p);
    assert_eq!(mem.u64(0x2_0000_1020), p.images[1].base + 0x5b0);
    assert_eq!(p.lookup(0, b"__mh_execute_header").unwrap(), Some(0x2_0000_0000));
    assert_eq!(p.images[0].tlv.as_ref().unwrap().descriptors[0], 0x2_0000_2000);
}

#[test]
fn classic_opcodes_bind_lazily_bound_pointers_at_load() {
    let mut fs = Files::default()
        .with("/bin/helloc", "helloc_x64")
        .with(&format!("{LIBDIR}/libgreetc.dylib"), "libgreetc_x64.dylib")
        .with("/usr/lib/libSystem.B.dylib", "libSystemc_x64.dylib");
    for opts in [Options::X86_64, Options { program_base: Some(0x3_0000_0000), ..Options::X86_64 }] {
        let p = link(&mut fs, "/bin/helloc", &opts).unwrap();
        let (exe, greet, sys) = (&p.images[0], &p.images[1], &p.images[2]);
        let s = exe.slide;
        let mem = Memory::load(&p);
        let poly_write = p.lookup(2, b"_poly_write").unwrap().unwrap();
        // Non-lazy pointers (__got).
        assert_eq!(mem.u64(0x1_0000_1000 + s), p.lookup(1, b"_greet_inits").unwrap().unwrap());
        assert_eq!(mem.u64(0x1_0000_1008 + s), 0, "_poly_missing");
        assert_eq!(mem.u64(0x1_0000_1010 + s), p.lookup(2, b"dyld_stub_binder").unwrap().unwrap());
        // Lazy pointers, bound now (their rebase to the stub helper first).
        assert_eq!(mem.u64(0x1_0000_2000 + s), poly_write);
        assert_eq!(mem.u64(0x1_0000_2008 + s), p.lookup(1, b"_greet_word").unwrap().unwrap());
        assert_eq!(mem.u64(0x1_0000_2010 + s), p.lookup(1, b"_greet").unwrap().unwrap());
        // __mod_init_func: rebased pointers to the initializers.
        assert_eq!(exe.initializers.len(), 1);
        assert_eq!(exe.initializers[0], mem.u64(0x1_0000_1018 + s));
        assert!(exe.segments[0].addr <= exe.initializers[0] && exe.initializers[0] < exe.segments[0].addr + 4096);
        assert_eq!(greet.initializers, [mem.u64(greet.base + 0x1008)]);
        // The library's data pointers (rebases by opcode).
        let words = mem.u64(greet.base + 0x2010);
        assert!(greet.range().contains(&words));
        assert!(sys.initializers.is_empty());
        assert_eq!(p.init_order, [2, 1, 0]);
    }
}

#[test]
fn arm64_program_links_with_16k_pages() {
    let mut fs = Files::default()
        .with("/bin/hello", "hello_a64")
        .with(&format!("{LIBDIR}/libgreet.dylib"), "libgreet_a64.dylib")
        .with("/usr/lib/libSystem.B.dylib", "libSystem_a64.dylib");
    let opts = Options { cpu: CpuKind::Arm64, page_size: 16384, ..Options::X86_64 };
    let p = link(&mut fs, "/bin/hello", &opts).unwrap();
    assert_eq!(p.images.len(), 3);
    let greet = &p.images[1];
    let mem = Memory::load(&p);
    assert_eq!(mem.u64(greet.base + 0x8000), greet.base + 0x684);
    assert_eq!(mem.u64(greet.base + 0x4000), p.lookup(2, b"_poly_write").unwrap().unwrap());
    assert_eq!(p.init_order, [2, 1, 0]);
    assert!(matches!(p.entry, Entry::Main { .. }));
    // The x86-64 loader refuses it with a reason.
    let e = link(&mut fs, "/bin/hello", &Options::X86_64).unwrap_err();
    assert_eq!(e, Error::WrongCpu { path: "/bin/hello".into() });
}

#[test]
fn static_program_needs_no_dyld() {
    let mut fs = Files::default().with("/bin/static", "static_x64");
    let p = link(&mut fs, "/bin/static", &Options::X86_64).unwrap();
    assert!(!p.dynamic);
    assert_eq!(p.images.len(), 1);
    let Entry::Thread { pc, sp } = p.entry else { panic!("{:?}", p.entry) };
    assert_eq!(sp, 0);
    assert!(p.images[0].segments[0].addr <= pc && pc < p.images[0].range().end);
    assert!(p.initializers().is_empty());
}

#[test]
fn errors_name_what_is_missing() {
    let mut fs = Files::default().with("/bin/hello", "hello_x64");
    let e = link(&mut fs, "/bin/hello", &Options::X86_64).unwrap_err();
    assert_eq!(e, Error::LibraryNotFound { name: "@rpath/libgreet.dylib".into(), from: "/bin/hello".into() });
    assert_eq!(e.to_string(), "Library not loaded: @rpath/libgreet.dylib, referenced from /bin/hello");

    // A libSystem without _poly_write.
    let mut fs = Files::default()
        .with("/bin/hello", "hello_x64")
        .with(&format!("{LIBDIR}/libgreet.dylib"), "libgreet_x64.dylib")
        .with("/usr/lib/libSystem.B.dylib", "libgreetc_x64.dylib");
    let e = link(&mut fs, "/bin/hello", &Options::X86_64);
    // libgreetc's install name is not libSystem's: found by path all the
    // same, its exports lack the symbol (its own import of it fails first,
    // while its initializers are worked out).
    let e = e.and_then(|p| p.fixups(&mut |_, _| {}).map(|_| ())).unwrap_err();
    let Error::SymbolNotFound { symbol, expected_in, .. } = &e else { panic!("{e:?}") };
    assert_eq!((symbol.as_str(), expected_in.as_str()), ("_poly_write", "/usr/lib/libSystem.B.dylib"));
    assert!(e.to_string().starts_with("Symbol not found: _poly_write, referenced from "));

    // A library where a program is expected.
    let mut fs = Files::default().with("/bin/x", "libgreet_x64.dylib");
    assert_eq!(link(&mut fs, "/bin/x", &Options::X86_64).unwrap_err(), Error::WrongFileType { path: "/bin/x".into() });
    // Not Mach-O at all.
    let mut fs = Files::default();
    fs.0.insert("/bin/x".into(), b"#!/bin/sh\n".to_vec());
    assert!(matches!(link(&mut fs, "/bin/x", &Options::X86_64).unwrap_err(), Error::Macho { .. }));
}

#[test]
fn corrupt_files_never_panic() {
    // Every truncation and a sweep of byte flips of the program and its
    // library: link and fixups return errors, never panic.
    let exe = data("hello_x64");
    let lib = data("libgreet_x64.dylib");
    let sys = data("libSystem_x64.dylib");
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rand = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..3000 {
        let mut e = exe.clone();
        let mut l = lib.clone();
        let target = if round % 2 == 0 { &mut e } else { &mut l };
        if round % 7 == 0 {
            let n = (rand() as usize) % target.len();
            target.truncate(n);
        } else {
            for _ in 0..(1 + rand() % 4) {
                let i = (rand() as usize) % target.len();
                target[i] ^= 1 << (rand() % 8);
            }
        }
        let mut fs = Files::default();
        fs.0.insert("/bin/hello".into(), e);
        fs.0.insert(format!("{LIBDIR}/libgreet.dylib"), l);
        fs.0.insert("/usr/lib/libSystem.B.dylib".into(), sys.clone());
        if let Ok(p) = link(&mut fs, "/bin/hello", &Options::X86_64) {
            let _ = p.fixups(&mut |_, _| {});
            let _ = p.exit_function();
        }
    }
}
