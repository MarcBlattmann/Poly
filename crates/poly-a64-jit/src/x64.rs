//! An x86-64 machine-code emitter.
//!
//! Written from the Intel 64 and IA-32 Architectures Software Developer's
//! Manual, volume 2 (instruction formats: legacy prefixes, REX, opcode,
//! ModR/M, SIB, displacement, immediate) and AMD's APM volume 3. Only what
//! the translator needs is here. Every encoding is checked against the
//! system assembler (a black box) by the `x64_vectors` test.

use alloc::vec::Vec;

/// A general-purpose register (its number is the encoding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum R {
    Rax = 0,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

impl R {
    const fn n(self) -> u8 {
        self as u8
    }
    const fn lo(self) -> u8 {
        self as u8 & 7
    }
    const fn hi(self) -> u8 {
        (self as u8 >> 3) & 1
    }
}

/// An SSE register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Xmm(pub u8);

/// A memory operand: `[base + index * scale + disp]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct M {
    /// Base register.
    pub base: R,
    /// Index register and log2 of the scale (0 to 3). `Rsp` cannot index.
    pub index: Option<(R, u8)>,
    /// Displacement.
    pub disp: i32,
}

/// `[base + disp]`.
pub const fn mem(base: R, disp: i32) -> M {
    M { base, index: None, disp }
}

/// `[base + index + disp]`.
pub const fn mem_idx(base: R, index: R, disp: i32) -> M {
    M { base, index: Some((index, 0)), disp }
}

/// Operand size of an integer instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sz {
    /// 8 bits.
    B,
    /// 16 bits.
    H,
    /// 32 bits (writes zero the upper half).
    D,
    /// 64 bits.
    Q,
}

/// A condition code (the low nibble of `Jcc`/`SETcc`/`CMOVcc`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Cc {
    O = 0,
    No = 1,
    B = 2,
    Ae = 3,
    E = 4,
    Ne = 5,
    Be = 6,
    A = 7,
    S = 8,
    Ns = 9,
    P = 10,
    Np = 11,
    L = 12,
    Ge = 13,
    Le = 14,
    G = 15,
}

impl Cc {
    /// The opposite condition.
    pub const fn not(self) -> Cc {
        // SAFETY-free: every value 0..=15 is a variant.
        match self as u8 ^ 1 {
            0 => Cc::O,
            1 => Cc::No,
            2 => Cc::B,
            3 => Cc::Ae,
            4 => Cc::E,
            5 => Cc::Ne,
            6 => Cc::Be,
            7 => Cc::A,
            8 => Cc::S,
            9 => Cc::Ns,
            10 => Cc::P,
            11 => Cc::Np,
            12 => Cc::L,
            13 => Cc::Ge,
            14 => Cc::Le,
            _ => Cc::G,
        }
    }
}

/// The eight classic ALU operations (`/digit` of opcodes 0x81/0x83, and
/// `op * 8 + 1/3` for the register forms).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Alu {
    Add = 0,
    Or = 1,
    Adc = 2,
    Sbb = 3,
    And = 4,
    Sub = 5,
    Xor = 6,
    Cmp = 7,
}

/// Shift and rotate operations (`/digit` of 0xC1/0xD3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum Sh {
    Rol = 0,
    Ror = 1,
    Shl = 4,
    Shr = 5,
    Sar = 7,
}

/// A position in the code to jump to, bound later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label(pub u32);

/// The emitter: a growing byte buffer with labels.
#[derive(Debug, Default)]
pub struct Asm {
    /// The code.
    pub buf: Vec<u8>,
    labels: Vec<Option<u32>>,
    /// (offset of a rel32 field, label)
    fixups: Vec<(u32, Label)>,
    /// (offset of a rel32 field, target offset in the code region)
    ext: Vec<(u32, usize)>,
}

impl Asm {
    /// An empty emitter.
    pub fn new() -> Asm {
        Asm::default()
    }

    /// The current offset.
    pub fn pos(&self) -> usize {
        self.buf.len()
    }

    fn b(&mut self, x: u8) {
        self.buf.push(x);
    }
    fn d32(&mut self, x: i32) {
        self.buf.extend_from_slice(&x.to_le_bytes());
    }

    /// `jmp rel32` to an offset outside this buffer (`target`, relative to
    /// where the buffer will be placed); resolved by [`Asm::place`].
    pub fn jmp_ext(&mut self, target: usize) -> usize {
        self.b(0xE9);
        let at = self.buf.len();
        self.ext.push((at as u32, target));
        self.d32(0);
        at
    }

    /// `jcc rel32` to an offset outside this buffer.
    pub fn jcc_ext(&mut self, cc: Cc, target: usize) {
        self.buf.extend_from_slice(&[0x0F, 0x80 + cc as u8]);
        let at = self.buf.len();
        self.ext.push((at as u32, target));
        self.d32(0);
    }

    /// Resolve the external jumps for a placement of this buffer at `base`
    /// (both offsets in the same code region).
    pub fn place(&mut self, base: usize) {
        for &(at, target) in &self.ext {
            let rel = target as i64 - (base as i64 + at as i64 + 4);
            self.buf[at as usize..at as usize + 4].copy_from_slice(&(rel as i32).to_le_bytes());
        }
    }

    /// A new unbound label.
    pub fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() as u32 - 1)
    }

    /// Bind `l` to the current position.
    pub fn bind(&mut self, l: Label) {
        self.labels[l.0 as usize] = Some(self.buf.len() as u32);
    }

    /// Resolve the jumps to labels. Panics if a label used is unbound (a bug
    /// in the translator).
    pub fn finish(&mut self) {
        for &(at, l) in &self.fixups {
            let target = self.labels[l.0 as usize].expect("label bound");
            let rel = target as i64 - (at as i64 + 4);
            self.buf[at as usize..at as usize + 4].copy_from_slice(&(rel as i32).to_le_bytes());
        }
        self.fixups.clear();
    }

    // ---- prefix, ModR/M ----

    /// REX prefix if needed. `w`: 64-bit operand; `r`, `x`, `b`: high bits
    /// of the reg field, index and base. `force`: needed for SPL/BPL/SIL/DIL
    /// byte registers.
    fn rex(&mut self, w: bool, r: u8, x: u8, b: u8, force: bool) {
        let v = 0x40 | (u8::from(w) << 3) | (r << 2) | (x << 1) | b;
        if v != 0x40 || force {
            self.b(v);
        }
    }

    fn modrm_rr(&mut self, reg: u8, rm: u8) {
        self.b(0xC0 | ((reg & 7) << 3) | (rm & 7));
    }

    fn modrm_m(&mut self, reg: u8, m: M) {
        let reg = (reg & 7) << 3;
        let base = m.base.lo();
        // mod: 00 no disp (not for rbp/r13 base), 01 disp8, 10 disp32.
        let (md, disp8) = if m.disp == 0 && base != 5 {
            (0u8, false)
        } else if (-128..=127).contains(&m.disp) {
            (1, true)
        } else {
            (2, false)
        };
        match m.index {
            None if base != 4 => self.b((md << 6) | reg | base),
            None => {
                self.b((md << 6) | reg | 4);
                self.b(0x24); // SIB: no index, base rsp/r12
            }
            Some((idx, scale)) => {
                self.b((md << 6) | reg | 4);
                self.b((scale << 6) | (idx.lo() << 3) | base);
            }
        }
        match md {
            1 => self.b(m.disp as i8 as u8),
            2 => self.d32(m.disp),
            _ => {}
        }
        let _ = disp8;
    }

    fn mx(m: M) -> u8 {
        m.index.map_or(0, |(i, _)| i.hi())
    }

    /// `op reg, r/m` with a register r/m. `byte_regs`: the operands are byte
    /// registers (REX needed for spl..dil).
    fn op_rr(&mut self, sz: Sz, op: &[u8], reg: u8, rm: u8) {
        if sz == Sz::H {
            self.b(0x66);
        }
        let force = sz == Sz::B && ((4..8).contains(&reg) || (4..8).contains(&rm));
        self.rex(sz == Sz::Q, (reg >> 3) & 1, 0, (rm >> 3) & 1, force);
        self.buf.extend_from_slice(op);
        self.modrm_rr(reg, rm);
    }

    /// `op /digit, r/m` with a register r/m: like `op_rr`, but the reg
    /// field is an opcode extension, not a register.
    fn op_digit(&mut self, sz: Sz, op: &[u8], digit: u8, rm: u8) {
        if sz == Sz::H {
            self.b(0x66);
        }
        let force = sz == Sz::B && (4..8).contains(&rm);
        self.rex(sz == Sz::Q, 0, 0, (rm >> 3) & 1, force);
        self.buf.extend_from_slice(op);
        self.modrm_rr(digit, rm);
    }

    /// `op /digit, [m]`.
    fn op_mdigit(&mut self, sz: Sz, op: &[u8], digit: u8, m: M) {
        if sz == Sz::H {
            self.b(0x66);
        }
        self.rex(sz == Sz::Q, 0, Self::mx(m), m.base.hi(), false);
        self.buf.extend_from_slice(op);
        self.modrm_m(digit, m);
    }

    fn op_rm(&mut self, sz: Sz, op: &[u8], reg: u8, m: M) {
        self.op_rm_lock(sz, false, op, reg, m);
    }

    /// `op_rm` with an optional `lock` prefix (after the operand-size one,
    /// as assemblers order them).
    fn op_rm_lock(&mut self, sz: Sz, lock: bool, op: &[u8], reg: u8, m: M) {
        if sz == Sz::H {
            self.b(0x66);
        }
        if lock {
            self.b(0xF0);
        }
        let force = sz == Sz::B && (4..8).contains(&reg);
        self.rex(sz == Sz::Q, (reg >> 3) & 1, Self::mx(m), m.base.hi(), force);
        self.buf.extend_from_slice(op);
        self.modrm_m(reg, m);
    }

    // ---- moves ----

    /// `mov dst, src` (32-bit moves zero the upper half).
    pub fn mov_rr(&mut self, sz: Sz, dst: R, src: R) {
        let op = if sz == Sz::B { 0x88 } else { 0x89 };
        self.op_rr(sz, &[op], src.n(), dst.n());
    }

    /// `mov dst, [m]`.
    pub fn load(&mut self, sz: Sz, dst: R, m: M) {
        let op = if sz == Sz::B { 0x8A } else { 0x8B };
        self.op_rm(sz, &[op], dst.n(), m);
    }

    /// `mov [m], src`.
    pub fn store(&mut self, sz: Sz, m: M, src: R) {
        let op = if sz == Sz::B { 0x88 } else { 0x89 };
        self.op_rm(sz, &[op], src.n(), m);
    }

    /// `mov dword/qword [m], imm32` (sign-extended for qword).
    pub fn store_imm(&mut self, sz: Sz, m: M, imm: i32) {
        match sz {
            Sz::B => {
                self.op_mdigit(sz, &[0xC6], 0, m);
                self.b(imm as u8);
            }
            Sz::H => {
                self.op_mdigit(sz, &[0xC7], 0, m);
                self.buf.extend_from_slice(&(imm as u16).to_le_bytes());
            }
            _ => {
                self.op_mdigit(sz, &[0xC7], 0, m);
                self.d32(imm);
            }
        }
    }

    /// `movzx dst32, byte/word [m]`.
    pub fn load_zx(&mut self, from: Sz, dst: R, m: M) {
        let op = if from == Sz::B { 0xB6 } else { 0xB7 };
        self.op_rm(Sz::D, &[0x0F, op], dst.n(), m);
    }

    /// `movsx dst, byte/word/dword [m]` into 32 or 64 bits.
    pub fn load_sx(&mut self, from: Sz, to: Sz, dst: R, m: M) {
        match from {
            Sz::B => self.op_rm(to, &[0x0F, 0xBE], dst.n(), m),
            Sz::H => self.op_rm(to, &[0x0F, 0xBF], dst.n(), m),
            _ => self.op_rm(Sz::Q, &[0x63], dst.n(), m),
        }
    }

    /// `movzx dst32, src8/src16`.
    pub fn zx_rr(&mut self, from: Sz, dst: R, src: R) {
        let op = if from == Sz::B { 0xB6 } else { 0xB7 };
        let force = from == Sz::B && (4..8).contains(&src.n());
        self.rex(false, dst.hi(), 0, src.hi(), force);
        self.buf.extend_from_slice(&[0x0F, op]);
        self.modrm_rr(dst.n(), src.n());
    }

    /// `movsx dst, src8/src16/src32` into 32 or 64 bits.
    pub fn sx_rr(&mut self, from: Sz, to: Sz, dst: R, src: R) {
        match from {
            Sz::B => {
                let force = (4..8).contains(&src.n());
                self.rex(to == Sz::Q, dst.hi(), 0, src.hi(), force);
                self.buf.extend_from_slice(&[0x0F, 0xBE]);
                self.modrm_rr(dst.n(), src.n());
            }
            Sz::H => self.op_rr(to, &[0x0F, 0xBF], dst.n(), src.n()),
            _ => self.op_rr(Sz::Q, &[0x63], dst.n(), src.n()),
        }
    }

    /// Load a 64-bit constant with the shortest form.
    pub fn mov_imm(&mut self, dst: R, imm: u64) {
        if imm == 0 {
            self.alu_rr(Sz::D, Alu::Xor, dst, dst);
        } else if imm <= u64::from(u32::MAX) {
            // mov r32, imm32
            self.rex(false, 0, 0, dst.hi(), false);
            self.b(0xB8 + dst.lo());
            self.d32(imm as u32 as i32);
        } else if (imm as i64) >= i64::from(i32::MIN) && (imm as i64) < 0 {
            // mov r64, simm32
            self.rex(true, 0, 0, dst.hi(), false);
            self.b(0xC7);
            self.modrm_rr(0, dst.n());
            self.d32(imm as i32);
        } else {
            self.rex(true, 0, 0, dst.hi(), false);
            self.b(0xB8 + dst.lo());
            self.buf.extend_from_slice(&imm.to_le_bytes());
        }
    }

    /// `lea dst, [m]`.
    pub fn lea(&mut self, sz: Sz, dst: R, m: M) {
        self.op_rm(sz, &[0x8D], dst.n(), m);
    }

    // ---- arithmetic ----

    /// `op dst, src`.
    pub fn alu_rr(&mut self, sz: Sz, op: Alu, dst: R, src: R) {
        let o = (op as u8) * 8 + if sz == Sz::B { 0 } else { 1 };
        self.op_rr(sz, &[o], src.n(), dst.n());
    }

    /// `op dst, [m]`.
    pub fn alu_rm(&mut self, sz: Sz, op: Alu, dst: R, m: M) {
        let o = (op as u8) * 8 + if sz == Sz::B { 2 } else { 3 };
        self.op_rm(sz, &[o], dst.n(), m);
    }

    /// `op [m], src`.
    pub fn alu_mr(&mut self, sz: Sz, op: Alu, m: M, src: R) {
        let o = (op as u8) * 8 + if sz == Sz::B { 0 } else { 1 };
        self.op_rm(sz, &[o], src.n(), m);
    }

    /// `op dst, imm32` (sign-extended to 64 bits).
    pub fn alu_ri(&mut self, sz: Sz, op: Alu, dst: R, imm: i32) {
        if sz == Sz::B {
            if dst == R::Rax {
                self.b(op as u8 * 8 + 4);
            } else {
                self.op_digit(sz, &[0x80], op as u8, dst.n());
            }
            self.b(imm as u8);
        } else if (-128..=127).contains(&imm) {
            self.op_digit(sz, &[0x83], op as u8, dst.n());
            self.b(imm as i8 as u8);
        } else {
            if dst == R::Rax {
                // The accumulator form: op eAX, imm.
                if sz == Sz::H {
                    self.b(0x66);
                }
                self.rex(sz == Sz::Q, 0, 0, 0, false);
                self.b(op as u8 * 8 + 5);
            } else {
                self.op_digit(sz, &[0x81], op as u8, dst.n());
            }
            if sz == Sz::H {
                self.buf.extend_from_slice(&(imm as u16).to_le_bytes());
            } else {
                self.d32(imm);
            }
        }
    }

    /// `op qword/dword [m], imm` (sign-extended imm8 or imm32).
    pub fn alu_mi(&mut self, sz: Sz, op: Alu, m: M, imm: i32) {
        if sz == Sz::B {
            self.op_mdigit(sz, &[0x80], op as u8, m);
            self.b(imm as u8);
        } else if (-128..=127).contains(&imm) {
            self.op_mdigit(sz, &[0x83], op as u8, m);
            self.b(imm as i8 as u8);
        } else {
            self.op_mdigit(sz, &[0x81], op as u8, m);
            self.d32(imm);
        }
    }

    /// `test a, b`.
    pub fn test_rr(&mut self, sz: Sz, a: R, b: R) {
        let o = if sz == Sz::B { 0x84 } else { 0x85 };
        self.op_rr(sz, &[o], b.n(), a.n());
    }

    /// `test r, imm32`.
    pub fn test_ri(&mut self, sz: Sz, r: R, imm: i32) {
        if sz == Sz::B {
            if r == R::Rax {
                self.b(0xA8);
            } else {
                self.op_digit(sz, &[0xF6], 0, r.n());
            }
            self.b(imm as u8);
        } else {
            if r == R::Rax {
                if sz == Sz::H {
                    self.b(0x66);
                }
                self.rex(sz == Sz::Q, 0, 0, 0, false);
                self.b(0xA9);
            } else {
                self.op_digit(sz, &[0xF7], 0, r.n());
            }
            self.d32(imm);
        }
    }

    /// `test byte [m], imm8`.
    pub fn test_mi8(&mut self, m: M, imm: u8) {
        self.op_mdigit(Sz::B, &[0xF6], 0, m);
        self.b(imm);
    }

    /// Group 3 unary operation (`not` 2, `neg` 3, `mul` 4, `imul` 5, `div` 6, `idiv` 7).
    fn grp3(&mut self, sz: Sz, digit: u8, r: R) {
        let o = if sz == Sz::B { 0xF6 } else { 0xF7 };
        self.op_digit(sz, &[o], digit, r.n());
    }
    /// `not r`.
    pub fn not(&mut self, sz: Sz, r: R) {
        self.grp3(sz, 2, r);
    }
    /// `neg r`.
    pub fn neg(&mut self, sz: Sz, r: R) {
        self.grp3(sz, 3, r);
    }
    /// `mul r`: `rdx:rax = rax * r` (unsigned).
    pub fn mul(&mut self, sz: Sz, r: R) {
        self.grp3(sz, 4, r);
    }
    /// `imul r`: `rdx:rax = rax * r` (signed).
    pub fn imul1(&mut self, sz: Sz, r: R) {
        self.grp3(sz, 5, r);
    }
    /// `div r`: unsigned `rdx:rax / r`.
    pub fn div(&mut self, sz: Sz, r: R) {
        self.grp3(sz, 6, r);
    }
    /// `idiv r`: signed `rdx:rax / r`.
    pub fn idiv(&mut self, sz: Sz, r: R) {
        self.grp3(sz, 7, r);
    }

    /// `imul dst, src`.
    pub fn imul(&mut self, sz: Sz, dst: R, src: R) {
        self.op_rr(sz, &[0x0F, 0xAF], dst.n(), src.n());
    }

    /// `cqo` (64) or `cdq` (32): sign-extend `rax` into `rdx`.
    pub fn cqo(&mut self, sz: Sz) {
        if sz == Sz::Q {
            self.b(0x48);
        }
        self.b(0x99);
    }

    /// Shift or rotate by an immediate.
    pub fn shift_ri(&mut self, sz: Sz, op: Sh, r: R, n: u8) {
        let byte = u8::from(sz == Sz::B);
        if n == 1 {
            self.op_digit(sz, &[0xD1 - byte], op as u8, r.n());
        } else {
            self.op_digit(sz, &[0xC1 - byte], op as u8, r.n());
            self.b(n);
        }
    }

    /// Shift or rotate by `cl`.
    pub fn shift_cl(&mut self, sz: Sz, op: Sh, r: R) {
        self.op_digit(sz, &[0xD3 - u8::from(sz == Sz::B)], op as u8, r.n());
    }

    /// `bswap r` (32 or 64 bits).
    pub fn bswap(&mut self, sz: Sz, r: R) {
        self.rex(sz == Sz::Q, 0, 0, r.hi(), false);
        self.b(0x0F);
        self.b(0xC8 + r.lo());
    }

    /// `bsr dst, src`.
    pub fn bsr(&mut self, sz: Sz, dst: R, src: R) {
        self.op_rr(sz, &[0x0F, 0xBD], dst.n(), src.n());
    }

    /// `bt r, imm8`: CF = bit.
    pub fn bt_ri(&mut self, sz: Sz, r: R, bit: u8) {
        self.op_digit(sz, &[0x0F, 0xBA], 4, r.n());
        self.b(bit);
    }

    /// `setcc r8`.
    pub fn setcc(&mut self, cc: Cc, r: R) {
        self.rex(false, 0, 0, r.hi(), (4..8).contains(&r.n()));
        self.buf.extend_from_slice(&[0x0F, 0x90 + cc as u8]);
        self.modrm_rr(0, r.n());
    }

    /// `cmovcc dst, src`.
    pub fn cmov(&mut self, sz: Sz, cc: Cc, dst: R, src: R) {
        self.op_rr(sz, &[0x0F, 0x40 + cc as u8], dst.n(), src.n());
    }

    /// `xchg [m], r` (an implicitly locked swap).
    pub fn xchg_mr(&mut self, sz: Sz, m: M, r: R) {
        let o = if sz == Sz::B { 0x86 } else { 0x87 };
        self.op_rm(sz, &[o], r.n(), m);
    }

    /// `lock cmpxchg [m], r`: compare `rax` with `[m]`.
    pub fn lock_cmpxchg(&mut self, sz: Sz, m: M, r: R) {
        let o = if sz == Sz::B { 0xB0 } else { 0xB1 };
        self.op_rm_lock(sz, true, &[0x0F, o], r.n(), m);
    }

    /// `lock xadd [m], r`.
    pub fn lock_xadd(&mut self, sz: Sz, m: M, r: R) {
        let o = if sz == Sz::B { 0xC0 } else { 0xC1 };
        self.op_rm_lock(sz, true, &[0x0F, o], r.n(), m);
    }

    // ---- control flow ----

    /// `jcc label` (rel32).
    pub fn jcc(&mut self, cc: Cc, l: Label) {
        self.buf.extend_from_slice(&[0x0F, 0x80 + cc as u8]);
        self.fixups.push((self.buf.len() as u32, l));
        self.d32(0);
    }

    /// `jmp label` (rel32).
    pub fn jmp(&mut self, l: Label) {
        self.b(0xE9);
        self.fixups.push((self.buf.len() as u32, l));
        self.d32(0);
    }

    /// `jmp rel32` with a raw displacement; returns the offset of the rel32
    /// field (for patching).
    pub fn jmp_rel32(&mut self, rel: i32) -> usize {
        self.b(0xE9);
        let at = self.buf.len();
        self.d32(rel);
        at
    }

    /// `jmp r`.
    pub fn jmp_r(&mut self, r: R) {
        self.rex(false, 0, 0, r.hi(), false);
        self.b(0xFF);
        self.modrm_rr(4, r.n());
    }

    /// `jmp qword [m]`.
    pub fn jmp_m(&mut self, m: M) {
        self.op_mdigit(Sz::D, &[0xFF], 4, m);
    }

    /// `call r`.
    pub fn call_r(&mut self, r: R) {
        self.rex(false, 0, 0, r.hi(), false);
        self.b(0xFF);
        self.modrm_rr(2, r.n());
    }

    /// `ret`.
    pub fn ret(&mut self) {
        self.b(0xC3);
    }

    /// `push r`.
    pub fn push(&mut self, r: R) {
        self.rex(false, 0, 0, r.hi(), false);
        self.b(0x50 + r.lo());
    }

    /// `pushfq`: the flags onto the stack.
    pub fn pushfq(&mut self) {
        self.b(0x9C);
    }

    /// `pop r`.
    pub fn pop(&mut self, r: R) {
        self.rex(false, 0, 0, r.hi(), false);
        self.b(0x58 + r.lo());
    }

    /// `cmc`: complement the carry flag.
    pub fn cmc(&mut self) {
        self.b(0xF5);
    }

    /// `mov r32, imm32` (always this form, which leaves the flags alone,
    /// even for 0).
    pub fn mov_r32_imm(&mut self, dst: R, imm: u32) {
        self.rex(false, 0, 0, dst.hi(), false);
        self.b(0xB8 + dst.lo());
        self.d32(imm as i32);
    }

    /// Load a 64-bit constant without touching the flags.
    pub fn mov_imm_keep_flags(&mut self, dst: R, imm: u64) {
        if imm == 0 {
            self.mov_r32_imm(dst, 0);
        } else {
            self.mov_imm(dst, imm);
        }
    }

    /// `bt dword/qword [m], imm8`.
    pub fn bt_mi(&mut self, sz: Sz, m: M, bit: u8) {
        self.op_mdigit(sz, &[0x0F, 0xBA], 4, m);
        self.b(bit);
    }

    /// `ud2`.
    pub fn ud2(&mut self) {
        self.buf.extend_from_slice(&[0x0F, 0x0B]);
    }

    // ---- SSE ----

    fn sse_rm(&mut self, prefix: Option<u8>, w: bool, op: &[u8], reg: u8, m: M) {
        if let Some(p) = prefix {
            self.b(p);
        }
        self.rex(w, (reg >> 3) & 1, Self::mx(m), m.base.hi(), false);
        self.buf.extend_from_slice(op);
        self.modrm_m(reg, m);
    }

    fn sse_rr(&mut self, prefix: Option<u8>, w: bool, op: &[u8], reg: u8, rm: u8) {
        if let Some(p) = prefix {
            self.b(p);
        }
        self.rex(w, (reg >> 3) & 1, 0, (rm >> 3) & 1, false);
        self.buf.extend_from_slice(op);
        self.modrm_rr(reg, rm);
    }

    /// `movdqu xmm, [m]` (128-bit load).
    pub fn movdqu_load(&mut self, x: Xmm, m: M) {
        self.sse_rm(Some(0xF3), false, &[0x0F, 0x6F], x.0, m);
    }
    /// `movdqu [m], xmm`.
    pub fn movdqu_store(&mut self, m: M, x: Xmm) {
        self.sse_rm(Some(0xF3), false, &[0x0F, 0x7F], x.0, m);
    }
    /// `movsd xmm, qword [m]` (zeroes the upper half).
    pub fn movsd_load(&mut self, x: Xmm, m: M) {
        self.sse_rm(Some(0xF2), false, &[0x0F, 0x10], x.0, m);
    }
    /// `movsd qword [m], xmm`.
    pub fn movsd_store(&mut self, m: M, x: Xmm) {
        self.sse_rm(Some(0xF2), false, &[0x0F, 0x11], x.0, m);
    }
    /// `movss xmm, dword [m]`.
    pub fn movss_load(&mut self, x: Xmm, m: M) {
        self.sse_rm(Some(0xF3), false, &[0x0F, 0x10], x.0, m);
    }
    /// `movss dword [m], xmm`.
    pub fn movss_store(&mut self, m: M, x: Xmm) {
        self.sse_rm(Some(0xF3), false, &[0x0F, 0x11], x.0, m);
    }
    /// `movq xmm, r64`.
    pub fn movq_xr(&mut self, x: Xmm, r: R) {
        self.sse_rr(Some(0x66), true, &[0x0F, 0x6E], x.0, r.n());
    }
    /// `movq r64, xmm`.
    pub fn movq_rx(&mut self, r: R, x: Xmm) {
        self.sse_rr(Some(0x66), true, &[0x0F, 0x7E], x.0, r.n());
    }

    /// Scalar double/single arithmetic `op xmm_d, xmm_s`: `op` is the second
    /// opcode byte (0x58 add, 0x59 mul, 0x5C sub, 0x5E div, 0x51 sqrt).
    pub fn sse_scalar(&mut self, double: bool, op: u8, d: Xmm, s: Xmm) {
        self.sse_rr(Some(if double { 0xF2 } else { 0xF3 }), false, &[0x0F, op], d.0, s.0);
    }

    /// `ucomisd`/`ucomiss a, b`.
    pub fn ucomis(&mut self, double: bool, a: Xmm, b: Xmm) {
        self.sse_rr(if double { Some(0x66) } else { None }, false, &[0x0F, 0x2E], a.0, b.0);
    }

    /// `cvtsi2sd`/`cvtsi2ss xmm, r32/r64` (`w64`: a 64-bit source).
    pub fn cvtsi2(&mut self, double: bool, w64: bool, x: Xmm, r: R) {
        self.sse_rr(Some(if double { 0xF2 } else { 0xF3 }), w64, &[0x0F, 0x2A], x.0, r.n());
    }

    /// `cvttsd2si`/`cvttss2si r32/r64, xmm` (truncating).
    pub fn cvtt2si(&mut self, double: bool, w64: bool, r: R, x: Xmm) {
        self.sse_rr(Some(if double { 0xF2 } else { 0xF3 }), w64, &[0x0F, 0x2C], r.n(), x.0);
    }

    /// `cvtsd2ss` (`to_single`) or `cvtss2sd xmm_d, xmm_s`.
    pub fn cvt_fp(&mut self, to_single: bool, d: Xmm, s: Xmm) {
        self.sse_rr(Some(if to_single { 0xF2 } else { 0xF3 }), false, &[0x0F, 0x5A], d.0, s.0);
    }

    /// `movd xmm, r32`.
    pub fn movd_xr(&mut self, x: Xmm, r: R) {
        self.sse_rr(Some(0x66), false, &[0x0F, 0x6E], x.0, r.n());
    }

    /// `movd r32, xmm`.
    pub fn movd_rx(&mut self, r: R, x: Xmm) {
        self.sse_rr(Some(0x66), false, &[0x0F, 0x7E], x.0, r.n());
    }

    /// `crc32 r32, r/m` with an 8, 16, 32 or 64-bit source (CRC-32C).
    pub fn crc32(&mut self, sz: Sz, dst: R, src: R) {
        if sz == Sz::H {
            self.b(0x66);
        }
        self.b(0xF2);
        let force = sz == Sz::B && (4..8).contains(&src.n());
        self.rex(sz == Sz::Q, dst.hi(), 0, src.hi(), force);
        let op = if sz == Sz::B { 0xF0 } else { 0xF1 };
        self.buf.extend_from_slice(&[0x0F, 0x38, op]);
        self.modrm_rr(dst.n(), src.n());
    }

    /// A scalar FMA3 instruction `vf*231sd`/`ss xmm_d, xmm_a, xmm_b`
    /// (`d = ±(a * b) ± d`): `op` 0xB9 fmadd, 0xBB fmsub, 0xBD fnmadd, 0xBF
    /// fnmsub. Registers 0 to 15.
    pub fn vfma231(&mut self, double: bool, op: u8, d: Xmm, a: Xmm, b: Xmm) {
        // VEX three-byte form: C4, R̄X̄B̄ mmmmm (0F38), W v̄v̄v̄v̄ L pp (66).
        self.b(0xC4);
        self.b((((d.0 >> 3) ^ 1) << 7) | (1 << 6) | (((b.0 >> 3) ^ 1) << 5) | 0b00010);
        self.b((u8::from(double) << 7) | ((!a.0 & 15) << 3) | 0b01);
        self.b(op);
        self.modrm_rr(d.0, b.0);
    }

    /// Packed integer or logic op `op xmm_d, xmm_s` with the 0x66 prefix:
    /// `op` is the second opcode byte (0xFE paddd, 0xD4 paddq, 0xFC paddb,
    /// 0xFD paddw, 0xFA psubd, 0xFB psubq, 0xF8 psubb, 0xF9 psubw, 0xDB
    /// pand, 0xEB por, 0xEF pxor, 0xDF pandn, 0x74/0x75/0x76 pcmpeq b/w/d).
    pub fn sse_packed(&mut self, op: u8, d: Xmm, s: Xmm) {
        self.sse_rr(Some(0x66), false, &[0x0F, op], d.0, s.0);
    }
}

#[cfg(test)]
pub(crate) mod vectors;
