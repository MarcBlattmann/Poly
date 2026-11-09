//! Test-case generation for the differential fuzzer.
//!
//! Each *case* is one instruction (or a few lines of assembly around one
//! instruction, for branches and exclusives). A *block* is a case with a
//! random initial register state; the generated program runs many blocks one
//! after another and, after each, writes a fixed-size *record* of the whole
//! register state and a memory window to standard output.
//!
//! Instruction words are built from the encoding diagrams of the Arm
//! Architecture Reference Manual (DDI 0487, chapter C4) with small field
//! helpers. Fields are drawn so that the word is allocated; the program is
//! then run by the system `qemu-aarch64` and by poly-a64, and the records
//! are compared. Nothing here depends on poly-a64's decoder, so a decoder
//! bug cannot hide a test case.

use std::fmt::Write;

/// Record and init layout, in 64-bit words: tag, X0..X30, SP, NZCV, FPSR,
/// pad, V0..V31 (two words each), then the memory window.
pub const REC_WORDS: usize = 164;
/// Size of a record in bytes.
pub const REC_BYTES: usize = REC_WORDS * 8;
/// Word index of the first X register in a record (the tag is word 0).
pub const REC_X0: usize = 1;
/// Word index of SP.
pub const REC_SP: usize = 32;
/// Word index of NZCV.
pub const REC_NZCV: usize = 33;
/// Word index of FPSR.
pub const REC_FPSR: usize = 34;
/// Word index of V0 (low half).
pub const REC_V0: usize = 36;
/// Word index of the memory window.
pub const REC_WIN: usize = 100;
/// The memory window is this many bytes, centred on `x5` for memory cases.
pub const WINDOW: usize = 512;

/// A small deterministic PRNG (xorshift64*).
pub struct Rng(pub u64);

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
    pub fn flag(&mut self) -> u32 {
        (self.next() >> 40) as u32 & 1
    }
    pub fn pick<T: Copy>(&mut self, s: &[T]) -> T {
        s[self.below(s.len() as u32) as usize]
    }
    /// Any register 0..=31.
    pub fn reg(&mut self) -> u32 {
        self.below(32)
    }
    /// A register that is not one of `ex`.
    pub fn reg_not(&mut self, ex: &[u32]) -> u32 {
        loop {
            let r = self.reg();
            if !ex.contains(&r) {
                return r;
            }
        }
    }
}

/// One test: setup lines run first (after the register state is loaded),
/// then the body.
pub struct Case {
    pub family: &'static str,
    pub setup: Vec<String>,
    pub body: Vec<String>,
}

fn inst(w: u32) -> String {
    format!(".inst 0x{w:08x}")
}

fn word_case(family: &'static str, w: u32) -> Case {
    Case { family, setup: vec![], body: vec![inst(w)] }
}

/// Lines that point `x5` at the middle of the memory window.
fn base5() -> Vec<String> {
    vec!["adrp x5, scratch".into(), "add x5, x5, :lo12:scratch".into(), "add x5, x5, #2048".into()]
}

fn mem_case(family: &'static str, w: u32, extra: Vec<String>) -> Case {
    let mut setup = base5();
    setup.extend(extra);
    Case { family, setup, body: vec![inst(w)] }
}

// ---------------------------------------------------------------------
// Initial state
// ---------------------------------------------------------------------

/// The register state a block starts from.
pub struct State {
    pub x: [u64; 31],
    pub sp: u64,
    pub nzcv: u64,
    pub fpcr: u64,
    pub v: [u128; 32],
    pub window: [u8; WINDOW],
}

const INTERESTING64: [u64; 24] = [
    0,
    1,
    2,
    3,
    0x7F,
    0x80,
    0xFF,
    0x7FFF,
    0x8000,
    0xFFFF,
    0x7FFF_FFFF,
    0x8000_0000,
    0xFFFF_FFFF,
    0x1_0000_0000,
    0x1_0000_0001,
    0x7FFF_FFFF_FFFF_FFFF,
    0x8000_0000_0000_0000,
    0x8000_0000_0000_0001,
    0xFFFF_FFFF_FFFF_FFFF,
    0xFFFF_FFFF_FFFF_FFFE,
    0x5555_5555_5555_5555,
    0xAAAA_AAAA_AAAA_AAAA,
    0x0123_4567_89AB_CDEF,
    0xFEDC_BA98_7654_3210,
];

const F64S: [u64; 16] = [
    0x0000_0000_0000_0000,
    0x8000_0000_0000_0000,
    0x3FF0_0000_0000_0000, // 1.0
    0xBFF0_0000_0000_0000, // -1.0
    0x4000_0000_0000_0000, // 2.0
    0x3FE0_0000_0000_0000, // 0.5
    0x7FF0_0000_0000_0000, // +inf
    0xFFF0_0000_0000_0000, // -inf
    0x7FF8_0000_0000_0000, // qNaN
    0x7FF0_0000_0000_0001, // sNaN
    0x0000_0000_0000_0001, // smallest denormal
    0x000F_FFFF_FFFF_FFFF, // largest denormal
    0x7FEF_FFFF_FFFF_FFFF, // max
    0x4340_0000_0000_0000, // 2^53
    0x41DF_FFFF_FFC0_0000, // 2147483647.0
    0xC3E0_0000_0000_0000, // -2^63
];

const F32S: [u32; 14] = [
    0x0000_0000,
    0x8000_0000,
    0x3F80_0000,
    0xBF80_0000,
    0x4000_0000,
    0x3F00_0000,
    0x7F80_0000,
    0xFF80_0000,
    0x7FC0_0000,
    0x7F80_0001,
    0x0000_0001,
    0x007F_FFFF,
    0x7F7F_FFFF,
    0x4B00_0000, // 2^23
];

impl State {
    pub fn random(rng: &mut Rng) -> State {
        let mut x = [0u64; 31];
        for r in x.iter_mut() {
            *r = match rng.below(8) {
                0..=2 => rng.pick(&INTERESTING64),
                3 => rng.next(),
                4 => rng.next() & 0xFFFF_FFFF,
                5 => rng.next() & 0xFF,
                6 => rng.next() & 0xF,
                _ => rng.pick(&INTERESTING64) ^ (1 << rng.below(64)),
            };
        }
        let mut v = [0u128; 32];
        for r in v.iter_mut() {
            *r = match rng.below(7) {
                0 => (u128::from(rng.next()) << 64) | u128::from(rng.next()),
                1 => u128::from(rng.pick(&F64S)),
                2 => (u128::from(rng.pick(&F64S)) << 64) | u128::from(rng.pick(&F64S)),
                3 => {
                    let mut s = 0u128;
                    for i in 0..4 {
                        s |= u128::from(rng.pick(&F32S)) << (32 * i);
                    }
                    s
                }
                4 => u128::from(rng.pick(&F32S)),
                5 => {
                    let b = rng.pick(&[0u8, 0xFF, 0x80, 0x7F, 0x01, 0xAA, 0x55]);
                    u128::from_le_bytes([b; 16])
                }
                _ => (u128::from(rng.next()) << 64) | u128::from(rng.next() & 0x00FF_00FF_00FF_00FF),
            };
        }
        let mut window = [0u8; WINDOW];
        for chunk in window.chunks_mut(8) {
            let r = rng.next().to_le_bytes();
            chunk.copy_from_slice(&r);
        }
        State {
            x,
            sp: rng.next() & !0xF,
            nzcv: u64::from(rng.below(16)) << 28,
            fpcr: if rng.below(5) == 0 { 1 << 25 } else { 0 },
            v,
            window,
        }
    }

    fn words(&self) -> Vec<u64> {
        let mut w = Vec::with_capacity(REC_WORDS);
        w.extend_from_slice(&self.x);
        w.push(self.sp);
        w.push(self.nzcv);
        w.push(self.fpcr);
        w.push(0);
        w.push(0);
        for v in &self.v {
            w.push(*v as u64);
            w.push((*v >> 64) as u64);
        }
        for c in self.window.chunks(8) {
            w.push(u64::from_le_bytes(c.try_into().unwrap()));
        }
        assert_eq!(w.len(), REC_WORDS, "init layout");
        w
    }
}

// ---------------------------------------------------------------------
// Program text
// ---------------------------------------------------------------------

/// Emit the assembly of a program running `cases` (each with its state).
pub fn program(cases: &[(Case, State)], tags: &[u32]) -> String {
    let mut s = String::new();
    s.push_str(".text\n.globl _start\n_start:\n");
    for (i, (case, _)) in cases.iter().enumerate() {
        block_asm(&mut s, i, case, tags[i]);
    }
    s.push_str("blk_end:\n    mov x0, #0\n    mov x8, #93\n    svc #0\n");
    s.push_str(".bss\n.balign 64\nscratch: .space 4096\n.balign 64\nout: .space ");
    let _ = writeln!(s, "{REC_BYTES}");
    s.push_str(".data\n");
    for (i, (_, st)) in cases.iter().enumerate() {
        let _ = writeln!(s, ".balign 16\ninit_{i}:");
        for chunk in st.words().chunks(4) {
            let items: Vec<String> = chunk.iter().map(|w| format!("0x{w:x}")).collect();
            let _ = writeln!(s, "    .quad {}", items.join(", "));
        }
    }
    s
}

fn block_asm(s: &mut String, i: usize, case: &Case, tag: u32) {
    let w = |s: &mut String, line: &str| {
        s.push_str("    ");
        s.push_str(line);
        s.push('\n');
    };
    let _ = writeln!(s, "// block {i}: {}", case.family);
    let _ = writeln!(s, "blk_{i}:");
    w(s, &format!("adrp x0, init_{i}"));
    w(s, &format!("add x0, x0, :lo12:init_{i}"));
    // Restore the memory window from the init data.
    w(s, "adrp x1, scratch");
    w(s, "add x1, x1, :lo12:scratch");
    w(s, "add x1, x1, #1792");
    w(s, "add x2, x0, #800");
    for k in 0..16 {
        w(s, &format!("ldp q0, q1, [x2, #{}]", 32 * k));
        w(s, &format!("stp q0, q1, [x1, #{}]", 32 * k));
    }
    w(s, "add x2, x0, #288");
    for k in 0..16 {
        w(s, &format!("ldp q{}, q{}, [x2, #{}]", 2 * k, 2 * k + 1, 32 * k));
    }
    w(s, "ldr x2, [x0, #248]");
    w(s, "mov sp, x2");
    w(s, "ldr x2, [x0, #256]");
    w(s, "msr nzcv, x2");
    w(s, "ldr x2, [x0, #264]");
    w(s, "msr fpcr, x2");
    w(s, "msr fpsr, xzr");
    for k in 0..15 {
        let (a, b) = (1 + 2 * k, 2 + 2 * k);
        w(s, &format!("ldp x{a}, x{b}, [x0, #{}]", 8 * a));
    }
    w(s, "ldr x0, [x0]");
    for l in &case.setup {
        w(s, l);
    }
    for l in &case.body {
        w(s, l);
    }
    // Dump.
    w(s, "msr tpidr_el0, x0");
    w(s, "adrp x0, out");
    w(s, "add x0, x0, :lo12:out");
    for k in 0..15 {
        let (a, b) = (1 + 2 * k, 2 + 2 * k);
        w(s, &format!("stp x{a}, x{b}, [x0, #{}]", 8 + 8 * a));
    }
    w(s, "mov x1, sp");
    w(s, &format!("str x1, [x0, #{}]", 8 * REC_SP));
    w(s, "mrs x1, nzcv");
    w(s, &format!("str x1, [x0, #{}]", 8 * REC_NZCV));
    w(s, "mrs x1, fpsr");
    w(s, &format!("str x1, [x0, #{}]", 8 * REC_FPSR));
    w(s, &format!("add x1, x0, #{}", 8 * REC_V0));
    for k in 0..16 {
        w(s, &format!("stp q{}, q{}, [x1, #{}]", 2 * k, 2 * k + 1, 32 * k));
    }
    w(s, "adrp x2, scratch");
    w(s, "add x2, x2, :lo12:scratch");
    w(s, "add x2, x2, #1792");
    w(s, &format!("add x1, x0, #{}", 8 * REC_WIN));
    for k in 0..16 {
        w(s, &format!("ldp q0, q1, [x2, #{}]", 32 * k));
        w(s, &format!("stp q0, q1, [x1, #{}]", 32 * k));
    }
    w(s, "mrs x1, tpidr_el0");
    w(s, &format!("str x1, [x0, #{}]", 8 * REC_X0));
    w(s, &format!("movz x1, #0x{:x}", tag & 0xFFFF));
    w(s, &format!("movk x1, #0x{:x}, lsl #16", tag >> 16));
    w(s, "str x1, [x0]");
    w(s, "mov x1, x0");
    w(s, "mov x0, #1");
    w(s, &format!("mov x2, #{REC_BYTES}"));
    w(s, "mov x8, #64");
    w(s, "svc #0");
}

// ---------------------------------------------------------------------
// Families
// ---------------------------------------------------------------------

/// A family: a name and a case generator.
pub struct Family {
    pub name: &'static str,
    pub make: fn(&mut Rng) -> Case,
}

macro_rules! fams {
    ($($name:literal => $f:expr,)*) => {
        pub const FAMILIES: &[Family] = &[$(Family { name: $name, make: $f }),*];
    };
}

fn sf_mask(rng: &mut Rng) -> u32 {
    rng.flag() << 31
}

/// Encode a random bitmask immediate: returns `(N, immr, imms)`.
fn bitmask_imm(rng: &mut Rng, sf: bool) -> (u32, u32, u32) {
    let max_log = if sf { 6 } else { 5 };
    let log = 1 + rng.below(max_log); // esize 2..=64
    let esize = 1u32 << log;
    let ones = 1 + rng.below(esize - 1);
    let rot = rng.below(esize);
    let n = u32::from(esize == 64);
    let imms = if esize == 64 { ones - 1 } else { ((!(esize * 2 - 1)) & 0x3F) | (ones - 1) };
    (n, rot, imms)
}

fn add_sub_imm(rng: &mut Rng) -> Case {
    let w = sf_mask(rng)
        | rng.flag() << 30
        | rng.flag() << 29
        | 0b100010 << 23
        | rng.flag() << 22
        | [rng.below(4096), rng.below(16), 0xFFF, 0, 1][rng.below(5) as usize] << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("add-sub-imm", w)
}

fn add_sub_shifted(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let amount = if sf == 1 { rng.below(64) } else { rng.below(32) };
    let w = sf << 31
        | rng.flag() << 30
        | rng.flag() << 29
        | 0b01011 << 24
        | rng.below(3) << 22
        | rng.reg() << 16
        | amount << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("add-sub-shifted", w)
}

fn add_sub_ext(rng: &mut Rng) -> Case {
    let w = sf_mask(rng)
        | rng.flag() << 30
        | rng.flag() << 29
        | 0b01011 << 24
        | 1 << 21
        | rng.reg() << 16
        | rng.below(8) << 13
        | rng.below(5) << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("add-sub-ext", w)
}

fn add_sub_carry(rng: &mut Rng) -> Case {
    let w = sf_mask(rng)
        | rng.flag() << 30
        | rng.flag() << 29
        | 0b11010000 << 21
        | rng.reg() << 16
        | rng.reg() << 5
        | rng.reg();
    word_case("add-sub-carry", w)
}

fn logical_imm(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let (n, immr, imms) = bitmask_imm(rng, sf == 1);
    let w =
        sf << 31 | rng.below(4) << 29 | 0b100100 << 23 | n << 22 | immr << 16 | imms << 10 | rng.reg() << 5 | rng.reg();
    word_case("logical-imm", w)
}

fn logical_shifted(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let amount = if sf == 1 { rng.below(64) } else { rng.below(32) };
    let w = sf << 31
        | rng.below(4) << 29
        | 0b01010 << 24
        | rng.below(4) << 22
        | rng.flag() << 21
        | rng.reg() << 16
        | amount << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("logical-shifted", w)
}

fn move_wide(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let hw = if sf == 1 { rng.below(4) } else { rng.below(2) };
    let opc = rng.pick(&[0u32, 2, 3]);
    let w = sf << 31 | opc << 29 | 0b100101 << 23 | hw << 21 | (rng.next() as u32 & 0xFFFF) << 5 | rng.reg();
    word_case("move-wide", w)
}

fn adr(rng: &mut Rng) -> Case {
    let w = rng.flag() << 31 | rng.below(4) << 29 | 0b10000 << 24 | (rng.next() as u32 & 0x7FFFF) << 5 | rng.reg();
    word_case("adr-adrp", w)
}

fn bitfield(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let lim = if sf == 1 { 64 } else { 32 };
    let opc = rng.below(3);
    let w = sf << 31
        | opc << 29
        | 0b100110 << 23
        | sf << 22
        | rng.below(lim) << 16
        | rng.below(lim) << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("bitfield", w)
}

fn extr(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let lim = if sf == 1 { 64 } else { 32 };
    let w = sf << 31 | 0b100111 << 23 | sf << 22 | rng.reg() << 16 | rng.below(lim) << 10 | rng.reg() << 5 | rng.reg();
    word_case("extr", w)
}

fn cond_cmp(rng: &mut Rng) -> Case {
    let w = sf_mask(rng)
        | rng.flag() << 30
        | 1 << 29
        | 0b11010010 << 21
        | rng.reg() << 16
        | rng.below(16) << 12
        | rng.flag() << 11
        | rng.reg() << 5
        | rng.below(16);
    word_case("cond-cmp", w)
}

fn cond_sel(rng: &mut Rng) -> Case {
    let w = sf_mask(rng)
        | rng.flag() << 30
        | 0b11010100 << 21
        | rng.reg() << 16
        | rng.below(16) << 12
        | rng.flag() << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("cond-sel", w)
}

fn dp3(rng: &mut Rng) -> Case {
    // (op31, o0 options, needs sf = 1)
    let (op31, o0, sf1): (u32, u32, bool) = match rng.below(6) {
        0 => (0, rng.flag(), false),
        1 => (1, rng.flag(), true),
        2 => (5, rng.flag(), true),
        3 => (2, 0, true),
        4 => (6, 0, true),
        _ => (0, rng.flag(), false),
    };
    let sf = if sf1 { 1 } else { rng.flag() };
    let ra = if op31 == 2 || op31 == 6 { 31 } else { rng.reg() };
    let w = sf << 31 | 0b11011 << 24 | op31 << 21 | rng.reg() << 16 | o0 << 15 | ra << 10 | rng.reg() << 5 | rng.reg();
    word_case("mul-div3", w)
}

fn dp2(rng: &mut Rng) -> Case {
    let opc = rng.pick(&[2u32, 3, 8, 9, 10, 11]);
    let w = sf_mask(rng) | 0b11010110 << 21 | rng.reg() << 16 | opc << 10 | rng.reg() << 5 | rng.reg();
    word_case("div-shift-var", w)
}

fn dp1(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let opc = if sf == 1 { rng.below(6) } else { rng.pick(&[0u32, 1, 2, 4, 5]) };
    let w = sf << 31 | 1 << 30 | 0b11010110 << 21 | opc << 10 | rng.reg() << 5 | rng.reg();
    word_case("bit-rev-clz", w)
}

// ---- branches (text) ----

fn cond_name(c: u32) -> &'static str {
    ["eq", "ne", "cs", "cc", "mi", "pl", "vs", "vc", "hi", "ls", "ge", "lt", "gt", "le", "al", "al"][c as usize]
}

fn skeleton(branch: String) -> Vec<String> {
    vec![branch, "mov x7, #1".into(), "b 2f".into(), "1: mov x7, #2".into(), "2:".into()]
}

fn branch_cond(rng: &mut Rng) -> Case {
    let c = rng.below(15);
    Case { family: "b.cond", setup: vec![], body: skeleton(format!("b.{} 1f", cond_name(c))) }
}

fn cbz(rng: &mut Rng) -> Case {
    let op = if rng.flag() == 1 { "cbz" } else { "cbnz" };
    let r = if rng.flag() == 1 { format!("x{}", rng.below(31)) } else { format!("w{}", rng.below(31)) };
    Case { family: "cbz", setup: vec![], body: skeleton(format!("{op} {r}, 1f")) }
}

fn tbz(rng: &mut Rng) -> Case {
    let op = if rng.flag() == 1 { "tbz" } else { "tbnz" };
    let bit = rng.below(64);
    let r = if bit < 32 && rng.flag() == 1 { format!("w{}", rng.below(31)) } else { format!("x{}", rng.below(31)) };
    Case { family: "tbz", setup: vec![], body: skeleton(format!("{op} {r}, #{bit}, 1f")) }
}

fn branch_link(rng: &mut Rng) -> Case {
    let body: Vec<String> = match rng.below(4) {
        0 => vec!["bl 1f", "b 2f", "1: mov x7, #5", "ret", "2:"],
        1 => vec!["adr x9, 1f", "br x9", "mov x7, #1", "1: mov x7, #3"],
        2 => vec!["adr x9, 1f", "blr x9", "b 2f", "1: mov x7, #4", "ret", "2:"],
        _ => vec!["adr x9, 1f", "mov x10, x30", "mov x30, x9", "ret", "mov x7, #1", "1: mov x30, x10"],
    }
    .into_iter()
    .map(String::from)
    .collect();
    Case { family: "branch-link", setup: vec![], body }
}

// ---- system ----

fn system(rng: &mut Rng) -> Case {
    let regs = ["nzcv", "fpcr", "fpsr"];
    let body: Vec<String> = match rng.below(9) {
        0 => vec![format!("mrs x{}, nzcv", rng.below(31))],
        1 => vec![format!("msr nzcv, x{}", rng.below(31))],
        2 => vec![format!("mrs x{}, fpsr", rng.below(31))],
        3 => vec![format!("msr fpcr, x{}", rng.below(31))],
        4 => vec![format!("mrs x{}, {}", rng.below(31), regs[rng.below(3) as usize])],
        5 => {
            // Hints that behave the same under qemu without pointer authentication keys: NOP,
            // YIELD, XPACLRI, PACIA1716, PACIB1716, AUTIA1716, PACIASP, PACIBSP, AUTIASP, BTI.
            let h = rng.pick(&[0u32, 1, 7, 8, 10, 12, 0x19, 0x1B, 0x1D, 0x22, 0x24, 0x26]);
            vec![inst(0xD503_201F | h << 5)]
        }
        6 => vec![["dmb ish", "dsb sy", "isb", "clrex", "dmb ld", "dsb st"][rng.below(6) as usize].into()],
        7 => vec![format!("mrs x{}, dczid_el0", rng.below(31))],
        _ => vec![format!("mrs x{}, tpidrro_el0", rng.below(31))],
    };
    Case { family: "system", setup: vec![], body }
}

// ---- loads and stores ----

/// Register that is not the base (`x5`) or the offset register (`x6`).
fn rt_reg(rng: &mut Rng) -> u32 {
    rng.reg_not(&[5, 6])
}

fn ls_imm_common(rng: &mut Rng, v: bool) -> (u32, u32) {
    // (size, opc)
    if !v {
        rng.pick(&[
            (0, 0),
            (0, 1),
            (0, 2),
            (0, 3),
            (1, 0),
            (1, 1),
            (1, 2),
            (1, 3),
            (2, 0),
            (2, 1),
            (2, 2),
            (3, 0),
            (3, 1),
            (3, 0),
            (3, 1),
        ])
    } else {
        rng.pick(&[(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (2, 1), (3, 0), (3, 1), (0, 2), (0, 3)])
    }
}

fn log2_size(size: u32, opc: u32, v: bool) -> u32 {
    if v && size == 0 && opc >= 2 { 4 } else { size }
}

fn ls_uimm(rng: &mut Rng) -> Case {
    let v = rng.flag() == 1;
    let (size, opc) = ls_imm_common(rng, v);
    let sz = log2_size(size, opc, v);
    let imm12 = rng.below(224 >> sz);
    let w = size << 30 | 0b111 << 27 | u32::from(v) << 26 | 0b01 << 24 | opc << 22 | imm12 << 10 | 5 << 5 | rt_reg(rng);
    mem_case("ls-uimm", w, vec![])
}

fn ls_imm9(rng: &mut Rng) -> Case {
    let v = rng.flag() == 1;
    let (size, opc) = ls_imm_common(rng, v);
    let mode = rng.pick(&[0u32, 1, 3]);
    let imm9 = (rng.below(225) as i32 - 112) as u32 & 0x1FF;
    let w = size << 30 | 0b111 << 27 | u32::from(v) << 26 | opc << 22 | imm9 << 12 | mode << 10 | 5 << 5 | rt_reg(rng);
    // PRFM with pre/post index is not allocated; turn it into a plain load.
    let w = if !v && size == 3 && opc == 2 && mode != 0 { w & !(1 << 23) | 1 << 22 } else { w };
    let name = ["ls-unscaled", "ls-post", "", "ls-pre"][mode as usize];
    mem_case(name, w, vec![])
}

fn ls_unpriv(rng: &mut Rng) -> Case {
    let (size, opc) = rng.pick(&[
        (0, 0),
        (0, 1),
        (0, 2),
        (0, 3),
        (1, 0),
        (1, 1),
        (1, 2),
        (1, 3),
        (2, 0),
        (2, 1),
        (2, 2),
        (3, 0),
        (3, 1),
    ]);
    let imm9 = (rng.below(225) as i32 - 112) as u32 & 0x1FF;
    let w = size << 30 | 0b111 << 27 | opc << 22 | imm9 << 12 | 0b10 << 10 | 5 << 5 | rt_reg(rng);
    mem_case("ls-unpriv", w, vec![])
}

fn ls_reg(rng: &mut Rng) -> Case {
    let v = rng.flag() == 1;
    let (size, opc) = ls_imm_common(rng, v);
    let option = rng.pick(&[2u32, 3, 6, 7]);
    let w = size << 30
        | 0b111 << 27
        | u32::from(v) << 26
        | opc << 22
        | 1 << 21
        | 6 << 16
        | option << 13
        | rng.flag() << 12
        | 0b10 << 10
        | 5 << 5
        | rt_reg(rng);
    mem_case("ls-reg", w, vec![format!("mov x6, #{}", rng.below(16))])
}

fn ls_pair(rng: &mut Rng) -> Case {
    let v = rng.flag() == 1;
    let (opc, l): (u32, u32) =
        if v { (rng.below(3), rng.flag()) } else { rng.pick(&[(0, 0), (0, 1), (2, 0), (2, 1), (1, 1)]) };
    let bytes: i32 = if v {
        4 << opc
    } else if opc == 2 {
        8
    } else {
        4
    };
    let mut mode = rng.below(4); // no-alloc, post, offset, pre
    if opc == 1 && !v && mode == 0 {
        mode = 2; // LDPSW has no non-temporal form
    }
    let lim = 112 / bytes;
    let imm7 = (rng.below((2 * lim + 1) as u32) as i32 - lim) as u32 & 0x7F;
    let rt = rt_reg(rng);
    let mut rt2 = rt_reg(rng);
    if l == 1 {
        while rt2 == rt {
            rt2 = rt_reg(rng);
        }
    }
    let w = opc << 30 | 0b101 << 27 | u32::from(v) << 26 | mode << 23 | l << 22 | imm7 << 15 | rt2 << 10 | 5 << 5 | rt;
    mem_case("ls-pair", w, vec![])
}

fn ls_literal(rng: &mut Rng) -> Case {
    let (mn, bytes) = match rng.below(9) {
        0 => ("ldr w", 4),
        1 => ("ldr x", 8),
        2 => ("ldrsw x", 4),
        3 => ("ldr s", 4),
        4 => ("ldr d", 8),
        5 => ("ldr q", 16),
        6 => ("prfm pldl1keep, ", 8),
        7 => ("ldr w", 4),
        _ => ("ldr x", 8),
    };
    let rt = rng.below(31);
    let first = if mn.starts_with("prfm") { format!("{mn}1f") } else { format!("{mn}{rt}, 1f") };
    let mut body = vec![first, "b 2f".into(), ".balign 16".into(), "1:".into()];
    body.push(format!(".quad 0x{:x}", rng.next()));
    body.push(format!(".quad 0x{:x}", rng.next()));
    body.push("2:".into());
    let _ = bytes;
    Case { family: "ls-literal", setup: vec![], body }
}

fn excl(rng: &mut Rng) -> Case {
    let sz = rng.below(4);
    let (ld, st, r1, r2) = match sz {
        0 => (["ldxrb", "ldaxrb"][rng.below(2) as usize], ["stxrb", "stlxrb"][rng.below(2) as usize], "w", "w"),
        1 => (["ldxrh", "ldaxrh"][rng.below(2) as usize], ["stxrh", "stlxrh"][rng.below(2) as usize], "w", "w"),
        2 => (["ldxr", "ldaxr"][rng.below(2) as usize], ["stxr", "stlxr"][rng.below(2) as usize], "w", "w"),
        _ => (["ldxr", "ldaxr"][rng.below(2) as usize], ["stxr", "stlxr"][rng.below(2) as usize], "x", "x"),
    };
    let mut body = vec![format!("{ld} {r1}1, [x5]")];
    match rng.below(4) {
        0 => body.push("clrex".into()),
        1 => body.push("str x9, [x5]".into()),
        _ => {}
    }
    if rng.below(5) == 0 {
        // A store-exclusive with no load before it fails.
        body.remove(0);
    }
    body.push(format!("{st} w2, {r2}3, [x5]"));
    if rng.below(3) == 0 {
        body.push(format!("{st} w4, {r2}3, [x5]"));
    }
    Case { family: "exclusive", setup: base5(), body }
}

fn excl_pair(rng: &mut Rng) -> Case {
    let x = rng.flag() == 1;
    let r = if x { "x" } else { "w" };
    let (ld, st) = (["ldxp", "ldaxp"][rng.below(2) as usize], ["stxp", "stlxp"][rng.below(2) as usize]);
    let mut body = vec![format!("{ld} {r}1, {r}6, [x5]")];
    if rng.below(4) == 0 {
        body.push("clrex".into());
    }
    body.push(format!("{st} w2, {r}3, {r}4, [x5]"));
    Case { family: "exclusive-pair", setup: base5(), body }
}

fn ordered(rng: &mut Rng) -> Case {
    let size = rng.below(4);
    let o2 = 1u32;
    let l = rng.flag();
    let o0 = rng.flag(); // 0 = LORegion forms (LDLAR/STLLR), 1 = LDAR/STLR
    let w =
        size << 30 | 0b001000 << 24 | o2 << 23 | l << 22 | 0x1F << 16 | o0 << 15 | 0x1F << 10 | 5 << 5 | rt_reg(rng);
    mem_case("ordered", w, vec![])
}

fn atomic_rmw(rng: &mut Rng) -> Case {
    let (o3, opc) = if rng.below(8) == 0 { (1, 0) } else { (0, rng.below(8)) };
    let w = rng.below(4) << 30
        | 0b111 << 27
        | rng.flag() << 23
        | rng.flag() << 22
        | 1 << 21
        | rng.reg_not(&[5]) << 16
        | o3 << 15
        | opc << 12
        | 5 << 5
        | rng.reg_not(&[5]);
    mem_case("lse-atomic", w, vec![])
}

fn cas(rng: &mut Rng) -> Case {
    let rs = rng.reg_not(&[5]);
    let rt = rng.reg_not(&[5]);
    let w = rng.below(4) << 30
        | 0b001000 << 24
        | 1 << 23
        | rng.flag() << 22
        | 1 << 21
        | rs << 16
        | rng.flag() << 15
        | 0x1F << 10
        | 5 << 5
        | rt;
    let extra = if rs != 31 && rng.flag() == 1 { vec![format!("ldr x{rs}, [x5]")] } else { vec![] };
    mem_case("cas", w, extra)
}

fn casp(rng: &mut Rng) -> Case {
    let mut rs = rng.below(15) * 2;
    if rs == 4 {
        rs = 6; // the setup load would overwrite the base register x5
    }
    let rt = rng.below(15) * 2;
    let sz = rng.flag();
    let w =
        sz << 30 | 0b001000 << 24 | rng.flag() << 22 | 1 << 21 | rs << 16 | rng.flag() << 15 | 0x1F << 10 | 5 << 5 | rt;
    let r = if sz == 1 { "x" } else { "w" };
    let extra = if rs < 28 && rng.flag() == 1 { vec![format!("ldp {r}{rs}, {r}{}, [x5]", rs + 1)] } else { vec![] };
    mem_case("casp", w, extra)
}

fn dc_zva(rng: &mut Rng) -> Case {
    let _ = rng;
    Case { family: "dc-zva", setup: base5(), body: vec!["add x5, x5, #17".into(), "dc zva, x5".into()] }
}

// ---- scalar floating point ----

fn fp_type(rng: &mut Rng) -> u32 {
    rng.flag()
}

fn fp_2src(rng: &mut Rng) -> Case {
    let w = 0x1E20_0800 | fp_type(rng) << 22 | rng.reg() << 16 | rng.below(9) << 12 | rng.reg() << 5 | rng.reg();
    word_case("fp-2src", w)
}

fn fp_1src(rng: &mut Rng) -> Case {
    let ft = fp_type(rng);
    let opc = match rng.below(10) {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => {
            if ft == 0 {
                5
            } else {
                4
            }
        }
        _ => rng.pick(&[8u32, 9, 10, 11, 12, 14, 15]),
    };
    let w = 0x1E20_4000 | ft << 22 | opc << 15 | rng.reg() << 5 | rng.reg();
    word_case("fp-1src", w)
}

fn fp_3src(rng: &mut Rng) -> Case {
    let w = 0x1F00_0000
        | fp_type(rng) << 22
        | rng.flag() << 21
        | rng.reg() << 16
        | rng.flag() << 15
        | rng.reg() << 10
        | rng.reg() << 5
        | rng.reg();
    word_case("fp-3src", w)
}

fn fp_cmp(rng: &mut Rng) -> Case {
    let opc2 = rng.pick(&[0u32, 8, 16, 24]);
    let rm = if opc2 & 8 != 0 { 0 } else { rng.reg() };
    let w = 0x1E20_2000 | fp_type(rng) << 22 | rm << 16 | rng.reg() << 5 | opc2;
    word_case("fcmp", w)
}

fn fp_ccmp(rng: &mut Rng) -> Case {
    let w = 0x1E20_0400
        | fp_type(rng) << 22
        | rng.reg() << 16
        | rng.below(16) << 12
        | rng.reg() << 5
        | rng.flag() << 4
        | rng.below(16);
    word_case("fccmp", w)
}

fn fp_csel(rng: &mut Rng) -> Case {
    let w = 0x1E20_0C00 | fp_type(rng) << 22 | rng.reg() << 16 | rng.below(16) << 12 | rng.reg() << 5 | rng.reg();
    word_case("fcsel", w)
}

fn fp_imm(rng: &mut Rng) -> Case {
    let w = 0x1E20_1000 | fp_type(rng) << 22 | (rng.next() as u32 & 0xFF) << 13 | rng.reg();
    word_case("fmov-imm", w)
}

fn fp_int(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let ft = fp_type(rng);
    let (rmode, opc, sf, ft) = match rng.below(14) {
        0 => (0, 0, sf, ft),
        1 => (0, 1, sf, ft),
        2 => (0, 4, sf, ft),
        3 => (0, 5, sf, ft),
        4 => (1, rng.flag(), sf, ft),
        5 => (2, rng.flag(), sf, ft),
        6 | 7 => (3, rng.flag(), sf, ft),
        8 | 9 => (0, 2 + rng.flag(), sf, ft),
        10 => (0, 6 + rng.flag(), ft, ft),
        11 => (1, 6 + rng.flag(), 1, 2),
        _ => (3, rng.flag(), sf, ft),
    };
    let w = 0x1E20_0000 | sf << 31 | ft << 22 | rmode << 19 | opc << 16 | rng.reg() << 5 | rng.reg();
    word_case("fp-int", w)
}

fn fp_fixed(rng: &mut Rng) -> Case {
    let sf = rng.flag();
    let ft = fp_type(rng);
    let (rmode, opc) = rng.pick(&[(0u32, 2u32), (0, 3), (3, 0), (3, 1)]);
    let scale = if sf == 1 { rng.below(64) } else { 32 + rng.below(32) };
    let w = 0x1E00_0000 | sf << 31 | ft << 22 | rmode << 19 | opc << 16 | scale << 10 | rng.reg() << 5 | rng.reg();
    word_case("fp-fixed", w)
}

// ---- Advanced SIMD ----

fn q_size(rng: &mut Rng, max_size: u32) -> (u32, u32) {
    let q = rng.flag();
    let mut size = rng.below(max_size + 1);
    if size == 3 && q == 0 {
        size = rng.below(3);
    }
    (q, size)
}

fn v3same(rng: &mut Rng) -> Case {
    // (U, opcode, max size)
    let (u, opc, maxs) = rng.pick(&[
        (0u32, 0x10u32, 3u32),
        (1, 0x10, 3),
        (1, 0x11, 3),
        (0, 0x11, 3),
        (0, 0x06, 3),
        (0, 0x07, 3),
        (1, 0x06, 3),
        (1, 0x07, 3),
        (0, 0x13, 2),
        (0, 0x12, 2),
        (1, 0x12, 2),
        (0, 0x0C, 2),
        (0, 0x0D, 2),
        (1, 0x0C, 2),
        (1, 0x0D, 2),
        (0, 0x17, 3),
        (0, 0x08, 3),
        (1, 0x08, 3),
        (0, 0x03, 3),
        (1, 0x03, 3),
        (0, 0x03, 3),
        (1, 0x03, 3),
    ]);
    let (q, size) = if opc == 0x03 { (rng.flag(), rng.below(4)) } else { q_size(rng, maxs) };
    let w = 0x0E20_0400 | q << 30 | u << 29 | size << 22 | rng.reg() << 16 | opc << 11 | rng.reg() << 5 | rng.reg();
    word_case("v-3same", w)
}

fn v2misc(rng: &mut Rng) -> Case {
    // (U, opcode, sizes)
    let (u, opc, maxs, fixed): (u32, u32, u32, Option<u32>) = rng.pick(&[
        (0, 0x00, 2, None),
        (0, 0x01, 0, Some(0)),
        (1, 0x00, 1, None),
        (0, 0x04, 2, None),
        (1, 0x04, 2, None),
        (0, 0x05, 0, Some(0)),
        (1, 0x05, 0, Some(0)),
        (1, 0x05, 1, Some(1)),
        (0, 0x08, 3, None),
        (0, 0x09, 3, None),
        (0, 0x0A, 3, None),
        (0, 0x0B, 3, None),
        (1, 0x08, 3, None),
        (1, 0x09, 3, None),
        (1, 0x0B, 3, None),
        (0, 0x12, 2, None),
    ]);
    let (q, mut size) = q_size(rng, maxs);
    if let Some(f) = fixed {
        size = f;
    }
    let w = 0x0E20_0800 | q << 30 | u << 29 | size << 22 | opc << 12 | rng.reg() << 5 | rng.reg();
    word_case("v-2misc", w)
}

fn v_across(rng: &mut Rng) -> Case {
    let (u, opc) = rng.pick(&[(0u32, 3u32), (1, 3), (0, 0x0A), (1, 0x0A), (0, 0x1A), (1, 0x1A), (0, 0x1B)]);
    let q = rng.flag();
    let size = if q == 1 { rng.below(3) } else { rng.below(2) };
    let w = 0x0E30_0800 | q << 30 | u << 29 | size << 22 | opc << 12 | rng.reg() << 5 | rng.reg();
    word_case("v-across", w)
}

fn v_copy(rng: &mut Rng) -> Case {
    let kind = rng.below(6);
    let size = rng.below(4);
    let lanes = 16 >> size;
    let index = rng.below(lanes);
    let imm5 = index << (size + 1) | 1 << size;
    let (q, op, imm4) = match kind {
        0 => (if size == 3 { 1 } else { rng.flag() }, 0, 0),
        1 => (if size == 3 { 1 } else { rng.flag() }, 0, 1),
        2 => (1, 0, 3),
        3 => {
            // SMOV: B/H to W or X, S to X.
            let q = if size == 2 { 1 } else { rng.flag() };
            (q, 0, 5)
        }
        4 => (u32::from(size == 3), 0, 7),
        _ => (1, 1, rng.below(lanes) << size),
    };
    let size_ok = !(kind == 3 && size == 3);
    let (imm5, q, op, imm4) = if size_ok { (imm5, q, op, imm4) } else { (1 << 1 | 1, 0, 0, 5) };
    let w = 0x0E00_0400 | q << 30 | op << 29 | imm5 << 16 | imm4 << 11 | rng.reg() << 5 | rng.reg();
    word_case("v-copy", w)
}

fn v_perm(rng: &mut Rng) -> Case {
    let (q, size) = q_size(rng, 3);
    let opc = rng.pick(&[1u32, 2, 3, 5, 6, 7]);
    let w = 0x0E00_0800 | q << 30 | size << 22 | rng.reg() << 16 | opc << 12 | rng.reg() << 5 | rng.reg();
    word_case("v-perm", w)
}

fn v_ext(rng: &mut Rng) -> Case {
    let q = rng.flag();
    let imm4 = if q == 1 { rng.below(16) } else { rng.below(8) };
    let w = 0x2E00_0000 | q << 30 | rng.reg() << 16 | imm4 << 11 | rng.reg() << 5 | rng.reg();
    word_case("v-ext", w)
}

fn v_tbl(rng: &mut Rng) -> Case {
    let w = 0x0E00_0000
        | rng.flag() << 30
        | rng.reg() << 16
        | rng.below(4) << 13
        | rng.flag() << 12
        | rng.reg() << 5
        | rng.reg();
    // Index bytes come from Vm; make them mostly in range.
    word_case("v-tbl", w)
}

fn v_modimm(rng: &mut Rng) -> Case {
    let q = rng.flag();
    let op = rng.flag();
    let mut cmode = rng.below(16);
    if op == 1 && cmode == 15 && q == 0 {
        cmode = 14;
    }
    let imm8 = rng.next() as u32 & 0xFF;
    let w = 0x0F00_0400 | q << 30 | op << 29 | (imm8 >> 5) << 16 | cmode << 12 | (imm8 & 0x1F) << 5 | rng.reg();
    word_case("v-modimm", w)
}

fn v_shift(rng: &mut Rng) -> Case {
    // (U, opcode, kind) kind 0 = same size, 1 = narrow/widen (immh < 8)
    let (u, opc, kind) = rng.pick(&[
        (0u32, 0x00u32, 0u32),
        (1, 0x00, 0),
        (0, 0x02, 0),
        (1, 0x02, 0),
        (0, 0x0A, 0),
        (1, 0x0A, 0),
        (1, 0x08, 0),
        (0, 0x10, 1),
        (0, 0x14, 1),
        (1, 0x14, 1),
    ]);
    let q = rng.flag();
    let immh = if kind == 1 || q == 0 { 1 + rng.below(7) } else { 1 + rng.below(15) };
    let w = 0x0F00_0400 | q << 30 | u << 29 | immh << 19 | rng.below(8) << 16 | opc << 11 | rng.reg() << 5 | rng.reg();
    word_case("v-shift", w)
}

fn s_simd(rng: &mut Rng) -> Case {
    let w = match rng.below(4) {
        0 => {
            let (u, opc) =
                rng.pick(&[(0u32, 0x10u32), (1, 0x10), (1, 0x11), (0, 0x11), (0, 6), (0, 7), (1, 6), (1, 7)]);
            0x5E20_0400 | u << 29 | 3 << 22 | rng.reg() << 16 | opc << 11 | rng.reg() << 5 | rng.reg()
        }
        1 => {
            let (u, opc) = rng.pick(&[(0u32, 8u32), (0, 9), (0, 10), (0, 11), (1, 8), (1, 9), (1, 11)]);
            0x5E20_0800 | u << 29 | 3 << 22 | opc << 12 | rng.reg() << 5 | rng.reg()
        }
        2 => {
            let size = rng.below(4);
            let imm5 = rng.below(16 >> size) << (size + 1) | 1 << size;
            0x5E00_0400 | imm5 << 16 | rng.reg() << 5 | rng.reg()
        }
        _ => {
            let (u, opc) = rng.pick(&[(0u32, 0u32), (1, 0), (0, 2), (1, 2), (0, 0x0A), (1, 0x0A), (1, 8)]);
            0x5F00_0400
                | u << 29
                | (8 + rng.below(8)) << 19
                | rng.below(8) << 16
                | opc << 11
                | rng.reg() << 5
                | rng.reg()
        }
    };
    word_case("s-simd", w)
}

fn v_ldst_multi(rng: &mut Rng) -> Case {
    let q = rng.flag();
    let opc = rng.pick(&[0u32, 2, 4, 6, 7, 8, 10]);
    let interleave = matches!(opc, 0 | 4 | 8);
    let mut size = rng.below(4);
    if interleave && size == 3 && q == 0 {
        size = 2;
    }
    let post = rng.below(3); // none, imm, reg
    let l = rng.flag();
    let mut w = q << 30 | 0x0C00_0000 | l << 22 | opc << 12 | size << 10 | 5 << 5 | rng.reg();
    let mut extra = vec![];
    match post {
        1 => w |= 1 << 23 | 31 << 16,
        2 => {
            w |= 1 << 23 | 6 << 16;
            extra.push(format!("mov x6, #{}", rng.below(16)));
        }
        _ => {}
    }
    mem_case("v-ldst-multi", w, extra)
}

fn v_ldst_single(rng: &mut Rng) -> Case {
    let q = rng.flag();
    let l = rng.flag();
    let r = rng.flag();
    let o0 = rng.flag();
    let scale = rng.below(4);
    let (opcode, s, size) = match scale {
        0 => (o0, rng.flag(), rng.below(4)),
        1 => (1 << 1 | o0, rng.flag(), rng.below(2) << 1),
        2 => {
            if rng.flag() == 1 {
                (2 << 1 | o0, rng.flag(), 0)
            } else {
                (2 << 1 | o0, 0, 1)
            }
        }
        _ => (3 << 1 | o0, 0, rng.below(4)),
    };
    let l = if scale == 3 { 1 } else { l };
    let post = rng.below(3);
    let mut w = q << 30 | 0x0D00_0000 | l << 22 | r << 21 | opcode << 13 | s << 12 | size << 10 | 5 << 5 | rng.reg();
    let mut extra = vec![];
    match post {
        1 => w |= 1 << 23 | 31 << 16,
        2 => {
            w |= 1 << 23 | 6 << 16;
            extra.push(format!("mov x6, #{}", rng.below(16)));
        }
        _ => {}
    }
    mem_case("v-ldst-single", w, extra)
}

fams! {
    "add-sub-imm" => add_sub_imm,
    "add-sub-shifted" => add_sub_shifted,
    "add-sub-ext" => add_sub_ext,
    "add-sub-carry" => add_sub_carry,
    "logical-imm" => logical_imm,
    "logical-shifted" => logical_shifted,
    "move-wide" => move_wide,
    "adr-adrp" => adr,
    "bitfield" => bitfield,
    "extr" => extr,
    "cond-cmp" => cond_cmp,
    "cond-sel" => cond_sel,
    "mul-div3" => dp3,
    "div-shift-var" => dp2,
    "bit-rev-clz" => dp1,
    "b.cond" => branch_cond,
    "cbz" => cbz,
    "tbz" => tbz,
    "branch-link" => branch_link,
    "system" => system,
    "ls-uimm" => ls_uimm,
    "ls-imm9" => ls_imm9,
    "ls-unpriv" => ls_unpriv,
    "ls-reg" => ls_reg,
    "ls-pair" => ls_pair,
    "ls-literal" => ls_literal,
    "exclusive" => excl,
    "exclusive-pair" => excl_pair,
    "ordered" => ordered,
    "lse-atomic" => atomic_rmw,
    "cas" => cas,
    "casp" => casp,
    "dc-zva" => dc_zva,
    "fp-2src" => fp_2src,
    "fp-1src" => fp_1src,
    "fp-3src" => fp_3src,
    "fcmp" => fp_cmp,
    "fccmp" => fp_ccmp,
    "fcsel" => fp_csel,
    "fmov-imm" => fp_imm,
    "fp-int" => fp_int,
    "fp-fixed" => fp_fixed,
    "v-3same" => v3same,
    "v-2misc" => v2misc,
    "v-across" => v_across,
    "v-copy" => v_copy,
    "v-perm" => v_perm,
    "v-ext" => v_ext,
    "v-tbl" => v_tbl,
    "v-modimm" => v_modimm,
    "v-shift" => v_shift,
    "s-simd" => s_simd,
    "v-ldst-multi" => v_ldst_multi,
    "v-ldst-single" => v_ldst_single,
}
