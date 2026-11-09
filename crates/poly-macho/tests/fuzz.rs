//! A deterministic fuzz test: mutate every test file with a seeded PRNG (bit
//! flips, byte overwrites, interesting values, splices, truncations) and run
//! every parser and every iterator to the end on the result. The assertion
//! is that nothing panics, reads out of bounds, overflows or loops for long.
//!
//! To reproduce a failure: the message names the corpus entry, the seed and
//! the iteration; `SEED` and `iteration` fix the input completely.

mod support;

use poly_macho::*;
use std::ops::ControlFlow;
use std::panic::{AssertUnwindSafe, catch_unwind};
use support::slot::*;
use support::*;

/// Iterations per corpus entry: with twelve entries, 300 thousand runs, and
/// some 40 thousand more in the other two tests.
const ITERATIONS: u64 = 25_000;
const SEED: u64 = 0x504f_4c59_4d41_4348; // "POLYMACH"

/// More items than any iterator can legitimately yield from a file of this
/// size; hitting it means a loop that does not stop.
fn drain<T>(it: impl Iterator<Item = T>, file_len: usize) -> usize {
    let cap = 64 * file_len + 4096;
    let mut n = 0;
    for x in it {
        std::hint::black_box(&x);
        n += 1;
        assert!(n <= cap, "iterator yielded more than {cap} items");
    }
    n
}

/// Runs every API over `data`, ignoring all results.
fn exercise(data: &[u8]) {
    let _ = FileKind::detect(data);
    for kind in [CpuKind::X86_64, CpuKind::Arm64, CpuKind::Arm64e] {
        let _ = select_slice(data, kind);
    }
    if let Ok(fat) = Fat::parse(data) {
        for a in fat.arches() {
            let _ = fat.find(a.cpu);
            let _ = fat.find_kind(a.cpu.kind());
            if let Ok(s) = fat.slice(&a) {
                exercise_thin(s);
            }
        }
    }
    exercise_thin(data);
}

fn exercise_thin(data: &[u8]) {
    let _ = Header::parse(data);
    let Ok(m) = MachO::parse(data) else { return };
    let n = data.len();
    let _ = m.header().file_type();
    let _ = m.cpu().kind();
    drain(m.raw_commands(), n);

    for c in m.commands() {
        match c {
            Command::Segment(s) => {
                let _ = s.name();
                let _ = s.initprot.to_string();
                for sec in s.sections() {
                    let _ = (sec.name(), sec.segment_name(), sec.has_file_contents());
                }
            }
            Command::Thread(t) => {
                drain(t.states(), n);
                for k in [CpuKind::X86_64, CpuKind::Arm64, CpuKind::Other] {
                    let _ = (t.entry_pc(k), t.stack_pointer(k));
                }
            }
            Command::BuildVersion(b) => {
                drain(b.tools(), n);
                let _ = (b.minos.to_string(), b.sdk.to_string());
            }
            Command::Dylib(d) => {
                let _ = (d.current_version.to_string(), d.name.len());
            }
            Command::SourceVersion(v) => {
                let _ = v.parts();
            }
            _ => {}
        }
    }
    drain(m.segments(), n);
    for (i, s) in m.segment_table().iter().enumerate() {
        let _ = (s.name(), s.vm_range(), s.file_range(), s.is_pagezero());
        let _ = m.segment_table().get(i);
    }
    let _ = m.find_segment(b"__TEXT");
    let _ = m.header_segment();
    let _ = m.vm_range();
    let _ = m.entry();
    let _ = m.summary();
    let _ = m.uuid();
    let _ = m.platform();
    let _ = m.source_version();
    let _ = m.dylib_id();
    let _ = m.dylinker();
    drain(m.dylibs(), n);
    drain(m.rpaths(), n);
    for o in [0, 1, 2, 3, 100, u32::MAX] {
        let _ = m.dylib_by_ordinal(o);
    }

    // symbols
    if let Ok(Some(st)) = m.symtab() {
        let mut names = Vec::new();
        for sym in st.iter().take(2000) {
            let _ = (sym.kind(), sym.is_external(), sym.library_ordinal(), sym.is_weak_def());
            if let Ok(name) = st.name(&sym) {
                names.push(name);
            }
        }
        let _ = st.get(u32::MAX);
        let _ = st.name_at(u32::MAX);
        if let Ok(Some(ex)) = m.exports() {
            for name in names.iter().take(50) {
                let _ = ex.lookup(name);
            }
        }
        if let Ok(Some(dy)) = m.dysymtab() {
            let _ = (dy.local_range(), dy.extdef_range(), dy.undef_range());
            for ind in dy.indirect_symbols() {
                let _ = ind.symbol_index().map(|i| st.get(i));
            }
        }
    } else {
        let _ = m.dysymtab();
    }

    // linkedit blobs
    for kind in [
        LinkeditKind::CodeSignature,
        LinkeditKind::SegmentSplitInfo,
        LinkeditKind::FunctionStarts,
        LinkeditKind::DataInCode,
        LinkeditKind::ExportsTrie,
        LinkeditKind::ChainedFixups,
        LinkeditKind::AtomInfo,
    ] {
        let _ = m.linkedit_data(kind);
    }
    if let Ok(Some(fs)) = m.function_starts() {
        drain(fs, n);
    }
    if let Ok(Some(d)) = m.data_in_code() {
        drain(d, n);
    }

    // export trie
    if let Ok(Some(ex)) = m.exports() {
        let mut count = 0usize;
        let _ = ex.walk(|name, e| {
            count += 1;
            let _ = (name.len(), e.kind(), e.is_reexport());
            assert!(count <= 64 * n + 4096, "walk does not stop");
            ControlFlow::Continue(())
        });
        let _ = ex.count();
        let _ = ex.lookup(b"_main");
        let _ = ex.lookup(b"");
        let _ = ex.lookup(&[b'_'; 300]);
    }

    // classic fixups
    if let Ok(Some(info)) = m.dyld_info() {
        drain(info.rebases(), n);
        drain(info.binds(), n);
        drain(info.weak_binds(), n);
        drain(info.weak_bind_events(), n);
        drain(info.lazy_binds(), n);
    }

    // chained fixups
    if let Ok(Some(cf)) = m.chained_fixups() {
        let _ = (cf.header(), cf.segment_count(), cf.import_count());
        let mut i = 0;
        for imp in cf.imports() {
            let _ = imp.map(|x| (x.name.len(), x.lib_ordinal));
            i += 1;
            assert!(i <= n);
        }
        let _ = cf.import(u32::MAX);
        for s in 0..cf.segment_count().min(64) {
            let _ = cf.starts_in_segment(s).map(|o| {
                o.map(|st| {
                    let _ = st.stride();
                    for p in 0..st.page_count.min(64) {
                        let _ = st.page_start(p);
                    }
                })
            });
        }
        drain(cf.fixups(), n);
    }
}

// ---------------------------------------------------------------------------
// The corpus: the real files and a few built here
// ---------------------------------------------------------------------------

fn synthetic_corpus() -> Vec<(&'static str, Vec<u8>)> {
    let mut v = Vec::new();

    // arm64e chained fixups with imports, in two page sizes.
    let imports = [
        ImportSpec { ordinal: 1, weak: false, name: "_one", addend: 0 },
        ImportSpec { ordinal: 0xfe, weak: true, name: "_two", addend: -3 },
    ];
    let payload = chained_payload(DYLD_CHAINED_PTR_ARM64E_USERLAND, 0x4000, &[0], DYLD_CHAINED_IMPORT_ADDEND, &imports);
    let slots = [
        (0, arm64e_auth_rebase(0x1234, 0xbeef, 1, 2, 1)),
        (8, arm64e_auth_bind(1, 0x1234, 0, 3, 2)),
        (24, arm64e_rebase(0x7ffff, 0x5a, 1)),
        (32, arm64e_bind(0, -5, 0)),
    ];
    v.push(("synthetic arm64e chains", chained_image(&payload, &slots)));
    let payload = chained_payload(
        DYLD_CHAINED_PTR_64_OFFSET,
        0x1000,
        &[0x10, DYLD_CHAINED_PTR_START_NONE, 0x20],
        DYLD_CHAINED_IMPORT_ADDEND64,
        &imports,
    );
    let slots = [(0x10, ptr64_rebase(0x11, 0, 2)), (0x18, ptr64_bind(1, 7, 0)), (0x2020, ptr64_bind(0, 0, 0))];
    v.push(("synthetic 64-bit chains", chained_image(&payload, &slots)));

    // classic streams using most opcodes
    let rebase = {
        let mut s = vec![REBASE_OPCODE_SET_TYPE_IMM | 1, REBASE_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 1, 0x10];
        s.extend([REBASE_OPCODE_DO_REBASE_IMM_TIMES | 3, REBASE_OPCODE_ADD_ADDR_IMM_SCALED | 2]);
        s.extend([REBASE_OPCODE_DO_REBASE_ULEB_TIMES, 2, REBASE_OPCODE_ADD_ADDR_ULEB, 0x10]);
        s.extend([REBASE_OPCODE_DO_REBASE_ADD_ADDR_ULEB, 0x20]);
        s.extend([REBASE_OPCODE_DO_REBASE_ULEB_TIMES_SKIPPING_ULEB, 3, 8, REBASE_OPCODE_DONE]);
        s
    };
    let bind = {
        let mut s = vec![BIND_OPCODE_SET_DYLIB_ORDINAL_IMM | 2, BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM | 1];
        s.extend(b"_foo\0");
        s.extend([BIND_OPCODE_SET_TYPE_IMM | 1, BIND_OPCODE_SET_ADDEND_SLEB, 0x70]);
        s.extend([BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 1, 0x20, BIND_OPCODE_DO_BIND]);
        s.extend([BIND_OPCODE_DO_BIND_ADD_ADDR_ULEB, 0x10, BIND_OPCODE_DO_BIND_ADD_ADDR_IMM_SCALED | 3]);
        s.extend([BIND_OPCODE_DO_BIND_ULEB_TIMES_SKIPPING_ULEB, 2, 8]);
        s.extend([BIND_OPCODE_SET_DYLIB_SPECIAL_IMM | 0xf, BIND_OPCODE_ADD_ADDR_ULEB, 8, BIND_OPCODE_DO_BIND]);
        s.push(BIND_OPCODE_DONE);
        s
    };
    let weak = {
        let mut s = vec![BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM | 8];
        s.extend(b"_strong\0");
        s.push(BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM);
        s.extend(b"_w\0");
        s.extend([BIND_OPCODE_SET_TYPE_IMM | 1, BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 1, 8]);
        s.extend([BIND_OPCODE_DO_BIND, BIND_OPCODE_DONE]);
        s
    };
    let lazy = {
        let mut s = Vec::new();
        for (off, name) in [(0x10u8, "_a"), (0x18, "_b")] {
            s.extend([BIND_OPCODE_SET_SEGMENT_AND_OFFSET_ULEB | 1, off, BIND_OPCODE_SET_DYLIB_ORDINAL_IMM | 1]);
            s.push(BIND_OPCODE_SET_SYMBOL_TRAILING_FLAGS_IMM);
            s.extend(name.as_bytes());
            s.extend([0, BIND_OPCODE_DO_BIND, BIND_OPCODE_DONE]);
        }
        s
    };
    v.push(("synthetic classic streams", classic_image(&rebase, &bind, &weak, &lazy)));

    // an export trie in a dylib's linkedit, plus thread and version commands
    let mut reexport = uleb(EXPORT_SYMBOL_FLAGS_REEXPORT);
    reexport.extend(uleb(1));
    reexport.extend(b"_other\0");
    let mut stub = uleb(EXPORT_SYMBOL_FLAGS_STUB_AND_RESOLVER);
    stub.extend(uleb(0x10));
    stub.extend(uleb(0x20));
    let trie = trie_bytes(&[
        TrieNode { terminal: None, edges: vec![(b"_".to_vec(), 1)] },
        TrieNode {
            terminal: None,
            edges: vec![(b"a".to_vec(), 2), (b"b".to_vec(), 3), (b"c".to_vec(), 4), (b"d".to_vec(), 5)],
        },
        TrieNode { terminal: Some(regular(0, 0x10)), edges: vec![(b"x".to_vec(), 5)] },
        TrieNode { terminal: Some(reexport), edges: vec![] },
        TrieNode { terminal: Some(stub), edges: vec![] },
        TrieNode { terminal: Some(regular(EXPORT_SYMBOL_FLAGS_WEAK_DEFINITION, 0x40)), edges: vec![] },
    ]);
    let img = Image::new(CPU_TYPE_X86_64, MH_EXECUTE)
        .segment(
            "__TEXT",
            0x1000,
            0x1000,
            0,
            0x1000,
            5,
            &[Sect {
                name: "__text",
                addr: 0x1100,
                size: 0x40,
                offset: 0x100,
                flags: S_REGULAR | S_ATTR_PURE_INSTRUCTIONS,
            }],
        )
        .segment("__LINKEDIT", 0x2000, 0x1000, 0x1000, 0x1000, 1, &[])
        .linkedit(LC_DYLD_EXPORTS_TRIE, 0x1000, trie.len() as u32)
        .thread(LC_UNIXTHREAD, X86_THREAD_STATE64, X86_THREAD_STATE64_COUNT, &[(16, 0x1100), (7, 0x1ff0)])
        .dylib(LC_LOAD_DYLIB, "/usr/lib/libx.dylib", 0x0001_0203, 0x0001_0000)
        .dylib(LC_REEXPORT_DYLIB, "/usr/lib/liby.dylib", 0, 0)
        .string_cmd(LC_RPATH, "@loader_path")
        .cmd(LC_VERSION_MIN_MACOSX, &[0, 5, 10, 0, 0, 3, 11, 0])
        .cmd(LC_SOURCE_VERSION, &7u64.to_le_bytes())
        .cmd(LC_UUID, &[3u8; 16]);
    let mut f = img.build(0x2000);
    f[0x1000..0x1000 + trie.len()].copy_from_slice(&trie);
    v.push(("synthetic trie and commands", f));
    v
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut c: Vec<(String, Vec<u8>)> = THIN_FIXTURES.iter().map(|n| (n.to_string(), fixture(n))).collect();
    c.push(("universal.bin".to_string(), fixture("universal.bin")));
    for (n, d) in synthetic_corpus() {
        c.push((n.to_string(), d));
    }
    c
}

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

const INTERESTING32: [u32; 12] =
    [0, 1, 2, 7, 8, 0xff, 0xffff, 0x7fff_ffff, 0x8000_0000, 0xffff_fff8, 0xffff_ffff, 0x4000_0000];

/// One input: the original with mutations applied, restorable.
struct Mutant {
    work: Vec<u8>,
    undo: Vec<(usize, u8)>,
    len: usize,
}

impl Mutant {
    fn new(orig: &[u8]) -> Mutant {
        Mutant { work: orig.to_vec(), undo: Vec::new(), len: orig.len() }
    }

    fn set(&mut self, at: usize, v: u8) {
        if at < self.work.len() {
            self.undo.push((at, self.work[at]));
            self.work[at] = v;
        }
    }

    fn restore(&mut self, orig_len: usize) {
        while let Some((at, v)) = self.undo.pop() {
            self.work[at] = v;
        }
        self.len = orig_len;
    }

    fn bytes(&self) -> &[u8] {
        &self.work[..self.len]
    }
}

/// Positions of the bytes that are not zero in the original: headers, load
/// commands, tables and streams. Most mutations aim there.
fn hot_positions(orig: &[u8]) -> Vec<usize> {
    orig.iter().enumerate().filter(|(_, b)| **b != 0).map(|(i, _)| i).collect()
}

fn mutate(m: &mut Mutant, hot: &[usize], rng: &mut Rng) {
    let len = m.work.len();
    let pick = |rng: &mut Rng| -> usize {
        if !hot.is_empty() && rng.below(10) < 8 {
            hot[rng.below(hot.len() as u64) as usize]
        } else {
            rng.below(len as u64) as usize
        }
    };
    let rounds = 1 + rng.below(3);
    for _ in 0..rounds {
        match rng.below(8) {
            0 => {
                // flip 1..8 bits
                for _ in 0..1 + rng.below(8) {
                    let at = pick(rng);
                    let bit = rng.below(8) as u8;
                    let v = m.work.get(at).copied().unwrap_or(0) ^ (1 << bit);
                    m.set(at, v);
                }
            }
            1 => {
                // overwrite a few bytes
                for _ in 0..1 + rng.below(8) {
                    let at = pick(rng);
                    m.set(at, rng.next() as u8);
                }
            }
            2 | 3 => {
                // a 32-bit word of an interesting value, little-endian
                let at = pick(rng) & !3;
                let v = INTERESTING32[rng.below(INTERESTING32.len() as u64) as usize];
                for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                    m.set(at + i, b);
                }
            }
            4 => {
                // a 64-bit word
                let at = pick(rng) & !7;
                let v = [0u64, u64::MAX, 1 << 63, 1 << 32, 0xffff_ffff, u64::MAX - 7][rng.below(6) as usize];
                for (i, b) in v.to_le_bytes().into_iter().enumerate() {
                    m.set(at + i, b);
                }
            }
            5 => {
                // splice: copy 4..16 bytes from one hot place to another
                let from = pick(rng);
                let to = pick(rng);
                for i in 0..4 + rng.below(13) as usize {
                    if let Some(&b) = m.work.get(from + i) {
                        m.set(to + i, b);
                    }
                }
            }
            6 => {
                // truncate somewhere, often near a hot byte
                let at = pick(rng).max(1);
                m.len = m.len.min(at + rng.below(3) as usize);
            }
            _ => {
                // clear a short run
                let at = pick(rng);
                for i in 0..1 + rng.below(32) as usize {
                    m.set(at + i, 0);
                }
            }
        }
    }
}

fn fuzz(name: &str, orig: &[u8], iterations: u64, seed: u64) {
    let hot = hot_positions(orig);
    let mut rng = Rng(seed | 1);
    let mut m = Mutant::new(orig);
    for it in 0..iterations {
        mutate(&mut m, &hot, &mut rng);
        let input = m.bytes();
        let r = catch_unwind(AssertUnwindSafe(|| exercise(input)));
        if r.is_err() {
            let dump = std::env::temp_dir().join(format!("poly-macho-fuzz-{it}.bin"));
            let _ = std::fs::write(&dump, input);
            panic!("{name}: panic at iteration {it} (seed {seed:#x}); input saved to {}", dump.display());
        }
        m.restore(orig.len());
    }
}

#[test]
fn originals_exercise_without_error_paths_panicking() {
    for (name, data) in corpus() {
        exercise(&data);
        // The first runs also cover every truncation of the headers.
        let upto = data.len().min(0x700);
        for len in 0..=upto {
            exercise(&data[..len]);
        }
        // And the last bytes of the file.
        for len in data.len().saturating_sub(300)..=data.len() {
            exercise(&data[..len]);
        }
        let _ = name;
    }
}

/// One test per corpus entry, so the test harness runs them on several
/// threads.
macro_rules! fuzz_entries {
    ($($test:ident => $index:expr),* $(,)?) => {
        $(
            #[test]
            fn $test() {
                let c = corpus();
                let (name, data) = &c[$index];
                fuzz(name, data, ITERATIONS, SEED.wrapping_add($index as u64 * 0x9e37_79b9));
            }
        )*

        /// A new test file needs a line above.
        #[test]
        fn every_corpus_entry_has_a_fuzz_test() {
            let n = [$($index),*].len();
            assert_eq!(corpus().len(), n);
        }
    };
}

fuzz_entries! {
    fuzz_arm64_chained => 0,
    fuzz_arm64_classic => 1,
    fuzz_x86_64_classic => 2,
    fuzz_x86_64_chained => 3,
    fuzz_x86_64_nopie => 4,
    fuzz_libtiny => 5,
    fuzz_libre => 6,
    fuzz_universal => 7,
    fuzz_synthetic_arm64e_chains => 8,
    fuzz_synthetic_64bit_chains => 9,
    fuzz_synthetic_classic_streams => 10,
    fuzz_synthetic_trie_and_commands => 11,
}

#[test]
fn every_single_byte_of_the_headers_and_commands_set_to_extremes() {
    // Exhaustive instead of random for the first bytes: header and the start
    // of the load commands, each byte to 0x00, 0x01, 0x7f, 0x80 and 0xff.
    for (name, data) in corpus() {
        let upto = data.len().min(0x180);
        let mut work = data.clone();
        for at in 0..upto {
            let orig = work[at];
            for v in [0x00u8, 0x01, 0x7f, 0x80, 0xff] {
                work[at] = v;
                let r = catch_unwind(AssertUnwindSafe(|| exercise(&work)));
                assert!(r.is_ok(), "{name}: byte {at:#x} = {v:#x}");
            }
            work[at] = orig;
        }
    }
}
