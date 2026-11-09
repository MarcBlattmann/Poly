//! The CPU state, the memory interface and the instruction step.
//!
//! [`Cpu`] holds everything an EL0 program can observe: `X0`..`X30`, `SP`,
//! `PC`, the flags, `TPIDR_EL0`, the 32 SIMD&FP registers as 128-bit values
//! and `FPCR`/`FPSR`. [`Cpu::step`] fetches, decodes and executes one
//! instruction and returns an [`Outcome`] saying whether to go on or what
//! the caller has to handle (a system call, a breakpoint, a memory fault,
//! an undefined instruction).
//!
//! # Memory
//!
//! The interpreter never touches host memory itself. Every access goes
//! through the [`Memory`] trait with sizes in bytes and little-endian data,
//! and answers with a `Result`: an unmapped or protected address is
//! reported to the caller as [`Outcome::Fault`], never a panic. The caller
//! owns the address space (page tables, `mmap`, copy-on-write); this crate
//! only asks for bytes. Normal loads and stores may be misaligned (as on
//! Linux and macOS user space); exclusives, ordered accesses and atomics
//! must be aligned and report a fault with `alignment` set otherwise.
//!
//! # Exclusives and atomics
//!
//! `LDXR`/`STXR` and friends use a *value* monitor: `LDXR` remembers the
//! address, size and value it read; `STXR` succeeds if that monitor is still
//! open (no `CLREX`, no other `STXR` since, [`Cpu::clear_exclusive`] not
//! called) and [`Memory::cas`] finds the remembered value still in memory.
//! With the default `cas` this is single-threaded semantics: it is exactly
//! right for one guest thread at a time and a conservative approximation
//! otherwise (a store between the two that writes the same value goes
//! unnoticed). A host that runs guest threads in parallel overrides `cas`
//! with a real atomic compare-and-swap and gets correct `STXR` and
//! `LDADD`/`CAS` family behaviour. The LSE atomics are done with `cas` in a
//! loop.
//!
//! # What is not modelled
//!
//! Only EL0 is modelled: no exceptions are taken (the caller handles
//! `SVC`), no MMU, no alignment checking of the stack pointer, no
//! privileged system registers. Floating point is IEEE 754 round to nearest
//! even with Arm's NaN rules; see [`crate::fp`].

use crate::decode::decode;
use crate::inst::*;

/// What kind of access faulted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// A data read.
    Read,
    /// A data write.
    Write,
    /// An instruction fetch.
    Fetch,
}

/// A failed memory access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemFault {
    /// The first address that could not be accessed.
    pub addr: u64,
    /// What the program was doing.
    pub access: Access,
    /// True for an alignment fault (exclusive, ordered or atomic access, or
    /// an unaligned `PC`); false for an unmapped or protected address.
    pub alignment: bool,
}

/// The memory a [`Cpu`] runs against.
///
/// Implement `read` and `write`; the rest have defaults. Multi-byte values
/// are little-endian. An access that crosses into an unmapped page returns
/// the fault of the first unmapped address; a write that fails part way may
/// have written the bytes before it (the Arm ARM allows this).
pub trait Memory {
    /// Read `buf.len()` bytes at `addr`.
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), MemFault>;

    /// Write `data` at `addr`.
    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), MemFault>;

    /// Fetch the instruction word at `addr` (already known to be 4-aligned).
    fn fetch(&mut self, addr: u64) -> Result<u32, MemFault> {
        let mut b = [0u8; 4];
        self.read(addr, &mut b).map_err(|f| MemFault { access: Access::Fetch, ..f })?;
        Ok(u32::from_le_bytes(b))
    }

    /// Compare-and-swap `size` bytes (1, 2, 4, 8 or 16) at `addr`: if the
    /// current value equals `expected`, store `new`. Returns the value that
    /// was in memory. Override with an atomic operation if guest threads
    /// run in parallel (see the module docs).
    fn cas(&mut self, addr: u64, size: usize, expected: u128, new: u128) -> Result<u128, MemFault> {
        let old = self.read_uint(addr, size)?;
        if old == expected {
            self.write_uint(addr, size, new)?;
        }
        Ok(old)
    }

    /// Read a little-endian unsigned integer of `size` bytes (1 to 16).
    fn read_uint(&mut self, addr: u64, size: usize) -> Result<u128, MemFault> {
        let mut b = [0u8; 16];
        self.read(addr, &mut b[..size])?;
        Ok(u128::from_le_bytes(b))
    }

    /// Write the low `size` bytes (1 to 16) of `value`, little-endian.
    fn write_uint(&mut self, addr: u64, size: usize, value: u128) -> Result<(), MemFault> {
        let b = value.to_le_bytes();
        self.write(addr, &b[..size])
    }
}

/// A [`Memory`] over one byte slice mapped at `base`, for tests and tools.
#[derive(Debug)]
pub struct SliceMemory<'a> {
    /// The address of `data[0]`.
    pub base: u64,
    /// The bytes.
    pub data: &'a mut [u8],
}

impl<'a> SliceMemory<'a> {
    /// Memory covering `base..base + data.len()`.
    pub fn new(base: u64, data: &'a mut [u8]) -> Self {
        SliceMemory { base, data }
    }

    fn range(&self, addr: u64, len: usize, access: Access) -> Result<core::ops::Range<usize>, MemFault> {
        let fail = MemFault { addr, access, alignment: false };
        let off = addr.checked_sub(self.base).ok_or(fail)?;
        let start = usize::try_from(off).map_err(|_| fail)?;
        let end = start.checked_add(len).ok_or(fail)?;
        if end > self.data.len() {
            // The first address past the end of the mapping.
            let bad = self.base + (self.data.len().max(start) as u64);
            return Err(MemFault { addr: bad, access, alignment: false });
        }
        Ok(start..end)
    }
}

impl Memory for SliceMemory<'_> {
    fn read(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), MemFault> {
        let r = self.range(addr, buf.len(), Access::Read)?;
        buf.copy_from_slice(&self.data[r]);
        Ok(())
    }

    fn write(&mut self, addr: u64, data: &[u8]) -> Result<(), MemFault> {
        let r = self.range(addr, data.len(), Access::Write)?;
        self.data[r].copy_from_slice(data);
        Ok(())
    }
}

/// The result of executing one instruction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The instruction completed; `PC` points at the next one.
    Continue,
    /// `SVC #imm` executed. `PC` already points past it: handle the system
    /// call (arguments are in the registers) and call `step` again.
    Svc(u16),
    /// `BRK #imm` executed. `PC` still points at the `BRK`.
    Brk(u16),
    /// A memory access faulted. `PC` points at the faulting instruction and
    /// the registers are as before it, except that the registers of a load
    /// pair or a store pair may be partly done.
    Fault(MemFault),
    /// An unallocated encoding (or a system register or system instruction
    /// not allowed at EL0). `PC` points at it. The word is the instruction.
    Undefined(u32),
    /// An allocated encoding that this crate does not implement. `PC`
    /// points at it.
    Unsupported(u32),
}

/// `N` in `NZCV` (bit 31).
pub const NZCV_N: u32 = 1 << 31;
/// `Z` in `NZCV` (bit 30).
pub const NZCV_Z: u32 = 1 << 30;
/// `C` in `NZCV` (bit 29).
pub const NZCV_C: u32 = 1 << 29;
/// `V` in `NZCV` (bit 28).
pub const NZCV_V: u32 = 1 << 28;

/// `FPSR.IOC`: invalid operation.
pub const FPSR_IOC: u32 = 1;
/// `FPSR.DZC`: division by zero.
pub const FPSR_DZC: u32 = 2;

/// `FPCR` bits that are kept (AHP, DN, FZ, RMode, FZ16). The others read as
/// zero.
const FPCR_MASK: u64 = 0x07C8_0000;
/// `FPSR` bits that are kept (QC and the cumulative exception flags).
const FPSR_MASK: u64 = 0x0800_009F;

/// The value `DCZID_EL0` reads: block size 2^4 words = 64 bytes, `DC ZVA`
/// permitted.
pub const DCZID_DEFAULT: u64 = 4;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Monitor {
    pub(crate) addr: u64,
    pub(crate) size: usize,
    pub(crate) value: u128,
}

/// The register state of one EL0 thread.
#[derive(Clone, Debug)]
pub struct Cpu {
    /// `X0` to `X30` (`X30` is the link register).
    pub x: [u64; 31],
    /// The stack pointer.
    pub sp: u64,
    /// The program counter.
    pub pc: u64,
    /// The flags in their `NZCV` register positions (bits 31 to 28); see
    /// [`NZCV_N`] and friends.
    pub nzcv: u32,
    /// `TPIDR_EL0`, the thread pointer (read-write).
    pub tpidr_el0: u64,
    /// `TPIDRRO_EL0` (read-only for the program; the host sets it).
    pub tpidrro_el0: u64,
    /// `V0` to `V31`.
    pub v: [u128; 32],
    /// `FPCR` (only AHP, DN, FZ, RMode and FZ16 are kept; only DN has an
    /// effect, see [`crate::fp`]).
    pub fpcr: u32,
    /// `FPSR`; this crate sets `IOC` and `DZC`.
    pub fpsr: u32,
    /// What `CNTVCT_EL0` reads next. The host advances it with real time;
    /// each read also adds [`Cpu::cnt_read_step`].
    pub cntvct: u64,
    /// Added to `cntvct` after each read of it, so a program waiting on
    /// the counter makes progress even if the host does not advance it.
    pub cnt_read_step: u64,
    /// What `CNTFRQ_EL0` reads (24 MHz, the usual Apple value).
    pub cntfrq: u64,
    /// What `DCZID_EL0` reads. The low 4 bits give the `DC ZVA` block as
    /// 2^n words; bit 4 set prohibits `DC ZVA`.
    pub dczid: u64,
    /// Counts `IC IVAU`/`IC IALLU` executions: a JIT that caches
    /// translations compares it before running a block to notice that the
    /// program invalidated its instruction cache.
    pub icache_flushes: u64,
    pub(crate) monitor: Option<Monitor>,
}

impl Default for Cpu {
    fn default() -> Self {
        Cpu::new()
    }
}

/// How an instruction ended, inside the interpreter.
pub(crate) enum Trap {
    Svc(u16),
    Brk(u16),
    Fault(MemFault),
    Undef,
    Unsupported,
}

impl From<MemFault> for Trap {
    fn from(f: MemFault) -> Trap {
        Trap::Fault(f)
    }
}

/// `Ok(None)`: go on with the next instruction. `Ok(Some(t))`: jump to `t`.
pub(crate) type ExecResult = Result<Option<u64>, Trap>;

impl Cpu {
    /// All registers zero, `PC` 0.
    pub fn new() -> Cpu {
        Cpu {
            x: [0; 31],
            sp: 0,
            pc: 0,
            nzcv: 0,
            tpidr_el0: 0,
            tpidrro_el0: 0,
            v: [0; 32],
            fpcr: 0,
            fpsr: 0,
            cntvct: 0,
            cnt_read_step: 1,
            cntfrq: 24_000_000,
            dczid: DCZID_DEFAULT,
            icache_flushes: 0,
            monitor: None,
        }
    }

    /// Forget the exclusive monitor (what an exception return or a context
    /// switch does).
    pub fn clear_exclusive(&mut self) {
        self.monitor = None;
    }

    // ---- register access ----

    /// Read `Xn`, with 31 as `XZR`.
    #[inline]
    pub fn xr(&self, r: Reg) -> u64 {
        if r == 31 { 0 } else { self.x[r as usize] }
    }

    /// Read `Xn`, with 31 as `SP`.
    #[inline]
    pub fn xsp(&self, r: Reg) -> u64 {
        if r == 31 { self.sp } else { self.x[r as usize] }
    }

    /// Write `Xn`, with 31 as `XZR` (the write is dropped).
    #[inline]
    pub fn set_xr(&mut self, r: Reg, v: u64) {
        if r != 31 {
            self.x[r as usize] = v;
        }
    }

    /// Write `Xn`, with 31 as `SP`.
    #[inline]
    pub fn set_xsp(&mut self, r: Reg, v: u64) {
        if r == 31 {
            self.sp = v;
        } else {
            self.x[r as usize] = v;
        }
    }

    /// Read `Rn` as `w` bits (`XZR`/`WZR` for 31).
    #[inline]
    pub(crate) fn rw(&self, r: Reg, w: Width) -> u64 {
        self.xr(r) & w.mask()
    }

    /// Write `Rn` as `w` bits: a `W` result is zero-extended.
    #[inline]
    pub(crate) fn set_rw(&mut self, r: Reg, w: Width, v: u64) {
        self.set_xr(r, v & w.mask());
    }

    /// Write `Rn` (31 is `SP`) as `w` bits.
    #[inline]
    pub(crate) fn set_rw_sp(&mut self, r: Reg, w: Width, v: u64) {
        self.set_xsp(r, v & w.mask());
    }

    /// Does `cond` hold for the current flags (`ConditionHolds`)?
    pub fn cond_holds(&self, cond: Cond) -> bool {
        let n = self.nzcv & NZCV_N != 0;
        let z = self.nzcv & NZCV_Z != 0;
        let c = self.nzcv & NZCV_C != 0;
        let v = self.nzcv & NZCV_V != 0;
        match cond {
            Cond::Eq => z,
            Cond::Ne => !z,
            Cond::Cs => c,
            Cond::Cc => !c,
            Cond::Mi => n,
            Cond::Pl => !n,
            Cond::Vs => v,
            Cond::Vc => !v,
            Cond::Hi => c && !z,
            Cond::Ls => !(c && !z),
            Cond::Ge => n == v,
            Cond::Lt => n != v,
            Cond::Gt => !z && n == v,
            Cond::Le => !(!z && n == v),
            Cond::Al | Cond::Nv => true,
        }
    }

    // ---- stepping ----

    /// Fetch, decode and execute the instruction at `PC`.
    pub fn step<M: Memory + ?Sized>(&mut self, mem: &mut M) -> Outcome {
        if self.pc & 3 != 0 {
            return Outcome::Fault(MemFault { addr: self.pc, access: Access::Fetch, alignment: true });
        }
        let word = match mem.fetch(self.pc) {
            Ok(w) => w,
            Err(f) => return Outcome::Fault(f),
        };
        match decode(word) {
            Inst::Undefined(w) => Outcome::Undefined(w),
            Inst::Unsupported(w) => Outcome::Unsupported(w),
            inst => self.execute_word(inst, word, mem),
        }
    }

    /// Execute an already decoded instruction at the current `PC` (what a
    /// decode cache or a JIT's interpreter fallback calls). `Undefined` and
    /// `Unsupported` instructions report word 0 here; use [`Cpu::step`] to
    /// get the real word.
    pub fn execute<M: Memory + ?Sized>(&mut self, inst: Inst, mem: &mut M) -> Outcome {
        match inst {
            Inst::Undefined(w) => Outcome::Undefined(w),
            Inst::Unsupported(w) => Outcome::Unsupported(w),
            _ => self.execute_word(inst, 0, mem),
        }
    }

    fn execute_word<M: Memory + ?Sized>(&mut self, inst: Inst, word: u32, mem: &mut M) -> Outcome {
        let pc = self.pc;
        match self.exec(inst, mem) {
            Ok(None) => {
                self.pc = pc.wrapping_add(4);
                Outcome::Continue
            }
            Ok(Some(t)) => {
                self.pc = t;
                Outcome::Continue
            }
            Err(Trap::Svc(i)) => {
                self.pc = pc.wrapping_add(4);
                Outcome::Svc(i)
            }
            Err(Trap::Brk(i)) => Outcome::Brk(i),
            Err(Trap::Fault(f)) => Outcome::Fault(f),
            Err(Trap::Undef) => Outcome::Undefined(word),
            Err(Trap::Unsupported) => Outcome::Unsupported(word),
        }
    }

    /// Run until something other than `Continue` happens or `max_steps`
    /// instructions have executed. Returns the outcome and the number of
    /// instructions executed (the last one included when it was not a
    /// fault or undefined instruction).
    pub fn run<M: Memory + ?Sized>(&mut self, mem: &mut M, max_steps: u64) -> (Outcome, u64) {
        let mut n = 0;
        while n < max_steps {
            n += 1;
            match self.step(mem) {
                Outcome::Continue => {}
                other => return (other, n),
            }
        }
        (Outcome::Continue, n)
    }

    fn exec<M: Memory + ?Sized>(&mut self, inst: Inst, mem: &mut M) -> ExecResult {
        match inst {
            Inst::Undefined(_) => Err(Trap::Undef),
            Inst::Unsupported(_) => Err(Trap::Unsupported),
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
            | Inst::Bit1 { .. } => {
                self.exec_int(inst);
                Ok(None)
            }
            Inst::Branch { .. }
            | Inst::BranchCond { .. }
            | Inst::CompareBranch { .. }
            | Inst::TestBranch { .. }
            | Inst::BranchReg { .. }
            | Inst::BranchLinkReg { .. }
            | Inst::Ret { .. } => Ok(self.exec_branch(inst)),
            Inst::Svc(i) => Err(Trap::Svc(i)),
            Inst::Brk(i) => Err(Trap::Brk(i)),
            Inst::Hint { .. } => Ok(None),
            Inst::Barrier { kind, .. } => {
                if kind == Barrier::Clrex {
                    self.monitor = None;
                }
                Ok(None)
            }
            Inst::Mrs { rt, reg } => self.exec_mrs(rt, reg),
            Inst::Msr { rt, reg } => self.exec_msr(rt, reg),
            Inst::Sys { op1, crn, crm, op2, rt } => self.exec_sys(op1, crn, crm, op2, rt, mem),
            Inst::LoadStore { .. }
            | Inst::LoadStorePair { .. }
            | Inst::Exclusive { .. }
            | Inst::Ordered { .. }
            | Inst::Atomic { .. }
            | Inst::Cas { .. }
            | Inst::Casp { .. } => {
                self.exec_mem(inst, mem)?;
                Ok(None)
            }
            Inst::FpUnary { .. }
            | Inst::FpCvt { .. }
            | Inst::FpBinary { .. }
            | Inst::FpFused { .. }
            | Inst::FpCmp { .. }
            | Inst::FpCondCmp { .. }
            | Inst::FpCondSel { .. }
            | Inst::FpMovImm { .. }
            | Inst::FpToInt { .. }
            | Inst::IntToFp { .. }
            | Inst::FmovToGen { .. }
            | Inst::FmovFromGen { .. } => {
                self.exec_fp(inst)?;
                Ok(None)
            }
            Inst::VecLdstMulti { .. } | Inst::VecLdstSingle { .. } => {
                self.exec_simd_mem(inst, mem)?;
                Ok(None)
            }
            _ => {
                self.exec_simd(inst)?;
                Ok(None)
            }
        }
    }

    fn exec_mrs(&mut self, rt: Reg, reg: SysReg) -> ExecResult {
        let v = match reg {
            SysReg::NZCV => u64::from(self.nzcv),
            SysReg::FPCR => u64::from(self.fpcr),
            SysReg::FPSR => u64::from(self.fpsr),
            SysReg::TPIDR_EL0 => self.tpidr_el0,
            SysReg::TPIDRRO_EL0 => self.tpidrro_el0,
            SysReg::CNTFRQ_EL0 => self.cntfrq,
            SysReg::DCZID_EL0 => self.dczid,
            SysReg::CNTVCT_EL0 => {
                let v = self.cntvct;
                self.cntvct = v.wrapping_add(self.cnt_read_step);
                v
            }
            _ => return Err(Trap::Undef),
        };
        self.set_xr(rt, v);
        Ok(None)
    }

    fn exec_msr(&mut self, rt: Reg, reg: SysReg) -> ExecResult {
        let v = self.xr(rt);
        match reg {
            SysReg::NZCV => self.nzcv = (v as u32) & 0xF000_0000,
            SysReg::FPCR => self.fpcr = (v & FPCR_MASK) as u32,
            SysReg::FPSR => self.fpsr = (v & FPSR_MASK) as u32,
            SysReg::TPIDR_EL0 => self.tpidr_el0 = v,
            _ => return Err(Trap::Undef),
        }
        Ok(None)
    }

    fn exec_sys<M: Memory + ?Sized>(&mut self, op1: u8, crn: u8, crm: u8, op2: u8, rt: Reg, mem: &mut M) -> ExecResult {
        match (op1, crn, crm, op2) {
            // DC ZVA: zero one block.
            (3, 7, 4, 1) => {
                if self.dczid & 0x10 != 0 {
                    return Err(Trap::Undef);
                }
                let bytes = 4u64 << (self.dczid & 15);
                let base = self.xr(rt) & !(bytes - 1);
                let zeros = [0u8; 64];
                let mut off = 0;
                while off < bytes {
                    let n = (bytes - off).min(64) as usize;
                    mem.write(base.wrapping_add(off), &zeros[..n])?;
                    off += n as u64;
                }
                Ok(None)
            }
            // DC CVAC, CVAU, CVAP, CIVAC: cache maintenance has no effect on this model.
            (3, 7, 10, 1) | (3, 7, 11, 1) | (3, 7, 12, 1) | (3, 7, 14, 1) => Ok(None),
            // IC IVAU, IC IALLU (EL1 only in the architecture, but user code uses IVAU).
            (3, 7, 5, 1) | (0, 7, 5, 0) => {
                self.icache_flushes = self.icache_flushes.wrapping_add(1);
                Ok(None)
            }
            _ => Err(Trap::Undef),
        }
    }
}
