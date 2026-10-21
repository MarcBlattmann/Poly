//! The function table of an x64 image: the `RUNTIME_FUNCTION` entries of its
//! exception directory (`.pdata`).
//!
//! PE Format, ".pdata section" (x64): each entry is 12 bytes, the function's
//! start RVA, its end RVA and the RVA of its unwind information. Learn ("x64
//! exception handling", "struct RUNTIME_FUNCTION"): the table is sorted by
//! start address, so the entry covering an address is found by binary
//! search, as `RtlLookupFunctionEntry` does. An address no entry covers is
//! in a leaf function (or not in the image's code at all).

use crate::{Memory, UnwindError, read_u32};

/// One function (or function fragment) of the table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RuntimeFunction {
    /// `BeginAddress`: RVA of the first byte.
    pub begin: u32,
    /// `EndAddress`: RVA one past the last byte.
    pub end: u32,
    /// `UnwindData` / `UnwindInfoAddress`: RVA of the `UNWIND_INFO`.
    pub unwind: u32,
}

impl RuntimeFunction {
    /// `sizeof(RUNTIME_FUNCTION)`.
    pub const SIZE: usize = 12;

    pub fn from_bytes(b: &[u8; 12]) -> RuntimeFunction {
        let w = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        RuntimeFunction { begin: w(0), end: w(4), unwind: w(8) }
    }

    pub fn to_bytes(&self) -> [u8; 12] {
        let mut b = [0u8; 12];
        b[0..4].copy_from_slice(&self.begin.to_le_bytes());
        b[4..8].copy_from_slice(&self.end.to_le_bytes());
        b[8..12].copy_from_slice(&self.unwind.to_le_bytes());
        b
    }

    /// Whether the function covers `rva`.
    pub fn contains(&self, rva: u32) -> bool {
        self.begin <= rva && rva < self.end
    }

    /// Read the entry at `addr`.
    pub fn read(mem: &dyn Memory, addr: u64) -> Result<RuntimeFunction, UnwindError> {
        let end_addr = addr.checked_add(8).ok_or(UnwindError::Overflow)?;
        Ok(RuntimeFunction {
            begin: read_u32(mem, addr)?,
            end: read_u32(mem, addr + 4)?,
            unwind: read_u32(mem, end_addr)?,
        })
    }
}

/// Find the entry of a sorted table covering `rva`, given the number of
/// entries and a way to read entry `i`. Returns its index and the entry.
/// An unsorted (malformed) table gives a wrong answer or none, but never
/// fails in any other way; an entry that cannot be read is an error.
pub fn search<E>(
    count: usize,
    rva: u32,
    mut entry: impl FnMut(usize) -> Result<RuntimeFunction, E>,
) -> Result<Option<(usize, RuntimeFunction)>, E> {
    // The last entry that begins at or before `rva`.
    let (mut lo, mut hi) = (0usize, count);
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if entry(mid)?.begin <= rva {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        return Ok(None);
    }
    let found = entry(lo - 1)?;
    Ok(found.contains(rva).then_some((lo - 1, found)))
}

/// Find the entry covering `rva` in the table of `count` entries at the
/// absolute address `table` (`RtlLookupFunctionEntry` for one image).
pub fn lookup(
    mem: &dyn Memory,
    table: u64,
    count: usize,
    rva: u32,
) -> Result<Option<(usize, RuntimeFunction)>, UnwindError> {
    search(count, rva, |i| {
        let at = (i as u64)
            .checked_mul(RuntimeFunction::SIZE as u64)
            .and_then(|o| table.checked_add(o))
            .ok_or(UnwindError::Overflow)?;
        RuntimeFunction::read(mem, at)
    })
}

/// A function table held as bytes: an exception directory read from an
/// image file.
#[derive(Clone, Copy, Debug)]
pub struct FunctionTable<'a> {
    bytes: &'a [u8],
}

impl<'a> FunctionTable<'a> {
    /// Take the exception directory's bytes. Its size must be a multiple of
    /// 12 (PE Format: the directory is an array of entries).
    pub fn new(bytes: &'a [u8]) -> Result<FunctionTable<'a>, UnwindError> {
        if !bytes.len().is_multiple_of(RuntimeFunction::SIZE) {
            return Err(UnwindError::BadTable);
        }
        Ok(FunctionTable { bytes })
    }

    pub fn len(&self) -> usize {
        self.bytes.len() / RuntimeFunction::SIZE
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Entry `i`, if there is one.
    pub fn get(&self, i: usize) -> Option<RuntimeFunction> {
        let at = i.checked_mul(RuntimeFunction::SIZE)?;
        let b: &[u8; 12] = self.bytes.get(at..at.checked_add(RuntimeFunction::SIZE)?)?.try_into().ok()?;
        Some(RuntimeFunction::from_bytes(b))
    }

    pub fn iter(&self) -> impl Iterator<Item = RuntimeFunction> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }

    /// The entry covering `rva`.
    pub fn lookup(&self, rva: u32) -> Option<(usize, RuntimeFunction)> {
        search(self.len(), rva, |i| self.get(i).ok_or(())).ok().flatten()
    }

    /// Check what the binary search relies on: every entry ends after it
    /// begins, and the entries are sorted and do not overlap.
    pub fn validate(&self) -> Result<(), UnwindError> {
        let mut prev_end = 0u32;
        for (i, f) in self.iter().enumerate() {
            if f.end <= f.begin || (i > 0 && f.begin < prev_end) {
                return Err(UnwindError::BadTable);
            }
            prev_end = f.end;
        }
        Ok(())
    }
}
