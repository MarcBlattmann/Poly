//! Region attributes: wipe-on-fork, locking, sealing, userfault registration and anonymous names.

use super::*;

impl<F: Clone + PartialEq> VmMap<F> {
    /// `MADV_WIPEONFORK` (`on`) or `MADV_KEEPONFORK` for the mapped parts
    /// of `[addr, addr + len)` (page aligned, `len` non-zero). Wiping is
    /// for private anonymous memory only: any other region in the range is
    /// [`Error::Invalid`] and nothing changes. A range with unmapped pages
    /// is [`Error::NoMemory`], after the mapped parts got the setting.
    pub fn set_wipe_on_fork(&mut self, addr: u64, len: u64, on: bool) -> Result<(), Error> {
        let end = addr.checked_add(len).ok_or(Error::NoMemory)?;
        if !is_page_aligned(addr) || !is_page_aligned(end) || len == 0 {
            return Err(Error::Invalid);
        }
        let mut covered = addr;
        let mut hole = false;
        let mut parts = Vec::new();
        for v in self.regions_in(addr, end) {
            if on && !(matches!(v.backing, Backing::Anonymous)) {
                return Err(Error::Invalid);
            }
            hole |= v.start > covered;
            parts.push((v.start.max(addr), v.end.min(end)));
            covered = v.end;
        }
        hole |= covered < end;
        for (s, e) in parts {
            self.rewrite(s, e, Op::WipeOnFork(on))?;
        }
        if hole { Err(Error::NoMemory) } else { Ok(()) }
    }

    /// Lock (`on`) or unlock the regions of `[addr, addr + len)` (page
    /// aligned, `len` non-zero; mlock(2) / munlock(2) bookkeeping). The
    /// range must be mapped: at the first hole the call stops with
    /// [`Error::NoMemory`], the regions before it already changed and
    /// nothing after it (mlock2's spec sheet).
    pub fn set_locked(&mut self, addr: u64, len: u64, on: bool) -> Result<(), Error> {
        let end = addr.checked_add(len).ok_or(Error::NoMemory)?;
        if !is_page_aligned(addr) || !is_page_aligned(end) || len == 0 {
            return Err(Error::Invalid);
        }
        let mut covered = addr;
        let mut parts = Vec::new();
        for v in self.regions_in(addr, end) {
            if v.start > covered {
                break;
            }
            parts.push((v.start.max(addr), v.end.min(end)));
            covered = v.end;
        }
        for (s, e) in parts {
            self.rewrite(s, e, Op::Lock(on))?;
        }
        if covered < end { Err(Error::NoMemory) } else { Ok(()) }
    }

    /// Lock or unlock every region (mlockall(2) `MCL_CURRENT`,
    /// munlockall(2)).
    pub fn set_all_locked(&mut self, on: bool) {
        let ranges: Vec<(u64, u64)> = self.vmas.values().map(|v| (v.start, v.end)).collect();
        for (s, e) in ranges {
            // Changing a flag on whole regions never splits one: the map
            // count cannot grow.
            let _ = self.rewrite(s, e, Op::Lock(on));
        }
    }

    /// The `MCL_FUTURE` setting: new regions are created locked while it
    /// is `Some`.
    pub fn lock_future(&self) -> Option<FutureLock> {
        self.lock_future
    }

    pub fn set_lock_future(&mut self, future: Option<FutureLock>) {
        self.lock_future = future;
    }

    /// Bytes in locked regions (`VmLck`).
    pub fn locked_bytes(&self) -> u64 {
        self.vmas.values().filter(|v| v.locked).map(Vma::size).sum()
    }

    /// Bytes of `[addr, end)` in locked regions.
    pub fn locked_bytes_in(&self, addr: u64, end: u64) -> u64 {
        self.regions_in(addr, end).filter(|v| v.locked).map(|v| v.end.min(end) - v.start.max(addr)).sum()
    }

    /// Bytes mapped in all (the size of the address space, `VmSize`).
    pub fn mapped_bytes(&self) -> u64 {
        self.vmas.values().map(Vma::size).sum()
    }

    /// Whether any part of `[start, end)` is sealed ([`Vma::sealed`]).
    pub fn is_sealed(&self, start: u64, end: u64) -> bool {
        self.regions_in(start, end).any(|v| v.sealed)
    }

    /// Seal `[addr, addr + len)` (mseal(2)): `len` is rounded up to whole
    /// pages; 0 succeeds without doing anything. Sealing a sealed range
    /// again is fine.
    ///
    /// Errors, nothing sealed then: a misaligned `addr` or a range that
    /// wraps around ([`Error::Invalid`]); any part of the range unmapped
    /// ([`Error::NoMemory`]); too many regions ([`Error::NoMemory`]).
    pub fn seal(&mut self, addr: u64, len: u64) -> Result<(), Error> {
        if !is_page_aligned(addr) {
            return Err(Error::Invalid);
        }
        let end = page_up(len).and_then(|len| addr.checked_add(len)).ok_or(Error::Invalid)?;
        if len == 0 {
            return Ok(());
        }
        if !self.is_covered(addr, end) {
            return Err(Error::NoMemory);
        }
        self.rewrite(addr, end, Op::Seal).map(|_| ())
    }

    /// Whether a discarding madvise(2) (`MADV_DONTNEED` and the like) over
    /// `[start, end)` must be refused because of a seal: some sealed part
    /// is private anonymous memory without write permission, whose
    /// contents the process could not have written back itself.
    pub fn discard_refused(&self, start: u64, end: u64) -> bool {
        self.regions_in(start, end)
            .any(|v| v.sealed && matches!(v.backing, Backing::Anonymous) && !v.prot.contains(Prot::WRITE))
    }

    /// Whether every page of `[start, end)` is mapped.
    pub(super) fn is_covered(&self, start: u64, end: u64) -> bool {
        let mut covered = start;
        for v in self.regions_in(start, end) {
            if v.start > covered {
                return false;
            }
            covered = v.end;
        }
        covered >= end
    }

    /// Register `[addr, addr + len)` with userfaultfd object `uffd`
    /// (non-zero) for missing-page faults (`UFFDIO_REGISTER`). Registering
    /// again replaces the registration.
    ///
    /// Errors, nothing registered then: a misaligned `addr` or `len`, a zero
    /// `len`, a range that wraps around or leaves the address space, any
    /// part of the range unmapped ([`Error::Invalid`]); too many regions
    /// ([`Error::NoMemory`]).
    pub fn set_userfault(&mut self, addr: u64, len: u64, uffd: u32) -> Result<(), Error> {
        let end = addr.checked_add(len).ok_or(Error::Invalid)?;
        if !is_page_aligned(addr) || !is_page_aligned(len) || len == 0 || end > self.layout.max_addr {
            return Err(Error::Invalid);
        }
        if !self.is_covered(addr, end) {
            return Err(Error::Invalid);
        }
        self.rewrite(addr, end, Op::UserFault(uffd)).map(|_| ())
    }

    /// Name the regions of `[addr, addr + len)` (prctl(2) `PR_SET_VMA`,
    /// `PR_SET_VMA_ANON_NAME`; `len` is rounded up to a page), or remove
    /// their names (`None`). Naming part of a region splits it.
    ///
    /// Errors, nothing changed then: a misaligned `addr`, a range that
    /// wraps around ([`Error::Invalid`]); a page of the range not mapped
    /// ([`Error::NoMemory`]); a region of it that is not private anonymous
    /// memory ([`Error::NotAnonymous`]); too many regions
    /// ([`Error::NoMemory`]). A zero `len` changes nothing.
    pub fn set_anon_name(&mut self, addr: u64, len: u64, name: Option<Arc<str>>) -> Result<(), Error> {
        if !is_page_aligned(addr) {
            return Err(Error::Invalid);
        }
        let len = page_up(len).ok_or(Error::Invalid)?;
        let end = addr.checked_add(len).ok_or(Error::Invalid)?;
        if len == 0 {
            return Ok(());
        }
        if !self.is_covered(addr, end) {
            return Err(Error::NoMemory);
        }
        if self.regions_in(addr, end).any(|v| !matches!(v.backing, Backing::Anonymous)) {
            return Err(Error::NotAnonymous);
        }
        self.rewrite(addr, end, Op::Name(name)).map(|_| ())
    }

    /// Undo the registrations with `uffd` of the mapped parts of `[addr,
    /// addr + len)` (`UFFDIO_UNREGISTER`).
    ///
    /// Errors: a misaligned `addr` or `len`, a zero `len`, a range that
    /// wraps around or leaves the address space ([`Error::Invalid`]); too
    /// many regions ([`Error::NoMemory`]).
    pub fn clear_userfault(&mut self, addr: u64, len: u64, uffd: u32) -> Result<(), Error> {
        let end = addr.checked_add(len).ok_or(Error::Invalid)?;
        if !is_page_aligned(addr) || !is_page_aligned(len) || len == 0 || end > self.layout.max_addr {
            return Err(Error::Invalid);
        }
        let parts: Vec<(u64, u64)> = self
            .regions_in(addr, end)
            .filter(|v| v.uffd == uffd)
            .map(|v| (v.start.max(addr), v.end.min(end)))
            .collect();
        for (s, e) in parts {
            self.rewrite(s, e, Op::UserFault(0))?;
        }
        Ok(())
    }

    /// Whether `addr` lies in a region registered with `uffd`.
    pub fn userfault_registered(&self, addr: u64, uffd: u32) -> bool {
        self.find(addr).is_some_and(|v| v.uffd == uffd)
    }

    /// Drop every registration with userfaultfd object `uffd` (the object
    /// was closed), or with any object (`None`: a fork child's copy of the
    /// map). Merging regions back only shrinks the map, so this cannot
    /// fail.
    pub fn forget_userfaults(&mut self, uffd: Option<u32>) {
        let parts: Vec<(u64, u64)> = self
            .regions()
            .filter(|v| v.uffd != 0 && uffd.is_none_or(|u| v.uffd == u))
            .map(|v| (v.start, v.end))
            .collect();
        for (s, e) in parts {
            let _ = self.rewrite(s, e, Op::UserFault(0));
        }
    }
}
