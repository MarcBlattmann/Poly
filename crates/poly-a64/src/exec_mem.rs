//! Loads, stores, exclusives, ordered accesses and the LSE atomics.

use crate::cpu::{Access, Cpu, MemFault, Memory, Monitor, Trap};
use crate::exec_int::extend_value;
use crate::inst::*;

/// Sign-extend the low `bytes` bytes of `v` to 64 bits.
pub(crate) fn sign_extend(v: u64, bytes: usize) -> u64 {
    let sh = 64 - 8 * bytes as u32;
    (((v << sh) as i64) >> sh) as u64
}

/// The mask of the low `bytes` bytes.
pub(crate) fn size_mask(bytes: usize) -> u128 {
    if bytes >= 16 { u128::MAX } else { (1u128 << (8 * bytes)) - 1 }
}

impl Cpu {
    /// The effective address of `addr` and the writeback value, if any.
    fn effective(&self, addr: Addr) -> (u64, Option<(Reg, u64)>) {
        match addr {
            Addr::Imm { rn, offset } => (self.xsp(rn).wrapping_add(offset as u64), None),
            Addr::Pre { rn, offset } => {
                let a = self.xsp(rn).wrapping_add(offset as u64);
                (a, Some((rn, a)))
            }
            Addr::Post { rn, offset } => {
                let b = self.xsp(rn);
                (b, Some((rn, b.wrapping_add(offset as u64))))
            }
            Addr::Reg { rn, rm, ext, shift } => {
                let off = extend_value(self.xr(rm), ext, u32::from(shift));
                (self.xsp(rn).wrapping_add(off), None)
            }
            Addr::Literal { offset } => (self.pc.wrapping_add(offset as u64), None),
        }
    }

    fn align_check(addr: u64, bytes: usize, access: Access) -> Result<(), Trap> {
        if addr & (bytes as u64 - 1) != 0 {
            Err(Trap::Fault(MemFault { addr, access, alignment: true }))
        } else {
            Ok(())
        }
    }

    /// Read a register value for a store: an integer register (31 is zero)
    /// or the low bytes of a SIMD&FP register.
    fn store_value(&self, fp: bool, rt: Reg) -> u128 {
        if fp { self.v[rt as usize] } else { u128::from(self.xr(rt)) }
    }

    pub(crate) fn exec_mem<M: Memory + ?Sized>(&mut self, inst: Inst, mem: &mut M) -> Result<(), Trap> {
        match inst {
            Inst::LoadStore { kind, size, fp, rt, addr } => {
                let bytes = 1usize << size;
                let (a, wb) = self.effective(addr);
                match kind {
                    MemKind::Prefetch => {}
                    MemKind::Store => {
                        let data = self.store_value(fp, rt);
                        mem.write_uint(a, bytes, data & size_mask(bytes))?;
                    }
                    MemKind::Load => {
                        let v = mem.read_uint(a, bytes)?;
                        if fp {
                            self.v[rt as usize] = v;
                        } else {
                            self.set_xr(rt, v as u64);
                        }
                    }
                    MemKind::LoadSigned(w) => {
                        let v = mem.read_uint(a, bytes)? as u64;
                        let e = sign_extend(v, bytes);
                        self.set_rw(rt, w, e);
                    }
                }
                if let Some((rn, nv)) = wb {
                    self.set_xsp(rn, nv);
                }
                Ok(())
            }
            Inst::LoadStorePair { kind, size, fp, rt, rt2, addr } => {
                let bytes = 1usize << size;
                let (a, wb) = self.effective(addr);
                match kind {
                    PairKind::Store => {
                        let d1 = self.store_value(fp, rt) & size_mask(bytes);
                        let d2 = self.store_value(fp, rt2) & size_mask(bytes);
                        mem.write_uint(a, bytes, d1)?;
                        mem.write_uint(a.wrapping_add(bytes as u64), bytes, d2)?;
                    }
                    PairKind::Load | PairKind::LoadSw => {
                        let v1 = mem.read_uint(a, bytes)?;
                        let v2 = mem.read_uint(a.wrapping_add(bytes as u64), bytes)?;
                        if fp {
                            self.v[rt as usize] = v1;
                            self.v[rt2 as usize] = v2;
                        } else if kind == PairKind::LoadSw {
                            self.set_xr(rt, sign_extend(v1 as u64, 4));
                            self.set_xr(rt2, sign_extend(v2 as u64, 4));
                        } else {
                            self.set_xr(rt, v1 as u64);
                            self.set_xr(rt2, v2 as u64);
                        }
                    }
                }
                if let Some((rn, nv)) = wb {
                    self.set_xsp(rn, nv);
                }
                Ok(())
            }
            Inst::Exclusive { kind, size, rs, rt, rt2, rn, .. } => {
                self.exec_exclusive(mem, kind, size, rs, rt, rt2, rn)
            }
            Inst::Ordered { store, size, rt, rn } => {
                let bytes = 1usize << size;
                let a = self.xsp(rn);
                if store {
                    Self::align_check(a, bytes, Access::Write)?;
                    mem.write_uint(a, bytes, u128::from(self.xr(rt)) & size_mask(bytes))?;
                } else {
                    Self::align_check(a, bytes, Access::Read)?;
                    let v = mem.read_uint(a, bytes)?;
                    self.set_xr(rt, v as u64);
                }
                Ok(())
            }
            Inst::Atomic { op, size, rs, rt, rn, .. } => {
                let bytes = 1usize << size;
                let a = self.xsp(rn);
                Self::align_check(a, bytes, Access::Write)?;
                let operand = u128::from(self.xr(rs)) & size_mask(bytes);
                let mask = size_mask(bytes);
                let mut old = mem.read_uint(a, bytes)?;
                loop {
                    let new = atomic_new(op, old, operand, bytes) & mask;
                    let seen = mem.cas(a, bytes, old, new)?;
                    if seen == old {
                        break;
                    }
                    old = seen;
                }
                self.set_xr(rt, old as u64);
                Ok(())
            }
            Inst::Cas { size, rs, rt, rn, .. } => {
                let bytes = 1usize << size;
                let a = self.xsp(rn);
                Self::align_check(a, bytes, Access::Write)?;
                let mask = size_mask(bytes);
                let expected = u128::from(self.xr(rs)) & mask;
                let new = u128::from(self.xr(rt)) & mask;
                let old = mem.cas(a, bytes, expected, new)?;
                self.set_xr(rs, old as u64);
                Ok(())
            }
            Inst::Casp { size, rs, rt, rn, .. } => {
                let half = 1usize << size;
                let total = half * 2;
                let a = self.xsp(rn);
                Self::align_check(a, total, Access::Write)?;
                let hm = size_mask(half);
                let pair = |lo: u64, hi: u64| (u128::from(lo) & hm) | ((u128::from(hi) & hm) << (8 * half));
                let expected = pair(self.xr(rs), self.xr(rs.wrapping_add(1)));
                let new = pair(self.xr(rt), self.xr(rt.wrapping_add(1)));
                let old = mem.cas(a, total, expected, new)?;
                self.set_xr(rs, (old & hm) as u64);
                self.set_xr(rs.wrapping_add(1), ((old >> (8 * half)) & hm) as u64);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn exec_exclusive<M: Memory + ?Sized>(
        &mut self,
        mem: &mut M,
        kind: ExclKind,
        size: u8,
        rs: Reg,
        rt: Reg,
        rt2: Reg,
        rn: Reg,
    ) -> Result<(), Trap> {
        let a = self.xsp(rn);
        match kind {
            ExclKind::Load => {
                let bytes = 1usize << size;
                Self::align_check(a, bytes, Access::Read)?;
                let v = mem.read_uint(a, bytes)?;
                self.monitor = Some(Monitor { addr: a, size: bytes, value: v });
                self.set_xr(rt, v as u64);
            }
            ExclKind::LoadPair => {
                let half = 1usize << size;
                let total = half * 2;
                Self::align_check(a, total, Access::Read)?;
                let v = mem.read_uint(a, total)?;
                self.monitor = Some(Monitor { addr: a, size: total, value: v });
                let hm = size_mask(half);
                self.set_xr(rt, (v & hm) as u64);
                self.set_xr(rt2, ((v >> (8 * half)) & hm) as u64);
            }
            ExclKind::Store | ExclKind::StorePair => {
                let (total, new) = if kind == ExclKind::Store {
                    let bytes = 1usize << size;
                    (bytes, u128::from(self.xr(rt)) & size_mask(bytes))
                } else {
                    let half = 1usize << size;
                    let hm = size_mask(half);
                    (half * 2, (u128::from(self.xr(rt)) & hm) | ((u128::from(self.xr(rt2)) & hm) << (8 * half)))
                };
                Self::align_check(a, total, Access::Write)?;
                let status = match self.monitor.take() {
                    Some(m) if m.addr == a && m.size == total => {
                        let old = mem.cas(a, total, m.value, new)?;
                        u64::from(old != m.value)
                    }
                    _ => 1,
                };
                self.set_xr(rs, status);
            }
        }
        Ok(())
    }
}

/// The value an atomic operation stores, given the old value and the
/// register operand (both already masked to the access size).
fn atomic_new(op: AtomOp, old: u128, operand: u128, bytes: usize) -> u128 {
    let sx = |v: u128| sign_extend(v as u64, bytes) as i64;
    match op {
        AtomOp::Add => old.wrapping_add(operand),
        AtomOp::Clr => old & !operand,
        AtomOp::Eor => old ^ operand,
        AtomOp::Set => old | operand,
        AtomOp::Smax => {
            if sx(old) >= sx(operand) {
                old
            } else {
                operand
            }
        }
        AtomOp::Smin => {
            if sx(old) <= sx(operand) {
                old
            } else {
                operand
            }
        }
        AtomOp::Umax => old.max(operand),
        AtomOp::Umin => old.min(operand),
        AtomOp::Swp => operand,
    }
}
