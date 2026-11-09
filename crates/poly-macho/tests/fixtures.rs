//! The real test files in `tests/data/`, made by clang and ld64.lld (see the
//! README there), against values recorded from `llvm-objdump --macho`,
//! `llvm-nm` and `llvm-lipo` run on the same files. The values are literals:
//! regenerating the files with other tools means checking them again.

mod support;

use poly_macho::*;
use std::ops::ControlFlow;
use support::*;

fn s(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap()
}

/// `(name, flags, target)` of every export, in trie order.
fn exports(m: &MachO) -> Vec<(String, u64, ExportTarget<'static>)> {
    let mut v = Vec::new();
    m.exports()
        .unwrap()
        .expect("exports")
        .walk(|name, e| {
            let t = match e.target {
                ExportTarget::Address(a) => ExportTarget::Address(a),
                ExportTarget::StubAndResolver { stub, resolver } => ExportTarget::StubAndResolver { stub, resolver },
                ExportTarget::Reexport { ordinal, .. } => ExportTarget::Reexport { ordinal, name: &[] },
            };
            v.push((s(name).to_string(), e.flags, t));
            ControlFlow::Continue(())
        })
        .unwrap();
    v
}

fn sorted_exports(m: &MachO) -> Vec<(String, u64, u64)> {
    let mut v: Vec<_> = exports(m)
        .into_iter()
        .map(|(n, f, t)| match t {
            ExportTarget::Address(a) => (n, f, a),
            _ => panic!("unexpected export target"),
        })
        .collect();
    v.sort();
    v
}

fn symbol_names(m: &MachO) -> Vec<String> {
    let st = m.symtab().unwrap().unwrap();
    st.iter().map(|n| s(st.name(&n).unwrap()).to_string()).collect()
}

fn segment_names(m: &MachO) -> Vec<String> {
    m.segment_table().iter().map(|g| s(g.name()).to_string()).collect()
}

type BindRow = (u8, u64, i32, &'static str, u8);

fn binds(m: &MachO) -> Vec<BindRow> {
    let info = m.dyld_info().unwrap().unwrap();
    info.binds()
        .map(|b| {
            let b = b.unwrap();
            // The leaked name keeps the row type simple in the tests.
            (
                b.segment,
                b.segment_offset,
                b.ordinal,
                Box::leak(s(b.symbol).to_string().into_boxed_str()) as &'static str,
                b.flags,
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// arm64, classic opcodes
// ---------------------------------------------------------------------------

#[test]
fn arm64_classic_header_and_summary() {
    let data = fixture("arm64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    let h = m.header();
    assert_eq!(h.cpu.kind(), CpuKind::Arm64);
    assert_eq!(h.cpu.cputype, CPU_TYPE_ARM64);
    assert_eq!(h.cpu.subtype(), CPU_SUBTYPE_ARM64_ALL);
    assert_eq!(h.file_type(), FileType::Execute);
    assert_eq!((h.ncmds, h.sizeofcmds), (20, 1456));
    // NOUNDEFS DYLDLINK TWOLEVEL WEAK_DEFINES BINDS_TO_WEAK PIE
    assert_eq!(h.flags, MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_WEAK_DEFINES | MH_BINDS_TO_WEAK | MH_PIE);
    assert!(h.is_pie() && h.is_two_level());

    let sum = m.summary().unwrap();
    assert!(sum.is_pie && sum.is_slidable && !sum.has_chained_fixups);
    assert_eq!(sum.base_address, 0x1_0000_0000);
    assert_eq!(sum.vm_range, 0x1_0000_0000..0x1_0000_c540);
    assert_eq!(sum.entry, Some(Entry::Main { entryoff: 0x600, vmaddr: 0x1_0000_0600, stacksize: 0 }));
    assert_eq!(
        sum.platform,
        Some(Platform { platform: PLATFORM_MACOS, minos: Version(0x000b_0000), sdk: Version(0x000b_0000) })
    );
    assert_eq!(sum.dylinker, Some("/usr/lib/dyld"));
    assert_eq!(sum.install_name, None);
    assert!(sum.uuid.is_some());
}

#[test]
fn arm64_classic_segments_and_sections() {
    let data = fixture("arm64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(segment_names(&m), ["__PAGEZERO", "__TEXT", "__DATA_CONST", "__DATA", "__LINKEDIT"]);
    // (vmaddr, vmsize, fileoff, filesize, initprot as r/w/x)
    let expect = [
        (0u64, 0x1_0000_0000u64, 0u64, 0u64, "---"),
        (0x1_0000_0000, 0x4000, 0, 0x4000, "r-x"),
        (0x1_0000_4000, 0x4000, 0x4000, 0x4000, "rw-"),
        (0x1_0000_8000, 0x4000, 0x8000, 0x4000, "rw-"),
        (0x1_0000_c000, 0x540, 0xc000, 0x540, "r--"),
    ];
    for (g, e) in m.segment_table().iter().zip(expect) {
        assert_eq!((g.vmaddr, g.vmsize, g.fileoff, g.filesize), (e.0, e.1, e.2, e.3));
        assert_eq!(g.initprot.to_string(), e.4);
    }
    assert!(m.segment_table()[0].is_pagezero());
    assert_eq!(m.header_segment().unwrap().vmaddr, 0x1_0000_0000);
    assert_eq!(m.find_segment(b"__DATA").unwrap().0, 3);
    assert_eq!(m.segment_table()[2].flags, SG_READ_ONLY);

    // (section, segment, addr, size, offset, type)
    let mut sections = Vec::new();
    for seg in m.segments() {
        for sec in seg.sections() {
            sections.push((
                s(sec.name()).to_string(),
                s(sec.segment_name()).to_string(),
                sec.addr,
                sec.size,
                sec.offset,
                sec.section_type(),
            ));
        }
    }
    let t = |a: &str, b: &str, c, d, e, f| (a.to_string(), b.to_string(), c, d, e, f);
    assert_eq!(
        sections,
        [
            t("__text", "__TEXT", 0x1_0000_05f0, 0x90, 0x5f0, S_REGULAR),
            t("__stubs", "__TEXT", 0x1_0000_0680, 0x30, 0x680, S_SYMBOL_STUBS),
            t("__stub_helper", "__TEXT", 0x1_0000_06b0, 0x30, 0x6b0, S_REGULAR),
            t("__cstring", "__TEXT", 0x1_0000_06e0, 5, 0x6e0, S_CSTRING_LITERALS),
            t("__got", "__DATA_CONST", 0x1_0000_4000, 0x18, 0x4000, S_NON_LAZY_SYMBOL_POINTERS),
            t("__la_symbol_ptr", "__DATA", 0x1_0000_8000, 0x20, 0x8000, S_LAZY_SYMBOL_POINTERS),
            t("__data", "__DATA", 0x1_0000_8020, 0x40, 0x8020, S_REGULAR),
        ]
    );
    let text = m.segments().nth(1).unwrap();
    let first = text.sections().next().unwrap();
    assert_eq!(first.flags & !SECTION_TYPE, S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS);
    let stubs = text.sections().nth(1).unwrap();
    assert_eq!((stubs.reserved1, stubs.reserved2), (3, 12));
}

#[test]
fn arm64_classic_dependencies_and_load_commands() {
    let data = fixture("arm64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    let libs: Vec<_> = m.dylibs().map(|d| (d.kind, d.name, d.current_version, d.compatibility_version)).collect();
    assert_eq!(
        libs,
        [
            (DylibKind::Load, "/usr/lib/libSystem.B.dylib", Version(0), Version(0)),
            (DylibKind::Load, "@rpath/libtiny.dylib", Version(0x0001_0203), Version(0x0001_0000)),
            (DylibKind::Weak, "@rpath/libweakdep.dylib", Version(0), Version(0)),
        ]
    );
    assert_eq!(m.dylib_by_ordinal(2).unwrap().name, "@rpath/libtiny.dylib");
    assert_eq!(m.dylib_by_ordinal(3).unwrap().kind, DylibKind::Weak);
    assert!(m.dylib_by_ordinal(0).is_none() && m.dylib_by_ordinal(4).is_none());
    assert!(m.dylib_id().is_none());
    assert_eq!(Version(0x0001_0203).to_string(), "1.2.3");

    let rp: Vec<_> = m.rpaths().collect();
    assert_eq!(rp, ["@executable_path/../lib", "/opt/poly/lib"]);

    // BUILD_VERSION lists lld 18.1.3 as the tool.
    let bv = m
        .commands()
        .find_map(|c| match c {
            Command::BuildVersion(b) => Some(b),
            _ => None,
        })
        .unwrap();
    let tools: Vec<_> = bv.tools().collect();
    assert_eq!(tools, [BuildTool { tool: TOOL_LLD, version: Version(0x0012_0103) }]);

    // The linkedit blobs: function starts, data in code, no signature
    // check here, just the ranges the tool printed.
    let fs = m.function_starts().unwrap().unwrap();
    let starts: Vec<u64> = fs.map(|r| r.unwrap()).collect();
    assert_eq!(starts, [0x5f0, 0x5f8, 0x600]);
    assert_eq!(m.data_in_code().unwrap().unwrap().count(), 0);
    let sig = m.linkedit_data(LinkeditKind::CodeSignature).unwrap().unwrap();
    assert_eq!(sig.len(), 560);
    assert!(m.linkedit_data(LinkeditKind::ExportsTrie).unwrap().is_none());
    assert!(m.chained_fixups().unwrap().is_none());
    assert!(m.source_version().is_none());
}

#[test]
fn arm64_classic_symbols() {
    let data = fixture("arm64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(
        symbol_names(&m),
        [
            "_local_fn",
            "_local_value",
            "__dyld_private",
            "_main",
            "_app_weak",
            "_fn_table",
            "_value_ptrs",
            "_name",
            "__mh_execute_header",
            "dyld_stub_binder",
            "_lib_add",
            "_lib_value",
            "_lib_sub",
            "_lib_weak",
            "_weakdep_fn",
        ]
    );
    let st = m.symtab().unwrap().unwrap();
    assert_eq!(st.len(), 15);
    let by_name = |n: &str| st.iter().find(|x| st.name(x).unwrap() == n.as_bytes()).unwrap();
    let main = by_name("_main");
    assert_eq!(main.kind(), SymbolKind::Section);
    assert!(main.is_external() && !main.is_weak_def());
    assert_eq!((main.n_sect, main.n_value), (1, 0x1_0000_0600));
    let local = by_name("_local_fn");
    assert!(!local.is_external());
    assert_eq!(local.n_value, 0x1_0000_05f0);
    assert!(by_name("_app_weak").is_weak_def());
    let add = by_name("_lib_add");
    assert!(add.is_undefined() && add.is_external());
    assert_eq!(add.library_ordinal(), 2);
    assert_eq!(by_name("_weakdep_fn").library_ordinal(), 3);
    assert!(by_name("_lib_weak").is_weak_ref());
    assert_eq!(by_name("dyld_stub_binder").library_ordinal(), 1);
    assert!(by_name("__mh_execute_header").n_desc & REFERENCED_DYNAMICALLY != 0);

    let dy = m.dysymtab().unwrap().unwrap();
    assert_eq!(dy.local_range(), 0..3);
    assert_eq!(dy.extdef_range(), 3..9);
    assert_eq!(dy.undef_range(), 9..15);
    // Section order: __got (3), __stubs (4), __la_symbol_ptr (4).
    let table: Vec<_> = dy
        .indirect_symbols()
        .map(|i| s(st.name(&st.get(i.symbol_index().unwrap()).unwrap()).unwrap()).to_string())
        .collect();
    let want = [
        "_lib_weak",
        "_weakdep_fn",
        "dyld_stub_binder", // __got
        "_app_weak",
        "_lib_weak",
        "_weakdep_fn",
        "_lib_sub", // __stubs
        "_app_weak",
        "_lib_weak",
        "_weakdep_fn",
        "_lib_sub", // __la_symbol_ptr
    ];
    assert_eq!(table, want);
    assert!(st.get(15).is_err());
    assert_eq!(st.name_at(0).unwrap(), b"");
    assert!(st.name_at(st.strings().len() as u32).is_err());
}

#[test]
fn arm64_classic_exports() {
    let data = fixture("arm64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(
        sorted_exports(&m),
        [
            ("__mh_execute_header".to_string(), 0, 0),
            ("_app_weak".to_string(), EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION, 0x5f8),
            ("_fn_table".to_string(), 0, 0x8020),
            ("_main".to_string(), 0, 0x600),
            ("_name".to_string(), 0, 0x8050),
            ("_value_ptrs".to_string(), 0, 0x8040),
        ]
    );
    let ex = m.exports().unwrap().unwrap();
    assert_eq!(ex.count().unwrap(), 6);
    let e = ex.lookup(b"_main").unwrap().unwrap();
    assert_eq!(e.target, ExportTarget::Address(0x600));
    assert_eq!(e.kind(), ExportKind::Regular);
    let w = ex.lookup(b"_app_weak").unwrap().unwrap();
    assert!(w.is_weak_definition() && !w.is_reexport());
    assert_eq!(ex.lookup(b"_nothing").unwrap(), None);
    assert_eq!(ex.lookup(b"_ma").unwrap(), None);
    assert_eq!(ex.lookup(b"_main_and_more").unwrap(), None);
    assert_eq!(ex.lookup(b"").unwrap(), None);
}

#[test]
fn arm64_classic_rebase_bind_weak_lazy() {
    let data = fixture("arm64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    let info = m.dyld_info().unwrap().unwrap();
    assert_eq!(info.command().rebase_size, 16);
    assert_eq!(info.command().lazy_bind_size, 40);

    // llvm-objdump --rebase: six pointers in __DATA (segment 3, base 0x100008000).
    let rebases: Vec<_> = info.rebases().map(|r| r.unwrap()).collect();
    let offs: Vec<_> = rebases.iter().map(|r| r.segment_offset).collect();
    assert_eq!(offs, [0, 16, 24, 48, 72, 80]);
    assert!(rebases.iter().all(|r| r.segment == 3 && r.kind == REBASE_TYPE_POINTER));
    let addrs: Vec<_> = rebases.iter().map(|r| r.address).collect();
    assert_eq!(addrs, [0x1_0000_8000, 0x1_0000_8010, 0x1_0000_8018, 0x1_0000_8030, 0x1_0000_8048, 0x1_0000_8050]);

    // llvm-objdump --bind (libSystem is ordinal 1, libtiny 2, libweakdep 3).
    let want: Vec<BindRow> = vec![
        (2, 0x0, 2, "_lib_weak", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (3, 0x8, 2, "_lib_weak", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x8, 3, "_weakdep_fn", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x10, 1, "dyld_stub_binder", 0),
        (3, 0x20, 2, "_lib_add", 0),
        (3, 0x28, 2, "_lib_sub", 0),
        (3, 0x40, 2, "_lib_value", 0),
    ];
    assert_eq!(binds(&m), want);
    let b: Vec<_> = info.binds().map(|b| b.unwrap()).collect();
    assert_eq!(b[0].address, 0x1_0000_4000);
    assert!(b[0].is_weak_import() && !b[3].is_weak_import());
    assert!(b.iter().all(|b| b.kind == BIND_TYPE_POINTER && b.addend == 0));

    // llvm-objdump --weak-bind. The stream has no ordinals.
    let weak: Vec<_> = info
        .weak_binds()
        .map(|b| {
            let b = b.unwrap();
            (b.segment, b.segment_offset, s(b.symbol).to_string())
        })
        .collect();
    assert_eq!(
        weak,
        [(2, 0, "_lib_weak".to_string()), (3, 8, "_lib_weak".to_string()), (3, 0, "_app_weak".to_string()),]
    );
    // No strong-definition markers in this stream.
    assert!(info.weak_bind_events().all(|e| matches!(e.unwrap(), BindEvent::Bind(_))));

    // llvm-objdump --lazy-bind; the second entry starts at byte 18.
    let lazy: Vec<_> = info
        .lazy_binds()
        .map(|b| {
            let b = b.unwrap();
            (b.segment, b.segment_offset, b.ordinal, s(b.symbol).to_string(), b.stream_offset)
        })
        .collect();
    assert_eq!(lazy, [(3, 16, 3, "_weakdep_fn".to_string(), 0), (3, 24, 2, "_lib_sub".to_string(), 18)]);
}

// ---------------------------------------------------------------------------
// arm64, chained fixups
// ---------------------------------------------------------------------------

#[test]
fn arm64_chained_header_and_fixups_description() {
    let data = fixture("arm64_chained.exe");
    let m = MachO::parse(&data).unwrap();
    assert_eq!((m.header().ncmds, m.header().sizeofcmds), (21, 1280));
    let sum = m.summary().unwrap();
    assert!(sum.has_chained_fixups);
    assert_eq!(sum.entry, Some(Entry::Main { entryoff: 0x550, vmaddr: 0x1_0000_0550, stacksize: 0 }));
    assert!(m.dyld_info().unwrap().is_none());
    assert_eq!(segment_names(&m), ["__PAGEZERO", "__TEXT", "__DATA_CONST", "__DATA", "__LINKEDIT"]);

    let cf = m.chained_fixups().unwrap().unwrap();
    // llvm-objdump --chained-fixups
    assert_eq!(
        *cf.header(),
        ChainedHeader {
            fixups_version: 0,
            starts_offset: 32,
            imports_offset: 104,
            symbols_offset: 128,
            imports_count: 6,
            imports_format: DYLD_CHAINED_IMPORT,
            symbols_format: 0,
        }
    );
    assert_eq!(cf.segment_count(), 5);
    assert_eq!(cf.import_count(), 6);
    let imports: Vec<_> = cf
        .imports()
        .map(|i| {
            let i = i.unwrap();
            (i.lib_ordinal, i.weak_import, s(i.name).to_string(), i.addend)
        })
        .collect();
    let want = [
        (-3, false, "_app_weak", 0),
        (-3, true, "_lib_weak", 0),
        (3, true, "_weakdep_fn", 0),
        (2, false, "_lib_sub", 0),
        (2, false, "_lib_add", 0),
        (2, false, "_lib_value", 0),
    ];
    let want: Vec<_> = want.iter().map(|&(a, b, c, d)| (a, b, c.to_string(), d)).collect();
    assert_eq!(imports, want);
    assert_eq!(cf.import(6), Err(Error::ImportOutOfRange));

    for i in [0, 1, 4] {
        assert_eq!(cf.starts_in_segment(i).unwrap(), None, "segment {i}");
    }
    assert_eq!(cf.starts_in_segment(5), Err(Error::BadSegmentIndex));
    let st2 = cf.starts_in_segment(2).unwrap().unwrap();
    assert_eq!(
        (st2.size, st2.page_size, st2.pointer_format, st2.segment_offset),
        (24, 0x4000, DYLD_CHAINED_PTR_64, 0x4000)
    );
    assert_eq!((st2.max_valid_pointer, st2.page_count), (0, 1));
    assert_eq!(st2.page_start(0), Some(0));
    assert_eq!(st2.page_start(1), None);
    assert_eq!(st2.stride().unwrap(), 4);
    let st3 = cf.starts_in_segment(3).unwrap().unwrap();
    assert_eq!(st3.segment_offset, 0x8000);
}

#[test]
fn arm64_chained_walk_matches_the_classic_build_of_the_same_program() {
    let data = fixture("arm64_chained.exe");
    let m = MachO::parse(&data).unwrap();
    let cf = m.chained_fixups().unwrap().unwrap();
    let all: Vec<Fixup> = cf.fixups().map(|f| f.unwrap()).collect();
    assert_eq!(all.len(), 10);

    // (segment, vm offset, what), recorded by decoding the slots in
    // __DATA_CONST,__got and __DATA,__data by hand.
    let bind = |ord: u32| FixupKind::Bind(ChainedBind { ordinal: ord, addend: 0, auth: None });
    let rebase = |t: u64| FixupKind::Rebase(ChainedRebase { target: t, target_is_vmaddr: true, high8: 0, auth: None });
    let expect = [
        (2, 0x4000, bind(0)),
        (2, 0x4008, bind(1)),
        (2, 0x4010, bind(2)),
        (2, 0x4018, bind(3)),
        (3, 0x8000, bind(4)),               // fn_table[0] = _lib_add
        (3, 0x8008, bind(3)),               // fn_table[1] = _lib_sub
        (3, 0x8010, rebase(0x1_0000_0540)), // fn_table[2] = _local_fn
        (3, 0x8020, bind(5)),               // value_ptrs[0] = &_lib_value
        (3, 0x8028, rebase(0x1_0000_8018)), // value_ptrs[1] = &_local_value
        (3, 0x8030, rebase(0x1_0000_0600)), // name = "poly"
    ];
    for (f, e) in all.iter().zip(expect) {
        assert_eq!((f.segment, f.vm_offset, f.kind), e);
        // The file is mapped page for page: file and vm offsets agree here.
        assert_eq!(f.file_offset, f.vm_offset);
        assert_eq!((f.page, f.pointer_format), (0, DYLD_CHAINED_PTR_64));
    }
    // The ordinals name the imports: _lib_add, _lib_sub, _lib_value.
    let name = |o: u32| s(cf.import(o).unwrap().name).to_string();
    assert_eq!(name(4), "_lib_add");
    assert_eq!(name(3), "_lib_sub");
    assert_eq!(name(5), "_lib_value");

    // The classic build of the same program has the same binds and rebases.
    let classic = fixture("arm64_classic.exe");
    let c = MachO::parse(&classic).unwrap();
    let info = c.dyld_info().unwrap().unwrap();
    let classic_binds = ["_lib_add", "_lib_sub", "_lib_value"];
    for sym in classic_binds {
        assert!(info.binds().any(|b| b.unwrap().symbol == sym.as_bytes()));
    }
    // 3 lazy pointers (the stub helper's addresses move with the slide) and
    // 3 data pointers; the chained build has no lazy pointers.
    assert_eq!(info.rebases().count(), 6);
    assert_eq!(all.iter().filter(|f| matches!(f.kind, FixupKind::Rebase(_))).count(), 3);
}

#[test]
fn arm64_chained_exports_and_function_starts() {
    let data = fixture("arm64_chained.exe");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(
        sorted_exports(&m),
        [
            ("__mh_execute_header".to_string(), 0, 0),
            ("_app_weak".to_string(), EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION, 0x548),
            ("_fn_table".to_string(), 0, 0x8000),
            ("_main".to_string(), 0, 0x550),
            ("_name".to_string(), 0, 0x8030),
            ("_value_ptrs".to_string(), 0, 0x8020),
        ]
    );
    let starts: Vec<u64> = m.function_starts().unwrap().unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(starts, [0x540, 0x548, 0x550]);
}

// ---------------------------------------------------------------------------
// x86-64
// ---------------------------------------------------------------------------

#[test]
fn x86_64_classic_is_described() {
    let data = fixture("x86_64_classic.exe");
    let m = MachO::parse(&data).unwrap();
    let h = m.header();
    assert_eq!(h.cpu.kind(), CpuKind::X86_64);
    assert_eq!(h.cpu.cpusubtype, CPU_SUBTYPE_LIB64 | CPU_SUBTYPE_X86_64_ALL);
    assert_eq!(h.cpu.subtype(), CPU_SUBTYPE_X86_64_ALL);
    assert_eq!(h.cpu.capabilities(), CPU_SUBTYPE_LIB64 as u32);
    assert_eq!((h.ncmds, h.sizeofcmds), (18, 1408));
    let sum = m.summary().unwrap();
    assert_eq!(sum.vm_range, 0x1_0000_0000..0x1_0000_3310);
    assert_eq!(sum.entry, Some(Entry::Main { entryoff: 0x5d6, vmaddr: 0x1_0000_05d6, stacksize: 0 }));
    assert_eq!(
        sum.platform.unwrap(),
        Platform { platform: PLATFORM_MACOS, minos: Version(0x000a_0f00), sdk: Version(0x000a_0f00) }
    );
    // 4 KiB pages: the segments are 0x1000 long.
    let sizes: Vec<_> = m.segment_table().iter().map(|g| g.vmsize).collect();
    assert_eq!(sizes, [0x1_0000_0000, 0x1000, 0x1000, 0x1000, 0x310]);
    // libtiny is ordinal 1 here: it was linked before libSystem.
    let libs: Vec<_> = m.dylibs().map(|d| d.name).collect();
    assert_eq!(libs, ["@rpath/libtiny.dylib", "/usr/lib/libSystem.B.dylib", "@rpath/libweakdep.dylib"]);
    assert_eq!(m.rpaths().collect::<Vec<_>>(), ["@executable_path/../lib"]);

    let want: Vec<BindRow> = vec![
        (2, 0x0, 1, "_lib_weak", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (3, 0x8, 1, "_lib_weak", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x8, 3, "_weakdep_fn", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x10, 2, "dyld_stub_binder", 0),
        (3, 0x20, 1, "_lib_add", 0),
        (3, 0x28, 1, "_lib_sub", 0),
        (3, 0x40, 1, "_lib_value", 0),
    ];
    assert_eq!(binds(&m), want);
    let info = m.dyld_info().unwrap().unwrap();
    let offs: Vec<_> = info.rebases().map(|r| r.unwrap().segment_offset).collect();
    assert_eq!(offs, [0, 16, 24, 48, 72, 80]);
    let first = info.rebases().next().unwrap().unwrap();
    assert_eq!(first.address, 0x1_0000_2000);
    let lazy: Vec<_> = info
        .lazy_binds()
        .map(|b| {
            let b = b.unwrap();
            (b.segment_offset, b.ordinal, s(b.symbol).to_string(), b.stream_offset)
        })
        .collect();
    assert_eq!(lazy, [(16, 3, "_weakdep_fn".to_string(), 0), (24, 1, "_lib_sub".to_string(), 18)]);
    assert_eq!(sorted_exports(&m)[1], ("_app_weak".to_string(), EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION, 0x5cb));
    let starts: Vec<u64> = m.function_starts().unwrap().unwrap().map(|r| r.unwrap()).collect();
    assert_eq!(starts, [0x5c0, 0x5cb, 0x5d6]);
}

#[test]
fn x86_64_chained_uses_the_same_format_with_4k_pages() {
    let data = fixture("x86_64_chained.exe");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(m.cpu().kind(), CpuKind::X86_64);
    // Chained fixups need macOS 11 as the minimum, whatever the CPU.
    assert_eq!(m.platform().unwrap().minos, Version(0x000b_0000));
    let cf = m.chained_fixups().unwrap().unwrap();
    assert_eq!(cf.header().imports_count, 6);
    let st = cf.starts_in_segment(3).unwrap().unwrap();
    assert_eq!((st.page_size, st.pointer_format, st.segment_offset), (0x1000, 2, 0x2000));
    let all: Vec<Fixup> = cf.fixups().map(|f| f.unwrap()).collect();
    assert_eq!(all.len(), 10);
    let offs: Vec<_> = all.iter().map(|f| (f.segment, f.vm_offset)).collect();
    assert_eq!(
        offs,
        [
            (2, 0x1000),
            (2, 0x1008),
            (2, 0x1010),
            (2, 0x1018),
            (3, 0x2000),
            (3, 0x2008),
            (3, 0x2010),
            (3, 0x2020),
            (3, 0x2028),
            (3, 0x2030),
        ]
    );
    // fn_table[2] points at _local_fn, name at the C string.
    let FixupKind::Rebase(r) = all[6].kind else { panic!() };
    assert_eq!(r.target, 0x1_0000_0510);
    let FixupKind::Rebase(r) = all[9].kind else { panic!() };
    assert_eq!(r.target, 0x1_0000_05a4);
}

#[test]
fn x86_64_without_pie_has_no_rebases_and_an_absolute_header_symbol() {
    let data = fixture("x86_64_nopie.exe");
    let m = MachO::parse(&data).unwrap();
    assert!(!m.header().is_pie());
    let sum = m.summary().unwrap();
    assert!(!sum.is_pie && !sum.is_slidable);
    assert_eq!(segment_names(&m), ["__PAGEZERO", "__TEXT", "__DATA", "__LINKEDIT"]);
    let info = m.dyld_info().unwrap().unwrap();
    assert_eq!(info.rebases().count(), 0);
    let want: Vec<BindRow> = vec![
        (2, 0x0, 1, "_lib_weak", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x20, 1, "_lib_weak", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x8, 3, "_weakdep_fn", BIND_SYMBOL_FLAGS_WEAK_IMPORT),
        (2, 0x10, 2, "dyld_stub_binder", 0),
        (2, 0x40, 1, "_lib_add", 0),
        (2, 0x48, 1, "_lib_sub", 0),
        (2, 0x60, 1, "_lib_value", 0),
    ];
    assert_eq!(binds(&m), want);
    let ex = m.exports().unwrap().unwrap();
    let h = ex.lookup(b"__mh_execute_header").unwrap().unwrap();
    assert_eq!(h.kind(), ExportKind::Absolute);
    // llvm-objdump prints [absolute]; the value is -(image base) as 64 bits.
    assert_eq!(h.target, ExportTarget::Address(0xffff_ffff_0000_0000));
}

// ---------------------------------------------------------------------------
// dylibs
// ---------------------------------------------------------------------------

#[test]
fn libtiny_is_a_dylib_with_exports() {
    let data = fixture("libtiny.dylib");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(m.header().file_type(), FileType::Dylib);
    assert_eq!(
        m.header().flags,
        MH_NOUNDEFS | MH_DYLDLINK | MH_TWOLEVEL | MH_WEAK_DEFINES | MH_BINDS_TO_WEAK | MH_NO_REEXPORTED_DYLIBS
    );
    let sum = m.summary().unwrap();
    assert!(sum.entry.is_none() && sum.dylinker.is_none());
    assert!(!sum.is_pie && sum.is_slidable);
    assert_eq!(sum.install_name, Some("@rpath/libtiny.dylib"));
    assert_eq!(sum.vm_range, 0..0x82a0);
    assert_eq!(sum.base_address, 0);
    let id = m.dylib_id().unwrap();
    assert_eq!(id.kind, DylibKind::Id);
    assert_eq!(id.current_version, Version(0x0001_0203));
    assert_eq!(id.compatibility_version, Version(0x0001_0000));
    assert_eq!(m.dylibs().count(), 0);
    assert_eq!(segment_names(&m), ["__TEXT", "__DATA", "__LINKEDIT"]);

    assert_eq!(
        sorted_exports(&m),
        [
            ("_lib_add".to_string(), 0, 0x2e8),
            ("_lib_sub".to_string(), 0, 0x2f0),
            ("_lib_sum".to_string(), 0, 0x2f8),
            ("_lib_value".to_string(), 0, 0x4000),
            ("_lib_values".to_string(), 0, 0x4004),
            ("_lib_weak".to_string(), EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION, 0x300),
        ]
    );
    // Names that share prefixes (`_lib_`, `_lib_s`, `_lib_value`) walk the
    // same trie edges: look every one up and a few near misses.
    let ex = m.exports().unwrap().unwrap();
    for n in ["_lib_add", "_lib_sub", "_lib_sum", "_lib_value", "_lib_values", "_lib_weak"] {
        assert!(ex.lookup(n.as_bytes()).unwrap().is_some(), "{n}");
    }
    for n in ["_lib_", "_lib_s", "_lib_valuez", "_lib_values2", "lib_add", "_"] {
        assert!(ex.lookup(n.as_bytes()).unwrap().is_none(), "{n}");
    }
    // Symbols: all six are external definitions, sorted into one group.
    let st = m.symtab().unwrap().unwrap();
    assert_eq!(st.len(), 6);
    assert!(st.iter().all(|n| n.is_external() && n.kind() == SymbolKind::Section));
    let dy = m.dysymtab().unwrap().unwrap();
    assert_eq!((dy.extdef_range(), dy.undef_range()), (0..6, 6..6));
    assert_eq!(dy.indirect_symbols().count(), 0);
    // Data: __DATA,__data holds lib_value and lib_values (0x14 bytes).
    let data_seg = m.segments().nth(1).unwrap();
    let sec = data_seg.sections().next().unwrap();
    assert_eq!((sec.addr, sec.size), (0x4000, 0x14));
    // No fixups at all in this library.
    let info = m.dyld_info().unwrap().unwrap();
    assert_eq!(info.rebases().count() + info.binds().count() + info.lazy_binds().count(), 0);
}

#[test]
fn libre_reexports_libtiny() {
    let data = fixture("libre.dylib");
    let m = MachO::parse(&data).unwrap();
    assert_eq!(m.dylib_id().unwrap().name, "@rpath/libre.dylib");
    let libs: Vec<_> = m.dylibs().map(|d| (d.kind, d.name)).collect();
    assert_eq!(libs, [(DylibKind::Load, "@rpath/libtiny.dylib"), (DylibKind::Reexport, "@rpath/libtiny.dylib"),]);
    assert_eq!(m.rpaths().collect::<Vec<_>>(), ["@loader_path/../lib"]);
    assert_eq!(sorted_exports(&m), [("_re_marker".to_string(), 0, 0x4000)]);
}

// ---------------------------------------------------------------------------
// universal
// ---------------------------------------------------------------------------

#[test]
fn universal_file_has_two_slices() {
    let data = fixture("universal.bin");
    assert_eq!(FileKind::detect(&data), Ok(FileKind::Fat));
    let fat = Fat::parse(&data).unwrap();
    assert_eq!(fat.len(), 2);
    assert!(!fat.is_64() && !fat.is_empty());
    let arches: Vec<_> = fat.arches().collect();
    // llvm-objdump --universal-headers / llvm-lipo -info.
    assert_eq!(
        arches[0],
        FatArch {
            cpu: Cpu { cputype: CPU_TYPE_X86_64, cpusubtype: CPU_SUBTYPE_LIB64 | CPU_SUBTYPE_X86_64_ALL },
            offset: 4096,
            size: 13072,
            align: 12
        }
    );
    assert_eq!(arches[1], FatArch { cpu: Cpu::ARM64, offset: 32768, size: 50432, align: 14 });
    assert_eq!(fat.arches().len(), 2);

    let x = fat.find_kind(CpuKind::X86_64).unwrap();
    assert_eq!(x, arches[0]);
    assert_eq!(fat.find(Cpu::X86_64).unwrap(), arches[0]);
    assert_eq!(fat.find(Cpu::ARM64).unwrap(), arches[1]);
    assert!(fat.find_kind(CpuKind::Arm64e).is_none());
    assert!(fat.find(Cpu::ARM64E).is_none());

    // The slices are the thin files they were made from.
    let a64 = fixture("arm64_chained.exe");
    let x64 = fixture("x86_64_classic.exe");
    assert_eq!(fat.slice(&arches[1]).unwrap(), &a64[..]);
    assert_eq!(fat.slice(&arches[0]).unwrap(), &x64[..]);

    let m = MachO::parse_universal(&data, CpuKind::Arm64).unwrap();
    assert_eq!(m.cpu().kind(), CpuKind::Arm64);
    assert!(m.chained_fixups().unwrap().is_some());
    let m = MachO::parse_universal(&data, CpuKind::X86_64).unwrap();
    assert_eq!(m.cpu().kind(), CpuKind::X86_64);
    assert!(m.dyld_info().unwrap().is_some());
    assert_eq!(MachO::parse_universal(&data, CpuKind::Arm64e).unwrap_err(), Error::NoMatchingArch);
    assert_eq!(MachO::parse(&data).unwrap_err(), Error::FatFile);
    assert_eq!(select_slice(&a64, CpuKind::Arm64).unwrap().len(), a64.len());
    assert_eq!(select_slice(&a64, CpuKind::X86_64).unwrap_err(), Error::NoMatchingArch);
}
