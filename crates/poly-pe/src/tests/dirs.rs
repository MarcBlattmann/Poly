use std::vec;

use super::builder::*;
use super::*;

#[test]
fn tls_directory() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let pe = Pe::parse(&s.file).unwrap();
        let tls = pe.tls().unwrap().unwrap();
        assert_eq!(tls.start_address_of_raw_data, s.base + u64::from(s.tls));
        assert_eq!(tls.end_address_of_raw_data, s.base + u64::from(s.tls) + 8);
        assert_eq!(tls.size_of_zero_fill, 0x18);
        assert_eq!(tls.template_size().unwrap(), 8);
        assert_eq!(tls.total_size().unwrap(), 0x20);
        assert_eq!(tls.alignment(), Some(4));
        assert_eq!(pe.tls_template(&tls).unwrap(), b"TLSDATA!");
        assert_eq!(pe.tls_index_rva(&tls).unwrap(), s.data + DATA_TLS_INDEX);
        let callbacks: Vec<u64> = pe.tls_callbacks(&tls).unwrap().map(Result::unwrap).collect();
        assert_eq!(callbacks, [s.base + u64::from(s.text) + 0x10, s.base + u64::from(s.text) + 0x20]);
    }
}

#[test]
fn tls_edge_cases() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let tls = pe.tls().unwrap().unwrap();

    let none = TlsDirectory { address_of_callbacks: 0, ..tls };
    assert_eq!(pe.tls_callbacks(&none).unwrap().count(), 0);
    let empty = TlsDirectory { end_address_of_raw_data: tls.start_address_of_raw_data, ..tls };
    assert_eq!(pe.tls_template(&empty).unwrap(), b"");
    let backwards = TlsDirectory { end_address_of_raw_data: tls.start_address_of_raw_data - 1, ..tls };
    assert!(backwards.template_size().is_err());
    let huge = TlsDirectory { size_of_zero_fill: u32::MAX, ..tls };
    assert!(huge.total_size().is_err());
    let outside = TlsDirectory { address_of_callbacks: 0x10, ..tls };
    assert_eq!(pe.tls_callbacks(&outside).unwrap_err(), Error::BadVa(0x10));
    let below = TlsDirectory { address_of_index: s.base - 8, ..tls };
    assert_eq!(pe.tls_index_rva(&below).unwrap_err(), Error::BadVa(s.base - 8));
    let past = TlsDirectory { start_address_of_raw_data: s.base + 0x100_0000, ..tls };
    assert!(pe.tls_template(&past).is_err());

    // A callback array without a terminator ends in an error, not a hang.
    let mut image = map(&s.file);
    let data = s.data as usize;
    let len = image.len();
    for at in (data + DATA_CALLBACKS as usize..len).step_by(8) {
        image[at..at + 8].copy_from_slice(&(s.base + 0x1000).to_le_bytes());
    }
    let pe = Pe::parse_mapped(&image).unwrap();
    let results: Vec<Result<u64, Error>> = pe.tls_callbacks(&tls).unwrap().collect();
    assert!(results.last().unwrap().is_err());
    assert!(results.len() <= len / 8);
}

#[test]
fn load_config() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let pe = Pe::parse(&s.file).unwrap();
        let lc = pe.load_config().unwrap().unwrap();
        assert_eq!(lc.size, load_config_size(bits));
        assert_eq!(lc.time_date_stamp, 0x1234_5678);
        assert_eq!(lc.major_version, 1);
        assert_eq!(lc.security_cookie, s.base + u64::from(s.data + DATA_COOKIE));
        assert_eq!(lc.guard_flags, guard_flags::CF_INSTRUMENTED | guard_flags::CF_FUNCTION_TABLE_PRESENT);
        assert_eq!(lc.se_handler_count, 0);

        // An older, shorter structure: fields past its size read as zero.
        let mut f = s.file.clone();
        let off = pe.rva_to_offset(s.rdata + RDATA_LOAD_CONFIG).unwrap();
        put32(&mut f, off, 0x30);
        let lc = Pe::parse(&f).unwrap().load_config().unwrap().unwrap();
        assert_eq!((lc.size, lc.security_cookie, lc.guard_flags), (0x30, 0, 0));
        assert_eq!(lc.time_date_stamp, 0x1234_5678);
        // Sizes that cannot be right.
        put32(&mut f, off, 2);
        assert!(Pe::parse(&f).unwrap().load_config().is_err());
        put32(&mut f, off, 0x10_0000);
        assert_eq!(Pe::parse(&f).unwrap().load_config().unwrap_err(), Error::Truncated);
    }
}

#[test]
fn exception_directory_and_unwind_info() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let funcs = pe.runtime_functions().unwrap().unwrap();
    assert_eq!(funcs.len(), 3);
    let all: Vec<RuntimeFunction> = funcs.iter().collect();
    assert_eq!(all[1].begin_address, s.text + 0x10);
    assert_eq!(funcs.lookup(s.text + 0x15), Some(all[1]));
    assert_eq!(funcs.lookup(s.text), Some(all[0]));
    assert_eq!(funcs.lookup(s.text + 0x3f), Some(all[2]));
    assert_eq!(funcs.lookup(s.text + 0x40), None);
    assert_eq!(funcs.lookup(0), None);

    let u0 = pe.unwind_info(&all[0]).unwrap();
    assert_eq!((u0.version, u0.flags, u0.size_of_prolog, u0.count_of_codes), (1, 0, 6, 2));
    assert_eq!((u0.frame_register, u0.frame_offset), (0, 0));
    assert_eq!((u0.handler, u0.chained), (None, None));
    let codes: Vec<UnwindCode> = u0.codes().map(Result::unwrap).collect();
    assert_eq!(
        codes,
        [
            UnwindCode { code_offset: 6, op: UnwindOp::Alloc { size: 0x28 } },
            UnwindCode { code_offset: 2, op: UnwindOp::PushNonvol { reg: 3 } },
        ]
    );

    let u1 = pe.unwind_info(&all[1]).unwrap();
    assert_eq!(u1.flags, unwind_flags::EHANDLER);
    assert_eq!((u1.frame_register, u1.frame_offset), (5, 0x20));
    assert_eq!(u1.handler, Some(s.text + 0x70));
    let xdata = all[1].unwind_info_address;
    assert_eq!(u1.handler_data, Some(xdata + 0x14));
    assert_eq!(pe.bytes_at_rva(u1.handler_data.unwrap(), 4).unwrap(), &0xdead_beefu32.to_le_bytes());
    let codes: Vec<UnwindOp> = u1.codes().map(|c| c.unwrap().op).collect();
    assert_eq!(
        codes,
        [UnwindOp::SetFpreg, UnwindOp::SaveNonvol { reg: 12, offset: 0x40 }, UnwindOp::Alloc { size: 0x800 }]
    );

    let u2 = pe.unwind_info(&all[2]).unwrap();
    assert_eq!(u2.flags, unwind_flags::CHAININFO);
    assert_eq!(u2.chained, Some(all[0]));
    assert_eq!(u2.codes().count(), 0);
}

#[test]
fn unwind_code_decoding() {
    // Every operation, written into a scratch image.
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    b.section(".text", CODE, vec![0xc3; 0x10], None);
    let x = b.next_rva();
    let mut d = Vec::new();
    let codes: &[u8] = &[
        0x01, 0x11, // ALLOC_LARGE(1): 32-bit size
        0x78, 0x56, 0x34, 0x12, //
        0x02, 0x55, // SAVE_NONVOL_FAR(RBP)
        0x00, 0x00, 0x01, 0x00, //
        0x03, 0x28, // SAVE_XMM128(XMM2) at slot * 16
        0x02, 0x00, //
        0x04, 0x69, // SAVE_XMM128_FAR(XMM6)
        0x10, 0x00, 0x00, 0x00, //
        0x05, 0x1a, // PUSH_MACHFRAME with error code
        0x06, 0x0a, // PUSH_MACHFRAME without
        0x07, 0x06, // op 6: unknown here
        0x08, 0xf0, // PUSH_NONVOL(R15)
    ];
    put_bytes(&mut d, 0, &[0x01, 0x10, (codes.len() / 2) as u8, 0x00]);
    put_bytes(&mut d, 4, codes);
    // A second record whose last code is missing its extra slot.
    put_bytes(&mut d, 0x40, &[0x01, 0x10, 0x01, 0x00, 0x00, 0x14]);
    // A third with an ALLOC_LARGE info value that does not exist.
    put_bytes(&mut d, 0x60, &[0x01, 0x10, 0x02, 0x00, 0x00, 0x21, 0x00, 0x00]);
    b.section(".xdata", RDATA, d, None);
    let p = b.next_rva();
    let mut f = Vec::new();
    for (i, u) in [0, 0x40, 0x60].into_iter().enumerate() {
        put32(&mut f, i * 12, 0x1000 + i as u32);
        put32(&mut f, i * 12 + 4, 0x1001 + i as u32);
        put32(&mut f, i * 12 + 8, x + u);
    }
    b.section(".pdata", RDATA, f, None);
    b.dir(dir::EXCEPTION, p, 36);
    let file = b.build();
    let pe = Pe::parse(&file).unwrap();
    let funcs: Vec<RuntimeFunction> = pe.runtime_functions().unwrap().unwrap().iter().collect();
    let ops: Vec<UnwindOp> = pe.unwind_info(&funcs[0]).unwrap().codes().map(|c| c.unwrap().op).collect();
    assert_eq!(
        ops,
        [
            UnwindOp::Alloc { size: 0x1234_5678 },
            UnwindOp::SaveNonvol { reg: 5, offset: 0x1_0000 },
            UnwindOp::SaveXmm128 { reg: 2, offset: 0x20 },
            UnwindOp::SaveXmm128 { reg: 6, offset: 0x10 },
            UnwindOp::PushMachframe { error_code: true },
            UnwindOp::PushMachframe { error_code: false },
            UnwindOp::Other { op: 6, info: 0 },
            UnwindOp::PushNonvol { reg: 15 },
        ]
    );
    let short: Vec<_> = pe.unwind_info(&funcs[1]).unwrap().codes().collect();
    assert_eq!(short.len(), 1);
    assert!(short[0].is_err());
    let bad: Vec<_> = pe.unwind_info(&funcs[2]).unwrap().codes().collect();
    assert!(bad[0].is_err());

    // Only x64 .pdata is understood.
    let mut f = file.clone();
    put16(&mut f, NT_OFFSET + 4, machine::ARM64);
    assert_eq!(Pe::parse(&f).unwrap().runtime_functions().unwrap_err(), Error::UnsupportedMachine(machine::ARM64));
}

#[test]
fn resources() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let root = pe.resources().unwrap().unwrap();
    assert_eq!((root.number_of_named_entries, root.number_of_id_entries), (1, 1));

    let get = |kind, name, lang| {
        let d = pe.find_resource(kind, name, lang).unwrap()?;
        Some(pe.resource_bytes(&d).unwrap())
    };
    use ResourceKey::{Id, Name};
    assert_eq!(get(Id(resource_type::RCDATA), Id(1), None), Some(&b"poly"[..]));
    assert_eq!(get(Id(resource_type::RCDATA), Id(1), Some(1033)), Some(&b"poly"[..]));
    assert_eq!(get(Id(resource_type::RCDATA), Id(1), Some(1031)), None);
    assert_eq!(get(Id(resource_type::RCDATA), Name("fixture"), Some(1031)), Some(&b"named-de"[..]));
    assert_eq!(get(Id(resource_type::RCDATA), Name("FIXTURE"), Some(1033)), Some(&b"named-en"[..]));
    assert_eq!(get(Name("mytype"), Id(7), None), Some(&b"custom"[..]));
    assert_eq!(get(Id(resource_type::ICON), Id(1), None), None);
    assert_eq!(get(Id(resource_type::RCDATA), Name("FIXTUR"), None), None);
    assert_eq!(get(Id(resource_type::RCDATA), Id(2), None), None);

    let mut leaves = Vec::new();
    root.walk(|path, data| {
        let path: Vec<std::string::String> = path
            .iter()
            .map(|n| match n {
                ResourceName::Id(i) => std::format!("{i}"),
                ResourceName::Name(s) => std::string::String::from_utf16(&s.units().collect::<Vec<u16>>()).unwrap(),
            })
            .collect();
        leaves.push((path.join("/"), pe.resource_bytes(&data).unwrap().to_vec()));
    })
    .unwrap();
    let leaves: Vec<(&str, &[u8])> = leaves.iter().map(|(p, d)| (p.as_str(), d.as_slice())).collect();
    assert_eq!(
        leaves,
        [
            ("MYTYPE/7/0", &b"custom"[..]),
            ("10/FIXTURE/1031", b"named-de"),
            ("10/FIXTURE/1033", b"named-en"),
            ("10/1/1033", b"poly"),
        ]
    );
}

#[test]
fn resource_cycles_and_fan_out_terminate() {
    let s = sample_exe(Bits::Pe32Plus);
    let pe = Pe::parse(&s.file).unwrap();
    let rsrc = pe.info().directory(dir::RESOURCE).unwrap().virtual_address;
    // Point the root's first entry back at the root: a cycle.
    let mut image = map(&s.file);
    let entry = rsrc as usize + 16;
    image[entry + 4..entry + 8].copy_from_slice(&0x8000_0000u32.to_le_bytes());
    let pe = Pe::parse_mapped(&image).unwrap();
    let root = pe.resources().unwrap().unwrap();
    assert!(root.walk(|_, _| {}).is_err());

    // Three levels of 2000 entries, each level's entries all leading to the
    // same next table: 8e9 leaves from 48 KiB. The walk stops at its budget.
    const N: usize = 2000;
    let table = 16 + N * 8;
    let mut b = PeBuilder::new(Bits::Pe32Plus);
    let r = b.next_rva();
    let mut t = Vec::new();
    for level in 0..3 {
        let at = level * table;
        put16(&mut t, at + 14, N as u16);
        for i in 0..N {
            put32(&mut t, at + 16 + i * 8, i as u32);
            let target = if level < 2 { 0x8000_0000 | ((level + 1) * table) as u32 } else { (3 * table) as u32 };
            put32(&mut t, at + 20 + i * 8, target);
        }
    }
    put32(&mut t, 3 * table, r);
    put32(&mut t, 3 * table + 4, 4);
    put32(&mut t, 3 * table + 12, 0);
    let len = t.len() as u32;
    b.section(".rsrc", RDATA, t, None);
    b.dir(dir::RESOURCE, r, len);
    let f = b.build();
    let pe = Pe::parse(&f).unwrap();
    let mut leaves = 0usize;
    assert_eq!(pe.resources().unwrap().unwrap().walk(|_, _| leaves += 1).unwrap_err(), Error::TooMany);
    assert!(leaves < MAX_RESOURCE_VISITS);

    // Entry counts past the section, and a name string past it.
    let off = pe.rva_to_offset(r).unwrap();
    let mut f2 = f.clone();
    put16(&mut f2, off + 2 * table + 12, 0xffff);
    let pe2 = Pe::parse(&f2).unwrap();
    let last = pe2.resources().unwrap().unwrap().entry(0).unwrap().directory().unwrap().unwrap();
    let last = last.entry(0).unwrap().directory().unwrap().unwrap();
    assert!(last.entries().any(|e| e.is_err()));
    let mut f3 = f.clone();
    put32(&mut f3, off + 16, 0x8000_ffff);
    let pe3 = Pe::parse(&f3).unwrap();
    assert!(pe3.resources().unwrap().unwrap().entry(0).is_err());
}

#[test]
fn debug_directory_and_codeview() {
    for bits in [Bits::Pe32Plus, Bits::Pe32] {
        let s = sample_exe(bits);
        let pe = Pe::parse(&s.file).unwrap();
        let entries: Vec<DebugEntry> = pe.debug_entries().unwrap().unwrap().collect();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].kind, debug_type::CODEVIEW);
        assert_eq!(entries[0].time_date_stamp, 0x5eed_0001);
        assert_eq!(entries[1].kind, debug_type::REPRO);
        assert_eq!(pe.debug_data(&entries[1]).unwrap(), None);
        assert_eq!(pe.codeview(&entries[1]).unwrap(), None);
        let cv = pe.pdb_info().unwrap().unwrap();
        assert_eq!(cv, CodeView { guid: PDB_GUID, age: 3, path: b"C:\\build\\sample.pdb" });

        // Mapped, the record is found through its RVA.
        let image = map(&s.file);
        let m = Pe::parse_mapped(&image).unwrap();
        assert_eq!(m.pdb_info().unwrap(), Some(cv));

        // A file offset past the end; a record that is not RSDS.
        let mut f = s.file.clone();
        let off = pe.rva_to_offset(s.rdata + RDATA_DEBUG).unwrap();
        put32(&mut f, off + 24, 0xffff_0000);
        assert_eq!(Pe::parse(&f).unwrap().pdb_info().unwrap_err(), Error::Truncated);
        let mut f = s.file.clone();
        let rsds = pe.rva_to_offset(s.rdata + RDATA_RSDS).unwrap();
        put_bytes(&mut f, rsds, b"NB10");
        assert_eq!(Pe::parse(&f).unwrap().pdb_info().unwrap(), None);
    }
}
