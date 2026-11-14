//! The loader in a 32-bit process ([`Bitness::X86`]): PE32 images with
//! 4-byte pointers everywhere the loader writes one.

use super::*;
use crate::builder::DATA;

const EXE32: u64 = 0x40_0000;
const NTDLL32: u64 = 0x7700_0000;

fn config() -> Config {
    Config { dirs: dirs(), bitness: Bitness::X86 }
}

fn ntdll32_image() -> Vec<u8> {
    PeBuilder {
        image_base: NTDLL32,
        export_name: "ntdll.dll".into(),
        sections: vec![text(0x100)],
        exports: named(&[("RtlAllocateHeap", 0x1010), ("RtlFreeHeap", 0x1020), ("RtlUpcaseUnicodeChar", 0x1030)]),
        ..PeBuilder::x86()
    }
    .write()
}

fn dll32(name: &str, base: u64, exports: Vec<Export>, imps: Vec<Import>) -> PeBuilder {
    PeBuilder {
        image_base: base,
        export_name: name.into(),
        entry: 0x1000,
        sections: vec![text(0x200)],
        exports,
        imports: imps,
        ..PeBuilder::x86()
    }
}

fn exe32(imps: Vec<Import>) -> PeBuilder {
    PeBuilder {
        image_base: EXE32,
        dll: false,
        entry: 0x1000,
        sections: vec![text(0x100)],
        imports: imps,
        ..PeBuilder::x86()
    }
}

/// A 32-bit process with ntdll and `exe_image` mapped by "the kernel".
fn world32(exe_image: &[u8]) -> World {
    let mut p = Fake::new32();
    p.insert(PEB, 0x1000);
    p.insert(TEB, 0x2000);
    p.insert(TEB2, 0x2000);
    let mut l = Loader::new(config());
    l.init_ldr_data(&mut p, PEB).unwrap();
    let eb = p.map_image(exe_image);
    let nb = p.map_image(&ntdll32_image());
    let exe = l.add_mapped(&mut p, eb, &wide("C:\\app\\test.exe"), false).unwrap();
    let ntdll = l.add_mapped(&mut p, nb, &wide("C:\\Windows\\System32\\ntdll.dll"), true).unwrap();
    World { p, l, exe, ntdll }
}

fn rd(w: &mut World, at: u64) -> u64 {
    u64::from(read_u32(&mut w.p, at).unwrap())
}

fn iat32(w: &mut World, b: &PeBuilder, i: usize, j: usize) -> u64 {
    rd(w, b.image_base + u64::from(b.layout().iat[i][j]))
}

/// The base names of the entries of a 32-bit list (`BaseDllName` at 0x2C).
fn names32(list: &[u64], w: &mut World) -> Vec<String> {
    list.iter()
        .map(|&e| {
            let len = u16::from_le_bytes(w.p.memory(e + 0x2C, 2).unwrap().try_into().unwrap()) as usize;
            let buf = rd(w, e + 0x30);
            let bytes = w.p.memory(buf, len).unwrap().to_vec();
            let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            s(&units)
        })
        .collect()
}

fn lists(w: &mut World) -> (Vec<u64>, Vec<u64>, Vec<u64>) {
    let ldr = rd(w, PEB + 0x0C);
    let load = walk_list_in(&mut w.p, Bitness::X86, ldr + 0x0C, 0x00, 100).unwrap();
    let mem = walk_list_in(&mut w.p, Bitness::X86, ldr + 0x14, 0x08, 100).unwrap();
    let init = walk_list_in(&mut w.p, Bitness::X86, ldr + 0x1C, 0x10, 100).unwrap();
    (load, mem, init)
}

#[test]
fn the_layouts_are_the_documented_offsets() {
    let x64 = Bitness::X64.layout();
    assert_eq!((x64.ptr, x64.peb_ldr, x64.teb_tls_pointer, x64.ldr_size), (8, PEB_LDR, TEB_TLS_POINTER, 0x58));
    assert_eq!((x64.ldr_in_load_order, x64.ldr_in_memory_order, x64.ldr_in_init_order), (0x10, 0x20, 0x30));
    assert_eq!((x64.entry_dll_base, x64.entry_base_name, x64.entry_size), (0x30, 0x58, 0x100));
    let x86 = Bitness::X86.layout();
    assert_eq!((x86.ptr, x86.peb_ldr, x86.teb_tls_pointer, x86.ldr_size), (4, 0x0C, 0x2C, 0x30));
    assert_eq!((x86.ldr_in_load_order, x86.ldr_in_memory_order, x86.ldr_in_init_order), (0x0C, 0x14, 0x1C));
    assert_eq!((x86.entry_in_load_order, x86.entry_in_memory_order, x86.entry_in_init_order), (0, 0x08, 0x10));
    assert_eq!((x86.entry_dll_base, x86.entry_entry_point, x86.entry_size_of_image), (0x18, 0x1C, 0x20));
    assert_eq!((x86.entry_full_name, x86.entry_base_name), (0x24, 0x2C));
    assert_eq!((x86.entry_checksum, x86.entry_time_date_stamp, x86.entry_size), (0x40, 0x44, 0x80));
    assert_eq!((x86.list_entry_size(), x86.unicode_string_size(), x64.unicode_string_size()), (8, 8, 16));
    assert_eq!(Bitness::X86.pointer_size(), 4);
    // The default is the 64-bit process.
    assert_eq!(Config::default().bitness, Bitness::X64);
    assert_eq!(Loader::new(Config::default()).bitness(), Bitness::X64);
    assert_eq!(Loader::new(config()).bitness(), Bitness::X86);
    assert_eq!(Loader::new(config()).layout(), x86);
}

#[test]
fn a_pointer_a_4_byte_slot_cannot_hold_is_refused() {
    let mut p = Fake::new32();
    p.insert(0x1000_0000, 0x1000);
    assert_eq!(write_ptr(&mut p, 4, 0x1000_0000, 1 << 32).unwrap_err(), STATUS_INVALID_PARAMETER);
    write_ptr(&mut p, 4, 0x1000_0000, 0xFFFF_FFFF).unwrap();
    assert_eq!(read_ptr(&mut p, 4, 0x1000_0000).unwrap(), 0xFFFF_FFFF);
    assert_eq!(p.memory(0x1000_0004, 4).unwrap(), &[0; 4], "only four bytes were written");
}

#[test]
fn process_start_binds_4_byte_imports_in_dependency_order() {
    // exe -> a.dll (by name and ordinal), ntdll; a -> b.dll; b has TLS.
    let b = PeBuilder {
        tls: Some(Tls { template: vec![1, 2, 3, 4], zero_fill: 12, callbacks: vec![0x1100], align_flag: 0 }),
        ..dll32("b.dll", 0x1100_0000, named(&[("BFunc", 0x1040)]), vec![])
    };
    let a = dll32(
        "a.dll",
        0x1200_0000,
        vec![
            Export { name: Some("AFunc".into()), ordinal: 1, target: Target::Rva(0x1010) },
            Export { name: None, ordinal: 5, target: Target::Rva(0x1050) },
        ],
        vec![imports("b.dll", &[Sym::name("BFunc")])],
    );
    let e = exe32(vec![
        imports("A.DLL", &[Sym::name("AFunc"), Sym::Ordinal(5)]),
        imports("ntdll.dll", &[Sym::Name { name: "RtlFreeHeap".into(), hint: 1 }]),
    ]);
    let mut w = world32(&e.write());
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
    let (ab, bb) = (base_of(&w, "a.dll"), base_of(&w, "b"));
    assert_eq!((ab, bb), (0x1200_0000, 0x1100_0000), "preferred bases were free");
    assert_eq!(c[0].routine, bb + 0x1100);
    assert_eq!(c[1].routine, bb + 0x1000);
    // The import address tables hold 4-byte addresses, 4 bytes apart.
    let lay = e.layout();
    assert_eq!(lay.iat[0][1] - lay.iat[0][0], 4);
    assert_eq!(iat32(&mut w, &e, 0, 0), ab + 0x1010);
    assert_eq!(iat32(&mut w, &e, 0, 1), ab + 0x1050);
    assert_eq!(iat32(&mut w, &e, 1, 0), NTDLL32 + 0x1020);
    assert_eq!(rd(&mut w, EXE32 + u64::from(lay.iat[0][1]) + 4), 0, "the terminating slot stays empty");
    assert_eq!(iat32(&mut w, &a, 0, 0), bb + 0x1040);
    // Section protections as for any image.
    assert_eq!(w.p.prot_at(ab), page::READONLY);
    assert_eq!(w.p.prot_at(ab + 0x1000), page::EXECUTE_READ);
    // TLS: b got index 0 (a 4-byte slot in its image), the first thread a
    // block with the template, found through TEB + 0x2C.
    let index = read_u32(&mut w.p, bb + u64::from(b.layout().tls_index.unwrap())).unwrap();
    assert_eq!(index, 0);
    assert_eq!(rd(&mut w, TEB + 0x58), 0, "nothing at the 64-bit offset");
    let array = rd(&mut w, TEB + 0x2C);
    let block = rd(&mut w, array);
    assert_eq!(w.p.memory(block, 16).unwrap(), &[1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(w.l.tls_block(TEB, 0), Some(block));
    // PEB_LDR_DATA.
    let ldr = rd(&mut w, PEB + 0x0C);
    assert_eq!(w.l.ldr_data(), ldr);
    assert_eq!(rd(&mut w, ldr), 0x30, "Length");
    assert_eq!(w.p.memory(ldr + 4, 1).unwrap(), &[1], "Initialized");
    let (load, mem, init) = lists(&mut w);
    assert_eq!(
        names32(&load, &mut w),
        ["test.exe", "ntdll.dll", "A.DLL", "b.dll"],
        "names as the importer spelled them"
    );
    assert_eq!(load, mem);
    assert_eq!(names32(&init, &mut w), ["ntdll.dll", "b.dll", "A.DLL"]);
    // The list heads are two 4-byte pointers: first and last link.
    assert_eq!(rd(&mut w, ldr + 0x14), load[0] + 0x08);
    assert_eq!(rd(&mut w, ldr + 0x14 + 4), load[3] + 0x08);
    assert_eq!(rd(&mut w, load[0] + 0x08 + 4), ldr + 0x14, "the first entry's Blink is the head");
    // One entry, field by field.
    let e = load[2];
    assert_eq!(w.l.modules()[2].record, e);
    assert_eq!(rd(&mut w, e + 0x18), ab, "DllBase");
    assert_eq!(rd(&mut w, e + 0x1C), ab + 0x1000, "EntryPoint");
    assert_eq!(rd(&mut w, e + 0x20), w.l.module(w.l.modules()[2].id).unwrap().size, "SizeOfImage");
    let path = w.l.modules()[2].path.clone();
    let full_len = u16::from_le_bytes(w.p.memory(e + 0x24, 2).unwrap().try_into().unwrap());
    let full_max = u16::from_le_bytes(w.p.memory(e + 0x26, 2).unwrap().try_into().unwrap());
    assert_eq!((usize::from(full_len), usize::from(full_max)), (path.len() * 2, path.len() * 2 + 2));
    let buf = rd(&mut w, e + 0x28);
    let bytes = w.p.memory(buf, usize::from(full_len) + 2).unwrap().to_vec();
    let units: Vec<u16> = bytes.chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(units.last(), Some(&0), "NUL-terminated");
    assert_eq!(&units[..units.len() - 1], &path[..]);
    assert_eq!(read_u32(&mut w.p, e + 0x40).unwrap(), 0, "CheckSum");
    assert_eq!(read_u32(&mut w.p, e + 0x44).unwrap(), 0x5000_0000, "TimeDateStamp");
    assert!(w.l.modules().iter().all(|m| m.pinned));
    assert_eq!(w.l.find(ab + 0x1234).unwrap().base, ab);
}

#[test]
fn a_dll_whose_base_is_taken_is_relocated_with_highlow() {
    let mut d = dll32("r.dll", EXE32, named(&[("Get", 0x1010)]), vec![]);
    // A data pointer to its own export, and TLS whose directory holds VAs.
    let mut data = vec![0u8; 16];
    data[..4].copy_from_slice(&(EXE32 as u32 + 0x1010).to_le_bytes());
    d.sections.push(Section::new(".data", data, DATA));
    d.relocations.push(0x2000);
    d.tls = Some(Tls { template: vec![9; 8], zero_fill: 0, callbacks: vec![0x1020, 0x1030], align_flag: 0 });
    let mut w = world32(&exe32(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    w.p.file("C:\\app\\r.dll", d.write());
    let (id, base, op) = w.l.load(&mut w.p, &wide("r"), None).unwrap();
    assert_ne!(base, EXE32);
    assert_eq!(base % 0x1_0000, 0);
    assert!(base + w.l.module(id).unwrap().size <= 1 << 32, "below 4 GiB");
    assert_eq!(rd(&mut w, base + 0x2000), base + 0x1010, "the pointer moved with the image");
    let routines: Vec<u64> = op.callouts.iter().map(|c| c.routine).collect();
    assert_eq!(routines, [base + 0x1020, base + 0x1030, base + 0x1000]);
    assert!(op.callouts.iter().all(|c| c.reserved == 0 && c.base == base));
    // Its TLS index and a block for the running thread, from the moved template.
    let index = w.l.module(id).unwrap().tls_index.unwrap();
    let block = w.l.tls_block(TEB, index).unwrap();
    assert_eq!(w.p.memory(block, 8).unwrap(), &[9; 8]);
    let array = rd(&mut w, TEB + 0x2C);
    assert_eq!(rd(&mut w, array + 4 * u64::from(index)), block);
    let (pa, _) = w.l.get_procedure(&mut w.p, base, Some(b"Get"), 0).unwrap();
    assert_eq!(pa, base + 0x1010);
    assert_eq!(w.l.get_procedure(&mut w.p, base, None, 1).unwrap().0, base + 0x1010);
    // The entry is on the lists, and gone from them after the unload.
    let (load, _, init) = lists(&mut w);
    assert_eq!(names32(&load, &mut w), ["test.exe", "ntdll.dll", "r.dll"]);
    assert_eq!(names32(&init, &mut w), ["ntdll.dll", "r.dll"]);
    let heap = w.p.heap_blocks();
    let u = w.l.unload(base).unwrap();
    w.l.finish_unload(&mut w.p, u);
    let (load, mem, init) = lists(&mut w);
    assert_eq!(names32(&load, &mut w), ["test.exe", "ntdll.dll"]);
    assert_eq!((load.len(), mem.len(), init.len()), (2, 2, 1));
    assert!(w.p.heap_blocks() < heap, "its entry and TLS blocks were freed");
}

#[test]
fn bound_imports_are_4_bytes_and_kept_only_while_valid() {
    let t =
        PeBuilder { time_date_stamp: 0x1234_5678, ..dll32("t.dll", 0x1300_0000, named(&[("Real", 0x1080)]), vec![]) };
    let bound = |stamp: u32| {
        exe32(vec![Import {
            dll: "t.dll".into(),
            symbols: vec![Sym::name("Real"), Sym::Ordinal(1)],
            bound: Some((stamp, vec![0xDEAD_BEEF, 0xCAFE_F00D])),
            delay_thunks: vec![],
        }])
    };
    for (stamp, expect) in [(0x1234_5678u32, [0xDEAD_BEEFu64, 0xCAFE_F00D]), (0x1111_1111, [0x1300_1080, 0x1300_1080])]
    {
        let e = bound(stamp);
        let mut w = world32(&e.write());
        w.p.file("C:\\app\\t.dll", t.write());
        w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
        assert_eq!(iat32(&mut w, &e, 0, 0), expect[0], "stamp {stamp:#x}");
        assert_eq!(iat32(&mut w, &e, 0, 1), expect[1], "stamp {stamp:#x}");
    }
    // Valid stamp, but t.dll had to move: rebound.
    let e = bound(0x1234_5678);
    let mut w = world32(&e.write());
    w.p.insert(0x1300_0000, 0x1000);
    w.p.file("C:\\app\\t.dll", t.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let tb = base_of(&w, "t.dll");
    assert_ne!(tb, 0x1300_0000);
    assert_eq!(iat32(&mut w, &e, 0, 0), tb + 0x1080);
}

#[test]
fn delay_load_imports_use_4_byte_slots_and_the_31_bit_ordinal_flag() {
    let d = dll32("late.dll", 0x1800_0000, named(&[("Mul", 0x1010), ("Add", 0x1020)]), vec![]);
    let e = PeBuilder {
        delay_imports: vec![Import {
            dll: "late.dll".into(),
            symbols: vec![Sym::name("Mul"), Sym::Ordinal(2), Sym::name("Gone")],
            bound: None,
            delay_thunks: vec![0x1040, 0x1048, 0x1050],
        }],
        ..exe32(vec![])
    };
    let lay = e.layout();
    let mut w = world32(&e.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    assert!(w.l.get_handle(&wide("late.dll")).is_err(), "not loaded at start");
    let slot0 = EXE32 + u64::from(lay.delay_iat[0][0]);
    assert_eq!(EXE32 + u64::from(lay.delay_iat[0][1]), slot0 + 4);
    assert_eq!(rd(&mut w, slot0), EXE32 + 0x1040, "the slots start at their thunks");
    let desc = EXE32 + u64::from(lay.delay_descriptors[0]);
    let f = w.l.resolve_delay(&mut w.p, EXE32, desc, slot0).unwrap_err();
    assert!(f.dll_failed);
    assert_eq!(
        (f.status, f.dll.as_slice(), f.name.as_deref()),
        (STATUS_DLL_NOT_FOUND, &b"late.dll"[..], Some(&b"Mul"[..]))
    );
    w.p.file("C:\\app\\late.dll", d.write());
    let (a, op) = w.l.resolve_delay(&mut w.p, EXE32, desc, slot0).unwrap();
    let lb = base_of(&w, "late.dll");
    assert_eq!(a, lb + 0x1010);
    assert_eq!(kinds(&op.callouts, &w), [("late.dll".to_string(), CalloutKind::Entry, reason::PROCESS_ATTACH)]);
    assert_eq!(rd(&mut w, slot0), a, "the slot now holds the address");
    assert_eq!(rd(&mut w, EXE32 + u64::from(lay.delay_handles[0])), lb, "and the handle is kept");
    assert_eq!(rd(&mut w, slot0 + 4), EXE32 + 0x1048, "the next slot is untouched");
    // The ordinal import: bit 31 of its name table entry.
    let slot1 = EXE32 + u64::from(lay.delay_iat[0][1]);
    let (a1, op1) = w.l.resolve_delay(&mut w.p, EXE32, desc, slot1).unwrap();
    assert_eq!((a1, op1.callouts.len()), (lb + 0x1020, 0));
    let slot2 = EXE32 + u64::from(lay.delay_iat[0][2]);
    let f = w.l.resolve_delay(&mut w.p, EXE32, desc, slot2).unwrap_err();
    assert!(!f.dll_failed);
    assert_eq!(f.status, STATUS_PROCEDURE_NOT_FOUND);
    // An address that is not a slot (4-byte slots: +2 is not one).
    assert!(w.l.resolve_delay(&mut w.p, EXE32, desc, slot0 + 2).is_err());
}

#[test]
fn threads_get_4_byte_tls_slots_at_teb_0x2c() {
    let tls = |cb: u32| Some(Tls { template: vec![0xAB; 4], zero_fill: 4, callbacks: vec![cb], align_flag: 0 });
    let a = PeBuilder { tls: tls(0x1100), ..dll32("a.dll", 0x1200_0000, named(&[("A", 0x1010)]), vec![]) };
    let e = PeBuilder { tls: tls(0x1080), ..exe32(vec![imports("a.dll", &[Sym::name("A")])]) };
    let mut w = world32(&e.write());
    w.p.file("C:\\app\\a.dll", a.write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let c = w.l.thread_attach(&mut w.p, TEB2).unwrap();
    assert_eq!(c.len(), 3);
    let array = rd(&mut w, TEB2 + 0x2C);
    for i in 0..2 {
        let b = rd(&mut w, array + 4 * i);
        assert_eq!(w.p.memory(b, 8).unwrap(), &[0xAB, 0xAB, 0xAB, 0xAB, 0, 0, 0, 0]);
    }
    // More TLS DLLs than the array holds: it grows, in both threads.
    let mut later = Vec::new();
    for k in 0..6u64 {
        let name = std::format!("t{k}.dll");
        let d =
            PeBuilder { tls: tls(0x1100), ..dll32(&name, 0x2000_0000 + (k << 24), named(&[("X", 0x1010)]), vec![]) };
        w.p.file(&std::format!("C:\\app\\{name}"), d.write());
        let (id, _, _) = w.l.load(&mut w.p, &wide(&name), None).unwrap();
        later.push(id);
    }
    for teb in [TEB, TEB2] {
        let array = rd(&mut w, teb + 0x2C);
        for &id in &later {
            let idx = w.l.module(id).unwrap().tls_index.unwrap();
            let b = rd(&mut w, array + 4 * u64::from(idx));
            assert_eq!(b, w.l.tls_block(teb, idx).unwrap());
            assert_eq!(w.p.memory(b, 4).unwrap(), &[0xAB; 4]);
        }
    }
    // The thread ends: its blocks and array go, and the TEB slot is cleared.
    let heap = w.p.heap_blocks();
    w.l.thread_exit(&mut w.p, TEB2);
    assert!(w.p.heap_blocks() < heap);
    assert_eq!(rd(&mut w, TEB2 + 0x2C), 0);
    assert_eq!(rd(&mut w, TEB2 + 0x58), 0);
}

#[test]
fn each_bitness_refuses_the_other_kind_of_image() {
    let dll64 = dll("w64.dll", 0x1_8100_0000, named(&[("F", 0x1010)]), vec![]);
    let pe32 = dll32("w32.dll", 0x1400_0000, named(&[("F", 0x1010)]), vec![]);
    // 32-bit process: a PE32+ image, a PE32 image that says AMD64 and a PE32+
    // image that says I386 are all invalid, and nothing stays mapped.
    let mut w = world32(&exe32(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    let before = (w.p.images(), w.p.heap_blocks());
    for (name, image) in [
        ("w64.dll", dll64.write()),
        ("amd.dll", PeBuilder { machine: 0x8664, ..dll32("amd.dll", 0x1500_0000, vec![], vec![]) }.write()),
        ("i386.dll", PeBuilder { machine: 0x014C, ..dll("i386.dll", 0x1_8200_0000, vec![], vec![]) }.write()),
    ] {
        w.p.file(&std::format!("C:\\app\\{name}"), image);
        assert_eq!(w.l.load(&mut w.p, &wide(name), None).unwrap_err(), STATUS_INVALID_IMAGE_FORMAT, "{name}");
    }
    assert_eq!((w.p.images(), w.p.heap_blocks()), before);
    // The PE32 one is fine.
    w.p.file("C:\\app\\w32.dll", pe32.write());
    assert_eq!(w.l.load(&mut w.p, &wide("w32.dll"), None).unwrap().1, 0x1400_0000);
    // 64-bit process: the PE32 image is invalid, as an import too.
    let mut w = world(&exe(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    w.p.file("C:\\app\\w32.dll", pe32.write());
    assert_eq!(w.l.load(&mut w.p, &wide("w32.dll"), None).unwrap_err(), STATUS_INVALID_IMAGE_FORMAT);
    assert!(w.l.get_handle(&wide("w32.dll")).is_err());
    let importer = exe(vec![imports("w32.dll", &[Sym::name("F")])]);
    let mut w = world(&importer.write());
    w.p.file("C:\\app\\w32.dll", pe32.write());
    assert_eq!(w.l.process_init(&mut w.p, w.exe, TEB).unwrap_err(), STATUS_INVALID_IMAGE_FORMAT);
    // The kernel-mapped images are checked too.
    let mut p = Fake::new();
    p.insert(PEB, 0x1000);
    let img32 = p.map_image(&exe32(vec![]).write());
    let img64 = p.map_image(&exe(vec![]).write());
    let mut x64 = Loader::new(Config { dirs: dirs(), ..Config::default() });
    assert_eq!(x64.add_mapped(&mut p, img32, &wide("C:\\app\\a.exe"), false).unwrap_err(), STATUS_INVALID_IMAGE_FORMAT);
    assert!(x64.add_mapped(&mut p, img64, &wide("C:\\app\\a.exe"), false).is_ok());
    let mut x86 = Loader::new(config());
    assert_eq!(x86.add_mapped(&mut p, img64, &wide("C:\\app\\a.exe"), false).unwrap_err(), STATUS_INVALID_IMAGE_FORMAT);
    assert!(x86.add_mapped(&mut p, img32, &wide("C:\\app\\a.exe"), false).is_ok());
}

#[test]
fn an_image_that_would_end_above_4_gib_is_not_loaded() {
    let d = dll32("high.dll", EXE32, named(&[("F", 0x1010)]), vec![]);
    let mut w = world32(&exe32(vec![]).write());
    w.l.process_init(&mut w.p, w.exe, TEB).unwrap();
    // The process has no free address below 4 GiB for it: the allocator
    // hands out one above.
    w.p.next_any = 0x7_0000_0000;
    w.p.file("C:\\app\\high.dll", d.write());
    let before = (w.p.images(), w.p.heap_blocks());
    assert_eq!(w.l.load(&mut w.p, &wide("high"), None).unwrap_err(), STATUS_NO_MEMORY);
    assert_eq!((w.p.images(), w.p.heap_blocks()), before, "nothing stays mapped");
}
