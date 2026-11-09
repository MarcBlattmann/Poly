//! Mapping: mmap, munmap, mprotect and mremap, and the region rewrite they share.

use super::*;

impl<F: Clone + PartialEq> VmMap<F> {
    /// Create a mapping (`mmap`). Returns its address and, for
    /// [`Placement::Fixed`], the regions it replaced.
    ///
    /// Errors, checked in this order: zero length ([`Error::Invalid`]);
    /// `prot` beyond `max_prot` ([`Error::Access`]); a file offset that is
    /// not page-aligned ([`Error::Invalid`]); a length that does not fit in
    /// the address space ([`Error::NoMemory`]); a file offset plus length
    /// that overflows ([`Error::Overflow`]); for fixed placements a
    /// misaligned address ([`Error::Invalid`]), an address below
    /// [`Layout::min_addr`] ([`Error::Permission`]), a range past
    /// [`Layout::max_addr`] ([`Error::NoMemory`]), an occupied range for
    /// [`Placement::FixedNoReplace`] ([`Error::Exists`]); no hole large
    /// enough, or too many regions ([`Error::NoMemory`]).
    pub fn map(&mut self, req: MapRequest<F>) -> Result<Mapped<F>, Error> {
        let MapRequest { placement, len, prot, max_prot, backing } = req;
        if len == 0 {
            return Err(Error::Invalid);
        }
        if !max_prot.contains(prot) {
            return Err(Error::Access);
        }
        if let Backing::File { offset, .. } = backing
            && !is_page_aligned(offset)
        {
            return Err(Error::Invalid);
        }
        let len = page_up(len).ok_or(Error::NoMemory)?;
        if len > self.layout.max_addr - self.layout.min_addr {
            return Err(Error::NoMemory);
        }
        if let Backing::File { offset, .. } = backing
            && offset.checked_add(len).is_none()
        {
            return Err(Error::Overflow);
        }
        let addr = match placement {
            Placement::Fixed(addr) | Placement::FixedNoReplace(addr) => {
                if !is_page_aligned(addr) {
                    return Err(Error::Invalid);
                }
                if addr < self.layout.min_addr {
                    return Err(Error::Permission);
                }
                if addr.checked_add(len).is_none_or(|end| end > self.layout.max_addr) {
                    return Err(Error::NoMemory);
                }
                if matches!(placement, Placement::FixedNoReplace(_))
                    && self.regions_in(addr, addr + len).next().is_some()
                {
                    return Err(Error::Exists);
                }
                if self.is_sealed(addr, addr + len) {
                    return Err(Error::Permission);
                }
                addr
            }
            Placement::Hint(hint) => self.place(hint, len).ok_or(Error::NoMemory)?,
        };
        let vma = Vma {
            start: addr,
            end: addr + len,
            prot,
            max_prot,
            kind: Kind::Normal,
            backing,
            wipe_on_fork: false,
            locked: self.lock_future.is_some(),
            sealed: false,
            uffd: 0,
            name: None,
        };
        let replaced = self.rewrite(addr, addr + len, Op::Insert(vma))?;
        Ok(Mapped { addr, replaced })
    }

    /// Remove every mapping in `[addr, addr + len)` (`munmap`), splitting
    /// regions that straddle the ends. Holes are fine. Returns the removed
    /// parts in address order; the kernel unmaps their pages.
    ///
    /// Errors: a misaligned `addr`, a zero length or a range that leaves the
    /// address space ([`Error::Invalid`]); a split that would exceed the map
    /// count ([`Error::NoMemory`]).
    pub fn unmap(&mut self, addr: u64, len: u64) -> Result<Vec<Vma<F>>, Error> {
        if !is_page_aligned(addr) || len == 0 {
            return Err(Error::Invalid);
        }
        let end = page_up(len).and_then(|len| addr.checked_add(len)).ok_or(Error::Invalid)?;
        if end > self.layout.max_addr {
            return Err(Error::Invalid);
        }
        if self.is_sealed(addr, end) {
            return Err(Error::Permission);
        }
        self.rewrite(addr, end, Op::Remove)
    }

    /// Change the protection of `[addr, addr + len)` (`mprotect`). A zero
    /// length succeeds without doing anything. The kernel then updates the
    /// present pages in the range (see the crate docs).
    ///
    /// Errors: a misaligned `addr` ([`Error::Invalid`]); a range that leaves
    /// the address space or contains unmapped pages ([`Error::NoMemory`],
    /// checked before the next one); `prot` beyond some region's
    /// [`Vma::max_prot`] ([`Error::Access`]); too many regions
    /// ([`Error::NoMemory`]).
    pub fn protect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Error> {
        if !is_page_aligned(addr) {
            return Err(Error::Invalid);
        }
        if len == 0 {
            return Ok(());
        }
        let end = page_up(len).and_then(|len| addr.checked_add(len)).ok_or(Error::NoMemory)?;
        if end > self.layout.max_addr {
            return Err(Error::NoMemory);
        }
        let mut covered = addr;
        let mut allowed = true;
        for v in self.regions_in(addr, end) {
            if v.start > covered {
                return Err(Error::NoMemory);
            }
            allowed &= v.max_prot.contains(prot);
            covered = v.end;
        }
        if covered < end {
            return Err(Error::NoMemory);
        }
        if self.is_sealed(addr, end) {
            return Err(Error::Permission);
        }
        if !allowed {
            return Err(Error::Access);
        }
        self.rewrite(addr, end, Op::Protect(prot)).map(|_| ())
    }

    /// Resize the mapping `[old, old + old_len)` to `new_len` bytes, moving
    /// it if `how` allows (`mremap`). Lengths are rounded up to pages.
    ///
    /// The range must lie inside one region of [`Kind::Normal`] (the break
    /// area and the stack are not remapped; they are the kernel's to grow).
    /// Shrinking removes the end of the range. Growing extends the region
    /// in place when the range ends where the region does and the space
    /// after it is free; otherwise, with [`RemapPlacement::MayMove`], the
    /// range moves to a free hole of the new size (search order as for
    /// [`VmMap::map`]) and grows there, the rest of the old region staying
    /// where it was. The moved part keeps its protection and backing (a
    /// file mapping keeps its offset; the grown pages continue the file).
    /// [`RemapPlacement::Fixed`] moves it to that address in every case,
    /// replacing whatever was there.
    ///
    /// Errors, in this order (mremap(2)): a misaligned `old` or a zero
    /// `new_len` ([`Error::Invalid`]), a zero `old_len` (Poly does not
    /// duplicate shared mappings: [`Error::Invalid`]); a range outside
    /// the address space or not inside one region, or in the break area or
    /// the stack ([`Error::Fault`]); for `Fixed`, a misaligned address or a
    /// new range overlapping the old one ([`Error::Invalid`]) or leaving the
    /// address space ([`Error::NoMemory`]); no room to grow in place without
    /// `MayMove`, no hole, or too many regions ([`Error::NoMemory`]). Nothing
    /// changes on error.
    pub fn remap(&mut self, old: u64, old_len: u64, new_len: u64, how: RemapPlacement) -> Result<Remap<F>, Error> {
        if !is_page_aligned(old) || new_len == 0 || old_len == 0 {
            return Err(Error::Invalid);
        }
        let old_len = page_up(old_len).ok_or(Error::Fault)?;
        let new_len = page_up(new_len).ok_or(Error::NoMemory)?;
        let old_end = old.checked_add(old_len).ok_or(Error::Fault)?;
        let vma = self.find(old).filter(|v| v.end >= old_end && v.kind == Kind::Normal).ok_or(Error::Fault)?.clone();
        if let RemapPlacement::Fixed(to) = how {
            if !is_page_aligned(to) {
                return Err(Error::Invalid);
            }
            let to_end = to.checked_add(new_len).ok_or(Error::NoMemory)?;
            if to < old_end && old < to_end {
                return Err(Error::Invalid);
            }
            if to < self.layout.min_addr || to_end > self.layout.max_addr {
                return Err(Error::NoMemory);
            }
            if self.is_sealed(to, to_end) {
                return Err(Error::Permission);
            }
        }
        if vma.sealed {
            return Err(Error::Permission);
        }
        // Work on a copy, so that an error half-way changes nothing.
        let mut next = self.clone();
        let mut removed = Vec::new();
        let kept = old_len.min(new_len);
        if new_len < old_len {
            removed.extend(next.rewrite(old + new_len, old_end, Op::Remove)?);
        }
        let grow_in_place = |m: &VmMap<F>| {
            vma.end == old_end
                && old.checked_add(new_len).is_some_and(|end| end <= m.layout.max_addr && m.is_free(old_end, end))
        };
        let to = match how {
            RemapPlacement::Fixed(to) => to,
            _ if new_len <= old_len => {
                *self = next;
                return Ok(Remap { addr: old, moved: None, removed });
            }
            _ if grow_in_place(&next) => {
                let grown = vma.clip(old, old_end);
                let tail =
                    Vma { start: old_end, end: old + new_len, backing: shifted(&grown.backing, old_len), ..grown };
                next.rewrite(old_end, old + new_len, Op::Insert(tail))?;
                *self = next;
                return Ok(Remap { addr: old, moved: None, removed });
            }
            RemapPlacement::InPlace => return Err(Error::NoMemory),
            RemapPlacement::MayMove => next.place(0, new_len).ok_or(Error::NoMemory)?,
        };
        // Move: the kept part leaves its place (its pages move, nothing is
        // freed), the destination is cleared and gets the region.
        let part = vma.clip(old, old + kept);
        next.rewrite(old, old + kept, Op::Remove)?;
        let moved = Vma { start: to, end: to + new_len, ..part };
        removed.extend(next.rewrite(to, to + new_len, Op::Insert(moved))?);
        *self = next;
        Ok(Remap { addr: to, moved: Some(Move { from: old, to, len: kept }), removed })
    }

    /// Apply `op` to `[start, end)`: split the regions that straddle the
    /// ends, remove, replace or re-protect the inside, and merge what now
    /// fits together, including the neighbours just outside the range.
    /// Returns the parts that were removed or replaced. Fails, changing
    /// nothing, if the result would exceed the map count.
    pub(super) fn rewrite(&mut self, start: u64, end: u64, op: Op<F>) -> Result<Vec<Vma<F>>, Error> {
        debug_assert!(start < end && is_page_aligned(start) && is_page_aligned(end));
        // Everything that overlaps the range, plus the neighbours touching it.
        let left = self.vmas.range(..start).next_back().filter(|(_, v)| v.end >= start).map(|(&k, _)| k);
        let keys: Vec<u64> = left.into_iter().chain(self.vmas.range(start..=end).map(|(&k, _)| k)).collect();

        let mut pieces = Vec::with_capacity(keys.len() + 2);
        let mut removed = Vec::new();
        for k in &keys {
            let v = &self.vmas[k];
            if v.end <= start || v.start >= end {
                pieces.push(v.clone());
                continue;
            }
            if v.start < start {
                pieces.push(v.clip(v.start, start));
            }
            let inside = v.clip(v.start.max(start), v.end.min(end));
            match op {
                Op::Protect(prot) => pieces.push(Vma { prot, ..inside }),
                Op::WipeOnFork(wipe_on_fork) => pieces.push(Vma { wipe_on_fork, ..inside }),
                Op::Lock(locked) => pieces.push(Vma { locked, ..inside }),
                Op::Seal => pieces.push(Vma { sealed: true, ..inside }),
                Op::UserFault(uffd) => pieces.push(Vma { uffd, ..inside }),
                Op::Name(ref name) => pieces.push(Vma { name: name.clone(), ..inside }),
                Op::Remove | Op::Insert(_) => removed.push(inside),
            }
            if v.end > end {
                pieces.push(v.clip(end, v.end));
            }
        }
        if let Op::Insert(vma) = op {
            let at = pieces.partition_point(|p| p.start < vma.start);
            pieces.insert(at, vma);
        }

        let mut merged: Vec<Vma<F>> = Vec::with_capacity(pieces.len());
        for p in pieces {
            match merged.last_mut() {
                Some(last) if last.can_merge(&p) => last.end = p.end,
                _ => merged.push(p),
            }
        }

        if self.vmas.len() - keys.len() + merged.len() > self.layout.max_map_count {
            return Err(Error::NoMemory);
        }
        for k in &keys {
            self.vmas.remove(k);
        }
        for v in merged {
            self.vmas.insert(v.start, v);
        }
        Ok(removed)
    }
}
