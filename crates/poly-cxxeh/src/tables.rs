//! A function's C++ exception tables (the spec's `data_layouts`): every
//! field 32 bits, little endian, `rva` = offset from the image base of the
//! function, 0 = absent.
//!
//! ```text
//! FuncInfo          MagicNumber, MaxState, UnwindMap, NumTryBlocks, TryBlockMap,
//!                   IPMapEntries, IPToStateMap, UnwindHelp
//!                   [, ESTypeList (0x19930521+)] [, EHFlags (0x19930522)]
//! UnwindMapEntry    ToState, Action                       (8 bytes, entry i = state i)
//! TryBlockMapEntry  TryLow, TryHigh, CatchHigh, NumCatches, HandlerArray   (20 bytes)
//! HandlerType       Adjectives, TypeDescriptor, CatchObjOffset, Handler,
//!                   ParentFrameOffset                     (20 bytes)
//! IPToStateMapEntry IP, State                             (8 bytes, sorted by IP)
//! ```

use crate::consts::*;
use crate::{Bad, Memory, at, count, read_i32, read_u32};

/// A function's `FuncInfo`. Fields its magic number does not have are 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FuncInfo {
    /// The magic number (low 29 bits).
    pub magic: u32,
    pub max_state: i32,
    pub unwind_map: u32,
    pub num_try_blocks: u32,
    pub try_block_map: u32,
    pub ip_map_entries: u32,
    pub ip_to_state_map: u32,
    pub unwind_help: i32,
    /// 0x19930521 and later.
    pub es_type_list: u32,
    /// 0x19930522 only.
    pub eh_flags: u32,
}

impl FuncInfo {
    /// Read the FuncInfo at `addr`: only the fields its magic number has (a
    /// field the layout does not have is not read, whatever follows).
    pub fn read(mem: &dyn Memory, addr: u64) -> Result<FuncInfo, Bad> {
        let field = |i: u64| -> Result<u32, Bad> { read_u32(mem, addr.checked_add(4 * i).ok_or(Bad::Overflow)?) };
        let magic = field(0)? & MAGIC_MASK;
        if !(MAGIC_V1..=MAGIC_V3).contains(&magic) {
            return Err(Bad::Magic(magic));
        }
        Ok(FuncInfo {
            magic,
            max_state: field(1)? as i32,
            unwind_map: field(2)?,
            num_try_blocks: field(3)?,
            try_block_map: field(4)?,
            ip_map_entries: field(5)?,
            ip_to_state_map: field(6)?,
            unwind_help: field(7)? as i32,
            es_type_list: if magic >= MAGIC_V2 { field(8)? } else { 0 },
            eh_flags: if magic >= MAGIC_V3 { field(9)? } else { 0 },
        })
    }

    /// `EHFlags` bit 0x1: non-C++ exceptions are not this frame's business.
    pub fn synchronous_only(&self) -> bool {
        self.eh_flags & EH_SYNCHRONOUS != 0
    }

    /// `EHFlags` bit 0x4: the function is `noexcept`.
    pub fn is_noexcept(&self) -> bool {
        self.eh_flags & EH_NOEXCEPT != 0
    }
}

/// `UnwindMapEntry`: what leaving state *i* does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnwindEntry {
    pub to_state: i32,
    /// The unwind funclet (0: nothing to do).
    pub action: u32,
}

/// `TryBlockMapEntry`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TryBlock {
    pub try_low: i32,
    pub try_high: i32,
    pub catch_high: i32,
    pub num_catches: u32,
    pub handlers: u32,
}

impl TryBlock {
    /// The try body covers `state`.
    pub fn covers(&self, state: i32) -> bool {
        self.try_low <= state && state <= self.try_high
    }

    /// `state` is one of its catch blocks' states.
    pub fn in_catch(&self, state: i32) -> bool {
        self.try_high < state && state <= self.catch_high
    }
}

/// `HandlerType`: one catch clause.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HandlerType {
    pub adjectives: u32,
    /// The clause's type descriptor (0: `catch(...)`).
    pub type_descriptor: u32,
    /// Frame offset of the catch object (0: none).
    pub catch_obj_offset: i32,
    /// The catch funclet.
    pub handler: u32,
    pub parent_frame_offset: u32,
}

/// A function's tables in an image: where they are and how to read them.
#[derive(Clone, Copy)]
pub struct Func<'a> {
    pub mem: &'a dyn Memory,
    pub image_base: u64,
    pub info: FuncInfo,
}

impl core::fmt::Debug for Func<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Func").field("image_base", &self.image_base).field("info", &self.info).finish()
    }
}

impl<'a> Func<'a> {
    /// The function whose FuncInfo is at `image_base + rva`.
    pub fn read(mem: &'a dyn Memory, image_base: u64, rva: u32) -> Result<Func<'a>, Bad> {
        let info = FuncInfo::read(mem, at(image_base, rva, 0)?)?;
        Ok(Func { mem, image_base, info })
    }

    /// The number of states (`MaxState`), checked.
    pub fn states(&self) -> Result<u32, Bad> {
        let n = u32::try_from(self.info.max_state).map_err(|_| Bad::Count(i64::from(self.info.max_state)))?;
        count(n)
    }

    /// The unwind map entry of `state`.
    pub fn unwind_entry(&self, state: i32) -> Result<UnwindEntry, Bad> {
        let i = u32::try_from(state).map_err(|_| Bad::State(state))?;
        if i >= self.states()? {
            return Err(Bad::State(state));
        }
        let a = at(self.image_base, self.info.unwind_map, 8 * u64::from(i))?;
        Ok(UnwindEntry { to_state: read_i32(self.mem, a)?, action: read_u32(self.mem, a + 4)? })
    }

    /// Try block `i` of the map.
    pub fn try_block(&self, i: u32) -> Result<TryBlock, Bad> {
        if i >= count(self.info.num_try_blocks)? {
            return Err(Bad::Index(i));
        }
        let a = at(self.image_base, self.info.try_block_map, 20 * u64::from(i))?;
        Ok(TryBlock {
            try_low: read_i32(self.mem, a)?,
            try_high: read_i32(self.mem, a + 4)?,
            catch_high: read_i32(self.mem, a + 8)?,
            num_catches: read_u32(self.mem, a + 12)?,
            handlers: read_u32(self.mem, a + 16)?,
        })
    }

    /// Catch clause `j` of a try block.
    pub fn handler(&self, tb: &TryBlock, j: u32) -> Result<HandlerType, Bad> {
        if j >= count(tb.num_catches)? {
            return Err(Bad::Index(j));
        }
        let a = at(self.image_base, tb.handlers, 20 * u64::from(j))?;
        Ok(HandlerType {
            adjectives: read_u32(self.mem, a)?,
            type_descriptor: read_u32(self.mem, a + 4)?,
            catch_obj_offset: read_i32(self.mem, a + 8)?,
            handler: read_u32(self.mem, a + 12)?,
            parent_frame_offset: read_u32(self.mem, a + 16)?,
        })
    }

    /// The state at `rva` (an offset from the image base): the state of the
    /// last IP-to-state entry at or below it, -1 before the first.
    pub fn state_at(&self, rva: u32) -> Result<i32, Bad> {
        let n = count(self.info.ip_map_entries)?;
        let entry = |i: u32| at(self.image_base, self.info.ip_to_state_map, 8 * u64::from(i));
        let (mut lo, mut hi) = (0u32, n);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if read_u32(self.mem, entry(mid)?)? <= rva {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        if lo == 0 { Ok(-1) } else { read_i32(self.mem, entry(lo - 1)? + 4) }
    }
}
