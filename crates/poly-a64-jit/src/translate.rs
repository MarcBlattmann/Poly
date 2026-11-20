//! Translation of one A64 basic block to x86-64 (ADR 0239).
//!
//! # Conventions of translated code
//!
//! Translated code runs between the entry and exit routines of
//! [`crate::jit`] with these host registers pinned:
//!
//! | register | holds |
//! |---|---|
//! | `rbx` | the guest [`Cpu`] (registers are read and written there) |
//! | `rbp` | the [`crate::jit::Ctx`] |
//! | `r12` | the host address of the guest window's first byte |
//! | `r13` | the page flag table of [`crate::mem::GuestMem`] |
//! | `r14` | the guest window's first address |
//! | `r15` | the guest window's size |
//!
//! Guest registers live in the `Cpu` in memory; nothing is cached in host
//! registers across guest instructions, so the guest state is exact at every
//! instruction boundary and any instruction can hand over to the
//! interpreter.
//!
//! **Flags.** A flag-setting instruction leaves its result in the host's
//! flags (an x86 `add`/`sub`/`and` computes N, Z, V and C the way A64 does,
//! with C inverted after a subtraction). They are written to the `Cpu`'s
//! `NZCV` only if they are live: read by a later instruction that cannot use
//! the host flags, or still unread at a possible exit of the block. The
//! write uses only instructions that leave the host flags alone, so a
//! following conditional branch, select or carry instruction still branches
//! on the host flags directly.
//!
//! **Memory.** Every access checks the guest address against the window,
//! the page's flags (read; write and not translated code) and the 16 KiB
//! page boundary, then accesses `r12 + (addr - r14)`. A failed check leaves
//! the block before the instruction has changed anything and asks the run
//! loop to interpret it: the interpreter faults precisely, or writes and
//! records the page as changed code.
//!
//! **Exits.** A block ends with a branch, a system call, an instruction the
//! translator does not handle, or the end of its page. Direct exits store
//! the next `PC` and jump through a patchable `jmp` (block chaining).
//! Indirect branches look the target up in a small table in the context
//! (the indirect branch cache) before going back to the run loop.

use alloc::vec::Vec;
use core::mem::offset_of;

use poly_a64::{
    Addr, AtomOp, BfOp, Bit1Op, CmpRhs, Cond, Cpu, CselOp, Extend, Fp1Op, Fp2Op, FpRound, FpType, Inst, LogOp, MemKind,
    MovOp, PairKind, Reg, Shift, V2Op, V3Op, Width,
};

use crate::mem::{CODE, PAGE_SIZE, R as PR, W as PW};
use crate::x64::{Alu, Asm, Cc, Label, M, R, Sh, Sz, Xmm, mem, mem_idx};

/// What the host CPU offers beyond x86-64's baseline (SSE2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// FMA3 (and the AVX state the OS saves): fused multiply-add.
    pub fma: bool,
    /// SSE4.2: the `crc32` instruction (CRC-32C).
    pub sse42: bool,
}

impl Features {
    /// The running CPU's features (x86-64 only; nothing elsewhere).
    pub fn host() -> Features {
        #[cfg(target_arch = "x86_64")]
        {
            // SAFETY: CPUID is available on every x86-64 CPU; XGETBV only when
            // CPUID says OSXSAVE is set.
            unsafe {
                let l1 = core::arch::x86_64::__cpuid(1);
                let osxsave = l1.ecx & (1 << 27) != 0;
                let avx_state = if osxsave {
                    let (lo, hi): (u32, u32);
                    core::arch::asm!("xgetbv", in("ecx") 0u32, out("eax") lo, out("edx") hi, options(nomem, nostack));
                    let xcr0 = (u64::from(hi) << 32) | u64::from(lo);
                    xcr0 & 6 == 6
                } else {
                    false
                };
                Features {
                    fma: l1.ecx & (1 << 12) != 0 && l1.ecx & (1 << 28) != 0 && avx_state,
                    sse42: l1.ecx & (1 << 20) != 0,
                }
            }
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            Features::default()
        }
    }
}

/// Exit codes in `eax` when translated code returns to the run loop.
pub mod exit {
    /// `PC` is the next instruction: find (or make) its block.
    pub const NEXT: u32 = 0;
    /// Interpret the instruction at `PC`, then go on.
    pub const INTERPRET: u32 = 1;
    /// A system call: `PC` is past the `SVC`, `Ctx::arg` its immediate.
    pub const SVC: u32 = 2;
}

/// Offsets into the context (`rbp`), see [`crate::jit::Ctx`].
pub(crate) mod ctx {
    pub const BUDGET: i32 = 40;
    pub const ARG: i32 = 56;
    pub const SITE: i32 = 64;
    pub const IBTC: i32 = 72;
}

/// log2 of the indirect branch cache's entries.
pub const IBTC_BITS: u32 = 12;

const RCPU: R = R::Rbx;
const RCTX: R = R::Rbp;
const RHOST: R = R::R12;
const RFLAGS: R = R::R13;
const RLO: R = R::R14;
const RSIZE: R = R::R15;

fn ox(r: Reg) -> i32 {
    (offset_of!(Cpu, x) + 8 * r as usize) as i32
}
fn osp() -> i32 {
    offset_of!(Cpu, sp) as i32
}
fn opc() -> i32 {
    offset_of!(Cpu, pc) as i32
}
fn onzcv() -> i32 {
    offset_of!(Cpu, nzcv) as i32
}
fn ov(r: Reg) -> i32 {
    (offset_of!(Cpu, v) + 16 * r as usize) as i32
}

/// What the host flags hold after a flag-setting instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostFlags {
    /// From an x86 `sub`/`cmp`/`sbb`: C is the inverse of the host carry.
    Sub,
    /// From an x86 `add`/`adc`/`and`/`test`: C is the host carry.
    Add,
}

/// A direct exit of a block that can be chained.
#[derive(Clone, Copy, Debug)]
pub struct ExitSite {
    /// Offset (in the block) of the `rel32` of the patchable `jmp`.
    pub jmp_field: usize,
    /// Offset (in the block) of the stub the `jmp` goes to when unchained.
    pub stub: usize,
    /// Offset (in the block) of the `imm32` in the stub that reports the
    /// site to the run loop (patched to the site's offset in the code area).
    pub report_field: usize,
    /// The guest address it goes to.
    pub target: u64,
}

/// A translated block, not yet placed in the code area.
#[derive(Debug)]
pub struct Block {
    /// The code.
    pub asm: Asm,
    /// First guest address.
    pub start: u64,
    /// One past the last guest instruction.
    pub end: u64,
    /// Number of guest instructions translated.
    pub insts: usize,
    /// Chainable exits.
    pub exits: Vec<ExitSite>,
}

/// Can the translator handle this instruction (on a CPU with `f`)?
pub fn translatable_with(inst: &Inst, f: Features) -> bool {
    match *inst {
        Inst::FpFused { ft, .. } => f.fma && ft != FpType::H,
        Inst::Crc32 { c, .. } => c && f.sse42,
        _ => translatable(inst),
    }
}

/// Can the translator handle this instruction on any x86-64 CPU?
pub fn translatable(inst: &Inst) -> bool {
    let fp = |ft: FpType| ft != FpType::H;
    match *inst {
        Inst::FpBinary { op, ft, .. } => {
            fp(ft) && matches!(op, Fp2Op::Add | Fp2Op::Sub | Fp2Op::Mul | Fp2Op::Div | Fp2Op::Nmul)
        }
        Inst::FpUnary { op, ft, .. } => fp(ft) && matches!(op, Fp1Op::Mov | Fp1Op::Abs | Fp1Op::Neg | Fp1Op::Sqrt),
        Inst::FpCmp { ft, .. } | Inst::FpCondSel { ft, .. } | Inst::FpMovImm { ft, .. } => fp(ft),
        Inst::FpCvt { from, to, .. } => fp(from) && fp(to),
        Inst::IntToFp { ft, w, unsigned, fbits, .. } => fp(ft) && fbits == 0 && !(unsigned && w == Width::X),
        Inst::FpToInt { ft, unsigned, round, fbits, .. } => fp(ft) && fbits == 0 && !unsigned && round == FpRound::Zero,
        Inst::Vec3Same { op, size, .. } => match op {
            V3Op::And | V3Op::Orr | V3Op::Eor | V3Op::Bic | V3Op::Add | V3Op::Sub => true,
            V3Op::Cmeq => size < 3,
            _ => false,
        },
        Inst::Vec2Misc { op, size, scalar, .. } => !scalar && (op == V2Op::Not || (op == V2Op::CmeqZ && size < 3)),
        Inst::Movi { .. } | Inst::DupGen { .. } | Inst::MovToGen { .. } | Inst::InsGen { .. } => true,
        Inst::DupElem { scalar, .. } => !scalar,
        Inst::AddSubImm { .. }
        | Inst::AddSubShifted { .. }
        | Inst::AddSubExt { .. }
        | Inst::AddSubCarry { .. }
        | Inst::LogicalImm { .. }
        | Inst::LogicalShifted { .. }
        | Inst::MoveWide { .. }
        | Inst::Adr { .. }
        | Inst::Bitfield { .. }
        | Inst::Extr { .. }
        | Inst::CondCmp { .. }
        | Inst::CondSel { .. }
        | Inst::Madd { .. }
        | Inst::MaddLong { .. }
        | Inst::MulHigh { .. }
        | Inst::Div { .. }
        | Inst::ShiftVar { .. }
        | Inst::Branch { .. }
        | Inst::BranchCond { .. }
        | Inst::CompareBranch { .. }
        | Inst::TestBranch { .. }
        | Inst::BranchReg { .. }
        | Inst::BranchLinkReg { .. }
        | Inst::Ret { .. }
        | Inst::BranchAuth { .. }
        | Inst::Svc(_)
        | Inst::Pac { .. }
        | Inst::FmovToGen { .. }
        | Inst::FmovFromGen { .. } => true,
        Inst::Bit1 { op, .. } => matches!(op, Bit1Op::Rev16 | Bit1Op::Rev32 | Bit1Op::Rev64 | Bit1Op::Clz),
        Inst::Hint { crm_op2 } => crm_op2 != 7, // XPACLRI changes x30
        Inst::Barrier { kind, .. } => kind != poly_a64::Barrier::Clrex,
        Inst::LoadStore { kind, size, addr, .. } => {
            kind != MemKind::Prefetch
                || matches!(addr, Addr::Imm { .. } | Addr::Reg { .. } | Addr::Literal { .. }) && size <= 4
        }
        Inst::LoadStorePair { .. } => true,
        Inst::Atomic { op, .. } => matches!(op, AtomOp::Add | AtomOp::Swp),
        Inst::Cas { .. } => true,
        _ => false,
    }
}

/// Does the instruction end a block (a branch or a system call)?
fn ends_block(inst: &Inst) -> bool {
    matches!(
        inst,
        Inst::Branch { .. }
            | Inst::BranchCond { .. }
            | Inst::CompareBranch { .. }
            | Inst::TestBranch { .. }
            | Inst::BranchReg { .. }
            | Inst::BranchLinkReg { .. }
            | Inst::Ret { .. }
            | Inst::BranchAuth { .. }
            | Inst::Svc(_)
    )
}

/// (reads NZCV, writes NZCV, may leave the block in the middle)
fn flag_use(inst: &Inst) -> (bool, bool, bool) {
    match *inst {
        Inst::AddSubImm { s, .. } | Inst::AddSubShifted { s, .. } | Inst::AddSubExt { s, .. } => (false, s, false),
        Inst::AddSubCarry { s, .. } => (true, s, false),
        Inst::LogicalImm { op, .. } | Inst::LogicalShifted { op, .. } => (false, op == LogOp::Ands, false),
        Inst::CondCmp { .. } => (true, true, false),
        Inst::CondSel { .. } | Inst::BranchCond { .. } => (true, false, false),
        Inst::LoadStore { .. }
        | Inst::LoadStorePair { .. }
        | Inst::Atomic { .. }
        | Inst::Cas { .. }
        | Inst::Svc(_)
        | Inst::FpBinary { .. }
        | Inst::FpUnary { .. }
        | Inst::FpFused { .. }
        | Inst::FpCvt { .. }
        | Inst::FpToInt { .. } => (false, false, true),
        Inst::FpCmp { .. } => (false, true, true),
        Inst::FpCondSel { .. } => (true, false, false),
        _ => (false, false, false),
    }
}

/// The translator for one block.
struct Tr {
    a: Asm,
    exits: Vec<ExitSite>,
    /// The code-area offset of the exit routine.
    exit_routine: usize,
    /// What the host flags hold, if they are A64 flags right now.
    hflags: Option<HostFlags>,
    /// Out-of-line stubs to emit after the body: (label, guest pc).
    slow: Vec<(Label, u64)>,
}

/// Translate the block of decoded instructions `insts` (at most up to the
/// first one [`translatable`] rejects) starting at guest address `pc`.
/// `exit_routine` is the code-area offset of the routine that returns to the
/// run loop with `eax` as exit code. Returns `None` if the first instruction
/// cannot be translated.
pub fn translate(pc: u64, insts: &[Inst], exit_routine: usize, feat: Features) -> Option<Block> {
    let mut n = 0;
    while n < insts.len() && translatable_with(&insts[n], feat) {
        n += 1;
        if ends_block(&insts[n - 1]) {
            break;
        }
    }
    if n == 0 {
        return None;
    }
    let insts = &insts[..n];
    let ends_with_branch = ends_block(&insts[n - 1]);

    // Which flag results must be written to the Cpu (backwards liveness).
    let mut materialize = alloc::vec![false; n];
    let mut live = true;
    for i in (0..n).rev() {
        let (reads, writes, exits) = flag_use(&insts[i]);
        if i == n - 1 && ends_with_branch {
            // Live out at the block's exit.
            live = true;
        }
        if writes {
            materialize[i] = live;
            live = false;
        }
        if reads || exits {
            live = true;
        }
    }

    let mut t = Tr { a: Asm::new(), exits: Vec::new(), exit_routine, hflags: None, slow: Vec::new() };
    // Budget check at entry: leave with PC at the block when it runs out.
    {
        let a = &mut t.a;
        a.alu_mi(Sz::Q, Alu::Sub, mem(RCTX, ctx::BUDGET), 1);
        let ok = a.label();
        a.jcc(Cc::Ns, ok);
        a.mov_imm(R::Rax, pc);
        a.store(Sz::Q, mem(RCPU, opc()), R::Rax);
        a.mov_r32_imm(R::Rax, exit::NEXT);
        a.store_imm(Sz::D, mem(RCTX, ctx::SITE), -1);
        a.jmp_ext(exit_routine);
        a.bind(ok);
    }
    for (i, inst) in insts.iter().enumerate() {
        let ipc = pc + 4 * i as u64;
        t.inst(ipc, inst, materialize[i]);
    }
    if !ends_with_branch {
        // Fall through to the next instruction (or the untranslatable one).
        let next = pc + 4 * n as u64;
        t.direct_exit(next);
    }
    // Slow paths: interpret the instruction.
    for (l, ipc) in core::mem::take(&mut t.slow) {
        t.a.bind(l);
        t.a.mov_imm(R::Rax, ipc);
        t.a.store(Sz::Q, mem(RCPU, opc()), R::Rax);
        t.a.mov_r32_imm(R::Rax, exit::INTERPRET);
        t.a.jmp_ext(exit_routine);
    }
    t.a.finish();
    Some(Block { asm: t.a, start: pc, end: pc + 4 * n as u64, insts: n, exits: t.exits })
}

impl Tr {
    // ---- guest registers ----

    /// Load `Xr`/`Wr` (31 = zero register). Leaves the flags alone.
    fn rd(&mut self, dst: R, r: Reg, w: Width) {
        if r == 31 {
            self.a.mov_r32_imm(dst, 0);
        } else {
            self.a.load(sz(w), dst, mem(RCPU, ox(r)));
        }
    }

    /// Load `Xr`/`Wr` with 31 = SP.
    fn rd_sp(&mut self, dst: R, r: Reg, w: Width) {
        let off = if r == 31 { osp() } else { ox(r) };
        self.a.load(sz(w), dst, mem(RCPU, off));
    }

    /// Store `src` to `Xr` (31 = zero register: dropped). For `W` the value
    /// is zero-extended first. Leaves the flags alone.
    fn wr(&mut self, r: Reg, src: R, w: Width) {
        if r == 31 {
            return;
        }
        if w == Width::W {
            self.a.mov_rr(Sz::D, src, src);
        }
        self.a.store(Sz::Q, mem(RCPU, ox(r)), src);
    }

    /// Store `src` to `Xr` with 31 = SP.
    fn wr_sp(&mut self, r: Reg, src: R, w: Width) {
        if w == Width::W {
            self.a.mov_rr(Sz::D, src, src);
        }
        let off = if r == 31 { osp() } else { ox(r) };
        self.a.store(Sz::Q, mem(RCPU, off), src);
    }

    // ---- flags ----

    /// Write the host flags (from an operation of kind `k`) to the Cpu's
    /// NZCV, leaving the host flags as they are.
    fn materialize(&mut self, k: HostFlags) {
        let a = &mut self.a;
        a.setcc(Cc::S, R::R8);
        a.setcc(Cc::E, R::R9);
        a.setcc(if k == HostFlags::Sub { Cc::Ae } else { Cc::B }, R::R10);
        a.setcc(Cc::O, R::R11);
        for r in [R::R8, R::R9, R::R10, R::R11] {
            a.zx_rr(Sz::B, r, r);
        }
        // nibble = n*8 + z*4 + c*2 + v, with lea (no flag changes).
        a.lea(Sz::D, R::R11, M { base: R::R11, index: Some((R::R10, 1)), disp: 0 });
        a.lea(Sz::D, R::R11, M { base: R::R11, index: Some((R::R9, 2)), disp: 0 });
        a.lea(Sz::D, R::R11, M { base: R::R11, index: Some((R::R8, 3)), disp: 0 });
        for _ in 0..4 {
            a.lea(Sz::D, R::R11, M { base: R::R11, index: Some((R::R11, 0)), disp: 0 });
        }
        // NZCV is bits 31:28: the high nibble of the top byte; bits 27:0 are 0.
        a.store_imm(Sz::D, mem(RCPU, onzcv()), 0);
        a.store(Sz::B, mem(RCPU, onzcv() + 3), R::R11);
    }

    /// After a flag-setting operation of kind `k`: write NZCV if live, and
    /// remember that the host flags hold it.
    fn flags_set(&mut self, k: HostFlags, materialize: bool) {
        if materialize {
            self.materialize(k);
        }
        self.hflags = Some(k);
    }

    /// Set the host flags so that the returned condition code is true
    /// exactly when `cond` holds. `None`: always true. Uses `r10`, `r11`.
    fn cond(&mut self, cond: Cond, hf: Option<HostFlags>) -> Option<Cc> {
        if matches!(cond, Cond::Al | Cond::Nv) {
            return None;
        }
        if let Some(k) = hf {
            let cc = match (cond, k) {
                (Cond::Eq, _) => Some(Cc::E),
                (Cond::Ne, _) => Some(Cc::Ne),
                (Cond::Mi, _) => Some(Cc::S),
                (Cond::Pl, _) => Some(Cc::Ns),
                (Cond::Vs, _) => Some(Cc::O),
                (Cond::Vc, _) => Some(Cc::No),
                (Cond::Ge, _) => Some(Cc::Ge),
                (Cond::Lt, _) => Some(Cc::L),
                (Cond::Gt, _) => Some(Cc::G),
                (Cond::Le, _) => Some(Cc::Le),
                (Cond::Cs, HostFlags::Sub) => Some(Cc::Ae),
                (Cond::Cc, HostFlags::Sub) => Some(Cc::B),
                (Cond::Hi, HostFlags::Sub) => Some(Cc::A),
                (Cond::Ls, HostFlags::Sub) => Some(Cc::Be),
                (Cond::Cs, HostFlags::Add) => Some(Cc::B),
                (Cond::Cc, HostFlags::Add) => Some(Cc::Ae),
                _ => None,
            };
            if let Some(cc) = cc {
                return Some(cc);
            }
        }
        // From the Cpu's NZCV.
        let a = &mut self.a;
        a.load(Sz::D, R::R10, mem(RCPU, onzcv()));
        Some(match cond {
            Cond::Eq | Cond::Ne => {
                a.bt_ri(Sz::D, R::R10, 30);
                if cond == Cond::Eq { Cc::B } else { Cc::Ae }
            }
            Cond::Cs | Cond::Cc => {
                a.bt_ri(Sz::D, R::R10, 29);
                if cond == Cond::Cs { Cc::B } else { Cc::Ae }
            }
            Cond::Mi | Cond::Pl => {
                a.bt_ri(Sz::D, R::R10, 31);
                if cond == Cond::Mi { Cc::B } else { Cc::Ae }
            }
            Cond::Vs | Cond::Vc => {
                a.bt_ri(Sz::D, R::R10, 28);
                if cond == Cond::Vs { Cc::B } else { Cc::Ae }
            }
            Cond::Hi | Cond::Ls => {
                // bit 0 = C, bit 1 = Z: HI is exactly C=1, Z=0.
                a.shift_ri(Sz::D, Sh::Shr, R::R10, 29);
                a.alu_ri(Sz::D, Alu::And, R::R10, 3);
                a.alu_ri(Sz::D, Alu::Cmp, R::R10, 1);
                if cond == Cond::Hi { Cc::E } else { Cc::Ne }
            }
            Cond::Ge | Cond::Lt => {
                a.mov_rr(Sz::D, R::R11, R::R10);
                a.shift_ri(Sz::D, Sh::Shr, R::R11, 3);
                a.alu_rr(Sz::D, Alu::Xor, R::R10, R::R11);
                a.bt_ri(Sz::D, R::R10, 28); // CF = N xor V
                if cond == Cond::Ge { Cc::Ae } else { Cc::B }
            }
            _ => {
                // GT / LE: Z or (N xor V).
                a.mov_rr(Sz::D, R::R11, R::R10);
                a.shift_ri(Sz::D, Sh::Shr, R::R11, 3);
                a.alu_rr(Sz::D, Alu::Xor, R::R11, R::R10);
                a.shift_ri(Sz::D, Sh::Shr, R::R10, 2);
                a.alu_rr(Sz::D, Alu::Or, R::R10, R::R11);
                a.bt_ri(Sz::D, R::R10, 28);
                if cond == Cond::Gt { Cc::Ae } else { Cc::B }
            }
        })
    }

    // ---- exits ----

    /// Store `target` as PC and leave through a chainable jump.
    fn direct_exit(&mut self, target: u64) {
        let a = &mut self.a;
        a.mov_imm(R::Rax, target);
        a.store(Sz::Q, mem(RCPU, opc()), R::Rax);
        let stub = a.label();
        let jmp_field = a.pos() + 1;
        a.jmp(stub);
        // The stub is emitted right here, out of the fall-through path.
        a.bind(stub);
        let stub_off = a.pos();
        // mov dword [rbp + SITE], imm32 (patched with the site's offset)
        a.store_imm(Sz::D, mem(RCTX, ctx::SITE), 0);
        let report_field = a.pos() - 4;
        a.mov_r32_imm(R::Rax, exit::NEXT);
        a.jmp_ext(self.exit_routine);
        self.exits.push(ExitSite { jmp_field, stub: stub_off, report_field, target });
    }

    /// Leave to the address in `rax` (an indirect branch), through the
    /// indirect branch cache.
    fn indirect_exit(&mut self) {
        let a = &mut self.a;
        a.store(Sz::Q, mem(RCPU, opc()), R::Rax);
        // rcx = &ibtc[(pc >> 2) & mask]
        a.mov_rr(Sz::Q, R::Rcx, R::Rax);
        a.shift_ri(Sz::Q, Sh::Shr, R::Rcx, 2);
        a.alu_ri(Sz::D, Alu::And, R::Rcx, (1 << IBTC_BITS) - 1);
        a.shift_ri(Sz::D, Sh::Shl, R::Rcx, 4);
        a.alu_rm(Sz::Q, Alu::Add, R::Rcx, mem(RCTX, ctx::IBTC));
        a.alu_rm(Sz::Q, Alu::Cmp, R::Rax, mem(R::Rcx, 0));
        let miss = a.label();
        a.jcc(Cc::Ne, miss);
        // Budget: the target block checks it.
        a.jmp_m(mem(R::Rcx, 8));
        a.bind(miss);
        a.store_imm(Sz::D, mem(RCTX, ctx::SITE), -1);
        a.mov_r32_imm(R::Rax, exit::NEXT);
        a.jmp_ext(self.exit_routine);
    }

    // ---- memory ----

    /// Check guest address in `rax` for an access of `bytes` (`store`: a
    /// write) and leave `rcx` = its offset in the window. Jumps to the slow
    /// path (interpret this instruction) on any failure. Uses `rdx`.
    fn check(&mut self, pc: u64, bytes: u32, store: bool) {
        let slow = self.slow_label(pc);
        let a = &mut self.a;
        a.mov_rr(Sz::Q, R::Rcx, R::Rax);
        a.alu_rr(Sz::Q, Alu::Sub, R::Rcx, RLO);
        a.alu_rr(Sz::Q, Alu::Cmp, R::Rcx, RSIZE);
        a.jcc(Cc::Ae, slow);
        a.mov_rr(Sz::Q, R::Rdx, R::Rcx);
        a.shift_ri(Sz::Q, Sh::Shr, R::Rdx, crate::mem::PAGE_SHIFT as u8);
        a.load_zx(Sz::B, R::Rdx, mem_idx(RFLAGS, R::Rdx, 0));
        if store {
            a.alu_ri(Sz::D, Alu::And, R::Rdx, i32::from(PW | CODE));
            a.alu_ri(Sz::D, Alu::Cmp, R::Rdx, i32::from(PW));
            a.jcc(Cc::Ne, slow);
        } else {
            a.test_ri(Sz::B, R::Rdx, i32::from(PR));
            a.jcc(Cc::E, slow);
        }
        if bytes > 1 {
            a.mov_rr(Sz::D, R::Rdx, R::Rcx);
            a.alu_ri(Sz::D, Alu::And, R::Rdx, (PAGE_SIZE - 1) as i32);
            a.alu_ri(Sz::D, Alu::Cmp, R::Rdx, (PAGE_SIZE - u64::from(bytes)) as i32);
            a.jcc(Cc::A, slow);
        }
        self.hflags = None;
    }

    fn slow_label(&mut self, pc: u64) -> Label {
        if let Some(&(l, _)) = self.slow.iter().find(|(_, p)| *p == pc) {
            return l;
        }
        let l = self.a.label();
        self.slow.push((l, pc));
        l
    }

    /// The effective address of `addr` into `rax`; returns the writeback
    /// (base register, value register) if any. The value of a writeback is
    /// left in `rsi`.
    fn address(&mut self, pc: u64, addr: Addr) -> Option<Reg> {
        match addr {
            Addr::Imm { rn, offset } => {
                self.rd_sp(R::Rax, rn, Width::X);
                self.add_imm(R::Rax, offset);
                None
            }
            Addr::Pre { rn, offset } => {
                self.rd_sp(R::Rax, rn, Width::X);
                self.add_imm(R::Rax, offset);
                self.a.mov_rr(Sz::Q, R::Rsi, R::Rax);
                Some(rn)
            }
            Addr::Post { rn, offset } => {
                self.rd_sp(R::Rax, rn, Width::X);
                self.a.mov_rr(Sz::Q, R::Rsi, R::Rax);
                self.add_imm(R::Rsi, offset);
                Some(rn)
            }
            Addr::Reg { rn, rm, ext, shift } => {
                self.rd_sp(R::Rax, rn, Width::X);
                self.rd(R::Rsi, rm, Width::X);
                self.extend(R::Rsi, ext, shift);
                self.a.alu_rr(Sz::Q, Alu::Add, R::Rax, R::Rsi);
                None
            }
            Addr::Literal { offset } => {
                self.a.mov_imm(R::Rax, pc.wrapping_add(offset as u64));
                None
            }
        }
    }

    fn add_imm(&mut self, r: R, v: i64) {
        if v == 0 {
            return;
        }
        if let Ok(i) = i32::try_from(v) {
            self.a.alu_ri(Sz::Q, Alu::Add, r, i);
        } else {
            self.a.mov_imm(R::Rdx, v as u64);
            self.a.alu_rr(Sz::Q, Alu::Add, r, R::Rdx);
        }
    }

    /// Apply an `Extend` and a left shift to the 64-bit value in `r`.
    fn extend(&mut self, r: R, ext: Extend, shift: u8) {
        match ext {
            Extend::Uxtb => self.a.zx_rr(Sz::B, r, r),
            Extend::Uxth => self.a.zx_rr(Sz::H, r, r),
            Extend::Uxtw => self.a.mov_rr(Sz::D, r, r),
            Extend::Sxtb => self.a.sx_rr(Sz::B, Sz::Q, r, r),
            Extend::Sxth => self.a.sx_rr(Sz::H, Sz::Q, r, r),
            Extend::Sxtw => self.a.sx_rr(Sz::D, Sz::Q, r, r),
            Extend::Uxtx | Extend::Sxtx => {}
        }
        if shift != 0 {
            self.a.shift_ri(Sz::Q, Sh::Shl, r, shift);
        }
    }

    /// Host memory operand of the checked guest access (offset in `rcx`).
    fn hmem(disp: i32) -> M {
        mem_idx(RHOST, R::Rcx, disp)
    }

    // ---- shifted operands ----

    fn shift(&mut self, r: R, w: Width, shift: Shift, amount: u8) {
        if amount == 0 {
            return;
        }
        let op = match shift {
            Shift::Lsl => Sh::Shl,
            Shift::Lsr => Sh::Shr,
            Shift::Asr => Sh::Sar,
            Shift::Ror => Sh::Ror,
        };
        self.a.shift_ri(sz(w), op, r, amount);
    }

    // ---- instructions ----

    fn inst(&mut self, pc: u64, inst: &Inst, mat: bool) {
        let hf = self.hflags.take();
        match *inst {
            Inst::AddSubImm { w, sub, s, rd, rn, imm } => {
                self.rd_sp(R::Rax, rn, w);
                self.a.alu_ri(sz(w), if sub { Alu::Sub } else { Alu::Add }, R::Rax, imm as i32);
                self.addsub_result(w, sub, s, rd, true, mat);
            }
            Inst::AddSubShifted { w, sub, s, rd, rn, rm, shift, amount } => {
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                self.shift(R::Rcx, w, shift, amount);
                self.a.alu_rr(sz(w), if sub { Alu::Sub } else { Alu::Add }, R::Rax, R::Rcx);
                self.addsub_result(w, sub, s, rd, false, mat);
            }
            Inst::AddSubExt { w, sub, s, rd, rn, rm, ext, amount } => {
                self.rd_sp(R::Rax, rn, w);
                self.rd(R::Rcx, rm, Width::X);
                self.extend(R::Rcx, ext, amount);
                self.a.alu_rr(sz(w), if sub { Alu::Sub } else { Alu::Add }, R::Rax, R::Rcx);
                self.addsub_result(w, sub, s, rd, true, mat);
            }
            Inst::AddSubCarry { w, sub, s, rd, rn, rm } => {
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                // CF = C (adc) or NOT C (sbb: borrow).
                match hf {
                    Some(HostFlags::Add) if !sub => {}
                    Some(HostFlags::Sub) if sub => {}
                    _ => {
                        self.a.bt_mi(Sz::D, mem(RCPU, onzcv()), 29);
                        if sub {
                            self.a.cmc();
                        }
                    }
                }
                self.a.alu_rr(sz(w), if sub { Alu::Sbb } else { Alu::Adc }, R::Rax, R::Rcx);
                self.wr(rd, R::Rax, w);
                if s {
                    self.flags_set(if sub { HostFlags::Sub } else { HostFlags::Add }, mat);
                }
            }
            Inst::LogicalImm { w, op, rd, rn, imm } => {
                self.rd(R::Rax, rn, w);
                self.a.mov_imm(R::Rcx, imm);
                self.logical(w, op, rd, mat, true);
            }
            Inst::LogicalShifted { w, op, invert, rd, rn, rm, shift, amount } => {
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                self.shift(R::Rcx, w, shift, amount);
                if invert {
                    self.a.not(sz(w), R::Rcx);
                }
                self.logical(w, op, rd, mat, false);
            }
            Inst::MoveWide { w, op, rd, imm, shift } => {
                let v = u64::from(imm) << shift;
                match op {
                    MovOp::Movz => self.a.mov_imm(R::Rax, v),
                    MovOp::Movn => self.a.mov_imm(R::Rax, !v & w.mask()),
                    MovOp::Movk => {
                        self.rd(R::Rax, rd, w);
                        self.a.mov_imm(R::Rcx, !(0xFFFFu64 << shift));
                        self.a.alu_rr(Sz::Q, Alu::And, R::Rax, R::Rcx);
                        if v != 0 {
                            self.a.mov_imm(R::Rcx, v);
                            self.a.alu_rr(Sz::Q, Alu::Or, R::Rax, R::Rcx);
                        }
                    }
                }
                self.wr(rd, R::Rax, w);
            }
            Inst::Adr { rd, offset, page } => {
                let base = if page { pc & !0xFFF } else { pc };
                self.a.mov_imm(R::Rax, base.wrapping_add(offset as u64));
                self.wr(rd, R::Rax, Width::X);
            }
            Inst::Bitfield { w, op, rd, rn, immr, imms, wmask, tmask } => {
                self.bitfield(w, op, rd, rn, immr, imms, wmask, tmask)
            }
            Inst::Extr { w, rd, rn, rm, lsb } => {
                self.rd(R::Rax, rm, w);
                if lsb != 0 {
                    self.rd(R::Rcx, rn, w);
                    self.a.shift_ri(sz(w), Sh::Shr, R::Rax, lsb);
                    self.a.shift_ri(sz(w), Sh::Shl, R::Rcx, w.bits() as u8 - lsb);
                    self.a.alu_rr(sz(w), Alu::Or, R::Rax, R::Rcx);
                }
                self.wr(rd, R::Rax, w);
            }
            Inst::CondCmp { w, neg, rn, rhs, cond, nzcv } => {
                let done = self.a.label();
                let other = self.a.label();
                let cc = self.cond(cond, hf);
                if let Some(cc) = cc {
                    self.a.jcc(cc.not(), other);
                }
                self.rd(R::Rax, rn, w);
                match rhs {
                    CmpRhs::Reg(rm) => self.rd(R::Rcx, rm, w),
                    CmpRhs::Imm(i) => self.a.mov_r32_imm(R::Rcx, u32::from(i)),
                }
                self.a.alu_rr(sz(w), if neg { Alu::Add } else { Alu::Sub }, R::Rax, R::Rcx);
                self.materialize(if neg { HostFlags::Add } else { HostFlags::Sub });
                self.a.jmp(done);
                self.a.bind(other);
                self.a.store_imm(Sz::D, mem(RCPU, onzcv()), (u32::from(nzcv) << 28) as i32);
                self.a.bind(done);
                let _ = mat;
            }
            Inst::CondSel { w, op, rd, rn, rm, cond } => {
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                match op {
                    CselOp::Csel => {}
                    CselOp::Csinc => self.a.lea(sz(w), R::Rcx, mem(R::Rcx, 1)),
                    CselOp::Csinv => self.a.not(sz(w), R::Rcx),
                    CselOp::Csneg => {
                        self.a.not(sz(w), R::Rcx);
                        self.a.lea(sz(w), R::Rcx, mem(R::Rcx, 1));
                    }
                }
                if let Some(cc) = self.cond(cond, hf) {
                    self.a.cmov(Sz::Q, cc.not(), R::Rax, R::Rcx);
                }
                self.wr(rd, R::Rax, w);
                self.hflags = hf;
            }
            Inst::Madd { w, sub, rd, rn, rm, ra } => {
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                self.a.imul(sz(w), R::Rax, R::Rcx);
                self.rd(R::Rcx, ra, w);
                if sub {
                    self.a.alu_rr(sz(w), Alu::Sub, R::Rcx, R::Rax);
                    self.wr(rd, R::Rcx, w);
                } else {
                    self.a.alu_rr(sz(w), Alu::Add, R::Rax, R::Rcx);
                    self.wr(rd, R::Rax, w);
                }
            }
            Inst::MaddLong { signed, sub, rd, rn, rm, ra } => {
                self.rd(R::Rax, rn, Width::W);
                self.rd(R::Rcx, rm, Width::W);
                if signed {
                    self.a.sx_rr(Sz::D, Sz::Q, R::Rax, R::Rax);
                    self.a.sx_rr(Sz::D, Sz::Q, R::Rcx, R::Rcx);
                }
                self.a.imul(Sz::Q, R::Rax, R::Rcx);
                self.rd(R::Rcx, ra, Width::X);
                if sub {
                    self.a.alu_rr(Sz::Q, Alu::Sub, R::Rcx, R::Rax);
                    self.wr(rd, R::Rcx, Width::X);
                } else {
                    self.a.alu_rr(Sz::Q, Alu::Add, R::Rax, R::Rcx);
                    self.wr(rd, R::Rax, Width::X);
                }
            }
            Inst::MulHigh { signed, rd, rn, rm } => {
                self.rd(R::Rax, rn, Width::X);
                self.rd(R::Rcx, rm, Width::X);
                if signed {
                    self.a.imul1(Sz::Q, R::Rcx);
                } else {
                    self.a.mul(Sz::Q, R::Rcx);
                }
                self.wr(rd, R::Rdx, Width::X);
            }
            Inst::Div { w, signed, rd, rn, rm } => {
                let s = sz(w);
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                let done = self.a.label();
                let zero = self.a.label();
                self.a.test_rr(s, R::Rcx, R::Rcx);
                self.a.jcc(Cc::E, zero);
                if signed {
                    let normal = self.a.label();
                    self.a.alu_ri(s, Alu::Cmp, R::Rcx, -1);
                    self.a.jcc(Cc::Ne, normal);
                    // x / -1 = -x (and INT_MIN / -1 = INT_MIN), where idiv would trap.
                    self.a.neg(s, R::Rax);
                    self.a.jmp(done);
                    self.a.bind(normal);
                    self.a.cqo(s);
                    self.a.idiv(s, R::Rcx);
                } else {
                    self.a.mov_r32_imm(R::Rdx, 0);
                    self.a.div(s, R::Rcx);
                }
                self.a.jmp(done);
                self.a.bind(zero);
                self.a.mov_r32_imm(R::Rax, 0);
                self.a.bind(done);
                self.wr(rd, R::Rax, w);
            }
            Inst::ShiftVar { w, op, rd, rn, rm } => {
                self.rd(R::Rax, rn, w);
                self.rd(R::Rcx, rm, w);
                let o = match op {
                    Shift::Lsl => Sh::Shl,
                    Shift::Lsr => Sh::Shr,
                    Shift::Asr => Sh::Sar,
                    Shift::Ror => Sh::Ror,
                };
                // x86 masks the count to 5 or 6 bits, as A64 takes it modulo the width.
                self.a.shift_cl(sz(w), o, R::Rax);
                self.wr(rd, R::Rax, w);
            }
            Inst::Bit1 { w, op, rd, rn } => {
                self.rd(R::Rax, rn, w);
                match op {
                    Bit1Op::Rev64 => self.a.bswap(Sz::Q, R::Rax),
                    Bit1Op::Rev32 => {
                        if w == Width::W {
                            self.a.bswap(Sz::D, R::Rax);
                        } else {
                            // Swap the bytes of each word: bswap all, then swap the words.
                            self.a.bswap(Sz::Q, R::Rax);
                            self.a.shift_ri(Sz::Q, Sh::Ror, R::Rax, 32);
                        }
                    }
                    Bit1Op::Rev16 => {
                        // ((x >> 8) & 0x00FF..) | ((x & 0x00FF..) << 8)
                        let m = if w == Width::W { 0x00FF_00FFu64 } else { 0x00FF_00FF_00FF_00FF };
                        self.a.mov_imm(R::Rdx, m);
                        self.a.mov_rr(Sz::Q, R::Rcx, R::Rax);
                        self.a.shift_ri(Sz::Q, Sh::Shr, R::Rax, 8);
                        self.a.alu_rr(Sz::Q, Alu::And, R::Rax, R::Rdx);
                        self.a.alu_rr(Sz::Q, Alu::And, R::Rcx, R::Rdx);
                        self.a.shift_ri(Sz::Q, Sh::Shl, R::Rcx, 8);
                        self.a.alu_rr(Sz::Q, Alu::Or, R::Rax, R::Rcx);
                    }
                    _ => {
                        // CLZ: width - 1 - bsr(x), and width for 0.
                        let s = sz(w);
                        self.a.mov_imm(R::Rdx, u64::MAX);
                        self.a.bsr(s, R::Rcx, R::Rax);
                        self.a.cmov(Sz::Q, Cc::E, R::Rcx, R::Rdx);
                        self.a.mov_r32_imm(R::Rax, w.bits() - 1);
                        self.a.alu_rr(Sz::Q, Alu::Sub, R::Rax, R::Rcx);
                    }
                }
                self.wr(rd, R::Rax, w);
            }

            // ---- branches ----
            Inst::Branch { link, offset } => {
                if link {
                    self.a.mov_imm(R::Rax, pc + 4);
                    self.a.store(Sz::Q, mem(RCPU, ox(30)), R::Rax);
                }
                self.direct_exit(pc.wrapping_add(offset as u64));
            }
            Inst::BranchCond { cond, offset } => {
                let target = pc.wrapping_add(offset as u64);
                match self.cond(cond, hf) {
                    None => self.direct_exit(target),
                    Some(cc) => self.cond_exits(cc, target, pc + 4),
                }
            }
            Inst::CompareBranch { w, nz, rt, offset } => {
                self.rd(R::Rax, rt, w);
                self.a.test_rr(sz(w), R::Rax, R::Rax);
                self.cond_exits(if nz { Cc::Ne } else { Cc::E }, pc.wrapping_add(offset as u64), pc + 4);
            }
            Inst::TestBranch { nz, rt, bit, offset } => {
                self.rd(R::Rax, rt, Width::X);
                self.a.bt_ri(Sz::Q, R::Rax, bit);
                self.cond_exits(if nz { Cc::B } else { Cc::Ae }, pc.wrapping_add(offset as u64), pc + 4);
            }
            Inst::BranchReg { rn } | Inst::Ret { rn } | Inst::BranchAuth { link: false, rn, .. } => {
                self.rd(R::Rax, rn, Width::X);
                self.indirect_exit();
            }
            Inst::BranchLinkReg { rn } | Inst::BranchAuth { link: true, rn, .. } => {
                self.rd(R::Rax, rn, Width::X);
                self.a.mov_imm(R::Rcx, pc + 4);
                self.a.store(Sz::Q, mem(RCPU, ox(30)), R::Rcx);
                self.indirect_exit();
            }
            Inst::Svc(imm) => {
                self.a.mov_imm(R::Rax, pc + 4);
                self.a.store(Sz::Q, mem(RCPU, opc()), R::Rax);
                self.a.store_imm(Sz::D, mem(RCTX, ctx::ARG), i32::from(imm));
                self.a.mov_r32_imm(R::Rax, exit::SVC);
                self.a.jmp_ext(self.exit_routine);
            }
            Inst::Hint { .. } | Inst::Barrier { .. } | Inst::Pac { .. } => {
                // No effect (keys disabled; x86's memory order is stronger than A64's
                // barriers ask for). Keep the host flags.
                self.hflags = hf;
            }
            Inst::FmovToGen { w, top, rd, rn } => {
                let off = ov(rn) + if top { 8 } else { 0 };
                self.a.load(sz(w), R::Rax, mem(RCPU, off));
                self.wr(rd, R::Rax, w);
            }
            Inst::FmovFromGen { w, top, rd, rn } => {
                self.rd(R::Rax, rn, w);
                if top {
                    self.a.store(Sz::Q, mem(RCPU, ov(rd) + 8), R::Rax);
                } else {
                    self.a.store(Sz::Q, mem(RCPU, ov(rd)), R::Rax);
                    self.a.store_imm(Sz::Q, mem(RCPU, ov(rd) + 8), 0);
                }
            }

            // ---- memory ----
            Inst::LoadStore { kind, size, fp, rt, addr } => self.load_store(pc, kind, size, fp, rt, addr),
            Inst::LoadStorePair { kind, size, fp, rt, rt2, addr } => self.pair(pc, kind, size, fp, rt, rt2, addr),
            Inst::Atomic { op, size, rs, rt, rn, .. } => {
                let bytes = 1u32 << size;
                self.rd_sp(R::Rax, rn, Width::X);
                self.align16(pc, bytes);
                self.check(pc, bytes, true);
                let s = size_sz(size);
                self.rd(R::Rax, rs, Width::X);
                match op {
                    AtomOp::Add => self.a.lock_xadd(s, Self::hmem(0), R::Rax),
                    _ => self.a.xchg_mr(s, Self::hmem(0), R::Rax),
                }
                if size < 2 {
                    self.a.zx_rr(s, R::Rax, R::Rax);
                }
                self.wr(rt, R::Rax, if size == 3 { Width::X } else { Width::W });
            }
            Inst::Cas { size, rs, rt, rn, .. } => {
                let bytes = 1u32 << size;
                self.rd_sp(R::Rax, rn, Width::X);
                self.align16(pc, bytes);
                self.check(pc, bytes, true);
                let s = size_sz(size);
                self.rd(R::Rax, rs, Width::X);
                self.rd(R::Rdx, rt, Width::X);
                self.a.lock_cmpxchg(s, Self::hmem(0), R::Rdx);
                if size < 2 {
                    self.a.zx_rr(s, R::Rax, R::Rax);
                }
                self.wr(rs, R::Rax, if size == 3 { Width::X } else { Width::W });
            }
            Inst::Crc32 { size, rd, rn, rm, .. } => {
                self.rd(R::Rax, rn, Width::W);
                self.rd(R::Rcx, rm, Width::X);
                self.a.crc32(size_sz(size), R::Rax, R::Rcx);
                self.wr(rd, R::Rax, Width::W);
            }
            _ => {
                if !self.fp_simd(pc, inst) {
                    unreachable!("translatable() admitted {inst:?}");
                }
            }
        }
    }

    // ---- floating point and SIMD ----

    fn fp_load(&mut self, x: Xmm, r: Reg, ft: FpType) {
        if ft == FpType::D {
            self.a.movsd_load(x, mem(RCPU, ov(r)));
        } else {
            self.a.movss_load(x, mem(RCPU, ov(r)));
        }
    }

    /// Write the scalar in `x` to `Vd` (zeroing the rest), unless it is an
    /// infinity or a NaN: then interpret the instruction (A64's NaN rules
    /// and exception flags).
    fn fp_store_checked(&mut self, pc: u64, x: Xmm, rd: Reg, ft: FpType) {
        let slow = self.slow_label(pc);
        let a = &mut self.a;
        if ft == FpType::D {
            a.movq_rx(R::Rax, x);
            a.mov_rr(Sz::Q, R::Rcx, R::Rax);
            a.shift_ri(Sz::Q, Sh::Shl, R::Rcx, 1);
            a.shift_ri(Sz::Q, Sh::Shr, R::Rcx, 53);
            a.alu_ri(Sz::D, Alu::Cmp, R::Rcx, 0x7FF);
        } else {
            a.movd_rx(R::Rax, x);
            a.mov_rr(Sz::D, R::Rcx, R::Rax);
            a.shift_ri(Sz::D, Sh::Shr, R::Rcx, 23);
            a.alu_ri(Sz::D, Alu::And, R::Rcx, 0xFF);
            a.alu_ri(Sz::D, Alu::Cmp, R::Rcx, 0xFF);
        }
        a.jcc(Cc::E, slow);
        self.v_write64(rd, R::Rax);
    }

    /// `Vd` = the 64-bit value in `r`, upper half zero.
    fn v_write64(&mut self, rd: Reg, r: R) {
        self.a.store(Sz::Q, mem(RCPU, ov(rd)), r);
        self.a.store_imm(Sz::Q, mem(RCPU, ov(rd) + 8), 0);
    }

    /// The low 64 bits of `Vn` replicated from an element value in `rax`
    /// (`size`: log2 bytes), in `rax`.
    fn replicate(&mut self, size: u8) {
        let m = match size {
            0 => {
                self.a.zx_rr(Sz::B, R::Rax, R::Rax);
                0x0101_0101_0101_0101u64
            }
            1 => {
                self.a.zx_rr(Sz::H, R::Rax, R::Rax);
                0x0001_0001_0001_0001
            }
            2 => {
                self.a.mov_rr(Sz::D, R::Rax, R::Rax);
                0x0000_0001_0000_0001
            }
            _ => return,
        };
        self.a.mov_imm(R::Rcx, m);
        self.a.imul(Sz::Q, R::Rax, R::Rcx);
    }

    /// Store a 64-bit pattern in `rax` to `Vd`'s low half and, with `q`, its
    /// high half too (else zero).
    fn v_write_rep(&mut self, rd: Reg, q: bool) {
        self.a.store(Sz::Q, mem(RCPU, ov(rd)), R::Rax);
        if q {
            self.a.store(Sz::Q, mem(RCPU, ov(rd) + 8), R::Rax);
        } else {
            self.a.store_imm(Sz::Q, mem(RCPU, ov(rd) + 8), 0);
        }
    }

    /// Store `xmm0` to `Vd`, all 128 bits or (`!q`) the low half and zeros.
    fn v_store_xmm0(&mut self, rd: Reg, q: bool) {
        if q {
            self.a.movdqu_store(mem(RCPU, ov(rd)), Xmm(0));
        } else {
            self.a.movq_rx(R::Rax, Xmm(0));
            self.v_write64(rd, R::Rax);
        }
    }

    /// The floating-point and SIMD instructions; false if not handled.
    fn fp_simd(&mut self, pc: u64, inst: &Inst) -> bool {
        let (x0, x1, x2) = (Xmm(0), Xmm(1), Xmm(2));
        match *inst {
            Inst::FpBinary { op, ft, rd, rn, rm } => {
                let d = ft == FpType::D;
                self.fp_load(x0, rn, ft);
                self.fp_load(x1, rm, ft);
                let code = match op {
                    Fp2Op::Add => 0x58,
                    Fp2Op::Sub => 0x5C,
                    Fp2Op::Mul | Fp2Op::Nmul => 0x59,
                    _ => 0x5E,
                };
                self.a.sse_scalar(d, code, x0, x1);
                if op == Fp2Op::Nmul {
                    // Negate after the check: flip the sign bit in the result word.
                    self.fp_store_checked(pc, x0, rd, ft);
                    let bit = if d { 63 } else { 31 };
                    self.a.mov_imm(R::Rcx, 1u64 << bit);
                    self.a.alu_mr(Sz::Q, Alu::Xor, mem(RCPU, ov(rd)), R::Rcx);
                } else {
                    self.fp_store_checked(pc, x0, rd, ft);
                }
            }
            Inst::FpUnary { op, ft, rd, rn } => {
                let d = ft == FpType::D;
                match op {
                    Fp1Op::Sqrt => {
                        self.fp_load(x0, rn, ft);
                        self.a.sse_scalar(d, 0x51, x0, x0);
                        self.fp_store_checked(pc, x0, rd, ft);
                    }
                    _ => {
                        let s = if d { Sz::Q } else { Sz::D };
                        self.a.load(s, R::Rax, mem(RCPU, ov(rn)));
                        let sign = if d { 1u64 << 63 } else { 1 << 31 };
                        match op {
                            Fp1Op::Abs => {
                                self.a.mov_imm(R::Rcx, !sign & if d { u64::MAX } else { 0xFFFF_FFFF });
                                self.a.alu_rr(s, Alu::And, R::Rax, R::Rcx);
                            }
                            Fp1Op::Neg => {
                                self.a.mov_imm(R::Rcx, sign);
                                self.a.alu_rr(s, Alu::Xor, R::Rax, R::Rcx);
                            }
                            _ => {}
                        }
                        self.v_write64(rd, R::Rax);
                    }
                }
            }
            Inst::FpFused { ft, neg_p, neg_a, rd, rn, rm, ra } => {
                let d = ft == FpType::D;
                self.fp_load(x0, ra, ft);
                self.fp_load(x1, rn, ft);
                self.fp_load(x2, rm, ft);
                let op = match (neg_a, neg_p) {
                    (false, false) => 0xB9, // a + n*m
                    (false, true) => 0xBD,  // a - n*m
                    (true, true) => 0xBF,   // -a - n*m
                    (true, false) => 0xBB,  // n*m - a
                };
                self.a.vfma231(d, op, x0, x1, x2);
                self.fp_store_checked(pc, x0, rd, ft);
            }
            Inst::FpCmp { ft, zero, rn, rm, .. } => {
                let d = ft == FpType::D;
                let slow = self.slow_label(pc);
                self.fp_load(x0, rn, ft);
                if zero {
                    self.a.sse_packed(0xEF, x1, x1); // pxor: +0.0
                } else {
                    self.fp_load(x1, rm, ft);
                }
                self.a.ucomis(d, x0, x1);
                // Unordered: the interpreter (FPSR.IOC rules of FCMP and FCMPE).
                self.a.jcc(Cc::P, slow);
                // Greater: 0010, less: 1000, equal: 0110.
                self.a.mov_r32_imm(R::Rax, 0x2000_0000);
                self.a.mov_r32_imm(R::Rcx, 0x8000_0000);
                self.a.cmov(Sz::D, Cc::B, R::Rax, R::Rcx);
                self.a.mov_r32_imm(R::Rcx, 0x6000_0000);
                self.a.cmov(Sz::D, Cc::E, R::Rax, R::Rcx);
                self.a.store(Sz::D, mem(RCPU, onzcv()), R::Rax);
            }
            Inst::FpCondSel { ft, rd, rn, rm, cond } => {
                let s = if ft == FpType::D { Sz::Q } else { Sz::D };
                self.a.load(s, R::Rax, mem(RCPU, ov(rn)));
                self.a.load(s, R::Rcx, mem(RCPU, ov(rm)));
                if let Some(cc) = self.cond(cond, None) {
                    self.a.cmov(Sz::Q, cc.not(), R::Rax, R::Rcx);
                }
                self.v_write64(rd, R::Rax);
            }
            Inst::FpMovImm { rd, bits, .. } => {
                self.a.mov_imm(R::Rax, bits);
                self.v_write64(rd, R::Rax);
            }
            Inst::FpCvt { from, to, rd, rn } => {
                self.fp_load(x1, rn, from);
                self.a.cvt_fp(to == FpType::S, x0, x1);
                self.fp_store_checked(pc, x0, rd, to);
            }
            Inst::IntToFp { ft, w, unsigned, rd, rn, .. } => {
                self.rd(R::Rax, rn, w);
                if w == Width::W && !unsigned {
                    self.a.sx_rr(Sz::D, Sz::Q, R::Rax, R::Rax);
                }
                // A W source is now a non-negative or sign-extended 64-bit value.
                self.a.cvtsi2(ft == FpType::D, true, x0, R::Rax);
                if ft == FpType::D {
                    self.a.movq_rx(R::Rax, x0);
                } else {
                    self.a.movd_rx(R::Rax, x0);
                }
                self.v_write64(rd, R::Rax);
            }
            Inst::FpToInt { ft, w, rd, rn, .. } => {
                let slow = self.slow_label(pc);
                self.fp_load(x0, rn, ft);
                let w64 = w == Width::X;
                self.a.cvtt2si(ft == FpType::D, w64, R::Rax, x0);
                // The "integer indefinite" value: NaN or out of range (A64
                // saturates); also a real minimum, which the interpreter gives too.
                if w64 {
                    self.a.mov_imm(R::Rcx, 1 << 63);
                    self.a.alu_rr(Sz::Q, Alu::Cmp, R::Rax, R::Rcx);
                } else {
                    self.a.alu_ri(Sz::D, Alu::Cmp, R::Rax, i32::MIN);
                }
                self.a.jcc(Cc::E, slow);
                self.wr(rd, R::Rax, w);
            }
            Inst::Vec3Same { op, q, size, scalar, rd, rn, rm } => {
                let q = q && !scalar;
                let (a, b) = if op == V3Op::Bic { (rm, rn) } else { (rn, rm) };
                self.a.movdqu_load(x0, mem(RCPU, ov(a)));
                self.a.movdqu_load(x1, mem(RCPU, ov(b)));
                let code = match op {
                    V3Op::And => 0xDB,
                    V3Op::Orr => 0xEB,
                    V3Op::Eor => 0xEF,
                    V3Op::Bic => 0xDF, // pandn: NOT xmm0 AND xmm1 = n AND NOT m
                    V3Op::Add => [0xFC, 0xFD, 0xFE, 0xD4][size as usize],
                    V3Op::Sub => [0xF8, 0xF9, 0xFA, 0xFB][size as usize],
                    _ => [0x74, 0x75, 0x76][size as usize], // pcmpeq
                };
                self.a.sse_packed(code, x0, x1);
                self.v_store_xmm0(rd, q);
            }
            Inst::Vec2Misc { op, q, size, rd, rn, .. } => {
                self.a.movdqu_load(x0, mem(RCPU, ov(rn)));
                if op == V2Op::Not {
                    self.a.sse_packed(0x76, x1, x1); // all ones
                    self.a.sse_packed(0xEF, x0, x1);
                } else {
                    self.a.sse_packed(0xEF, x1, x1); // zero
                    self.a.sse_packed([0x74, 0x75, 0x76][size as usize], x0, x1);
                }
                self.v_store_xmm0(rd, q);
            }
            Inst::Movi { q, rd, imm } => {
                self.a.mov_imm(R::Rax, imm);
                self.v_write_rep(rd, q);
            }
            Inst::DupGen { q, size, rd, rn } => {
                self.rd(R::Rax, rn, Width::X);
                self.replicate(size);
                self.v_write_rep(rd, q);
            }
            Inst::DupElem { q, size, rd, rn, index, .. } => {
                let off = ov(rn) + (i32::from(index) << size);
                self.load_zx(size, R::Rax, mem(RCPU, off));
                self.replicate(size);
                self.v_write_rep(rd, q);
            }
            Inst::MovToGen { signed, w, size, rd, rn, index } => {
                let off = ov(rn) + (i32::from(index) << size);
                if signed {
                    self.a.load_sx(size_sz(size), sz(w), R::Rax, mem(RCPU, off));
                } else {
                    self.load_zx(size, R::Rax, mem(RCPU, off));
                }
                self.wr(rd, R::Rax, w);
            }
            Inst::InsGen { size, rd, rn, index } => {
                self.rd(R::Rax, rn, Width::X);
                let off = ov(rd) + (i32::from(index) << size);
                self.a.store(size_sz(size), mem(RCPU, off), R::Rax);
            }
            _ => return false,
        }
        true
    }

    /// Alignment rule of atomics (FEAT_LSE2): within one 16-byte block, or
    /// the interpreter raises the fault. Address in `rax`.
    fn align16(&mut self, pc: u64, bytes: u32) {
        if bytes == 1 {
            return;
        }
        let slow = self.slow_label(pc);
        self.a.mov_rr(Sz::D, R::Rdx, R::Rax);
        self.a.alu_ri(Sz::D, Alu::And, R::Rdx, 15);
        self.a.alu_ri(Sz::D, Alu::Cmp, R::Rdx, 16 - bytes as i32);
        self.a.jcc(Cc::A, slow);
    }

    /// Two exits on host condition `cc`: taken to `t`, else to `f`.
    fn cond_exits(&mut self, cc: Cc, t: u64, f: u64) {
        let taken = self.a.label();
        self.a.jcc(cc, taken);
        self.direct_exit(f);
        self.a.bind(taken);
        self.direct_exit(t);
    }

    /// Result of an add/sub in `rax`: write it and the flags.
    fn addsub_result(&mut self, w: Width, sub: bool, s: bool, rd: Reg, sp_dest: bool, mat: bool) {
        if s {
            self.wr(rd, R::Rax, w);
            self.flags_set(if sub { HostFlags::Sub } else { HostFlags::Add }, mat);
        } else if sp_dest {
            self.wr_sp(rd, R::Rax, w);
        } else {
            self.wr(rd, R::Rax, w);
        }
    }

    /// `rax op rcx`, written to `rd` (`SP` for 31 when `imm_form` and not `ANDS`).
    fn logical(&mut self, w: Width, op: LogOp, rd: Reg, mat: bool, imm_form: bool) {
        let o = match op {
            LogOp::And | LogOp::Ands => Alu::And,
            LogOp::Orr => Alu::Or,
            LogOp::Eor => Alu::Xor,
        };
        self.a.alu_rr(sz(w), o, R::Rax, R::Rcx);
        if op == LogOp::Ands {
            self.wr(rd, R::Rax, w);
            self.flags_set(HostFlags::Add, mat);
        } else if imm_form {
            self.wr_sp(rd, R::Rax, w);
        } else {
            self.wr(rd, R::Rax, w);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn bitfield(&mut self, w: Width, op: BfOp, rd: Reg, rn: Reg, immr: u8, imms: u8, wmask: u64, tmask: u64) {
        let s = sz(w);
        // bot = ROR(src, immr) AND wmask (OR dst AND NOT wmask for BFM)
        self.rd(R::Rax, rn, w);
        if immr != 0 {
            self.a.shift_ri(s, Sh::Ror, R::Rax, immr);
        }
        self.a.mov_imm(R::Rcx, wmask);
        self.a.alu_rr(Sz::Q, Alu::And, R::Rax, R::Rcx);
        if op == BfOp::Bfm {
            self.rd(R::Rdx, rd, w);
            self.a.mov_imm(R::Rcx, !wmask & w.mask());
            self.a.alu_rr(Sz::Q, Alu::And, R::Rcx, R::Rdx);
            self.a.alu_rr(Sz::Q, Alu::Or, R::Rax, R::Rcx);
        }
        // result = (top AND NOT tmask) OR (bot AND tmask)
        self.a.mov_imm(R::Rcx, tmask);
        self.a.alu_rr(Sz::Q, Alu::And, R::Rax, R::Rcx);
        match op {
            BfOp::Ubfm => {}
            BfOp::Bfm => {
                // top = dst (still in rdx)
                self.a.mov_imm(R::Rcx, !tmask & w.mask());
                self.a.alu_rr(Sz::Q, Alu::And, R::Rcx, R::Rdx);
                self.a.alu_rr(Sz::Q, Alu::Or, R::Rax, R::Rcx);
            }
            BfOp::Sbfm => {
                // top = replicate bit imms of src
                self.rd(R::Rdx, rn, w);
                let bits = w.bits() as u8;
                self.a.shift_ri(s, Sh::Shl, R::Rdx, bits - 1 - imms);
                self.a.shift_ri(s, Sh::Sar, R::Rdx, bits - 1);
                self.a.mov_imm(R::Rcx, !tmask & w.mask());
                self.a.alu_rr(Sz::Q, Alu::And, R::Rdx, R::Rcx);
                self.a.alu_rr(Sz::Q, Alu::Or, R::Rax, R::Rdx);
            }
        }
        self.wr(rd, R::Rax, w);
    }

    fn load_store(&mut self, pc: u64, kind: MemKind, size: u8, fp: bool, rt: Reg, addr: Addr) {
        let wb = self.address(pc, addr);
        let bytes = 1u32 << size;
        if kind == MemKind::Prefetch {
            // A hint: no access, no fault.
            return;
        }
        let store = kind == MemKind::Store;
        self.check(pc, bytes, store);
        if store {
            if fp {
                self.a.load(Sz::Q, R::Rdx, mem(RCPU, ov(rt)));
                match size {
                    4 => {
                        self.a.store(Sz::Q, Self::hmem(0), R::Rdx);
                        self.a.load(Sz::Q, R::Rdx, mem(RCPU, ov(rt) + 8));
                        self.a.store(Sz::Q, Self::hmem(8), R::Rdx);
                    }
                    _ => self.a.store(size_sz(size), Self::hmem(0), R::Rdx),
                }
            } else {
                self.rd(R::Rdx, rt, Width::X);
                self.a.store(size_sz(size), Self::hmem(0), R::Rdx);
            }
        } else if fp {
            self.load_fp(size, rt, 0);
        } else {
            match kind {
                MemKind::LoadSigned(w) => {
                    self.a.load_sx(size_sz(size), sz(w), R::Rdx, Self::hmem(0));
                    self.wr(rt, R::Rdx, w);
                }
                _ => {
                    self.load_zx(size, R::Rdx, Self::hmem(0));
                    self.wr(rt, R::Rdx, Width::X);
                }
            }
        }
        if let Some(rn) = wb {
            self.wr_sp(rn, R::Rsi, Width::X);
        }
    }

    /// Zero-extending load of 1, 2, 4 or 8 bytes.
    fn load_zx(&mut self, size: u8, dst: R, m: M) {
        match size {
            0 => self.a.load_zx(Sz::B, dst, m),
            1 => self.a.load_zx(Sz::H, dst, m),
            2 => self.a.load(Sz::D, dst, m),
            _ => self.a.load(Sz::Q, dst, m),
        }
    }

    /// Load `Vt` (zero-extended to 128 bits) from the checked address plus `disp`.
    fn load_fp(&mut self, size: u8, rt: Reg, disp: i32) {
        if size == 4 {
            self.a.load(Sz::Q, R::Rdx, Self::hmem(disp));
            self.a.load(Sz::Q, R::Rdi, Self::hmem(disp + 8));
            self.a.store(Sz::Q, mem(RCPU, ov(rt)), R::Rdx);
            self.a.store(Sz::Q, mem(RCPU, ov(rt) + 8), R::Rdi);
        } else {
            self.load_zx(size, R::Rdx, Self::hmem(disp));
            self.a.store(Sz::Q, mem(RCPU, ov(rt)), R::Rdx);
            self.a.store_imm(Sz::Q, mem(RCPU, ov(rt) + 8), 0);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pair(&mut self, pc: u64, kind: PairKind, size: u8, fp: bool, rt: Reg, rt2: Reg, addr: Addr) {
        let wb = self.address(pc, addr);
        let bytes = 1u32 << size;
        let store = kind == PairKind::Store;
        self.check(pc, 2 * bytes, store);
        let d2 = bytes as i32;
        if store {
            for (r, d) in [(rt, 0), (rt2, d2)] {
                if fp {
                    self.a.load(Sz::Q, R::Rdx, mem(RCPU, ov(r)));
                    if size == 4 {
                        self.a.store(Sz::Q, Self::hmem(d), R::Rdx);
                        self.a.load(Sz::Q, R::Rdx, mem(RCPU, ov(r) + 8));
                        self.a.store(Sz::Q, Self::hmem(d + 8), R::Rdx);
                    } else {
                        self.a.store(size_sz(size), Self::hmem(d), R::Rdx);
                    }
                } else {
                    self.rd(R::Rdx, r, Width::X);
                    self.a.store(size_sz(size), Self::hmem(d), R::Rdx);
                }
            }
        } else if fp {
            self.load_fp(size, rt, 0);
            self.load_fp(size, rt2, d2);
        } else {
            // Both loads before any register write (rt may be the base).
            if kind == PairKind::LoadSw {
                self.a.load_sx(Sz::D, Sz::Q, R::Rdx, Self::hmem(0));
                self.a.load_sx(Sz::D, Sz::Q, R::Rdi, Self::hmem(d2));
            } else {
                self.load_zx(size, R::Rdx, Self::hmem(0));
                self.load_zx(size, R::Rdi, Self::hmem(d2));
            }
            self.wr(rt, R::Rdx, Width::X);
            self.wr(rt2, R::Rdi, Width::X);
        }
        if let Some(rn) = wb {
            self.wr_sp(rn, R::Rsi, Width::X);
        }
    }
}

fn sz(w: Width) -> Sz {
    match w {
        Width::W => Sz::D,
        Width::X => Sz::Q,
    }
}

fn size_sz(size: u8) -> Sz {
    match size {
        0 => Sz::B,
        1 => Sz::H,
        2 => Sz::D,
        _ => Sz::Q,
    }
}
