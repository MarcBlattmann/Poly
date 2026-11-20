//! Working sets: `SetProcessWorkingSetSize(Ex)`, `GetProcessWorkingSetSize(Ex)` and
//! `K32QueryWorkingSetEx` (`specs/windows/kernel32.dll/{SetProcessWorkingSetSize,
//! SetProcessWorkingSetSizeEx,K32QueryWorkingSetEx}.yaml`).
//!
//! What a process keeps is its minimum, its maximum and the hard-limit flags; the checks the
//! sheets observed before the handle is looked at are here, the rest (the handle, its rights, a
//! size beyond the machine's memory) is the kernel's. Constants: `winnt.h`
//! (`QUOTA_LIMITS_HARDWS_*`), `winerror.h`; the attribute word of `PSAPI_WORKING_SET_EX_BLOCK`
//! (`psapi.h`, Learn).

use crate::error::ERROR_INVALID_PARAMETER;

/// A new process's working set limits and flags (the sheets: 0x32000, 0x159000, both hard limits
/// disabled).
pub const DEFAULT_MINIMUM: u64 = 0x32000;
pub const DEFAULT_MAXIMUM: u64 = 0x159000;
pub const DEFAULT_FLAGS: u32 = QUOTA_LIMITS_HARDWS_MIN_DISABLE | QUOTA_LIMITS_HARDWS_MAX_DISABLE;

/// `QUOTA_LIMITS_HARDWS_*` (`winnt.h`).
pub const QUOTA_LIMITS_HARDWS_MIN_ENABLE: u32 = 0x1;
pub const QUOTA_LIMITS_HARDWS_MIN_DISABLE: u32 = 0x2;
pub const QUOTA_LIMITS_HARDWS_MAX_ENABLE: u32 = 0x4;
pub const QUOTA_LIMITS_HARDWS_MAX_DISABLE: u32 = 0x8;

/// `ERROR_BAD_LENGTH` (`winerror.h`): `K32QueryWorkingSetEx` with less than one entry.
pub const ERROR_BAD_LENGTH: u32 = 24;

/// What a valid `SetProcessWorkingSetSize(Ex)` asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    /// `(-1, -1)`: remove pages from the working set; the limits stay.
    Trim,
    /// New limits; a maximum of -1 keeps the maximum.
    Set { minimum: u64, maximum: Option<u64> },
}

/// The flags of `SetProcessWorkingSetSizeEx`: an enable and a disable of the same limit together
/// are `ERROR_INVALID_PARAMETER`; undefined bits are ignored. Judged before the handle.
pub fn check_flags(flags: u32) -> Result<(), u32> {
    let both = |e: u32, d: u32| flags & (e | d) == e | d;
    if both(QUOTA_LIMITS_HARDWS_MIN_ENABLE, QUOTA_LIMITS_HARDWS_MIN_DISABLE)
        || both(QUOTA_LIMITS_HARDWS_MAX_ENABLE, QUOTA_LIMITS_HARDWS_MAX_DISABLE)
    {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(())
}

/// The sizes, with `all_ones` the caller's `(SIZE_T)-1`: `(-1, -1)` is the trim; a minimum of 0 or
/// -1 (with any other maximum), or a minimum above the maximum, is `ERROR_INVALID_PARAMETER`; a
/// maximum of -1 keeps the maximum.
pub fn check_sizes(minimum: u64, maximum: u64, all_ones: u64) -> Result<Request, u32> {
    if minimum == all_ones && maximum == all_ones {
        return Ok(Request::Trim);
    }
    if minimum == 0 || minimum == all_ones {
        return Err(ERROR_INVALID_PARAMETER);
    }
    if maximum == all_ones {
        return Ok(Request::Set { minimum, maximum: None });
    }
    if minimum > maximum {
        return Err(ERROR_INVALID_PARAMETER);
    }
    Ok(Request::Set { minimum, maximum: Some(maximum) })
}

/// The flags after a call with `new`: each limit takes its enable or disable bit when the call
/// names one, else keeps its state.
pub fn merge_flags(old: u32, new: u32) -> u32 {
    let mut f = old;
    for (enable, disable) in [
        (QUOTA_LIMITS_HARDWS_MIN_ENABLE, QUOTA_LIMITS_HARDWS_MIN_DISABLE),
        (QUOTA_LIMITS_HARDWS_MAX_ENABLE, QUOTA_LIMITS_HARDWS_MAX_DISABLE),
    ] {
        if new & (enable | disable) != 0 {
            f = (f & !(enable | disable)) | (new & (enable | disable));
        }
    }
    f
}

/// One process's working set state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkingSet {
    pub minimum: u64,
    pub maximum: u64,
    pub flags: u32,
}

impl Default for WorkingSet {
    fn default() -> Self {
        WorkingSet { minimum: DEFAULT_MINIMUM, maximum: DEFAULT_MAXIMUM, flags: DEFAULT_FLAGS }
    }
}

impl WorkingSet {
    /// Apply a checked request; `limit` is the most a limit may be (the machine's memory): above
    /// it the call fails (`false`: `ERROR_NO_SYSTEM_RESOURCES`, the kernel's
    /// `STATUS_INSUFFICIENT_RESOURCES`) and nothing changes.
    pub fn apply(&mut self, r: Request, flags: u32, limit: u64) -> bool {
        if let Request::Set { minimum, maximum } = r {
            if minimum > limit || maximum.is_some_and(|m| m > limit) {
                return false;
            }
            self.minimum = minimum;
            if let Some(m) = maximum {
                self.maximum = m;
            }
        }
        self.flags = merge_flags(self.flags, flags);
        true
    }

    /// The bytes of Poly's process information class for it (three `u64`s: minimum, maximum,
    /// flags), the same for 32- and 64-bit callers.
    pub fn to_bytes(self) -> [u8; 24] {
        let mut b = [0u8; 24];
        b[0..8].copy_from_slice(&self.minimum.to_le_bytes());
        b[8..16].copy_from_slice(&self.maximum.to_le_bytes());
        b[16..24].copy_from_slice(&u64::from(self.flags).to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8; 24]) -> WorkingSet {
        let at = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        WorkingSet { minimum: at(0), maximum: at(8), flags: at(16) as u32 }
    }
}

/// A request for Poly's set class: the request kind (0 trim, 1 set, 2 set keeping the maximum),
/// the minimum, the maximum and the flags, four `u64`s.
pub fn request_bytes(r: Request, flags: u32) -> [u8; 32] {
    let (kind, min, max) = match r {
        Request::Trim => (0u64, 0, 0),
        Request::Set { minimum, maximum: Some(m) } => (1, minimum, m),
        Request::Set { minimum, maximum: None } => (2, minimum, 0),
    };
    let mut b = [0u8; 32];
    for (i, v) in [kind, min, max, u64::from(flags)].into_iter().enumerate() {
        b[8 * i..8 * i + 8].copy_from_slice(&v.to_le_bytes());
    }
    b
}

/// The request [`request_bytes`] wrote (`None` for a kind it does not write).
pub fn request_from_bytes(b: &[u8; 32]) -> Option<(Request, u32)> {
    let at = |i: usize| u64::from_le_bytes(b[8 * i..8 * i + 8].try_into().expect("8 bytes"));
    let r = match at(0) {
        0 => Request::Trim,
        1 => Request::Set { minimum: at(1), maximum: Some(at(2)) },
        2 => Request::Set { minimum: at(1), maximum: None },
        _ => return None,
    };
    Some((r, at(3) as u32))
}

/// The attribute word of `PSAPI_WORKING_SET_EX_BLOCK` (Learn): bit 0 valid, bits 1-3 the share
/// count, bits 4-14 the page protection (`PAGE_*`), bit 15 shared, bit 22 locked. A page that is
/// not in the working set is all zero.
pub fn ws_ex_attributes(valid: bool, protection: u32, shared: bool, locked: bool) -> u64 {
    if !valid {
        return 0;
    }
    let mut a = 1u64 | (u64::from(protection & 0x7FF) << 4);
    if shared {
        a |= 1 << 1 | 1 << 15;
    }
    if locked {
        a |= 1 << 22;
    }
    a
}

/// The pages a process locked with `VirtualLock` (Learn: "VirtualLock", "VirtualUnlock"), kept
/// by `kernelbase` for its own process: `K32QueryWorkingSetEx` reports them locked
/// (`specs/windows/kernel32.dll/K32QueryWorkingSetEx.yaml`, rule `lock`). Page numbers (address
/// / 4096), sorted, each once.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockedPages {
    pages: alloc::vec::Vec<u64>,
}

/// The pages `[address, address + size)` touches (Learn: the range is all pages that contain one
/// or more bytes of it); `size` 0 is none.
pub fn pages_of(address: u64, size: u64) -> core::ops::Range<u64> {
    if size == 0 {
        return 0..0;
    }
    let first = address / 4096;
    let last = address.saturating_add(size - 1) / 4096;
    first..last + 1
}

impl LockedPages {
    pub const fn new() -> LockedPages {
        LockedPages { pages: alloc::vec::Vec::new() }
    }

    /// Lock every page of the range (locking a locked page again is no error).
    pub fn lock(&mut self, pages: core::ops::Range<u64>) {
        for p in pages {
            if let Err(i) = self.pages.binary_search(&p) {
                self.pages.insert(i, p);
            }
        }
    }

    /// Unlock the range: whether every page was locked (Learn, "VirtualUnlock": pages that are
    /// not locked make the call fail with `ERROR_NOT_LOCKED`; the locked ones are unlocked all the
    /// same).
    pub fn unlock(&mut self, pages: core::ops::Range<u64>) -> bool {
        let mut all = true;
        for p in pages {
            match self.pages.binary_search(&p) {
                Ok(i) => {
                    self.pages.remove(i);
                }
                Err(_) => all = false,
            }
        }
        all
    }

    /// Forget the range (the memory was released).
    pub fn forget(&mut self, pages: core::ops::Range<u64>) {
        self.pages.retain(|p| !pages.contains(p));
    }

    pub fn is_locked(&self, address: u64) -> bool {
        self.pages.binary_search(&(address / 4096)).is_ok()
    }
}

/// The `locked` bit of `PSAPI_WORKING_SET_EX_BLOCK` (bit 22).
pub const WS_EX_LOCKED: u64 = 1 << 22;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locked_pages() {
        let mut l = LockedPages::new();
        assert_eq!(pages_of(0x1000, 0), 0..0);
        assert_eq!(pages_of(0x1FFF, 2), 1..3);
        l.lock(pages_of(0x3000, 0x1000));
        l.lock(pages_of(0x3123, 1));
        assert!(l.is_locked(0x3000) && l.is_locked(0x3FFF) && !l.is_locked(0x4000));
        assert!(!l.unlock(pages_of(0x3000, 0x2000)), "the second page was not locked");
        assert!(!l.is_locked(0x3000));
        l.lock(pages_of(0x5000, 0x3000));
        l.forget(pages_of(0x6000, 0x1000));
        assert!(l.is_locked(0x5000) && !l.is_locked(0x6000) && l.is_locked(0x7000));
        assert!(l.unlock(pages_of(0x7000, 0x10)));
    }

    const ALL: u64 = u64::MAX;
    const MIB: u64 = 1 << 20;

    #[test]
    fn sizes_of_the_sheet() {
        assert_eq!(check_sizes(ALL, ALL, ALL), Ok(Request::Trim));
        assert_eq!(check_sizes(MIB, 4 * MIB, ALL), Ok(Request::Set { minimum: MIB, maximum: Some(4 * MIB) }));
        assert_eq!(check_sizes(2 * MIB, 2 * MIB, ALL), Ok(Request::Set { minimum: 2 * MIB, maximum: Some(2 * MIB) }));
        assert!(check_sizes(1, 1, ALL).is_ok());
        assert!(check_sizes(4096, 8192, ALL).is_ok());
        assert_eq!(check_sizes(MIB, ALL, ALL), Ok(Request::Set { minimum: MIB, maximum: None }));
        for (a, b) in [(2 * MIB, MIB), (0, 0), (0, 4 * MIB), (ALL, 4 * MIB)] {
            assert_eq!(check_sizes(a, b, ALL), Err(87), "{a:#x} {b:#x}");
        }
        // A 32-bit caller's -1.
        assert_eq!(check_sizes(0xFFFF_FFFF, 0xFFFF_FFFF, 0xFFFF_FFFF), Ok(Request::Trim));
    }

    #[test]
    fn flags_of_the_sheet() {
        for bad in [3, 0xC, 0xFFFF_FFFF] {
            assert_eq!(check_flags(bad), Err(87));
        }
        for good in [0, 1, 2, 4, 8, 0x10, 5, 0xA] {
            assert_eq!(check_flags(good), Ok(()));
        }
        assert_eq!(merge_flags(0xA, 1), 0x9);
        assert_eq!(merge_flags(0x9, 2), 0xA);
        assert_eq!(merge_flags(0xA, 4), 0x6);
        assert_eq!(merge_flags(0x6, 8), 0xA);
        assert_eq!(merge_flags(0xA, 0x10), 0xA);
        assert_eq!(merge_flags(0x9, 0), 0x9);
    }

    #[test]
    fn apply_and_limits() {
        let mut ws = WorkingSet::default();
        assert_eq!(ws.to_bytes(), WorkingSet { minimum: 0x32000, maximum: 0x159000, flags: 0xA }.to_bytes());
        assert!(ws.apply(Request::Trim, 0, 1 << 33));
        assert_eq!(ws, WorkingSet::default());
        assert!(ws.apply(Request::Set { minimum: MIB, maximum: Some(4 * MIB) }, 0, 1 << 33));
        assert_eq!((ws.minimum, ws.maximum), (MIB, 4 * MIB));
        assert!(!ws.apply(Request::Set { minimum: 0x7FFF_FFFF_FFFF, maximum: Some(0x7FFF_FFFF_FFFF) }, 0, 1 << 33));
        assert_eq!((ws.minimum, ws.maximum), (MIB, 4 * MIB));
        assert!(ws.apply(Request::Set { minimum: 2 * MIB, maximum: None }, 0, 1 << 33));
        assert_eq!((ws.minimum, ws.maximum), (2 * MIB, 4 * MIB));
        assert_eq!(WorkingSet::from_bytes(&ws.to_bytes()), ws);
        for r in
            [Request::Trim, Request::Set { minimum: 5, maximum: Some(6) }, Request::Set { minimum: 5, maximum: None }]
        {
            assert_eq!(request_from_bytes(&request_bytes(r, 9)), Some((r, 9)));
        }
    }

    #[test]
    fn attribute_words() {
        assert_eq!(ws_ex_attributes(false, 4, true, true), 0);
        assert_eq!(ws_ex_attributes(true, 4, false, false), 0x41);
        assert_eq!(ws_ex_attributes(true, 0x20, true, false) >> 15 & 1, 1);
        assert_eq!(ws_ex_attributes(true, 4, false, true) >> 22 & 1, 1);
        assert_eq!(ws_ex_attributes(true, 0x40, false, false) >> 4 & 0x7FF, 0x40);
    }
}
