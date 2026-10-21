//! `UNWIND_INFO` and `UNWIND_CODE` (Learn: "x64 exception handling",
//! "struct UNWIND_INFO", "struct UNWIND_CODE", "Unwind operation code").
//!
//! ```text
//! UNWIND_INFO (DWORD aligned)
//!   UBYTE Version : 3, Flags : 5
//!   UBYTE SizeOfProlog
//!   UBYTE CountOfCodes            slots in the code array
//!   UBYTE FrameRegister : 4, FrameOffset : 4   (offset scaled by 16)
//!   UNWIND_CODE UnwindCode[CountOfCodes rounded up to even]
//!   then, with UNW_FLAG_EHANDLER or UNW_FLAG_UHANDLER:
//!     ULONG ExceptionHandler (RVA), language-specific data...
//!   or, with UNW_FLAG_CHAININFO:
//!     RUNTIME_FUNCTION of the chained (parent) unwind info
//!
//! UNWIND_CODE (one 16-bit slot)
//!   UBYTE CodeOffset              end of the prolog instruction, from the prolog's start
//!   UBYTE UnwindOp : 4, OpInfo : 4
//! ```
//!
//! The array is sorted by descending `CodeOffset`, so it lists the prolog's
//! operations last to first, the order in which unwinding undoes them.

use crate::consts::*;
use crate::pdata::RuntimeFunction;
use crate::{Memory, UnwindError, read_u8, read_u16, read_u32};
use alloc::vec::Vec;

/// The `UNWIND_INFO` version Learn documents ("currently 1").
pub const UNWIND_INFO_VERSION: u8 = 1;

/// One prolog operation, decoded from its unwind code (and the slots after
/// it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindOp {
    /// `UWOP_PUSH_NONVOL`: `push <reg>` (RSP -= 8).
    PushNonvol { reg: u8 },
    /// `UWOP_ALLOC_SMALL` (8 to 128 bytes) or `UWOP_ALLOC_LARGE` (up to
    /// 4 GiB - 8): `sub rsp, <size>`.
    Alloc { size: u32 },
    /// `UWOP_SET_FPREG`: the frame register is set to RSP +
    /// `FrameOffset * 16`.
    SetFpReg,
    /// `UWOP_SAVE_NONVOL` / `_FAR`: `mov [rsp + offset], <reg>`.
    SaveNonvol { reg: u8, offset: u32 },
    /// `UWOP_SAVE_XMM128` / `_FAR`: `movaps [rsp + offset], xmm<reg>`.
    SaveXmm128 { reg: u8, offset: u32 },
    /// `UWOP_PUSH_MACHFRAME`: a machine frame (an interrupt or exception)
    /// was pushed, with an error code if `error_code`.
    PushMachFrame { error_code: bool },
}

/// An operation and the prolog offset it is complete at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnwindEntry {
    /// `CodeOffset`: the offset from the start of the prolog of the
    /// instruction after the one that does the operation.
    pub prolog_offset: u8,
    pub op: UnwindOp,
}

/// A decoded `UNWIND_INFO`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnwindInfo {
    /// Where it was read (absolute).
    pub address: u64,
    pub version: u8,
    pub flags: u8,
    pub prolog_size: u8,
    /// 0 for none; else the register number of the frame pointer.
    pub frame_register: u8,
    /// `FrameOffset`, scaled: the frame register is set to RSP + this * 16.
    pub frame_offset: u8,
    /// The raw code slots (`CountOfCodes` of them).
    pub codes: Vec<u16>,
    /// `ExceptionHandler` (RVA), with `UNW_FLAG_EHANDLER` / `UHANDLER`.
    pub handler: Option<u32>,
    /// The chained entry, with `UNW_FLAG_CHAININFO`.
    pub chained: Option<RuntimeFunction>,
}

impl UnwindInfo {
    /// Read and check the `UNWIND_INFO` at `addr` (absolute).
    pub fn read(mem: &dyn Memory, addr: u64) -> Result<UnwindInfo, UnwindError> {
        if !addr.is_multiple_of(4) {
            return Err(UnwindError::Misaligned(addr));
        }
        let version_flags = read_u8(mem, addr)?;
        let (version, flags) = (version_flags & 0x7, version_flags >> 3);
        if version != UNWIND_INFO_VERSION {
            return Err(UnwindError::BadVersion(version));
        }
        let all = UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER | UNW_FLAG_CHAININFO;
        let handlers = UNW_FLAG_EHANDLER | UNW_FLAG_UHANDLER;
        if flags & !all != 0 || (flags & UNW_FLAG_CHAININFO != 0 && flags & handlers != 0) {
            return Err(UnwindError::BadFlags(flags));
        }
        let at = |off: u64| addr.checked_add(off).ok_or(UnwindError::Overflow);
        let prolog_size = read_u8(mem, at(1)?)?;
        let count = read_u8(mem, at(2)?)?;
        let frame = read_u8(mem, at(3)?)?;
        let mut codes = Vec::with_capacity(usize::from(count));
        for i in 0..u64::from(count) {
            codes.push(read_u16(mem, at(4 + 2 * i)?)?);
        }
        // The array always has an even number of slots.
        let slots = u64::from(count).div_ceil(2) * 2;
        let tail = at(4 + 2 * slots)?;
        let mut info = UnwindInfo {
            address: addr,
            version,
            flags,
            prolog_size,
            frame_register: frame & 0xF,
            frame_offset: frame >> 4,
            codes,
            handler: None,
            chained: None,
        };
        if flags & UNW_FLAG_CHAININFO != 0 {
            info.chained = Some(RuntimeFunction::read(mem, tail)?);
        } else if flags & handlers != 0 {
            info.handler = Some(read_u32(mem, tail)?);
        }
        Ok(info)
    }

    /// Where the trailing data starts (the handler RVA or the chained
    /// entry): after the code slots, rounded up to an even count.
    pub fn tail(&self) -> u64 {
        let slots = (self.codes.len() as u64).div_ceil(2) * 2;
        self.address.wrapping_add(4 + 2 * slots)
    }

    /// The language-specific handler data's address (right after the
    /// handler RVA), when there is a handler.
    pub fn handler_data(&self) -> Option<u64> {
        self.handler.map(|_| self.tail().wrapping_add(4))
    }

    /// The bytes the structure takes, including its trailer (but not the
    /// language-specific data, whose size only its handler knows).
    pub fn size(&self) -> u64 {
        let trailer = if self.chained.is_some() {
            RuntimeFunction::SIZE as u64
        } else if self.handler.is_some() {
            4
        } else {
            0
        };
        self.tail().wrapping_sub(self.address) + trailer
    }

    /// The operations of the code array, in its order (last prolog
    /// instruction first).
    pub fn ops(&self) -> Result<Vec<UnwindEntry>, UnwindError> {
        decode(&self.codes)
    }
}

/// Decode an unwind code array into operations. Every slot must belong to
/// an operation; undocumented operation codes or operation info are
/// [`UnwindError::BadCode`], and an operation whose extra slots are missing
/// is [`UnwindError::TruncatedCodes`].
pub fn decode(codes: &[u16]) -> Result<Vec<UnwindEntry>, UnwindError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < codes.len() {
        let slot = codes[i];
        let prolog_offset = (slot & 0xFF) as u8;
        let op = ((slot >> 8) & 0xF) as u8;
        let info = (slot >> 12) as u8;
        let extra = |n: usize| -> Result<&[u16], UnwindError> {
            codes.get(i + 1..i + 1 + n).ok_or(UnwindError::TruncatedCodes)
        };
        let (decoded, slots) = match op {
            UWOP_PUSH_NONVOL => (UnwindOp::PushNonvol { reg: info }, 1),
            UWOP_ALLOC_LARGE if info == 0 => (UnwindOp::Alloc { size: u32::from(extra(1)?[0]) * 8 }, 2),
            UWOP_ALLOC_LARGE if info == 1 => {
                let s = extra(2)?;
                (UnwindOp::Alloc { size: u32::from(s[0]) | u32::from(s[1]) << 16 }, 3)
            }
            UWOP_ALLOC_SMALL => (UnwindOp::Alloc { size: u32::from(info) * 8 + 8 }, 1),
            UWOP_SET_FPREG => (UnwindOp::SetFpReg, 1),
            UWOP_SAVE_NONVOL => (UnwindOp::SaveNonvol { reg: info, offset: u32::from(extra(1)?[0]) * 8 }, 2),
            UWOP_SAVE_NONVOL_FAR => {
                let s = extra(2)?;
                (UnwindOp::SaveNonvol { reg: info, offset: u32::from(s[0]) | u32::from(s[1]) << 16 }, 3)
            }
            UWOP_SAVE_XMM128 => (UnwindOp::SaveXmm128 { reg: info, offset: u32::from(extra(1)?[0]) * 16 }, 2),
            UWOP_SAVE_XMM128_FAR => {
                let s = extra(2)?;
                (UnwindOp::SaveXmm128 { reg: info, offset: u32::from(s[0]) | u32::from(s[1]) << 16 }, 3)
            }
            UWOP_PUSH_MACHFRAME if info <= 1 => (UnwindOp::PushMachFrame { error_code: info == 1 }, 1),
            _ => return Err(UnwindError::BadCode { slot: i }),
        };
        out.push(UnwindEntry { prolog_offset, op: decoded });
        i += slots;
    }
    Ok(out)
}

/// Encode operations as unwind code slots (the inverse of [`decode`]),
/// choosing the shortest documented form. For tests and Poly's own tools
/// that emit unwind data.
pub fn encode(entries: &[UnwindEntry]) -> Vec<u16> {
    let slot = |off: u8, op: u8, info: u8| u16::from(off) | u16::from(op) << 8 | u16::from(info) << 12;
    let mut out = Vec::new();
    for e in entries {
        let o = e.prolog_offset;
        match e.op {
            UnwindOp::PushNonvol { reg } => out.push(slot(o, UWOP_PUSH_NONVOL, reg)),
            UnwindOp::Alloc { size } if (8..=128).contains(&size) && size % 8 == 0 => {
                out.push(slot(o, UWOP_ALLOC_SMALL, (size / 8 - 1) as u8));
            }
            UnwindOp::Alloc { size } if size % 8 == 0 && size / 8 <= 0xFFFF => {
                out.extend([slot(o, UWOP_ALLOC_LARGE, 0), (size / 8) as u16]);
            }
            UnwindOp::Alloc { size } => out.extend([slot(o, UWOP_ALLOC_LARGE, 1), size as u16, (size >> 16) as u16]),
            UnwindOp::SetFpReg => out.push(slot(o, UWOP_SET_FPREG, 0)),
            UnwindOp::SaveNonvol { reg, offset } if offset % 8 == 0 && offset / 8 <= 0xFFFF => {
                out.extend([slot(o, UWOP_SAVE_NONVOL, reg), (offset / 8) as u16]);
            }
            UnwindOp::SaveNonvol { reg, offset } => {
                out.extend([slot(o, UWOP_SAVE_NONVOL_FAR, reg), offset as u16, (offset >> 16) as u16]);
            }
            UnwindOp::SaveXmm128 { reg, offset } if offset % 16 == 0 && offset / 16 <= 0xFFFF => {
                out.extend([slot(o, UWOP_SAVE_XMM128, reg), (offset / 16) as u16]);
            }
            UnwindOp::SaveXmm128 { reg, offset } => {
                out.extend([slot(o, UWOP_SAVE_XMM128_FAR, reg), offset as u16, (offset >> 16) as u16]);
            }
            UnwindOp::PushMachFrame { error_code } => out.push(slot(o, UWOP_PUSH_MACHFRAME, u8::from(error_code))),
        }
    }
    out
}

/// Build the bytes of an `UNWIND_INFO` (version 1) with the given fields
/// and trailer, for tests and Poly's own tools. `trailer` is the handler
/// RVA followed by its data, or a chained `RUNTIME_FUNCTION`, as `flags`
/// says.
pub fn build(
    flags: u8,
    prolog_size: u8,
    frame_register: u8,
    frame_offset: u8,
    codes: &[u16],
    trailer: &[u8],
) -> Vec<u8> {
    let mut b = alloc::vec![
        UNWIND_INFO_VERSION | flags << 3,
        prolog_size,
        codes.len() as u8,
        frame_register & 0xF | frame_offset << 4,
    ];
    for c in codes {
        b.extend_from_slice(&c.to_le_bytes());
    }
    if codes.len() % 2 == 1 {
        b.extend_from_slice(&[0, 0]);
    }
    b.extend_from_slice(trailer);
    b
}
