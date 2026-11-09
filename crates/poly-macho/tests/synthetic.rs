//! Images and streams built by the tests themselves, to reach the parts of
//! the format the real files do not use: arm64e chains, every opcode, every
//! export flag, odd load commands, 64-bit universal files.

mod support;

use poly_macho::*;
use std::ops::ControlFlow;
use std::result::Result;
use support::slot::*;
use support::*;

fn s(b: &[u8]) -> &str {
    core::str::from_utf8(b).unwrap()
}

// ===========================================================================
// Chained fixups
// ===========================================================================

fn walk(data: &[u8]) -> Vec<Fixup> {
    let m = MachO::parse(data).unwrap();
    m.chained_fixups().unwrap().unwrap().fixups().map(|f| f.unwrap()).collect()
}

fn walk_err(data: &[u8]) -> Error {
    let m = MachO::parse(data).unwrap();
    let cf = match m.chained_fixups() {
        Ok(Some(cf)) => cf,
        Ok(None) => panic!("no chained fixups"),
        Err(e) => return e,
    };
    cf.fixups().find_map(|f| f.err()).expect("walk should fail")
}

#[test]
fn arm64e_formats_decode_auth_and_plain_rebases_and_binds() {
    for format in [
        DYLD_CHAINED_PTR_ARM64E,
        DYLD_CHAINED_PTR_ARM64E_KERNEL,
        DYLD_CHAINED_PTR_ARM64E_USERLAND,
        DYLD_CHAINED_PTR_ARM64E_USERLAND24,
    ] {
        let stride = if format == DYLD_CHAINED_PTR_ARM64E_KERNEL { 4 } else { 8 };
        let ord = if format == DYLD_CHAINED_PTR_ARM64E_USERLAND24 { 0x12_3456 } else { 0xabcd };
        // Four fixups in one page at byte offsets 0, 8, 24 and 32; `next`
        // counts strides, so it depends on the format.
        let n = |bytes: usize| (bytes / stride) as u64;
        let slots = [
            (0, arm64e_auth_rebase(0x1234_5678, 0xbeef, 1, 2, n(8))),
            (8, arm64e_auth_bind(ord, 0x1234, 0, 3, n(16))),
            (24, arm64e_rebase(0x7_ffff_ffff, 0x5a, n(8))),
            (32, arm64e_bind(7, -5, 0)),
        ];
        let payload = chained_payload(
            format,
            0x4000,
            &[0],
            DYLD_CHAINED_IMPORT,
            &[ImportSpec { ordinal: 1, weak: false, name: "_x", addend: 0 }],
        );
        let mut slots = slots.to_vec();
        if format == DYLD_CHAINED_PTR_ARM64E_USERLAND24 {
            // 24-bit ordinals: re-encode the two binds (bit layout of the
            // `bind24` structs: ordinal 24 bits, 8 zero bits, then as usual).
            slots[1].1 = ord | 0x1234 << 32 | 3 << 49 | 2 << 51 | 1 << 62 | 1 << 63;
            slots[3].1 = 7 | ((-5i64 as u64) & 0x7ffff) << 32 | 1 << 62;
        }
        let all = walk(&chained_image(&payload, &slots));
        assert_eq!(all.len(), 4, "format {format}");
        let pos = [0, 8, 24, 32];
        for (f, p) in all.iter().zip(pos) {
            assert_eq!(f.segment, 1);
            assert_eq!(f.page, 0);
            assert_eq!(f.file_offset, (DATA_FILE + p) as u64);
            assert_eq!(f.vm_offset, 0x4000 + p as u64);
            assert_eq!(f.pointer_format, format);
        }
        // Authenticated rebase: a 32-bit offset from the header and a signature.
        assert_eq!(
            all[0].kind,
            FixupKind::Rebase(ChainedRebase {
                target: 0x1234_5678,
                target_is_vmaddr: false,
                high8: 0,
                auth: Some(PtrAuth { key: 2, diversity: 0xbeef, addr_div: true }),
            })
        );
        // Authenticated bind: no addend, a signature.
        assert_eq!(
            all[1].kind,
            FixupKind::Bind(ChainedBind {
                ordinal: ord as u32,
                addend: 0,
                auth: Some(PtrAuth { key: 3, diversity: 0x1234, addr_div: false }),
            })
        );
        // Plain rebase: 43-bit target (a vm address only in the first format), high8.
        assert_eq!(
            all[2].kind,
            FixupKind::Rebase(ChainedRebase {
                target: 0x7_ffff_ffff,
                target_is_vmaddr: format == DYLD_CHAINED_PTR_ARM64E,
                high8: 0x5a,
                auth: None,
            })
        );
        // Plain bind with a negative 19-bit addend.
        assert_eq!(all[3].kind, FixupKind::Bind(ChainedBind { ordinal: 7, addend: -5, auth: None }));
    }
}

#[test]
fn arm64e_addend_extremes() {
    let payload = chained_payload(DYLD_CHAINED_PTR_ARM64E, 0x4000, &[0], 1, &[]);
    let slots = [
        (0, arm64e_bind(1, 0x3ffff, 1)),  // +262143
        (8, arm64e_bind(2, -0x40000, 0)), // -262144
    ];
    let all = walk(&chained_image(&payload, &slots));
    let addends: Vec<_> = all
        .iter()
        .map(|f| match f.kind {
            FixupKind::Bind(b) => b.addend,
            _ => panic!(),
        })
        .collect();
    assert_eq!(addends, [0x3ffff, -0x40000]);
}

#[test]
fn ptr64_formats_decode_rebases_and_binds() {
    for format in [DYLD_CHAINED_PTR_64, DYLD_CHAINED_PTR_64_OFFSET] {
        let slots = [
            (0, ptr64_rebase(0xf_ffff_ffff, 0xab, 2)),
            (8, ptr64_bind(0xab_cdef, 255, 2)),
            (16, ptr64_rebase(0x1000, 0, 0)),
        ];
        let payload = chained_payload(format, 0x4000, &[0], 1, &[]);
        let all = walk(&chained_image(&payload, &slots));
        assert_eq!(all.len(), 3);
        let offs: Vec<_> = all.iter().map(|f| f.vm_offset).collect();
        assert_eq!(offs, [0x4000, 0x4008, 0x4010]);
        assert_eq!(
            all[0].kind,
            FixupKind::Rebase(ChainedRebase {
                target: 0xf_ffff_ffff,
                target_is_vmaddr: format == DYLD_CHAINED_PTR_64,
                high8: 0xab,
                auth: None,
            })
        );
        assert_eq!(all[1].kind, FixupKind::Bind(ChainedBind { ordinal: 0xab_cdef, addend: 255, auth: None }));
    }
}

#[test]
fn chains_start_on_several_pages() {
    // 4 KiB pages: pages 0 and 3 have chains, 1 and 2 have none.
    let payload = chained_payload(
        DYLD_CHAINED_PTR_64_OFFSET,
        0x1000,
        &[0x10, DYLD_CHAINED_PTR_START_NONE, DYLD_CHAINED_PTR_START_NONE, 0x20, DYLD_CHAINED_PTR_START_NONE],
        1,
        &[],
    );
    let slots = [
        (0x10, ptr64_rebase(0x11, 0, 2)),
        (0x18, ptr64_rebase(0x22, 0, 0)),
        (3 * 0x1000 + 0x20, ptr64_bind(9, 0, 0)),
        // Not on any chain: must not be reported.
        (0x1000 + 0x40, ptr64_bind(1, 0, 0)),
    ];
    let all = walk(&chained_image(&payload, &slots));
    let got: Vec<_> = all.iter().map(|f| (f.page, f.file_offset - DATA_FILE as u64)).collect();
    assert_eq!(got, [(0, 0x10), (0, 0x18), (3, 0x3020)]);
    assert_eq!(all[2].vm_offset, 0x4000 + 0x3020);
}

#[test]
fn a_page_full_of_fixups_is_walked_to_its_end() {
    let slots: Vec<_> =
        (0..2048usize).map(|i| (i * 8, ptr64_rebase(i as u64, 0, if i == 2047 { 0 } else { 2 }))).collect();
    let payload = chained_payload(DYLD_CHAINED_PTR_64, 0x4000, &[0], 1, &[]);
    let all = walk(&chained_image(&payload, &slots));
    assert_eq!(all.len(), 2048);
    assert_eq!(all[2047].vm_offset, 0x4000 + 2047 * 8);
}

#[test]
fn import_formats_and_ordinals() {
    let imports = [
        ImportSpec { ordinal: 0xfe, weak: false, name: "_flat", addend: 0 },
        ImportSpec { ordinal: 0xf1, weak: true, name: "_weak_special", addend: -7 },
        ImportSpec { ordinal: 0xf0, weak: false, name: "_big", addend: 12 },
        ImportSpec { ordinal: 0x00, weak: false, name: "_self", addend: 0 },
        ImportSpec { ordinal: 0xff, weak: false, name: "_main", addend: 0 },
    ];
    // Format 1: 8-bit ordinals, no addend (addends in the specs are dropped).
    let img = chained_image(&chained_payload(2, 0x4000, &[0], DYLD_CHAINED_IMPORT, &imports), &[]);
    let m = MachO::parse(&img).unwrap();
    let cf = m.chained_fixups().unwrap().unwrap();
    let v: Vec<_> = cf.imports().map(|i| i.unwrap()).collect();
    let ords: Vec<_> = v.iter().map(|i| i.lib_ordinal).collect();
    assert_eq!(ords, [-2, -15, 240, 0, -1]);
    assert!(v[1].weak_import && !v[0].weak_import);
    assert!(v.iter().all(|i| i.addend == 0));
    assert_eq!(s(v[1].name), "_weak_special");

    // Format 2 carries a 32-bit addend.
    let img = chained_image(&chained_payload(2, 0x4000, &[0], DYLD_CHAINED_IMPORT_ADDEND, &imports), &[]);
    let m = MachO::parse(&img).unwrap();
    let v: Vec<_> = m.chained_fixups().unwrap().unwrap().imports().map(|i| i.unwrap()).collect();
    let adds: Vec<_> = v.iter().map(|i| i.addend).collect();
    assert_eq!(adds, [0, -7, 12, 0, 0]);

    // Format 3: 16-bit ordinals and a 64-bit addend.
    let imports3 = [
        ImportSpec { ordinal: 0xffff, weak: false, name: "_a", addend: i64::MIN },
        ImportSpec { ordinal: 0xfff1, weak: true, name: "_b", addend: -1 },
        ImportSpec { ordinal: 0xfff0, weak: false, name: "_c", addend: i64::MAX },
        ImportSpec { ordinal: 300, weak: false, name: "_d", addend: 5 },
    ];
    let img = chained_image(&chained_payload(2, 0x4000, &[0], DYLD_CHAINED_IMPORT_ADDEND64, &imports3), &[]);
    let m = MachO::parse(&img).unwrap();
    let cf = m.chained_fixups().unwrap().unwrap();
    let v: Vec<_> = cf.imports().map(|i| i.unwrap()).collect();
    let ords: Vec<_> = v.iter().map(|i| i.lib_ordinal).collect();
    assert_eq!(ords, [-1, -15, 65520, 300]);
    let adds: Vec<_> = v.iter().map(|i| i.addend).collect();
    assert_eq!(adds, [i64::MIN, -1, i64::MAX, 5]);
    assert!(v[1].weak_import);
    assert_eq!(cf.import(4), Err(Error::ImportOutOfRange));
}

#[test]
fn chain_errors() {
    let good_slots = [(0usize, ptr64_rebase(1, 0, 0))];
    let page = |starts: &[u16], size: u16, format: u16| chained_payload(format, size, starts, 1, &[]);

    // Next fixup would start past the end of the 4 KiB page.
    let img = chained_image(&page(&[0xff8], 0x1000, 2), &[(0xff8, ptr64_rebase(1, 0, 1))]);
    assert_eq!(walk_err(&img), Error::ChainEscapesPage);

    // Page start at or past the end of the page, or without room for a slot.
    for start in [0x1000u16, 0xffc, 0x7fff] {
        let img = chained_image(&page(&[start], 0x1000, 2), &good_slots);
        assert_eq!(walk_err(&img), Error::BadPageStart, "start {start:#x}");
    }
    // The multiple-starts flag belongs to 32-bit formats.
    let img = chained_image(&page(&[0x8001], 0x1000, 2), &good_slots);
    assert_eq!(walk_err(&img), Error::BadPageStart);

    // A chain start beyond the segment's bytes in the file: make __DATA
    // 0x100 file bytes long (the rest is zero fill).
    let payload = page(&[0x200], 0x4000, 2);
    let mut img = chained_image(&payload, &good_slots);
    let seg = cmd_offsets(&img)[1];
    w64(&mut img, seg + 48, 0x100); // filesize
    assert_eq!(walk_err(&img), Error::ChainEscapesPage);

    // A format with no walker here.
    let img = chained_image(&page(&[0], 0x4000, DYLD_CHAINED_PTR_32), &good_slots);
    assert_eq!(walk_err(&img), Error::UnsupportedPointerFormat(3));
    let img = chained_image(&page(&[0], 0x4000, DYLD_CHAINED_PTR_ARM64E_SHARED_CACHE), &good_slots);
    assert_eq!(walk_err(&img), Error::UnsupportedPointerFormat(13));

    // Page size other than 4 KiB and 16 KiB.
    let img = chained_image(&page(&[0], 0x2000, 2), &good_slots);
    assert_eq!(walk_err(&img), Error::BadPageSize);

    // The record is too small for its page count.
    let mut payload = page(&[0, 0], 0x4000, 2);
    w32(&mut payload, 48, 24); // size: room for one page start only
    let img = chained_image(&payload, &good_slots);
    assert_eq!(walk_err(&img), Error::BadStartsInSegment);
    // ... or lies outside the payload.
    let mut payload = page(&[0], 0x4000, 2);
    w32(&mut payload, 36, 0x7fff_0000); // seg_info_offset[1]
    let img = chained_image(&payload, &good_slots);
    assert_eq!(walk_err(&img), Error::BadStartsInSegment);
}

#[test]
fn chained_header_errors() {
    let base = chained_payload(2, 0x4000, &[0], 1, &[ImportSpec { ordinal: 1, weak: false, name: "_x", addend: 0 }]);
    let open = |p: &[u8]| {
        let img = chained_image(p, &[]);
        MachO::parse(&img).unwrap().chained_fixups().map(|o| o.map(|_| ()))
    };
    assert_eq!(open(&base), Ok(Some(())));

    let mut p = base.clone();
    w32(&mut p, 0, 1);
    assert_eq!(open(&p), Err(Error::UnsupportedFixupsVersion));
    let mut p = base.clone();
    w32(&mut p, 24, 1);
    assert_eq!(open(&p), Err(Error::CompressedSymbols));
    let mut p = base.clone();
    w32(&mut p, 20, 9);
    assert_eq!(open(&p), Err(Error::UnsupportedImportsFormat));
    let mut p = base.clone();
    w32(&mut p, 16, 0x1000_0000); // imports_count
    assert_eq!(open(&p), Err(Error::Truncated));
    let mut p = base.clone();
    w32(&mut p, 4, 0x7fff_fff0); // starts_offset
    assert_eq!(open(&p), Err(Error::Truncated));
    let mut p = base.clone();
    w32(&mut p, 12, 0x7fff_fff0); // symbols_offset
    assert_eq!(open(&p), Err(Error::Truncated));
    // seg_count so large its table leaves the payload
    let mut p = base.clone();
    w32(&mut p, 32, 0x1000_0000);
    assert_eq!(open(&p), Err(Error::Truncated));
    // too short for the header
    assert_eq!(open(&base[..20]), Err(Error::Truncated));

    // A name offset out of the symbol strings.
    let mut p = base.clone();
    let imports_off = r32(&p, 8) as usize;
    w32(&mut p, imports_off, 1 | 0x7f_ffff << 9);
    let img = chained_image(&p, &[]);
    let m = MachO::parse(&img).unwrap();
    let cf = m.chained_fixups().unwrap().unwrap();
    assert_eq!(cf.import(0), Err(Error::BadImport));
    assert_eq!(cf.imports().count(), 1);

    // A name that never ends.
    let mut p = base.clone();
    let n = p.len();
    p[n - 2] = b'x';
    p[n - 1] = b'y';
    let img = chained_image(&p, &[]);
    let m = MachO::parse(&img).unwrap();
    assert_eq!(m.chained_fixups().unwrap().unwrap().import(0), Err(Error::BadImport));

    // Segment entries of the starts table beyond the image's segments.
    let payload = chained_payload(2, 0x4000, &[0], 1, &[]);
    let img = Image::arm64().segment("__TEXT", 0x1000, 0x1000, 0, 0x1000, 5, &[]).linkedit(
        LC_DYLD_CHAINED_FIXUPS,
        0x1000,
        payload.len() as u32,
    );
    let mut f = img.build(0x2000);
    f[0x1000..0x1000 + payload.len()].copy_from_slice(&payload);
    let m = MachO::parse(&f).unwrap();
    let cf = m.chained_fixups().unwrap().unwrap();
    assert_eq!(cf.fixups().find_map(|x| x.err()), Some(Error::BadSegmentIndex));
}

// ===========================================================================
// Export tries
// ===========================================================================

fn names(ex: &Exports) -> Vec<(String, u64)> {
    let mut v = Vec::new();
    ex.walk(|n, e| {
        v.push((s(n).to_string(), e.flags));
        ControlFlow::Continue(())
    })
    .unwrap();
    v
}

fn sample_trie() -> Vec<u8> {
    let mut reexport = uleb(EXPORT_SYMBOL_FLAGS_REEXPORT);
    reexport.extend(uleb(2));
    reexport.extend(b"_other\0");
    let mut reexport_same = uleb(EXPORT_SYMBOL_FLAGS_REEXPORT);
    reexport_same.extend(uleb(3));
    reexport_same.push(0);
    let mut stub = uleb(EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER | EXPORT_SYMBOL_FLAGS_STATIC_RESOLVER);
    stub.extend(uleb(0x100));
    stub.extend(uleb(0x200));
    let nodes = vec![
        // 0: root
        TrieNode { terminal: None, edges: vec![(b"_".to_vec(), 1)] },
        // 1: "_"
        TrieNode {
            terminal: None,
            edges: vec![
                (b"foo".to_vec(), 2),
                (b"ba".to_vec(), 10),
                (b"abs".to_vec(), 5),
                (b"re".to_vec(), 6),
                (b"stub".to_vec(), 8),
            ],
        },
        // 2: "_foo", which is an export and also a prefix of "_foox"
        TrieNode { terminal: Some(regular(0, 0x10)), edges: vec![(b"x".to_vec(), 9)] },
        // 3: "_bar", a weak definition
        TrieNode { terminal: Some(regular(EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION, 0x20)), edges: vec![] },
        // 4: "_baz", thread local
        TrieNode { terminal: Some(regular(EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL, 0x30)), edges: vec![] },
        // 5: absolute
        TrieNode { terminal: Some(regular(EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE, 0x5)), edges: vec![] },
        // 6: "_re", a re-export with a name; 7: "_re2", one with the same name
        TrieNode { terminal: Some(reexport), edges: vec![(b"2".to_vec(), 7)] },
        TrieNode { terminal: Some(reexport_same), edges: vec![] },
        // 8: stub and resolver
        TrieNode { terminal: Some(stub), edges: vec![] },
        // 9: "_foox"
        TrieNode { terminal: Some(regular(0, 0x11)), edges: vec![] },
        // 10: "_ba", no export of its own
        TrieNode { terminal: None, edges: vec![(b"r".to_vec(), 3), (b"z".to_vec(), 4)] },
    ];
    trie_bytes(&nodes)
}

#[test]
fn export_trie_flags_and_targets() {
    let t = sample_trie();
    let ex = Exports::new(&t);
    let got = names(&ex);
    let w = EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION;
    let want: Vec<(&str, u64)> = vec![
        ("_foo", 0),
        ("_foox", 0),
        ("_bar", w),
        ("_baz", EXPORT_SYMBOL_FLAGS_KIND_THREAD_LOCAL),
        ("_abs", EXPORT_SYMBOL_FLAGS_KIND_ABSOLUTE),
        ("_re", EXPORT_SYMBOL_FLAGS_REEXPORT),
        ("_re2", EXPORT_SYMBOL_FLAGS_REEXPORT),
        ("_stub", EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER | EXPORT_SYMBOL_FLAGS_STATIC_RESOLVER),
    ];
    let want: Vec<_> = want.into_iter().map(|(n, f)| (n.to_string(), f)).collect();
    assert_eq!(got, want);
    assert_eq!(ex.count().unwrap(), 8);

    let e = ex.lookup(b"_foo").unwrap().unwrap();
    assert_eq!((e.kind(), e.target), (ExportKind::Regular, ExportTarget::Address(0x10)));
    assert_eq!(ex.lookup(b"_foox").unwrap().unwrap().target, ExportTarget::Address(0x11));
    assert!(ex.lookup(b"_bar").unwrap().unwrap().is_weak_definition());
    assert_eq!(ex.lookup(b"_baz").unwrap().unwrap().kind(), ExportKind::ThreadLocal);
    let a = ex.lookup(b"_abs").unwrap().unwrap();
    assert_eq!((a.kind(), a.target), (ExportKind::Absolute, ExportTarget::Address(5)));
    let r = ex.lookup(b"_re").unwrap().unwrap();
    assert!(r.is_reexport() && !r.is_stub_and_resolver());
    assert_eq!(r.target, ExportTarget::Reexport { ordinal: 2, name: b"_other" });
    let r = ex.lookup(b"_re2").unwrap().unwrap();
    assert_eq!(r.target, ExportTarget::Reexport { ordinal: 3, name: b"" });
    let st = ex.lookup(b"_stub").unwrap().unwrap();
    assert!(st.is_stub_and_resolver() && st.is_static_resolver());
    assert_eq!(st.target, ExportTarget::StubAndResolver { stub: 0x100, resolver: 0x200 });
    assert_eq!(st.kind(), ExportKind::Regular);
    for miss in ["_fo", "_fooxx", "_f", "_", "", "_bazz", "x", "_re3"] {
        assert_eq!(ex.lookup(miss.as_bytes()).unwrap(), None, "{miss}");
    }
}

#[test]
fn export_walk_can_stop_early() {
    let t = sample_trie();
    let ex = Exports::new(&t);
    let mut seen = Vec::new();
    ex.walk(|n, _| {
        seen.push(s(n).to_string());
        if seen.len() == 3 { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
    })
    .unwrap();
    assert_eq!(seen, ["_foo", "_foox", "_bar"]);
}

#[test]
fn empty_and_rootless_tries() {
    let ex = Exports::new(&[]);
    assert_eq!(ex.count(), Ok(0));
    assert_eq!(ex.lookup(b"_x"), Ok(None));
    // A root with an export of its own: the empty name.
    let nodes = [TrieNode { terminal: Some(regular(0, 1)), edges: vec![] }];
    let t = trie_bytes(&nodes);
    let ex = Exports::new(&t);
    assert_eq!(names(&ex), [("".to_string(), 0)]);
    assert!(ex.lookup(b"").unwrap().is_some());
}

#[test]
fn export_trie_cycles_and_blowups_are_refused() {
    // A node whose edge leads back to itself.
    let t = trie_bytes(&[TrieNode { terminal: None, edges: vec![(b"a".to_vec(), 0)] }]);
    assert_eq!(Exports::new(&t).count(), Err(Error::TrieTooComplex));
    // The same with an empty label: no characters consumed at all.
    let t = trie_bytes(&[TrieNode { terminal: None, edges: vec![(vec![], 0)] }]);
    assert_eq!(Exports::new(&t).count(), Err(Error::TrieTooComplex));
    // Two nodes pointing at each other.
    let t = trie_bytes(&[
        TrieNode { terminal: None, edges: vec![(b"a".to_vec(), 1)] },
        TrieNode { terminal: None, edges: vec![(b"b".to_vec(), 0)] },
    ]);
    assert_eq!(Exports::new(&t).count(), Err(Error::TrieTooComplex));
    // Lookup follows the name only, but is bounded the same way.
    let long = vec![b'a'; 100_000];
    let t = trie_bytes(&[TrieNode { terminal: None, edges: vec![(b"a".to_vec(), 0)] }]);
    assert_eq!(Exports::new(&t).lookup(&long), Err(Error::TrieTooComplex));

    // A chain of 100 nodes, each with 200 edges to the next: 200^100 paths.
    let mut nodes = Vec::new();
    for i in 0..100 {
        let edges = (0..200u8).map(|c| (vec![b'a' + c % 26, c / 26 + b'0'], i + 1)).collect();
        nodes.push(TrieNode { terminal: None, edges });
    }
    nodes.push(TrieNode { terminal: Some(regular(0, 1)), edges: vec![] });
    let t = trie_bytes(&nodes);
    assert_eq!(Exports::new(&t).count(), Err(Error::TrieTooComplex));

    // Deeper than the bound, without sharing: a chain of 200 nodes.
    let mut nodes: Vec<TrieNode> =
        (0..200).map(|i| TrieNode { terminal: None, edges: vec![(b"a".to_vec(), i + 1)] }).collect();
    nodes.push(TrieNode { terminal: Some(regular(0, 1)), edges: vec![] });
    let t = trie_bytes(&nodes);
    assert_eq!(Exports::new(&t).count(), Err(Error::TrieTooComplex));
    // Up to the bound it works.
    let mut nodes: Vec<TrieNode> =
        (0..MAX_TRIE_DEPTH - 1).map(|i| TrieNode { terminal: None, edges: vec![(b"a".to_vec(), i + 1)] }).collect();
    nodes.push(TrieNode { terminal: Some(regular(0, 1)), edges: vec![] });
    let t = trie_bytes(&nodes);
    assert_eq!(Exports::new(&t).count(), Ok(1));
    let name = vec![b'a'; MAX_TRIE_DEPTH - 1];
    assert!(Exports::new(&t).lookup(&name).unwrap().is_some());
}

#[test]
fn export_names_are_limited() {
    // Three edges of 400 bytes: a name of 1200 bytes.
    let label = vec![b'n'; 400];
    let t = trie_bytes(&[
        TrieNode { terminal: None, edges: vec![(label.clone(), 1)] },
        TrieNode { terminal: None, edges: vec![(label.clone(), 2)] },
        TrieNode { terminal: None, edges: vec![(label.clone(), 3)] },
        TrieNode { terminal: Some(regular(0, 1)), edges: vec![] },
    ]);
    assert_eq!(Exports::new(&t).count(), Err(Error::NameTooLong));
    // A single label at the limit is fine; one byte more is not.
    let ok = vec![b'n'; MAX_EXPORT_NAME_LEN];
    let t = trie_bytes(&[
        TrieNode { terminal: None, edges: vec![(ok, 1)] },
        TrieNode { terminal: Some(regular(0, 1)), edges: vec![] },
    ]);
    assert_eq!(Exports::new(&t).count(), Ok(1));
    let over = vec![b'n'; MAX_EXPORT_NAME_LEN + 1];
    let t = trie_bytes(&[
        TrieNode { terminal: None, edges: vec![(over, 1)] },
        TrieNode { terminal: Some(regular(0, 1)), edges: vec![] },
    ]);
    assert!(Exports::new(&t).count().is_err());
}

#[test]
fn malformed_tries() {
    let good = sample_trie();
    // Truncated at every length: only errors or fewer-looking tries, never a panic.
    for n in 0..good.len() {
        let _ = Exports::new(&good[..n]).count();
        let _ = Exports::new(&good[..n]).lookup(b"_foox");
    }
    // Child offset out of range.
    let t = trie_bytes(&[TrieNode { terminal: None, edges: vec![(b"a".to_vec(), 0)] }]);
    let mut bad = t.clone();
    let n = bad.len();
    bad[n - 3..].copy_from_slice(&uleb3(0x1000));
    assert_eq!(Exports::new(&bad).count(), Err(Error::TrieOutOfRange));
    // Terminal size larger than the trie.
    assert_eq!(Exports::new(&[0x40, 0, 0]).count(), Err(Error::TrieOutOfRange));
    // Missing child count.
    assert_eq!(Exports::new(&[0x00]).count(), Err(Error::TrieOutOfRange));
    // Edge label without a NUL.
    assert_eq!(Exports::new(&[0x00, 0x01, b'a', b'b']).count(), Err(Error::TrieOutOfRange));
    // Bad ULEB128 for a child offset.
    let mut bad = vec![0x00, 0x01, b'a', 0];
    bad.extend([0x80; 11]);
    assert_eq!(Exports::new(&bad).count(), Err(Error::BadLeb128));
    // Re-export whose name has no NUL inside its terminal info.
    let mut info = uleb(EXPORT_SYMBOL_FLAGS_REEXPORT);
    info.extend(uleb(1));
    info.extend(b"abc");
    let mut bad = uleb(info.len() as u64);
    bad.extend(&info);
    bad.push(0);
    assert_eq!(Exports::new(&bad).count(), Err(Error::TrieOutOfRange));
    // Info with a bad ULEB inside.
    let info = [0x80u8; 11];
    let mut bad = uleb(info.len() as u64);
    bad.extend(info);
    bad.push(0);
    assert_eq!(Exports::new(&bad).count(), Err(Error::BadLeb128));
}

// ===========================================================================
// Classic rebase and bind streams
// ===========================================================================

fn rebases(img: &[u8]) -> Vec<Result<RebaseRecord, Error>> {
    let m = MachO::parse(img).unwrap();
    m.dyld_info().unwrap().unwrap().rebases().collect()
}

fn rebase_offsets(img: &[u8]) -> Vec<u64> {
    rebases(img).into_iter().map(|r| r.unwrap().segment_offset).collect()
}

fn seg_off(op: u8, seg: u8, off: u64) -> Vec<u8> {
    let mut v = vec![op | seg];
    v.extend(uleb(off));
    v
}

fn cat(parts: &[&[u8]]) -> Vec<u8> {
    parts.concat()
}

#[test]
fn every_rebase_opcode() {
    let stream = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | REBASE_TYPE_POINTER],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0x10),
        &[REBASE_OPCODE_DO_REBASE_IMM_TIMES | 3], // 0x10 0x18 0x20
        &[REBASE_OPCODE_ADD_ADDR_IMM_SCALED | 2], // 0x28 -> 0x38
        &[REBASE_OPCODE_DO_REBASE_ULEB_TIMES],
        &uleb(2), // 0x38 0x40
        &[REBASE_OPCODE_ADD_ADDR_ULEB],
        &uleb(0x10), // 0x48 -> 0x58
        &[REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB],
        &uleb(0x20), // 0x58, then 0x60 + 0x20 = 0x80
        &[REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB],
        &uleb(3),
        &uleb(8),                          // 0x80 0x90 0xa0
        &[REBASE_OPCODE_DONE, 0xff, 0xff], // whatever follows is not read
    ]);
    let img = classic_image(&stream, &[], &[], &[]);
    assert_eq!(rebase_offsets(&img), [0x10, 0x18, 0x20, 0x38, 0x40, 0x58, 0x80, 0x90, 0xa0]);
    let first = rebases(&img)[0].unwrap();
    assert_eq!((first.segment, first.address, first.kind), (1, 0x2010, REBASE_TYPE_POINTER));
}

#[test]
fn rebase_stream_without_done_ends_with_the_data() {
    let stream = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0),
        &[REBASE_OPCODE_DO_REBASE_IMM_TIMES | 2],
    ]);
    assert_eq!(rebase_offsets(&classic_image(&stream, &[], &[], &[])), [0, 8]);
}

#[test]
fn rebase_can_skip_backwards_with_a_wrapped_add() {
    let stream = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0x40),
        &[REBASE_OPCODE_ADD_ADDR_ULEB],
        &uleb((-8i64) as u64),
        &[REBASE_OPCODE_DO_REBASE_IMM_TIMES | 1],
    ]);
    assert_eq!(rebase_offsets(&classic_image(&stream, &[], &[], &[])), [0x38]);
}

#[test]
fn other_rebase_types_are_kept() {
    for t in [REBASE_TYPE_TEXT_ABSOLUTE32, REBASE_TYPE_TEXT_PCREL32] {
        let stream = cat(&[
            &[REBASE_OPCODE_SET_TYPE_IMM | t],
            &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 0, 0),
            &[REBASE_OPCODE_DO_REBASE_IMM_TIMES | 1],
        ]);
        let r = rebases(&classic_image(&stream, &[], &[], &[]));
        assert_eq!(r[0].unwrap().kind, t);
    }
}

#[test]
fn rebase_errors() {
    let run = |stream: &[u8]| {
        let r = rebases(&classic_image(stream, &[], &[], &[]));
        r.into_iter().find_map(|x| x.err())
    };
    // No segment set.
    assert_eq!(
        run(&[REBASE_OPCODE_SET_TYPE_IMM | 1, REBASE_OPCODE_DO_REBASE_IMM_TIMES | 1]),
        Some(Error::BadOpcode(REBASE_OPCODE_DO_REBASE_IMM_TIMES | 1))
    );
    // Segment index out of range.
    assert_eq!(run(&seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 5, 0)), Some(Error::BadSegmentIndex));
    // Reserved type values.
    assert_eq!(run(&[REBASE_OPCODE_SET_TYPE_IMM]), Some(Error::BadOpcode(0x10)));
    assert_eq!(run(&[REBASE_OPCODE_SET_TYPE_IMM | 4]), Some(Error::BadOpcode(0x14)));
    // Unassigned opcodes.
    for op in [0x90u8, 0xa0, 0xf0] {
        assert_eq!(run(&[op]), Some(Error::BadOpcode(op)));
    }
    // A pointer that does not fit the segment: __DATA is 0x2000 long.
    let near_end = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0x1ff8),
        &[REBASE_OPCODE_DO_REBASE_IMM_TIMES | 2],
    ]);
    let r = rebases(&classic_image(&near_end, &[], &[], &[]));
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].unwrap().segment_offset, 0x1ff8);
    assert_eq!(r[1], Err(Error::FixupOutsideSegment));
    // An offset that wrapped around.
    let wrapped = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, u64::MAX - 3),
        &[REBASE_OPCODE_DO_REBASE_IMM_TIMES | 1],
    ]);
    assert_eq!(run(&wrapped), Some(Error::FixupOutsideSegment));
    // A truncated or oversized LEB128.
    assert_eq!(run(&[REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 1, 0x80]), Some(Error::BadLeb128));
    let mut long = vec![REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 1];
    long.extend([0xff; 10]);
    long.push(0x7f);
    assert_eq!(run(&long), Some(Error::BadLeb128));
    // After an error the iterator is finished.
    let mut it = MachO::parse(&classic_image(&[0xf0, 0x00], &[], &[], &[]))
        .map(|m| m.dyld_info().unwrap().unwrap().rebases().collect::<Vec<_>>())
        .unwrap();
    assert_eq!(it.len(), 1);
    assert!(it.pop().unwrap().is_err());
}

#[test]
fn rebase_runs_stop_at_the_segment_and_at_the_budget() {
    // A count of 2^64-1 ends at the end of the segment.
    let huge = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0),
        &[REBASE_OPCODE_DO_REBASE_ULEB_TIMES],
        &uleb(u64::MAX),
    ]);
    let r = rebases(&classic_image(&huge, &[], &[], &[]));
    assert_eq!(r.len(), 0x2000 / 8 + 1);
    assert_eq!(*r.last().unwrap(), Err(Error::FixupOutsideSegment));
    // A skip that wraps the offset back is refused instead of repeating.
    let looping = cat(&[
        &[REBASE_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0x100),
        &[REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB],
        &uleb(u64::MAX),
        &uleb(u64::MAX - 7),
    ]);
    let r = rebases(&classic_image(&looping, &[], &[], &[]));
    assert_eq!(r.len(), 1);
    assert_eq!(r[0], Err(Error::FixupOutsideSegment));
    // Re-emitting the same slot again and again runs into the budget: the
    // image has (0x1000 + 0x2000 + 0x8000) / 8 pointer slots in segments with
    // contents, plus 16.
    let mut stream = vec![REBASE_OPCODE_SET_TYPE_IMM | 1];
    for _ in 0..5700 {
        stream.extend(seg_off(REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0));
        stream.push(REBASE_OPCODE_DO_REBASE_IMM_TIMES | 1);
    }
    let r = rebases(&classic_image(&stream, &[], &[], &[]));
    assert_eq!(*r.last().unwrap(), Err(Error::TooManyFixups));
    assert_eq!(r.len(), (0x1000 + 0x2000 + 0x8000) / 8 + 16 + 1);
}

fn binds(img: &[u8]) -> Vec<Result<BindRecord<'static>, Error>> {
    let m = MachO::parse(img).unwrap();
    let info = m.dyld_info().unwrap().unwrap();
    // The images in these tests live as long as the test; leak the symbols'
    // lifetime to keep the helper signatures simple.
    let v: Vec<_> = info.binds().collect();
    leak_symbols(v)
}

/// The records borrow from `img`; the tests keep `img` alive, and the
/// symbols are copied into leaked boxes so the result is `'static`.
fn leak_symbols(v: Vec<Result<BindRecord<'_>, Error>>) -> Vec<Result<BindRecord<'static>, Error>> {
    v.into_iter()
        .map(|r| r.map(|b| BindRecord { symbol: Box::leak(b.symbol.to_vec().into_boxed_slice()), ..b }))
        .collect()
}

fn sym(flags: u8, name: &str) -> Vec<u8> {
    let mut v = vec![BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM | flags];
    v.extend(name.as_bytes());
    v.push(0);
    v
}

#[test]
fn every_bind_opcode() {
    let stream = cat(&[
        &[BIND_OPCODE_SET_DYLIB_ORDINAL_IMM | 2],
        &sym(BIND_SYMBOL_FLAGS_WEAK_IMPORT, "_foo"),
        &[BIND_OPCODE_SET_TYPE_IMM | BIND_TYPE_POINTER],
        &[BIND_OPCODE_SET_ADDEND_SLEB],
        &sleb(-16),
        &seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0x20),
        &[BIND_OPCODE_DO_BIND], // 0x20; next 0x28
        &[BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB],
        &uleb(0x10),                                    // 0x28; next 0x28 + 8 + 0x10 = 0x40
        &[BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED | 3], // 0x40; next 0x40 + 24 + 8 = 0x60
        &[BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB],
        &uleb(2),
        &uleb(8), // 0x60 0x70; next 0x80
        &[BIND_OPCODE_ADD_ADDR_ULEB],
        &uleb(8), // 0x88
        &[BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB],
        &uleb(300),
        &sym(0, "_bar"),
        &[BIND_OPCODE_SET_ADDEND_SLEB],
        &sleb(1 << 40),
        &[BIND_OPCODE_DO_BIND], // 0x88
        &[BIND_OPCODE_SET_DYLIB_SPECIAL_IMM],
        &[BIND_OPCODE_DO_BIND], // 0x90
        &[BIND_OPCODE_SET_DYLIB_SPECIAL_IMM | 0xf],
        &[BIND_OPCODE_DO_BIND], // 0x98
        &[BIND_OPCODE_SET_DYLIB_SPECIAL_IMM | 0xe],
        &[BIND_OPCODE_DO_BIND], // 0xa0
        &[BIND_OPCODE_SET_DYLIB_SPECIAL_IMM | 0xd],
        &[BIND_OPCODE_DO_BIND], // 0xa8
        &[BIND_OPCODE_DONE, 0xff],
    ]);
    let img = classic_image(&[], &stream, &[], &[]);
    let r: Vec<_> = binds(&img).into_iter().map(|b| b.unwrap()).collect();
    let rows: Vec<_> =
        r.iter().map(|b| (b.segment_offset, b.ordinal, s(b.symbol).to_string(), b.flags, b.addend)).collect();
    let foo = |o: u64| (o, 2, "_foo".to_string(), BIND_SYMBOL_FLAGS_WEAK_IMPORT, -16);
    let bar = |o: u64, ord: i32| (o, ord, "_bar".to_string(), 0u8, 1i64 << 40);
    let want = vec![
        foo(0x20),
        foo(0x28),
        foo(0x40),
        foo(0x60),
        foo(0x70),
        bar(0x88, 300),
        bar(0x90, 0),
        bar(0x98, BIND_SPECIAL_DYLIB_MAIN_EXECUTABLE),
        bar(0xa0, BIND_SPECIAL_DYLIB_FLAT_LOOKUP),
        bar(0xa8, BIND_SPECIAL_DYLIB_WEAK_LOOKUP),
    ];
    assert_eq!(rows, want);
    assert!(r.iter().all(|b| b.segment == 1 && b.kind == BIND_TYPE_POINTER));
    assert_eq!(r[0].address, 0x2020);
    assert!(r[0].is_weak_import() && !r[5].is_weak_import());
}

#[test]
fn a_regular_bind_stream_ends_at_done() {
    let stream = cat(&[
        &[BIND_OPCODE_SET_DYLIB_ORDINAL_IMM | 1],
        &sym(0, "_a"),
        &[BIND_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0),
        &[BIND_OPCODE_DO_BIND, BIND_OPCODE_DONE],
        // Would bind a second time if the stream went on.
        &[BIND_OPCODE_DO_BIND],
    ]);
    let r = binds(&classic_image(&[], &stream, &[], &[]));
    assert_eq!(r.len(), 1);
}

#[test]
fn lazy_streams_are_a_series_of_entries() {
    let entry = |off: u64, ord: u8, name: &str| {
        cat(&[
            &seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, off),
            &[BIND_OPCODE_SET_DYLIB_ORDINAL_IMM | ord],
            &sym(0, name),
            &[BIND_OPCODE_DO_BIND, BIND_OPCODE_DONE],
        ])
    };
    let e1 = entry(0x10, 1, "_a");
    let e2 = entry(0x18, 2, "_bb");
    let e3 = entry(0x20, 3, "_ccc");
    let stream = cat(&[&e1, &e2, &e3]);
    let img = classic_image(&[], &[], &[], &stream);
    let m = MachO::parse(&img).unwrap();
    let r: Vec<_> = m.dyld_info().unwrap().unwrap().lazy_binds().map(|b| b.unwrap()).collect();
    let rows: Vec<_> =
        r.iter().map(|b| (b.segment_offset, b.ordinal, s(b.symbol).to_string(), b.stream_offset)).collect();
    assert_eq!(
        rows,
        [
            (0x10, 1, "_a".to_string(), 0),
            (0x18, 2, "_bb".to_string(), e1.len() as u32),
            (0x20, 3, "_ccc".to_string(), (e1.len() + e2.len()) as u32),
        ]
    );
}

#[test]
fn weak_streams_can_mark_strong_definitions() {
    let stream = cat(&[
        &sym(BIND_SYMBOL_FLAGS_NON_WEAK_DEFINITION, "_strong"),
        &sym(0, "_w"),
        &[BIND_OPCODE_SET_TYPE_IMM | 1],
        &seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 8),
        &[BIND_OPCODE_DO_BIND, BIND_OPCODE_DONE],
    ]);
    let img = classic_image(&[], &[], &stream, &[]);
    let m = MachO::parse(&img).unwrap();
    let info = m.dyld_info().unwrap().unwrap();
    let ev: Vec<_> = info.weak_bind_events().map(|e| e.unwrap()).collect();
    assert_eq!(ev.len(), 2);
    assert_eq!(ev[0], BindEvent::NonWeakDefinition { symbol: b"_strong" });
    match ev[1] {
        BindEvent::Bind(b) => assert_eq!((b.symbol, b.segment_offset), (&b"_w"[..], 8)),
        _ => panic!(),
    }
    assert_eq!(info.weak_binds().count(), 1);
}

#[test]
fn bind_errors() {
    let run = |stream: &[u8]| {
        let img = classic_image(&[], stream, &[], &[]);
        let m = MachO::parse(&img).unwrap();
        let v: Vec<_> = m.dyld_info().unwrap().unwrap().binds().collect();
        v.into_iter().find_map(|r| r.err())
    };
    // DO_BIND with no segment.
    assert_eq!(run(&[BIND_OPCODE_DO_BIND]), Some(Error::BadOpcode(BIND_OPCODE_DO_BIND)));
    // Threaded binds and unassigned opcodes are not supported.
    assert_eq!(run(&[BIND_OPCODE_THREADED | 1]), Some(Error::BadOpcode(0xd1)));
    assert_eq!(run(&[0xe0]), Some(Error::BadOpcode(0xe0)));
    assert_eq!(run(&[0xf0]), Some(Error::BadOpcode(0xf0)));
    // A symbol name with no terminator.
    assert_eq!(run(&[BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM, b'a', b'b']), Some(Error::BadString));
    // An ordinal that does not fit.
    let mut big = vec![BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB];
    big.extend(uleb(1 << 40));
    assert_eq!(run(&big), Some(Error::BadOpcode(BIND_OPCODE_SET_DYLIB_ORDINAL_ULEB)));
    // Bad types and LEB128s.
    assert_eq!(run(&[BIND_OPCODE_SET_TYPE_IMM | 9]), Some(Error::BadOpcode(0x59)));
    assert_eq!(run(&[BIND_OPCODE_SET_ADDEND_SLEB, 0x80]), Some(Error::BadLeb128));
    let mut bad_sleb = vec![BIND_OPCODE_SET_ADDEND_SLEB];
    bad_sleb.extend([0xff; 10]);
    bad_sleb.push(0x01);
    assert_eq!(run(&bad_sleb), Some(Error::BadLeb128));
    // Binding outside the segment and in a missing segment.
    let mut out = seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0x2000);
    out.push(BIND_OPCODE_DO_BIND);
    assert_eq!(run(&out), Some(Error::FixupOutsideSegment));
    assert_eq!(run(&seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 9, 0)), Some(Error::BadSegmentIndex));
    // A run of 2^64-1 binds ends with the segment.
    let mut run_all = seg_off(BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB, 1, 0);
    run_all.push(BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB);
    run_all.extend(uleb(u64::MAX));
    run_all.extend(uleb(0));
    assert_eq!(run(&run_all), Some(Error::FixupOutsideSegment));
}

#[test]
fn dyld_info_ranges_must_lie_in_the_file() {
    let good = classic_image(&[REBASE_OPCODE_DONE], &[], &[], &[]);
    let m = MachO::parse(&good).unwrap();
    assert!(m.dyld_info().unwrap().is_some());
    // Make the lazy stream claim a size past the end of the file.
    let mut bad = good.clone();
    let at = cmd_offset(&bad, LC_DYLD_INFO_ONLY);
    w32(&mut bad, at + 36, CLASSIC_LEN as u32);
    let m = MachO::parse(&bad).unwrap();
    assert_eq!(m.dyld_info().unwrap_err(), Error::TableOutsideFile);
    // A size of zero ignores the offset.
    let mut zero = good.clone();
    w32(&mut zero, at + 32, 0xffff_fff0);
    w32(&mut zero, at + 36, 0);
    let m = MachO::parse(&zero).unwrap();
    assert!(m.dyld_info().unwrap().is_some());
}

// ===========================================================================
// Load commands
// ===========================================================================

fn text_data(img: Image) -> Image {
    img.segment("__TEXT", 0x1000, 0x1000, 0, 0x1000, 5, &[]).segment("__DATA", 0x2000, 0x1000, 0x1000, 0x1000, 3, &[])
}

#[test]
fn unix_thread_entry_points() {
    // x86-64: rip is register 16, rsp register 7.
    let img = text_data(Image::new(CPU_TYPE_X86_64, MH_EXECUTE)).thread(
        LC_UNIXTHREAD,
        X86_THREAD_STATE64,
        X86_THREAD_STATE64_COUNT,
        &[(16, 0x1234), (7, 0x7000)],
    );
    let f = img.build(0x2000);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.entry().unwrap(), Some(Entry::UnixThread { pc: 0x1234, sp: 0x7000 }));
    let t = m
        .commands()
        .find_map(|c| match c {
            Command::Thread(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert!(t.is_unix_thread);
    let states: Vec<_> = t.states().map(|s| s.unwrap()).collect();
    assert_eq!(states.len(), 1);
    assert_eq!((states[0].flavor, states[0].count, states[0].state.len()), (4, 42, 168));
    assert_eq!(t.entry_pc(CpuKind::X86_64), Some(0x1234));
    assert_eq!(t.entry_pc(CpuKind::Arm64), None);
    assert_eq!(t.entry_pc(CpuKind::Other), None);

    // arm64: pc is word 32, sp word 31.
    let img = text_data(Image::arm64()).thread(
        LC_UNIXTHREAD,
        ARM_THREAD_STATE64,
        ARM_THREAD_STATE64_COUNT,
        &[(32, 0x1100), (31, 0)],
    );
    let f = img.build(0x2000);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.entry().unwrap(), Some(Entry::UnixThread { pc: 0x1100, sp: 0 }));
    assert_eq!(m.summary().unwrap().entry, m.entry().unwrap());

    // A program counter outside the image.
    let img =
        text_data(Image::arm64()).thread(LC_UNIXTHREAD, ARM_THREAD_STATE64, ARM_THREAD_STATE64_COUNT, &[(32, 0x9000)]);
    let f = img.build(0x2000);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.entry(), Err(Error::BadEntry));

    // A state of another flavor: no entry point to read.
    let img = text_data(Image::arm64()).thread(LC_UNIXTHREAD, 99, 4, &[]);
    let f = img.build(0x2000);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(m.entry(), Err(Error::BadEntry));

    // LC_THREAD is not a program entry.
    let img = text_data(Image::arm64()).thread(LC_THREAD, ARM_THREAD_STATE64, 68, &[(32, 0x1100)]);
    let f = img.build(0x2000);
    assert_eq!(MachO::parse(&f).unwrap().entry(), Ok(None));
}

#[test]
fn thread_states_must_fit_the_command() {
    // Two states in one command.
    let mut body = Vec::new();
    for (flavor, count) in [(1u32, 2u32), (2, 1)] {
        body.extend(flavor.to_le_bytes());
        body.extend(count.to_le_bytes());
        body.extend(vec![0u8; count as usize * 4]);
    }
    let f = Image::arm64().cmd(LC_THREAD, &body).build(0x1000);
    let m = MachO::parse(&f).unwrap();
    let t = m
        .commands()
        .find_map(|c| match c {
            Command::Thread(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(t.states().count(), 2);
    // A count larger than the bytes there.
    let mut bad = Vec::new();
    bad.extend(1u32.to_le_bytes());
    bad.extend(1000u32.to_le_bytes());
    bad.extend([0u8; 8]);
    let f = Image::arm64().cmd(LC_THREAD, &bad).build(0x1000);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::BadThreadState);
    // A count that overflows when multiplied.
    let mut bad = Vec::new();
    bad.extend(1u32.to_le_bytes());
    bad.extend(u32::MAX.to_le_bytes());
    let f = Image::arm64().cmd(LC_THREAD, &bad).build(0x1000);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::BadThreadState);
}

#[test]
fn dependency_kinds_and_ordinals() {
    let f = Image::new(CPU_TYPE_ARM64, MH_DYLIB)
        .dylib(LC_ID_DYLIB, "@rpath/me.dylib", 0x0002_0000, 0x0001_0000)
        .dylib(LC_LOAD_DYLIB, "/a", 0, 0)
        .dylib(LC_LOAD_WEAK_DYLIB, "/b", 0, 0)
        .dylib(LC_REEXPORT_DYLIB, "/c", 0, 0)
        .dylib(LC_LAZY_LOAD_DYLIB, "/d", 0, 0)
        .dylib(LC_LOAD_UPWARD_DYLIB, "/e", 0, 0)
        .build(0x1000);
    let m = MachO::parse(&f).unwrap();
    let k: Vec<_> = m.dylibs().map(|d| (d.name, d.kind)).collect();
    assert_eq!(
        k,
        [
            ("/a", DylibKind::Load),
            ("/b", DylibKind::Weak),
            ("/c", DylibKind::Reexport),
            ("/d", DylibKind::Lazy),
            ("/e", DylibKind::Upward),
        ]
    );
    assert_eq!(m.dylib_by_ordinal(1).unwrap().name, "/a");
    assert_eq!(m.dylib_by_ordinal(5).unwrap().name, "/e");
    assert!(m.dylib_by_ordinal(6).is_none());
    let id = m.dylib_id().unwrap();
    assert_eq!((id.name, id.timestamp), ("@rpath/me.dylib", 7));
    assert_eq!(id.current_version.to_string(), "2.0.0");
    assert!(id.current_version > id.compatibility_version);
}

#[test]
fn the_alternate_dylib_encoding_of_macos_15() {
    let mut body = Vec::new();
    body.extend(28u32.to_le_bytes()); // nameoff
    body.extend(DYLIB_USE_MARKER.to_le_bytes());
    body.extend(0x0003_0000u32.to_le_bytes());
    body.extend(0x0001_0000u32.to_le_bytes());
    body.extend((DYLIB_USE_WEAK_LINK | DYLIB_USE_DELAYED_INIT).to_le_bytes());
    body.extend(b"/weak\0");
    let mut re = body.clone();
    re[16..20].copy_from_slice(&DYLIB_USE_REEXPORT.to_le_bytes());
    let mut up = body.clone();
    up[16..20].copy_from_slice(&DYLIB_USE_UPWARD.to_le_bytes());
    let f = Image::arm64().cmd(LC_LOAD_DYLIB, &body).cmd(LC_LOAD_DYLIB, &re).cmd(LC_LOAD_DYLIB, &up).build(0x1000);
    let m = MachO::parse(&f).unwrap();
    let d: Vec<_> = m.dylibs().collect();
    assert_eq!(d[0].kind, DylibKind::Weak);
    assert_eq!(d[0].use_flags, DYLIB_USE_WEAK_LINK | DYLIB_USE_DELAYED_INIT);
    assert_eq!((d[0].name, d[0].timestamp, d[0].current_version.major()), ("/weak", 0, 3));
    assert_eq!(d[1].kind, DylibKind::Reexport);
    assert_eq!(d[2].kind, DylibKind::Upward);
}

#[test]
fn version_and_misc_commands() {
    let mut src = Vec::new();
    // A.B.C.D.E = 1234.5.6.7.8 as a24.b10.c10.d10.e10
    let v: u64 = (1234 << 40) | (5 << 30) | (6 << 20) | (7 << 10) | 8;
    src.extend(v.to_le_bytes());
    let mut enc = Vec::new();
    enc.extend(0x1000u32.to_le_bytes());
    enc.extend(0x2000u32.to_le_bytes());
    enc.extend(1u32.to_le_bytes());
    let f = Image::arm64()
        .cmd(LC_VERSION_MIN_MACOSX, &[0x00, 0x05, 0x0a, 0x00, 0x00, 0x03, 0x0b, 0x00]) // 10.5.0 / 11.3.0
        .cmd(LC_SOURCE_VERSION, &src)
        .cmd(LC_UUID, &[7u8; 16])
        .string_cmd(LC_TARGET_TRIPLE, "arm64e-apple-macosx15.0.0")
        .string_cmd(LC_DYLD_ENVIRONMENT, "DYLD_X=1")
        .string_cmd(LC_ID_DYLINKER, "/usr/lib/dyld")
        .string_cmd(LC_LOAD_DYLINKER, "/usr/lib/dyld")
        .cmd(LC_ENCRYPTION_INFO_64, &enc)
        .cmd(LC_SEGMENT, &[0u8; 56])
        .cmd(0x7fff_0000, &[1, 2, 3])
        .build(0x1000);
    let m = MachO::parse(&f).unwrap();
    assert_eq!(
        m.platform().unwrap(),
        Platform { platform: PLATFORM_MACOS, minos: Version(0x000a_0500), sdk: Version(0x000b_0300) }
    );
    assert_eq!(m.source_version().unwrap().parts(), [1234, 5, 6, 7, 8]);
    assert_eq!(m.uuid(), Some([7u8; 16]));
    assert_eq!(m.dylinker(), Some("/usr/lib/dyld"));
    let cmds: Vec<_> = m.commands().collect();
    assert!(cmds.iter().any(|c| matches!(c, Command::TargetTriple("arm64e-apple-macosx15.0.0"))));
    assert!(
        cmds.iter()
            .any(|c| matches!(c, Command::Dylinker(Dylinker { kind: DylinkerKind::Environment, name: "DYLD_X=1" })))
    );
    assert!(cmds.iter().any(|c| matches!(c, Command::Dylinker(Dylinker { kind: DylinkerKind::Id, .. }))));
    assert!(
        cmds.iter().any(|c| matches!(
            c,
            Command::Encryption(EncryptionInfo { cryptoff: 0x1000, cryptsize: 0x2000, cryptid: 1 })
        ))
    );
    // The 32-bit segment command and the unknown one stay raw.
    let others: Vec<_> = cmds
        .iter()
        .filter_map(|c| match c {
            Command::Other(l) => Some(l.cmd()),
            _ => None,
        })
        .collect();
    assert_eq!(others, [LC_SEGMENT, 0x7fff_0000]);
    assert!(!m.raw_commands().next().unwrap().unwrap().is_required_by_dyld());
    // Each LC_VERSION_MIN_* names its platform.
    for (cmd, p) in [
        (LC_VERSION_MIN_IPHONEOS, PLATFORM_IOS),
        (LC_VERSION_MIN_TVOS, PLATFORM_TVOS),
        (LC_VERSION_MIN_WATCHOS, PLATFORM_WATCHOS),
    ] {
        let f = Image::arm64().cmd(cmd, &[0, 0, 9, 0, 0, 0, 9, 0]).build(0x1000);
        assert_eq!(MachO::parse(&f).unwrap().platform().unwrap().platform, p);
    }
    // LC_BUILD_VERSION wins over LC_VERSION_MIN_*.
    let mut bv = Vec::new();
    for v in [PLATFORM_IOSSIMULATOR, 0x000e_0000, 0x000f_0000, 0] {
        bv.extend(u32::to_le_bytes(v));
    }
    let f =
        Image::arm64().cmd(LC_VERSION_MIN_MACOSX, &[0, 0, 10, 0, 0, 0, 10, 0]).cmd(LC_BUILD_VERSION, &bv).build(0x1000);
    let p = MachO::parse(&f).unwrap().platform().unwrap();
    assert_eq!(p.platform, PLATFORM_IOSSIMULATOR);
    assert_eq!(p.minos.to_string(), "14.0.0");
}

#[test]
fn load_command_walk_errors() {
    let good = Image::arm64().cmd(LC_UUID, &[1u8; 16]).cmd(LC_UUID, &[2u8; 16]).build(0x1000);
    assert!(MachO::parse(&good).is_ok());
    let first = 32usize;
    // cmdsize 0, 4, 12 (not a multiple of 8)
    for bad in [0u32, 4, 12, 20] {
        let mut f = good.clone();
        w32(&mut f, first + 4, bad);
        assert_eq!(MachO::parse(&f).unwrap_err(), Error::BadCommandSize, "cmdsize {bad}");
    }
    // cmdsize past the end of the commands
    let mut f = good.clone();
    w32(&mut f, first + 4, 0x1000);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::CommandOverrun);
    // ncmds higher / lower than the bytes hold
    let mut f = good.clone();
    w32(&mut f, 16, 3);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::CommandsSizeMismatch);
    let mut f = good.clone();
    w32(&mut f, 16, 1);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::CommandsSizeMismatch);
    let mut f = good.clone();
    w32(&mut f, 16, u32::MAX);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::CommandsSizeMismatch);
    // sizeofcmds past the end of the file
    let mut f = good.clone();
    w32(&mut f, 20, 0x2000);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::Truncated);
    let mut f = good.clone();
    w32(&mut f, 20, u32::MAX);
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::Truncated);
    // A command shorter than its structure
    let short = Image::arm64().cmd(LC_SYMTAB, &[0u8; 8]).build(0x1000);
    assert_eq!(MachO::parse(&short).unwrap_err(), Error::CommandOverrun);
    // The raw iterator reports one error and stops.
    let mut f = good.clone();
    w32(&mut f, first + 4, 0);
    let m = {
        let h = Header::parse(&f).unwrap();
        LoadCommands::new(&f[32..32 + h.sizeofcmds as usize], h.ncmds)
    };
    let items: Vec<_> = m.collect();
    assert_eq!(items.len(), 1);
    assert!(items[0].is_err());
}

#[test]
fn command_strings_are_checked() {
    let ok = Image::arm64().string_cmd(LC_RPATH, "@loader_path").build(0x1000);
    assert!(MachO::parse(&ok).is_ok());
    let at = 32;
    // Offset before the end of the fixed part, past the command, or on a
    // missing terminator.
    for off in [0u32, 8, 11, 100, u32::MAX] {
        let mut f = ok.clone();
        w32(&mut f, at + 8, off);
        assert_eq!(MachO::parse(&f).unwrap_err(), Error::BadCommandString, "offset {off}");
    }
    // The string fills the whole rest of the command: no NUL.
    let mut f = ok.clone();
    let size = r32(&f, at + 4) as usize;
    for b in &mut f[at + 12..at + size] {
        *b = b'x';
    }
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::BadCommandString);
    // Not UTF-8.
    let mut f = ok.clone();
    f[at + 12] = 0xff;
    assert_eq!(MachO::parse(&f).unwrap_err(), Error::InvalidUtf8);
}

// ===========================================================================
// Universal files
// ===========================================================================

fn fat_file(is64: bool, arches: &[(i32, i32, u64, u64, u32)], len: usize) -> Vec<u8> {
    let mut f = vec![0u8; len];
    be32(&mut f, 0, if is64 { FAT_MAGIC_64 } else { FAT_MAGIC });
    be32(&mut f, 4, arches.len() as u32);
    for (i, &(cpu, sub, off, size, align)) in arches.iter().enumerate() {
        if is64 {
            let b = 8 + i * 32;
            be32(&mut f, b, cpu as u32);
            be32(&mut f, b + 4, sub as u32);
            be64(&mut f, b + 8, off);
            be64(&mut f, b + 16, size);
            be32(&mut f, b + 24, align);
        } else {
            let b = 8 + i * 20;
            be32(&mut f, b, cpu as u32);
            be32(&mut f, b + 4, sub as u32);
            be32(&mut f, b + 8, off as u32);
            be32(&mut f, b + 12, size as u32);
            be32(&mut f, b + 16, align);
        }
    }
    f
}

#[test]
fn fat_64_files_and_cpu_selection() {
    let arches = [
        (CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64_ALL, 0x1000, 0x100, 12),
        (CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64E, 0x2000, 0x100, 12),
        (CPU_TYPE_X86_64, CPU_SUBTYPE_X86_64_ALL | CPU_SUBTYPE_LIB64, 0x3000, 0x100, 12),
        (CPU_TYPE_ARM, 9, 0x4000, 0x100, 12),
    ];
    for is64 in [false, true] {
        let f = fat_file(is64, &arches, 0x5000);
        let fat = Fat::parse(&f).unwrap();
        assert_eq!(fat.is_64(), is64);
        assert_eq!(fat.len(), 4);
        assert_eq!(FileKind::detect(&f), Ok(if is64 { FileKind::Fat64 } else { FileKind::Fat }));
        assert_eq!(fat.find_kind(CpuKind::Arm64).unwrap().offset, 0x1000);
        assert_eq!(fat.find_kind(CpuKind::Arm64e).unwrap().offset, 0x2000);
        assert_eq!(fat.find_kind(CpuKind::X86_64).unwrap().offset, 0x3000);
        assert_eq!(fat.find_kind(CpuKind::Other).unwrap().offset, 0x4000);
        assert_eq!(fat.find(Cpu::ARM64E).unwrap().offset, 0x2000);
        // The capability bits do not take part in the match.
        assert_eq!(fat.find(Cpu::X86_64).unwrap().offset, 0x3000);
        assert_eq!(fat.slice(&fat.find(Cpu::ARM64).unwrap()).unwrap().len(), 0x100);
    }
}

#[test]
fn fat_errors() {
    let one = [(CPU_TYPE_ARM64, 0, 0x1000, 0x100, 12)];
    assert!(Fat::parse(&fat_file(false, &one, 0x2000)).is_ok());
    let err = |f: &[u8]| Fat::parse(f).unwrap_err();
    // Slice outside the file, or wrapping around.
    assert_eq!(err(&fat_file(false, &one, 0x1050)), Error::Truncated);
    assert_eq!(err(&fat_file(true, &[(CPU_TYPE_ARM64, 0, u64::MAX - 4, 0x100, 12)], 0x2000)), Error::Truncated);
    // Empty slice.
    assert_eq!(err(&fat_file(false, &[(CPU_TYPE_ARM64, 0, 0x1000, 0, 12)], 0x2000)), Error::EmptySlice);
    // Slice over the table, or overlapping another.
    assert_eq!(err(&fat_file(false, &[(CPU_TYPE_ARM64, 0, 8, 0x100, 12)], 0x2000)), Error::SlicesOverlap);
    assert_eq!(
        err(&fat_file(
            false,
            &[(CPU_TYPE_ARM64, 0, 0x1000, 0x200, 12), (CPU_TYPE_X86_64, 3, 0x1100, 0x200, 12)],
            0x2000
        )),
        Error::SlicesOverlap
    );
    assert_eq!(
        err(&fat_file(
            false,
            &[(CPU_TYPE_ARM64, 0, 0x1000, 0x200, 12), (CPU_TYPE_X86_64, 3, 0x1000, 0x10, 12)],
            0x2000
        )),
        Error::SlicesOverlap
    );
    // Touching slices are fine.
    assert!(
        Fat::parse(&fat_file(
            false,
            &[(CPU_TYPE_ARM64, 0, 0x1000, 0x100, 12), (CPU_TYPE_X86_64, 3, 0x1100, 0x100, 12)],
            0x2000
        ))
        .is_ok()
    );
    // Alignment exponent out of range.
    assert_eq!(err(&fat_file(true, &[(CPU_TYPE_ARM64, 0, 0x1000, 0x100, 64)], 0x2000)), Error::BadSliceAlign);
    // Too many slices: also what a Java class file looks like.
    let mut f = fat_file(false, &one, 0x2000);
    be32(&mut f, 4, 65);
    assert_eq!(err(&f), Error::TooManyArches);
    be32(&mut f, 4, 52);
    // 52 slices: the first is fine, the second is all zeros.
    assert_eq!(err(&f), Error::EmptySlice);
    // A table that does not fit.
    let mut f = fat_file(false, &one, 0x20);
    be32(&mut f, 4, 30);
    assert_eq!(err(&f), Error::Truncated);
    // Not a universal file.
    assert_eq!(err(b"\x7fELF...."), Error::NotMachO);
    assert_eq!(err(&[0xca, 0xfe]), Error::Truncated);
}

#[test]
fn file_kinds() {
    let mut m32 = vec![0u8; 64];
    w32(&mut m32, 0, MH_MAGIC);
    assert_eq!(FileKind::detect(&m32), Ok(FileKind::Thin32));
    assert_eq!(MachO::parse(&m32).unwrap_err(), Error::Unsupported32Bit);
    assert_eq!(Header::parse(&m32).unwrap_err(), Error::Unsupported32Bit);
    for magic in [MH_CIGAM, MH_CIGAM_64] {
        let mut b = vec![0u8; 64];
        w32(&mut b, 0, magic);
        assert_eq!(FileKind::detect(&b), Ok(FileKind::ThinBigEndian));
        assert_eq!(MachO::parse(&b).unwrap_err(), Error::UnsupportedBigEndian);
    }
    assert_eq!(MachO::parse(&[0u8; 64]).unwrap_err(), Error::NotMachO);
    assert_eq!(MachO::parse(b"\x7fELF").unwrap_err(), Error::NotMachO);
    assert_eq!(MachO::parse(&[]).unwrap_err(), Error::Truncated);
    assert_eq!(MachO::parse(&[0xcf, 0xfa, 0xed, 0xfe][..2]).unwrap_err(), Error::Truncated);
    // The magic alone is a header that is too short.
    let mut h = vec![0u8; 20];
    w32(&mut h, 0, MH_MAGIC_64);
    assert_eq!(MachO::parse(&h).unwrap_err(), Error::Truncated);
    // A header with no commands is a valid, empty image.
    let mut h = vec![0u8; 32];
    w32(&mut h, 0, MH_MAGIC_64);
    w32(&mut h, 4, CPU_TYPE_ARM64 as u32);
    w32(&mut h, 12, MH_OBJECT);
    let m = MachO::parse(&h).unwrap();
    assert_eq!(m.header().file_type(), FileType::Object);
    assert_eq!(m.segment_table().len(), 0);
    assert_eq!(m.vm_range(), None);
    assert_eq!(m.entry(), Ok(None));
    assert_eq!(m.summary().unwrap().vm_range, 0..0);
}

#[test]
fn cpu_descriptions() {
    let c = |t, st| Cpu { cputype: t, cpusubtype: st };
    assert_eq!(c(CPU_TYPE_ARM64, 0).kind(), CpuKind::Arm64);
    assert_eq!(c(CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64_V8).kind(), CpuKind::Arm64);
    assert_eq!(c(CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64_X1).kind(), CpuKind::Arm64);
    assert_eq!(c(CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64E).kind(), CpuKind::Arm64e);
    assert_eq!(c(CPU_TYPE_ARM64, CPU_SUBTYPE_ARM64E_X1).kind(), CpuKind::Arm64e);
    // arm64e with the pointer authentication ABI version 1 in the capability bits.
    let e = c(CPU_TYPE_ARM64, 0x8100_0002_u32 as i32);
    assert_eq!(e.kind(), CpuKind::Arm64e);
    assert_eq!(e.subtype(), CPU_SUBTYPE_ARM64E);
    assert_eq!(e.capabilities(), 0x8100_0000);
    assert_eq!(e.ptrauth_version(), 1);
    assert_eq!(c(CPU_TYPE_X86_64, CPU_SUBTYPE_X86_64_H).kind(), CpuKind::X86_64);
    assert_eq!(c(CPU_TYPE_X86, 3).kind(), CpuKind::Other);
    assert_eq!(c(CPU_TYPE_ARM64_32, 1).kind(), CpuKind::Other);
    assert_eq!(c(CPU_TYPE_ARM, 9).kind(), CpuKind::Other);
    assert_eq!(Cpu::ARM64E.kind(), CpuKind::Arm64e);
    assert_eq!(Cpu::ARM64.ptrauth_version(), 0);
}
