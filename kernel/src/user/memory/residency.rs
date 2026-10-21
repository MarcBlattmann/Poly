//! Which pages are in, and bringing them in (ADR 0064): what mincore(2)
//! reports and what mlock(2) does. Nothing is ever paged out, so a page
//! that is in stays in and a lock has nothing more to do.
//!
//! A page is resident when it has a frame here (mapped, parked under
//! `PROT_NONE`, or a shared frame), or when it is a page of a file
//! mapping within the file whose source keeps pages in the page cache (it
//! is in the cache, or is read into it at the next access without waiting
//! for a device: tmpfs, or a page just written).

use super::{Access, Backing, Memory, PAGE_SIZE, Prot};

/// Why [`Memory::populate`] or [`Memory::residency`] failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// Part of the range is not mapped (or, to populate it, is `PROT_NONE`).
    NotMapped,
}

impl Memory {
    /// Whether `[addr, end)` is covered by regions without a gap, each
    /// passing `ok`.
    fn covered(&self, addr: u64, end: u64, ok: impl Fn(Prot) -> bool) -> bool {
        let inner = self.inner.lock();
        let mut at = addr;
        for v in inner.map.regions_in(addr, end) {
            if v.start > at || !ok(v.prot) {
                return false;
            }
            at = v.end;
            if at >= end {
                return true;
            }
        }
        at >= end
    }

    /// mlock(2) of `[addr, addr + len)` (page aligned) with `populate`, or
    /// munlock(2) without: the range must be mapped, and to populate it
    /// accessible (not `PROT_NONE`); its pages are then brought in as a
    /// read would. Pages past the end of a mapped file stay out.
    pub fn populate(&self, addr: u64, len: u64, populate: bool) -> Result<(), RangeError> {
        let end = addr.checked_add(len).ok_or(RangeError::NotMapped)?;
        let accessible = |p: Prot| !populate || p != Prot::NONE;
        if !self.covered(addr, end, accessible) {
            return Err(RangeError::NotMapped);
        }
        if populate {
            let mut page = addr;
            while page < end {
                // A page wholly past the end of its file cannot come in
                // (it would be SIGBUS); the lock still succeeds.
                let _ = self.fault(page, Access::Read);
                page += PAGE_SIZE;
            }
        }
        Ok(())
    }

    /// mincore(2): 1 or 0 into `out[i]` for whether page `i` from `addr`
    /// (page aligned) is resident (see the module docs).
    pub fn residency(&self, addr: u64, out: &mut [u8]) -> Result<(), RangeError> {
        let end = (out.len() as u64).checked_mul(PAGE_SIZE).and_then(|n| addr.checked_add(n));
        let end = end.ok_or(RangeError::NotMapped)?;
        if !self.covered(addr, end, |_| true) {
            return Err(RangeError::NotMapped);
        }
        let inner = self.inner.lock();
        for (i, byte) in out.iter_mut().enumerate() {
            let page = addr + i as u64 * PAGE_SIZE;
            let mut resident = inner.parked.contains_key(&page)
                || inner.shared.contains_key(&page)
                || inner.cached.contains_key(&page)
                || self.space.translate(page).is_some();
            if !resident
                && let Some(v) = inner.map.find(page)
                && let (Backing::File { file, .. }, Some(offset)) = (&v.backing, v.file_offset(page))
            {
                resident = file.source().shareable() && offset < file.source().len().min(file.limit());
            }
            *byte = u8::from(resident);
        }
        Ok(())
    }
}
