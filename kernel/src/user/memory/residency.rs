//! Which pages are in, and bringing them in (ADR 0064): what mincore(2)
//! reports and what mlock(2) does. Nothing is ever paged out, so a page
//! that is in stays in: a lock is a mark on the region (for `VmLck`, the
//! memory-lock limit and fork) plus, unless it is "on fault", bringing
//! the pages in.
//!
//! A page is resident when it has a frame here (mapped, parked under
//! `PROT_NONE`, or a shared frame), or when it is a page of a file
//! mapping within the file whose source keeps pages in the page cache (it
//! is in the cache, or is read into it at the next access without waiting
//! for a device: tmpfs, or a page just written).

use super::{Access, Backing, Memory, PAGE_SIZE, Prot};
use poly_vm::FutureLock;

/// Why [`Memory::lock`] or [`Memory::residency`] failed.
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

    /// Bring the pages of `[addr, end)` in as a read would. A page wholly
    /// past the end of its file cannot come in (it would be SIGBUS) and is
    /// skipped, like pages that fail.
    fn bring_in(&self, addr: u64, end: u64) {
        let mut page = addr;
        while page < end {
            let _ = self.fault(page, Access::Read);
            page += PAGE_SIZE;
        }
    }

    /// mlock(2) / mlock2(2) (`lock`) or munlock(2) of `[addr, addr + len)`
    /// (page aligned, `len` non-zero): the regions are marked locked or
    /// unlocked in the region map up to the first hole, which is
    /// [`RangeError::NotMapped`] with nothing brought in. With `populate`
    /// every page is then brought in, unless a region is `PROT_NONE`
    /// (`NotMapped`, the lock stays).
    pub fn lock(&self, addr: u64, len: u64, lock: bool, populate: bool) -> Result<(), RangeError> {
        let end = addr.checked_add(len).ok_or(RangeError::NotMapped)?;
        self.inner.lock().map.set_locked(addr, len, lock).map_err(|_| RangeError::NotMapped)?;
        if lock && populate {
            if !self.covered(addr, end, |p| p != Prot::NONE) {
                return Err(RangeError::NotMapped);
            }
            self.bring_in(addr, end);
        }
        Ok(())
    }

    /// mlockall(2): with `current` (`Some(populate)`) every region is
    /// locked and, with `populate`, the pages of every accessible region
    /// brought in; `future` becomes the `MCL_FUTURE` setting.
    pub fn lock_all(&self, current: Option<bool>, future: Option<FutureLock>) {
        let ranges: alloc::vec::Vec<(u64, u64)> = {
            let mut inner = self.inner.lock();
            inner.map.set_lock_future(future);
            if current.is_none() {
                return;
            }
            inner.map.set_all_locked(true);
            inner.map.regions().filter(|v| v.prot != Prot::NONE).map(|v| (v.start, v.end)).collect()
        };
        if current == Some(true) {
            for (start, end) in ranges {
                self.bring_in(start, end);
            }
        }
    }

    /// munlockall(2): every region unlocked, `MCL_FUTURE` ended.
    pub fn unlock_all(&self) {
        let mut inner = self.inner.lock();
        inner.map.set_all_locked(false);
        inner.map.set_lock_future(None);
    }

    /// Bytes locked in all, bytes locked in `[addr, addr + len)`, bytes
    /// mapped, and the `MCL_FUTURE` setting.
    pub fn locked(&self, addr: u64, len: u64) -> (u64, u64, u64, Option<FutureLock>) {
        let inner = self.inner.lock();
        let in_range = if len == 0 { 0 } else { inner.map.locked_bytes_in(addr, addr.saturating_add(len)) };
        (inner.map.locked_bytes(), in_range, inner.map.mapped_bytes(), inner.map.lock_future())
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
