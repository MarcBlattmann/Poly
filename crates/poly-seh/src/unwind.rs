//! The virtual unwinder: from a function's context to its caller's
//! (`RtlVirtualUnwind`; Learn: "x64 exception handling", "Unwind
//! procedure", "Prolog and epilog", "Chained unwind info structures").
//!
//! Given an address in a function and the function's table entry, the
//! context of the thread at that address becomes the context the caller
//! had right after its `call` returned: nonvolatile registers restored, RSP
//! above the return address and RIP the return address. Where the address
//! lies decides how:
//!
//! - **In an epilog** (the code from the address on is the tail of a legal
//!   epilog: `add rsp, n` or `lea rsp, [fp + n]`, `pop`s of nonvolatile
//!   registers, then `ret` or a `jmp` out of the function), the rest of the
//!   epilog is simulated. No handler is reported: control is leaving the
//!   function.
//! - **In the prolog** (the offset from the function start is less than
//!   `SizeOfProlog`), only the operations already done are undone: the
//!   unwind codes whose `CodeOffset` is at most the offset. No handler.
//! - **In the body**, every unwind code is undone, then those of the
//!   chained unwind info, and the language-specific handler (if the
//!   function has one of the kind asked for) is reported.
//!
//! The *establisher frame* (Learn: "the base of the fixed stack allocation
//! for this function") is the frame register minus `FrameOffset * 16` once
//! the frame register is set up, else RSP.
//!
//! What Learn leaves open, and what Poly picks (docs/adr/0068):
//!
//! - Only `UNWIND_INFO` version 1 is accepted (the documented one).
//! - A direct `jmp` (`E9`/`EB`) ends an epilog only if it leaves the
//!   function (a tail call); Learn names `jmp` with a mod-00 ModRM, which
//!   is accepted too, as are `ret`, `ret imm16` and `rep ret`.
//! - With a frame register, RSP is first set to the establisher frame, so
//!   a body that moved RSP after the prolog (`alloca`) unwinds correctly.
//! - A chained fragment's handler is the one of the first unwind info in
//!   its chain that has one.

use crate::consts::*;
use crate::context::Context;
use crate::info::{UnwindEntry, UnwindInfo, UnwindOp};
use crate::pdata::RuntimeFunction;
use crate::{MAX_CHAIN, Memory, UnwindError, read_u64, read_u128};
use alloc::vec::Vec;

/// A language-specific handler a function has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Handler {
    /// The handler routine (absolute).
    pub routine: u64,
    /// Its language-specific data, right after the handler RVA (absolute).
    pub data: u64,
}

/// Where in its function the unwound address was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Location {
    Prolog,
    Body,
    Epilog,
}

/// What [`virtual_unwind`] found besides the caller's context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unwound {
    /// The handler of the kind asked for, if the address is in the body.
    pub handler: Option<Handler>,
    pub establisher_frame: u64,
    pub location: Location,
}

/// Where restored registers were read from (`KNONVOLATILE_CONTEXT_POINTERS`):
/// the stack address of each integer and XMM register an unwind restored.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextPointers {
    pub integer: [Option<u64>; 16],
    pub xmm: [Option<u64>; 16],
}

impl ContextPointers {
    fn merge(&mut self, other: &ContextPointers) {
        for (a, b) in self.integer.iter_mut().zip(other.integer) {
            if b.is_some() {
                *a = b;
            }
        }
        for (a, b) in self.xmm.iter_mut().zip(other.xmm) {
            if b.is_some() {
                *a = b;
            }
        }
    }
}

fn add(a: u64, b: u64) -> Result<u64, UnwindError> {
    a.checked_add(b).ok_or(UnwindError::Overflow)
}

/// Unwind a leaf function (one without a table entry): RSP addresses the
/// return address (Learn: "If no function table entry is found, then it's
/// in a leaf function").
pub fn unwind_leaf(ctx: &mut Context, mem: &dyn Memory) -> Result<(), UnwindError> {
    let rsp = ctx.rsp();
    let ra = read_u64(mem, rsp)?;
    ctx.rip = ra;
    ctx.set_rsp(add(rsp, 8)?);
    Ok(())
}

/// Read the unwind info of `function` and every chained one after it.
pub fn chain(mem: &dyn Memory, image_base: u64, function: &RuntimeFunction) -> Result<Vec<UnwindInfo>, UnwindError> {
    let mut chain = Vec::new();
    let mut next = Some(*function);
    while let Some(f) = next {
        if chain.len() >= MAX_CHAIN {
            return Err(UnwindError::ChainTooDeep);
        }
        let info = UnwindInfo::read(mem, add(image_base, u64::from(f.unwind))?)?;
        next = info.chained;
        chain.push(info);
    }
    Ok(chain)
}

/// The frame register and offset in effect at `offset` into the primary
/// function, if its frame pointer has been set up by then.
fn frame_pointer(chain: &[UnwindInfo], ops: &[Vec<UnwindEntry>], offset: u64) -> Option<(u8, u8)> {
    for (k, (info, entries)) in chain.iter().zip(ops).enumerate() {
        let set = entries.iter().find(|e| e.op == UnwindOp::SetFpReg);
        match set {
            // The primary function's own prolog sets it: only once done.
            Some(e) if k == 0 && u64::from(e.prolog_offset) > offset => return None,
            Some(_) if info.frame_register != 0 => return Some((info.frame_register, info.frame_offset)),
            _ => {}
        }
    }
    None
}

/// One step of an epilog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Add(u64),
    Lea { reg: u8, disp: i64 },
    Pop(u8),
    Ret { extra: u64 },
}

/// Up to this many bytes of code are looked at for an epilog: the longest
/// legal one (`lea rsp` with a SIB byte and 32-bit displacement, 16 `pop`s,
/// `jmp [rip+disp32]` with a REX prefix) is shorter.
const EPILOG_WINDOW: usize = 64;

/// The code at `pc`, as much of [`EPILOG_WINDOW`] as can be read.
fn code_at(mem: &dyn Memory, pc: u64) -> Vec<u8> {
    let mut buf = alloc::vec![0u8; EPILOG_WINDOW];
    if mem.read(pc, &mut buf).is_ok() {
        return buf;
    }
    let mut out = Vec::new();
    for i in 0..EPILOG_WINDOW as u64 {
        let mut b = [0u8; 1];
        match pc.checked_add(i) {
            Some(a) if mem.read(a, &mut b).is_ok() => out.push(b[0]),
            _ => break,
        }
    }
    out
}

/// If the code at `pc` is (the tail of) a legal epilog, its steps.
fn epilog(code: &[u8], pc: u64, begin: u64, end: u64, frame_reg: Option<u8>) -> Option<Vec<Step>> {
    let at = |i: usize| code.get(i).copied();
    let imm32 = |i: usize| -> Option<i32> { Some(i32::from_le_bytes(code.get(i..i + 4)?.try_into().ok()?)) };
    let mut steps = Vec::new();
    let mut i = 0usize;
    // The stack deallocation may only come first.
    match (at(0), at(1), at(2)) {
        (Some(0x48), Some(0x83), Some(0xC4)) => {
            let n = at(3)? as i8;
            if n < 0 {
                return None;
            }
            steps.push(Step::Add(n as u64));
            i = 4;
        }
        (Some(0x48), Some(0x81), Some(0xC4)) => {
            let n = imm32(3)?;
            if n < 0 {
                return None;
            }
            steps.push(Step::Add(n as u64));
            i = 7;
        }
        (Some(rex @ (0x48 | 0x49)), Some(0x8D), Some(modrm)) if (modrm >> 3) & 7 == 4 => {
            let mode = modrm >> 6;
            let rm = modrm & 7;
            if mode != 1 && mode != 2 {
                return None;
            }
            let base = rm | (rex & 1) << 3;
            let mut j = 3;
            if rm == 4 {
                // A SIB byte with no index and the base in ModRM.rm.
                if at(j)? != 0x24 {
                    return None;
                }
                j += 1;
            }
            let disp = if mode == 1 {
                let d = i64::from(at(j)? as i8);
                j += 1;
                d
            } else {
                let d = i64::from(imm32(j)?);
                j += 4;
                d
            };
            if frame_reg != Some(base) {
                return None;
            }
            steps.push(Step::Lea { reg: base, disp });
            i = j;
        }
        _ => {}
    }
    loop {
        match (at(i), at(i + 1)) {
            (Some(b @ 0x58..=0x5F), _) => {
                steps.push(Step::Pop(b - 0x58));
                i += 1;
            }
            (Some(0x41), Some(b @ 0x58..=0x5F)) => {
                steps.push(Step::Pop(b - 0x58 + 8));
                i += 2;
            }
            _ => break,
        }
    }
    let leaves = |target: u64| target < begin || target >= end;
    let next = pc.wrapping_add(i as u64);
    let ret = match (at(i), at(i + 1)) {
        (Some(0xC3), _) | (Some(0xF3), Some(0xC3)) => Step::Ret { extra: 0 },
        (Some(0xC2), _) => {
            let n = u16::from_le_bytes([at(i + 1)?, at(i + 2)?]);
            Step::Ret { extra: u64::from(n) }
        }
        (Some(0xE9), _) => {
            let rel = imm32(i + 1)?;
            let target = next.wrapping_add(5).wrapping_add(rel as i64 as u64);
            if !leaves(target) {
                return None;
            }
            Step::Ret { extra: 0 }
        }
        (Some(0xEB), Some(rel)) => {
            let target = next.wrapping_add(2).wrapping_add(rel as i8 as i64 as u64);
            if !leaves(target) {
                return None;
            }
            Step::Ret { extra: 0 }
        }
        (Some(0xFF), Some(modrm)) if modrm >> 6 == 0 && (modrm >> 3) & 7 == 4 => Step::Ret { extra: 0 },
        (Some(0x40..=0x4F), Some(0xFF)) => {
            let modrm = at(i + 2)?;
            if modrm >> 6 != 0 || (modrm >> 3) & 7 != 4 {
                return None;
            }
            Step::Ret { extra: 0 }
        }
        _ => return None,
    };
    steps.push(ret);
    Some(steps)
}

/// Undo one prolog operation of `info` on `c`.
fn undo(
    op: UnwindOp,
    info: &UnwindInfo,
    c: &mut Context,
    mem: &dyn Memory,
    ptrs: &mut ContextPointers,
    machframe: &mut bool,
) -> Result<(), UnwindError> {
    let rsp = c.rsp();
    match op {
        UnwindOp::PushNonvol { reg } => {
            let v = read_u64(mem, rsp)?;
            c.set_rsp(add(rsp, 8)?);
            c.gpr[usize::from(reg & 15)] = v;
            ptrs.integer[usize::from(reg & 15)] = Some(rsp);
        }
        UnwindOp::Alloc { size } => c.set_rsp(add(rsp, u64::from(size))?),
        UnwindOp::SetFpReg => {
            if info.frame_register == 0 {
                return Err(UnwindError::NoFrameRegister);
            }
            let fp = c.gpr[usize::from(info.frame_register)];
            c.set_rsp(fp.checked_sub(16 * u64::from(info.frame_offset)).ok_or(UnwindError::Overflow)?);
        }
        UnwindOp::SaveNonvol { reg, offset } => {
            let at = add(rsp, u64::from(offset))?;
            c.gpr[usize::from(reg & 15)] = read_u64(mem, at)?;
            ptrs.integer[usize::from(reg & 15)] = Some(at);
        }
        UnwindOp::SaveXmm128 { reg, offset } => {
            let at = add(rsp, u64::from(offset))?;
            c.set_xmm(usize::from(reg & 15), read_u128(mem, at)?);
            ptrs.xmm[usize::from(reg & 15)] = Some(at);
        }
        UnwindOp::PushMachFrame { error_code } => {
            // RIP, CS, EFLAGS, the old RSP and SS, above the error code if
            // there is one (Learn: "UWOP_PUSH_MACHFRAME").
            let base = add(rsp, if error_code { 8 } else { 0 })?;
            c.rip = read_u64(mem, base)?;
            c.seg_cs = read_u64(mem, add(base, 8)?)? as u16;
            c.eflags = read_u64(mem, add(base, 16)?)? as u32;
            let old_rsp = read_u64(mem, add(base, 24)?)?;
            c.seg_ss = read_u64(mem, add(base, 32)?)? as u16;
            c.set_rsp(old_rsp);
            *machframe = true;
        }
    }
    Ok(())
}

/// Simulate the rest of an epilog on `c`.
fn run_epilog(
    steps: &[Step],
    c: &mut Context,
    mem: &dyn Memory,
    ptrs: &mut ContextPointers,
) -> Result<(), UnwindError> {
    for step in steps {
        let rsp = c.rsp();
        match *step {
            Step::Add(n) => c.set_rsp(add(rsp, n)?),
            Step::Lea { reg, disp } => {
                let v = c.gpr[usize::from(reg)].checked_add_signed(disp).ok_or(UnwindError::Overflow)?;
                c.set_rsp(v);
            }
            Step::Pop(reg) => {
                let v = read_u64(mem, rsp)?;
                c.set_rsp(add(rsp, 8)?);
                c.gpr[usize::from(reg)] = v;
                ptrs.integer[usize::from(reg)] = Some(rsp);
            }
            Step::Ret { extra } => {
                c.rip = read_u64(mem, rsp)?;
                c.set_rsp(add(add(rsp, 8)?, extra)?);
            }
        }
    }
    Ok(())
}

/// `RtlVirtualUnwind`: unwind `ctx`, the context of a thread at
/// `control_pc` in `function` of the image at `image_base`, to its
/// caller's. `handler_type` is `UNW_FLAG_EHANDLER` (looking for exception
/// handlers), `UNW_FLAG_UHANDLER` (unwinding) or `UNW_FLAG_NHANDLER` (a
/// plain stack walk); a handler of that kind is reported if the address is
/// in the function's body. Restored registers' stack addresses go to
/// `pointers`. On error `ctx` and `pointers` are unchanged.
pub fn virtual_unwind(
    handler_type: u8,
    image_base: u64,
    control_pc: u64,
    function: &RuntimeFunction,
    ctx: &mut Context,
    mem: &dyn Memory,
    pointers: Option<&mut ContextPointers>,
) -> Result<Unwound, UnwindError> {
    let begin = add(image_base, u64::from(function.begin))?;
    let end = add(image_base, u64::from(function.end))?;
    if control_pc < begin || control_pc >= end {
        return Err(UnwindError::NotInFunction);
    }
    let offset = control_pc - begin;
    let chain = chain(mem, image_base, function)?;
    let ops = chain.iter().map(UnwindInfo::ops).collect::<Result<Vec<_>, _>>()?;
    let primary = &chain[0];
    let fp = frame_pointer(&chain, &ops, offset);
    let mut c = ctx.clone();
    let mut ptrs = ContextPointers::default();
    let establisher = match fp {
        Some((reg, off)) => c.gpr[usize::from(reg)].checked_sub(16 * u64::from(off)).ok_or(UnwindError::Overflow)?,
        None => c.rsp(),
    };

    let lea_reg = chain.iter().map(|i| i.frame_register).find(|&r| r != 0);
    let location;
    if let Some(steps) = epilog(&code_at(mem, control_pc), control_pc, begin, end, lea_reg) {
        run_epilog(&steps, &mut c, mem, &mut ptrs)?;
        location = Location::Epilog;
    } else {
        if fp.is_some() {
            c.set_rsp(establisher);
        }
        let mut machframe = false;
        for (k, (info, entries)) in chain.iter().zip(&ops).enumerate() {
            for e in entries {
                if k == 0 && u64::from(e.prolog_offset) > offset {
                    continue;
                }
                undo(e.op, info, &mut c, mem, &mut ptrs, &mut machframe)?;
            }
        }
        if !machframe {
            let rsp = c.rsp();
            c.rip = read_u64(mem, rsp)?;
            c.set_rsp(add(rsp, 8)?);
        }
        location = if offset < u64::from(primary.prolog_size) { Location::Prolog } else { Location::Body };
    }

    let handler = if location == Location::Body {
        chain
            .iter()
            .find(|i| i.flags & (UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER) != 0)
            .filter(|i| i.flags & handler_type != 0)
            .and_then(|i| {
                let routine = image_base.checked_add(u64::from(i.handler?))?;
                Some(Handler { routine, data: i.handler_data()? })
            })
    } else {
        None
    };
    *ctx = c;
    if let Some(p) = pointers {
        p.merge(&ptrs);
    }
    Ok(Unwound { handler, establisher_frame: establisher, location })
}
