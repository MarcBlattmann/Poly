//! The x64 exception directory (spec "The .pdata Section"): an array of
//! `RUNTIME_FUNCTION` entries sorted by address, each pointing at unwind
//! data whose format is described on Microsoft's "x64 exception handling"
//! page (`UNWIND_INFO`, `UNWIND_CODE`). Parsed here for later SEH unwinding.

use crate::headers::Pe;
use crate::read::{bytes, u8_at, u16_at, u32_at};
use crate::view::View;
use crate::{Error, dir, machine};

/// Size of a `RUNTIME_FUNCTION` entry.
const RUNTIME_FUNCTION: usize = 12;

/// `UNW_FLAG_*` values of [`UnwindInfo::flags`].
pub mod unwind_flags {
    pub const EHANDLER: u8 = 0x1;
    pub const UHANDLER: u8 = 0x2;
    pub const CHAININFO: u8 = 0x4;
}

/// One function's entry: its code range and unwind data, as RVAs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuntimeFunction {
    pub begin_address: u32,
    pub end_address: u32,
    pub unwind_info_address: u32,
}

impl RuntimeFunction {
    fn parse(b: &[u8]) -> Option<Self> {
        Some(RuntimeFunction {
            begin_address: u32_at(b, 0)?,
            end_address: u32_at(b, 4)?,
            unwind_info_address: u32_at(b, 8)?,
        })
    }
}

/// The exception directory's entries.
#[derive(Clone, Copy, Debug)]
pub struct RuntimeFunctions<'a> {
    bytes: &'a [u8],
}

impl<'a> RuntimeFunctions<'a> {
    pub fn len(&self) -> usize {
        self.bytes.len() / RUNTIME_FUNCTION
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<RuntimeFunction> {
        RuntimeFunction::parse(bytes(self.bytes, index.checked_mul(RUNTIME_FUNCTION)?, RUNTIME_FUNCTION)?)
    }

    pub fn iter(&self) -> impl Iterator<Item = RuntimeFunction> + 'a {
        let this = *self;
        (0..this.len()).filter_map(move |i| this.get(i))
    }

    /// The entry whose `[begin, end)` contains `rva` (binary search; the
    /// table is sorted by address).
    pub fn lookup(&self, rva: u32) -> Option<RuntimeFunction> {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            let f = self.get(mid)?;
            if rva < f.begin_address {
                hi = mid;
            } else if rva >= f.end_address {
                lo = mid + 1;
            } else {
                return Some(f);
            }
        }
        None
    }
}

/// One decoded unwind operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnwindOp {
    /// `UWOP_PUSH_NONVOL`: push of integer register `reg` (0 = RAX ... 15 = R15).
    PushNonvol { reg: u8 },
    /// `UWOP_ALLOC_LARGE` / `UWOP_ALLOC_SMALL`: stack allocation of `size` bytes.
    Alloc { size: u32 },
    /// `UWOP_SET_FPREG`: the frame register is set to RSP + frame offset.
    SetFpreg,
    /// `UWOP_SAVE_NONVOL(_FAR)`: register `reg` saved at RSP + `offset`.
    SaveNonvol { reg: u8, offset: u32 },
    /// `UWOP_SAVE_XMM128(_FAR)`: XMM`reg` saved at RSP + `offset`.
    SaveXmm128 { reg: u8, offset: u32 },
    /// `UWOP_PUSH_MACHFRAME`: a machine frame (with an error code if set).
    PushMachframe { error_code: bool },
    /// An operation this decoder does not know; it takes one slot.
    Other { op: u8, info: u8 },
}

/// An unwind code: the prolog offset just past the instruction and what it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnwindCode {
    pub code_offset: u8,
    pub op: UnwindOp,
}

/// Iterator over an unwind info's codes, decoding multi-slot operations.
/// It stops after the first error.
#[derive(Clone, Debug)]
pub struct UnwindCodes<'a> {
    slots: &'a [u8],
    pos: usize,
}

impl UnwindCodes<'_> {
    fn slot(&self, i: usize) -> Result<u16, Error> {
        u16_at(self.slots, self.pos + 2 * i).ok_or(Error::Malformed("unwind code needs more slots"))
    }

    fn decode(&self) -> Result<(UnwindCode, usize), Error> {
        let code_offset = u8_at(self.slots, self.pos).ok_or(Error::Truncated)?;
        let b = u8_at(self.slots, self.pos + 1).ok_or(Error::Truncated)?;
        let (op, info) = (b & 0xf, b >> 4);
        let (op, used) = match op {
            0 => (UnwindOp::PushNonvol { reg: info }, 1),
            1 => match info {
                0 => (UnwindOp::Alloc { size: u32::from(self.slot(1)?) * 8 }, 2),
                1 => (UnwindOp::Alloc { size: u32::from(self.slot(1)?) | u32::from(self.slot(2)?) << 16 }, 3),
                _ => return Err(Error::Malformed("UWOP_ALLOC_LARGE with bad info")),
            },
            2 => (UnwindOp::Alloc { size: u32::from(info) * 8 + 8 }, 1),
            3 => (UnwindOp::SetFpreg, 1),
            4 => (UnwindOp::SaveNonvol { reg: info, offset: u32::from(self.slot(1)?) * 8 }, 2),
            5 => (
                UnwindOp::SaveNonvol { reg: info, offset: u32::from(self.slot(1)?) | u32::from(self.slot(2)?) << 16 },
                3,
            ),
            8 => (UnwindOp::SaveXmm128 { reg: info, offset: u32::from(self.slot(1)?) * 16 }, 2),
            9 => (
                UnwindOp::SaveXmm128 { reg: info, offset: u32::from(self.slot(1)?) | u32::from(self.slot(2)?) << 16 },
                3,
            ),
            10 => (UnwindOp::PushMachframe { error_code: info != 0 }, 1),
            _ => (UnwindOp::Other { op, info }, 1),
        };
        Ok((UnwindCode { code_offset, op }, used))
    }
}

impl Iterator for UnwindCodes<'_> {
    type Item = Result<UnwindCode, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.slots.len() {
            return None;
        }
        match self.decode() {
            Ok((code, used)) => {
                self.pos += 2 * used;
                Some(Ok(code))
            }
            Err(e) => {
                self.pos = self.slots.len();
                Some(Err(e))
            }
        }
    }
}

/// An `UNWIND_INFO` structure.
#[derive(Clone, Copy, Debug)]
pub struct UnwindInfo<'a> {
    pub version: u8,
    /// `UNW_FLAG_*`, see [`unwind_flags`].
    pub flags: u8,
    pub size_of_prolog: u8,
    pub count_of_codes: u8,
    /// Frame register number, 0 if none.
    pub frame_register: u8,
    /// Scaled offset from RSP to the frame pointer, in bytes (field × 16).
    pub frame_offset: u32,
    /// RVA of the language-specific handler (`EHANDLER`/`UHANDLER` set).
    pub handler: Option<u32>,
    /// RVA of the handler's data, just after the handler RVA.
    pub handler_data: Option<u32>,
    /// The entry this one continues (`CHAININFO` set).
    pub chained: Option<RuntimeFunction>,
    slots: &'a [u8],
}

impl<'a> UnwindInfo<'a> {
    fn parse(view: &View<'a>, rva: u32) -> Result<Self, Error> {
        let rest = view.rest(rva)?;
        let h = bytes(rest, 0, 4).ok_or(Error::Truncated)?;
        let (version, flags) = (h[0] & 0x7, h[0] >> 3);
        let count = h[2];
        let slots = bytes(rest, 4, usize::from(count) * 2).ok_or(Error::Truncated)?;
        // The codes array is padded to an even number of slots.
        let tail = 4 + usize::from(count).next_multiple_of(2) * 2;
        let mut info = UnwindInfo {
            version,
            flags,
            size_of_prolog: h[1],
            count_of_codes: count,
            frame_register: h[3] & 0xf,
            frame_offset: u32::from(h[3] >> 4) * 16,
            handler: None,
            handler_data: None,
            chained: None,
            slots,
        };
        if flags & unwind_flags::CHAININFO != 0 {
            let f = bytes(rest, tail, RUNTIME_FUNCTION).and_then(RuntimeFunction::parse).ok_or(Error::Truncated)?;
            info.chained = Some(f);
        } else if flags & (unwind_flags::EHANDLER | unwind_flags::UHANDLER) != 0 {
            info.handler = Some(u32_at(rest, tail).ok_or(Error::Truncated)?);
            info.handler_data = rva.checked_add(tail as u32 + 4);
        }
        Ok(info)
    }

    pub fn codes(&self) -> UnwindCodes<'a> {
        UnwindCodes { slots: self.slots, pos: 0 }
    }
}

impl<'a> Pe<'a> {
    /// The x64 exception directory, if the image has one. Other machines'
    /// `.pdata` formats differ and give [`Error::UnsupportedMachine`].
    pub fn runtime_functions(&self) -> Result<Option<RuntimeFunctions<'a>>, Error> {
        let Some(d) = self.info().directory(dir::EXCEPTION).filter(|d| d.size != 0) else { return Ok(None) };
        if self.info().machine() != machine::AMD64 {
            return Err(Error::UnsupportedMachine(self.info().machine()));
        }
        let len = d.size as usize / RUNTIME_FUNCTION * RUNTIME_FUNCTION;
        Ok(Some(RuntimeFunctions { bytes: self.view.slice(d.virtual_address, len)? }))
    }

    /// The unwind data of a runtime function.
    pub fn unwind_info(&self, f: &RuntimeFunction) -> Result<UnwindInfo<'a>, Error> {
        UnwindInfo::parse(&self.view, f.unwind_info_address)
    }
}
