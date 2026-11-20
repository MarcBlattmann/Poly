//! Tests: the arm64 test programs (tests/a64/macos) under the interpreter on
//! every host and under the translator on x86-64 hosts, and differential
//! tests of translated code against the interpreter.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use std::path::PathBuf;

use poly_a64::Cpu;
use poly_personality_xnu::{Class, arm64};

use crate::jit::{Engine, Kernel, Stop};
use crate::mem::GuestMem;

/// `write` and `exit` in the arm64 convention, output captured.
#[derive(Default)]
pub(crate) struct TestKernel {
    pub out: Vec<u8>,
}

impl Kernel for TestKernel {
    fn svc(&mut self, imm: u16, cpu: &mut Cpu, mem: &mut GuestMem) -> Option<i32> {
        assert_eq!(imm, arm64::SVC_IMM, "svc immediate");
        let (class, n) = arm64::decode(cpu.x[16]);
        let ok = |cpu: &mut Cpu, v: u64| {
            cpu.x[0] = v;
            cpu.nzcv &= !arm64::NZCV_C;
        };
        match (class, n) {
            (Class::Bsd, 1) => return Some(cpu.x[0] as i32),
            (Class::Bsd, 4) => {
                let mut buf = alloc::vec![0u8; cpu.x[2] as usize];
                if mem.peek(cpu.x[1], &mut buf).is_err() {
                    cpu.x[0] = 14; // EFAULT
                    cpu.nzcv |= arm64::NZCV_C;
                    return None;
                }
                self.out.extend_from_slice(&buf);
                let n = buf.len() as u64;
                ok(cpu, n);
            }
            _ => {
                cpu.x[0] = 78; // ENOSYS
                cpu.nzcv |= arm64::NZCV_C;
            }
        }
        None
    }
}

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/a64/macos/bin")
}

/// The test programs: (name, file, expected output with the exit line).
pub(crate) fn corpus() -> Vec<(String, Vec<u8>, String)> {
    let mut v = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(corpus_dir())
        .expect("tests/a64/macos/bin")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| !n.ends_with(".out"))
        .collect();
    names.sort();
    for n in names {
        let file = std::fs::read(corpus_dir().join(&n)).unwrap();
        let out = std::fs::read_to_string(corpus_dir().join(format!("{n}.out"))).unwrap();
        v.push((n, file, out));
    }
    v
}

/// Run a program to its exit; output plus "exit N".
pub(crate) fn run_program(file: &[u8], engine: &mut Engine) -> (String, Stop) {
    let mut p = crate::process::load(file, 256 << 20, &["prog"]).expect("loads");
    let mut k = TestKernel::default();
    let stop = loop {
        match engine.run(&mut p.cpu, &mut p.mem, &mut k, 1 << 20) {
            Stop::Budget => continue,
            s => break s,
        }
    };
    let mut text = String::from_utf8_lossy(&k.out).into_owned();
    if let Stop::Exit(c) = stop {
        text += &format!("exit {c}\n");
    } else {
        text += &format!("stopped {stop:x?} at pc {:#x}\n", p.cpu.pc);
    }
    (text, stop)
}

#[test]
fn corpus_under_the_interpreter() {
    let c = corpus();
    assert!(c.len() >= 18, "the test programs");
    for (name, file, want) in &c {
        let mut e = Engine::interpreter();
        let (got, _) = run_program(file, &mut e);
        assert_eq!(&got, want, "{name} under the interpreter");
    }
}

#[test]
fn guest_pages_are_16k_and_follow_map_jit() {
    use crate::mem::*;
    let mut m = GuestMem::new(0x1_0000_0000, 64 * PAGE_SIZE);
    assert_eq!(m.map(0x1_0000_1000, PAGE_SIZE, R | W, 0), Err(MapError::Unaligned));
    m.map(0x1_0000_4000, PAGE_SIZE, R | W, MAP_JIT).unwrap();
    // Execute mode by default: readable, executable, not writable.
    assert_eq!(m.page_flags(0x1_0000_4000) & (R | W | X), R | X);
    assert!(poly_a64::Memory::write(&mut m, 0x1_0000_4000, &[1]).is_err());
    m.set_jit_write_protect(false);
    assert_eq!(m.page_flags(0x1_0000_7FFF) & (R | W | X), R | W);
    poly_a64::Memory::write(&mut m, 0x1_0000_4000, &[0x1F, 0x20, 0x03, 0xD5]).unwrap();
    assert!(poly_a64::Memory::fetch(&mut m, 0x1_0000_4000).is_err(), "not executable in write mode");
    m.set_jit_write_protect(true);
    assert_eq!(poly_a64::Memory::fetch(&mut m, 0x1_0000_4000), Ok(0xD503_201F));
    // Code tracking: a write to a page with translations is reported once.
    m.mark_code(0x1_0000_4000, 4);
    m.set_jit_write_protect(false);
    assert_eq!(m.take_written_code(), alloc::vec![0x1_0000_4000]);
    poly_a64::Memory::write(&mut m, 0x1_0000_4000, &[0]).unwrap();
    assert!(m.take_written_code().is_empty());
    // An access that crosses into an unmapped page faults at that page.
    let mut b = [0u8; 8];
    let f = poly_a64::Memory::read(&mut m, 0x1_0000_7FFC, &mut b).unwrap_err();
    assert_eq!(f.addr, 0x1_0000_8000);
}

/// Print how each test program ends under the interpreter (for finding
/// the instructions to add next).
#[test]
#[ignore = "a report, not a check"]
fn corpus_report() {
    for (name, file, want) in corpus() {
        let mut e = Engine::interpreter();
        let (got, stop) = run_program(&file, &mut e);
        std::println!("{name:<12} {} {stop:x?}", if got == want { "ok  " } else { "FAIL" });
    }
}

// ---------------------------------------------------------------------
// Translated code (x86-64 hosts only)
// ---------------------------------------------------------------------

/// A code area for the translator: anonymous read-write-execute memory.
#[cfg(all(target_arch = "x86_64", unix))]
pub(crate) fn code_area(len: usize) -> *mut u8 {
    // SAFETY: an anonymous private mapping; the result is checked. It is
    // never unmapped (tests are short-lived processes).
    let p = unsafe {
        libc::mmap(
            core::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE | libc::PROT_EXEC,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        )
    };
    assert_ne!(p, libc::MAP_FAILED, "mmap of the code area");
    p.cast()
}

#[cfg(all(target_arch = "x86_64", unix))]
pub(crate) fn jit_engine() -> Engine {
    const LEN: usize = 16 << 20;
    // SAFETY: a fresh RWX mapping of LEN bytes, used only by this engine.
    unsafe { Engine::with_code(code_area(LEN), LEN) }
}

#[cfg(all(target_arch = "x86_64", unix))]
#[test]
fn corpus_under_the_translator() {
    for (name, file, want) in corpus() {
        let mut e = jit_engine();
        let (got, _) = run_program(&file, &mut e);
        assert_eq!(got, want, "{name} translated");
        assert!(e.stats.translated > 0, "{name}: something was translated");
        std::println!(
            "{name:<12} blocks {:>5} insts {:>6} interpreted {:>8} entries {:>8} chained {:>5}",
            e.stats.translated,
            e.stats.translated_insts,
            e.stats.interpreted,
            e.stats.entries,
            e.stats.chained
        );
    }
}

/// Time the test programs under both engines.
#[cfg(all(target_arch = "x86_64", unix))]
#[test]
#[ignore = "a measurement"]
fn corpus_speed() {
    use std::time::Instant;
    let (mut ti, mut tj) = (0f64, 0f64);
    for (name, file, _) in corpus() {
        let t0 = Instant::now();
        run_program(&file, &mut Engine::interpreter());
        let i = t0.elapsed().as_secs_f64();
        let t1 = Instant::now();
        run_program(&file, &mut jit_engine());
        let j = t1.elapsed().as_secs_f64();
        ti += i;
        tj += j;
        std::println!("{name:<12} interpreter {i:>8.3}s translator {j:>8.3}s  x{:.1}", i / j);
    }
    std::println!("total        interpreter {ti:>8.3}s translator {tj:>8.3}s  x{:.1}", ti / tj);
}

/// Random instructions and short sequences: translated code against the
/// interpreter, register by register and byte by byte.
#[cfg(all(target_arch = "x86_64", unix))]
#[test]
fn translated_matches_interpreted() {
    use crate::mem::{PAGE_SIZE, R as PR, W as PW, X as PX};
    use poly_a64::{Inst, decode};
    const LO: u64 = 0x1_0000_0000;
    let mut rng = Rng(0x5EED_1234_ABCD_0001);
    let mut engine = jit_engine();
    let mut interp = Engine::interpreter();
    let iterations = std::env::var("JIT_DIFF_ITERS").ok().and_then(|v| v.parse().ok()).unwrap_or(30_000u32);
    let mut compared = 0u32;
    let mut translated_any = 0u32;
    for it in 0..iterations {
        // A sequence of 1..=6 instructions, then BRKs.
        let len = 1 + rng.below(6) as usize;
        let mut words = Vec::new();
        while words.len() < len {
            let w = random_word(&mut rng);
            if matches!(decode(w), Inst::Undefined(_) | Inst::Unsupported(_)) {
                continue;
            }
            words.push(w);
        }
        let mut code = alloc::vec![0xD420_0000u32; (PAGE_SIZE / 4) as usize]; // brk #0
        code[..len].copy_from_slice(&words);
        let bytes: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let setup = |m: &mut GuestMem, rng: &mut Rng| {
            m.map(LO, PAGE_SIZE, PR | PW, 0).unwrap();
            m.poke(LO, &bytes).unwrap();
            m.protect(LO, PAGE_SIZE, PR | PX).unwrap();
            m.map(LO + PAGE_SIZE, 2 * PAGE_SIZE, PR | PW, 0).unwrap();
            let mut data = alloc::vec![0u8; 2 * PAGE_SIZE as usize];
            for c in data.chunks_mut(8) {
                c.copy_from_slice(&rng.next().to_le_bytes());
            }
            m.poke(LO + PAGE_SIZE, &data).unwrap();
            // A read-only page after it.
            m.map(LO + 3 * PAGE_SIZE, PAGE_SIZE, PR, 0).unwrap();
        };
        let seed = rng.next();
        let mut ma = GuestMem::new(LO, 8 * PAGE_SIZE);
        let mut mb = GuestMem::new(LO, 8 * PAGE_SIZE);
        setup(&mut ma, &mut Rng(seed));
        setup(&mut mb, &mut Rng(seed));
        let mut cpu = Cpu::new();
        for (i, x) in cpu.x.iter_mut().enumerate() {
            *x = random_value(&mut rng);
            if i == 5 || i == 6 {
                *x = LO + 2 * PAGE_SIZE + (rng.below(512) as u64) * 8 - 2048 + u64::from(rng.below(8));
            }
        }
        cpu.sp = LO + 2 * PAGE_SIZE + 4096;
        cpu.nzcv = rng.below(16) << 28;
        for v in cpu.v.iter_mut() {
            *v = (u128::from(random_fp(&mut rng)) << 64) | u128::from(random_fp(&mut rng));
        }
        cpu.fpcr = if rng.below(8) == 0 { 1 << 25 } else { 0 };
        cpu.pc = LO;
        let mut ca = cpu.clone();
        let mut cb = cpu;
        let mut ka = TestKernel::default();
        let mut kb = TestKernel::default();
        let before = engine.stats.translated;
        let sa = engine.run(&mut ca, &mut ma, &mut ka, 64);
        let sb = interp.run(&mut cb, &mut mb, &mut kb, 64);
        if engine.stats.translated > before {
            translated_any += 1;
        }
        let mut da = alloc::vec![0u8; 3 * PAGE_SIZE as usize];
        let mut db = alloc::vec![0u8; 3 * PAGE_SIZE as usize];
        ma.peek(LO + PAGE_SIZE, &mut da).unwrap();
        mb.peek(LO + PAGE_SIZE, &mut db).unwrap();
        if sa == Stop::Budget || sb == Stop::Budget {
            // A loop: the budget counts blocks in one engine, instructions in the other.
            engine.flush();
            continue;
        }
        let ctx = || {
            format!("iteration {it}: words {words:08x?} ({:?})", words.iter().map(|w| decode(*w)).collect::<Vec<_>>())
        };
        assert_eq!(sa, sb, "stop: {}", ctx());
        assert_eq!(ca.x, cb.x, "x registers: {}", ctx());
        assert_eq!((ca.sp, ca.pc), (cb.sp, cb.pc), "sp, pc: {}", ctx());
        assert_eq!(ca.nzcv, cb.nzcv, "nzcv: {}", ctx());
        assert_eq!(ca.v, cb.v, "v registers: {}", ctx());
        assert_eq!(ca.fpsr, cb.fpsr, "fpsr: {}", ctx());
        assert!(da == db, "memory: {}", ctx());
        compared += 1;
        // Each iteration maps the same addresses again: drop the old translations.
        engine.flush();
    }
    std::println!("{compared} sequences compared, {translated_any} with translated code");
    assert!(translated_any > compared / 4, "the translator took part");
}

#[cfg(all(target_arch = "x86_64", unix))]
pub(crate) struct Rng(pub u64);
#[cfg(all(target_arch = "x86_64", unix))]
impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 33) as u32 % n
    }
}

#[cfg(all(target_arch = "x86_64", unix))]
fn random_fp(rng: &mut Rng) -> u64 {
    const D: [u64; 10] = [
        0,
        0x8000_0000_0000_0000,
        0x3FF0_0000_0000_0000,
        0x7FF0_0000_0000_0000,
        0x7FF8_0000_0000_0001,
        0x7FF0_0000_0000_0001,
        0x0000_0000_0000_0001,
        0x7FEF_FFFF_FFFF_FFFF,
        0x43E0_0000_0000_0000,
        0xC1E0_0000_0020_0000,
    ];
    const S: [u32; 8] = [0, 0x8000_0000, 0x3F80_0000, 0x7F80_0000, 0x7FC0_0001, 0x7F80_0001, 0x0000_0001, 0x4F00_0000];
    match rng.below(5) {
        0 => D[rng.below(10) as usize],
        1 => u64::from(S[rng.below(8) as usize]) | (u64::from(S[rng.below(8) as usize]) << 32),
        2 => {
            // An ordinary double in a moderate range.
            (0x3F00u64 + u64::from(rng.below(0x200))) << 52 | (rng.next() >> 12) | (u64::from(rng.below(2)) << 63)
        }
        3 => {
            (u64::from(0x3C00_0000 | (rng.next() as u32 & 0x07FF_FFFF)) << 32)
                | u64::from(0x3C00_0000 | (rng.next() as u32 & 0x07FF_FFFF))
        }
        _ => rng.next(),
    }
}

#[cfg(all(target_arch = "x86_64", unix))]
fn random_value(rng: &mut Rng) -> u64 {
    const EDGE: [u64; 12] = [
        0,
        1,
        2,
        0x7F,
        0x8000_0000,
        0xFFFF_FFFF,
        0x7FFF_FFFF,
        0x1_0000_0000,
        0x7FFF_FFFF_FFFF_FFFF,
        0x8000_0000_0000_0000,
        u64::MAX,
        0xFFFF_FFFF_FFFF_FFFE,
    ];
    match rng.below(4) {
        0 => EDGE[rng.below(12) as usize],
        1 => rng.next() & 0xFFFF,
        _ => rng.next(),
    }
}

/// A random word from the classes the translator handles, with loads and
/// stores based on x5 or x6 (pointing into the data pages), and branches
/// that stay near.
#[cfg(all(target_arch = "x86_64", unix))]
fn random_word(rng: &mut Rng) -> u32 {
    let r = rng.next() as u32;
    let (rd, rn, rm, ra) = (rng.below(32), rng.below(32), rng.below(32), rng.below(32));
    let ft = rng.below(2) << 22;
    match rng.below(14) {
        // Scalar floating point.
        10 => match rng.below(6) {
            0 => 0x1E20_0800 | ft | (rm << 16) | (rng.below(9) << 12) | (rn << 5) | rd,
            1 => 0x1E20_4000 | ft | (rng.below(6) << 15) | (rn << 5) | rd,
            2 => {
                0x1F00_0000
                    | ft
                    | (rng.below(2) << 21)
                    | (rm << 16)
                    | (rng.below(2) << 15)
                    | (ra << 10)
                    | (rn << 5)
                    | rd
            }
            3 => 0x1E20_2000 | ft | (rm << 16) | (rn << 5) | (rng.below(4) << 3),
            4 => 0x1E20_0C00 | ft | (rm << 16) | (rng.below(16) << 12) | (rn << 5) | rd,
            _ => {
                let op = [0x1E22_0000u32, 0x1E23_0000, 0x1E38_0000, 0x1E39_0000][rng.below(4) as usize];
                op | (rng.below(2) << 31) | ft | (rn << 5) | rd
            }
        },
        // Advanced SIMD three same and two-register miscellaneous.
        11 => (r & 0x40E0_F800) | 0x0E20_0400 | (rng.below(2) << 29) | (rm << 16) | (rn << 5) | rd,
        12 => (r & 0x40C1_F000) | 0x0E20_0800 | (rng.below(2) << 29) | (rn << 5) | rd,
        // Moves between the register files, DUP, MOVI.
        13 => (r & 0x601F_7800) | 0x0E00_0400 | (rn << 5) | rd,
        // Data processing, immediate and register.
        0..=2 => r & !(0xF << 25) | [0b1000u32, 0b1001, 0b0101, 0b1101][rng.below(4) as usize] << 25,
        // Flag setters and readers, often: ADDS/SUBS/ANDS (imm), CCMP, CSEL/CSINC, ADC/SBC.
        3 => {
            let rd = rng.below(32);
            let rn = rng.below(32);
            match rng.below(6) {
                0 => 0x7100_0000 | (rng.below(2) << 31) | (rng.below(64) << 10) | (rn << 5) | 31, // cmp imm
                1 => 0x6B00_0000 | (rng.below(2) << 31) | (rng.below(32) << 16) | (rn << 5) | 31, // cmp reg
                2 => {
                    0x7A40_0800
                        | (rng.below(2) << 31)
                        | (rng.below(32) << 16)
                        | (rng.below(16) << 12)
                        | (rn << 5)
                        | rng.below(16)
                } // ccmp imm
                3 => {
                    0x1A80_0000
                        | (rng.below(2) << 31)
                        | (rng.below(4) << 29 & 0x4000_0000)
                        | (rng.below(32) << 16)
                        | (rng.below(16) << 12)
                        | (rng.below(2) << 10)
                        | (rn << 5)
                        | rd
                } // csel/csinc/csinv/csneg
                4 => 0x1A00_0000 | (rng.below(2) << 31) | (rng.below(4) << 29) | (rng.below(32) << 16) | (rn << 5) | rd, // adc/adcs/sbc/sbcs
                _ => 0x7200_0000 | (rng.below(2) << 31) | (rng.below(64) << 10) | (rn << 5) | rd & 0x3FF_FFFF, // ands imm (may be undefined)
            }
        }
        // Conditional branches and compare-and-branch to nearby targets.
        4 => {
            let off = (rng.below(16) as i32 - 2) as u32 & 0x7FFFF;
            match rng.below(3) {
                0 => 0x5400_0000 | (off << 5) | rng.below(15),
                1 => 0x3400_0000 | (rng.below(2) << 31) | (rng.below(2) << 24) | (off << 5) | rng.below(31),
                _ => {
                    0x3600_0000
                        | (rng.below(2) << 31)
                        | (rng.below(2) << 24)
                        | (rng.below(32) << 19)
                        | ((off & 0x3FFF) << 5)
                        | rng.below(31)
                }
            }
        }
        5 => 0x1400_0000 | (rng.below(2) << 31) | (rng.below(12)), // b / bl forward
        // Loads and stores on x5/x6.
        _ => {
            let op0 = [0b0100u32, 0b0110, 0b1100, 0b1110][rng.below(4) as usize];
            let base = 5 + rng.below(2);
            let mut w = r & !(0xF << 25) & !(0x1F << 5) | op0 << 25 | base << 5;
            // Register offsets index with x6/x5 (small values).
            if (w >> 21) & 1 == 1 && (w >> 10) & 3 == 2 {
                w = w & !(0x1F << 16) | ((11 - base) << 16);
            }
            // No literal loads (they read the code page) half of the time.
            w
        }
    }
}

/// Which instructions the test programs execute that the translator leaves
/// to the interpreter, by kind and count (runs the interpreter, any host).
#[test]
#[ignore = "a report"]
fn untranslated_profile() {
    use std::collections::BTreeMap;
    let mut hist: BTreeMap<String, u64> = BTreeMap::new();
    let mut total = 0u64;
    let mut untranslated = 0u64;
    for (_, file, _) in corpus() {
        let mut p = crate::process::load(&file, 256 << 20, &["prog"]).unwrap();
        let mut k = TestKernel::default();
        loop {
            let w = poly_a64::Memory::fetch(&mut p.mem, p.cpu.pc).unwrap();
            let i = poly_a64::decode(w);
            total += 1;
            if !crate::translate::translatable(&i) {
                untranslated += 1;
                let name = format!("{i:?}");
                let key: String = name.split([' ', '(']).next().unwrap_or("").into();
                let key = match i {
                    poly_a64::Inst::Vec3Same { op, .. } => format!("Vec3Same {op:?}"),
                    poly_a64::Inst::Vec2Misc { op, .. } => format!("Vec2Misc {op:?}"),
                    poly_a64::Inst::FpBinary { op, ft, .. } => format!("FpBinary {op:?} {ft:?}"),
                    poly_a64::Inst::FpUnary { op, ft, .. } => format!("FpUnary {op:?} {ft:?}"),
                    poly_a64::Inst::VecFp3 { op, .. } => format!("VecFp3 {op:?}"),
                    _ => key,
                };
                *hist.entry(key).or_default() += 1;
            }
            match p.cpu.step(&mut p.mem) {
                poly_a64::Outcome::Continue => {}
                poly_a64::Outcome::Svc(imm) => {
                    if k.svc(imm, &mut p.cpu, &mut p.mem).is_some() {
                        break;
                    }
                }
                o => panic!("{o:?}"),
            }
        }
    }
    let mut v: Vec<_> = hist.into_iter().collect();
    v.sort_by_key(|e| core::cmp::Reverse(e.1));
    std::println!("{untranslated} of {total} executed instructions untranslated");
    for (k, n) in v.iter().take(40) {
        std::println!("{n:>10} {k}");
    }
}
