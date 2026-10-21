//! Resizing and moving a mapping (mremap(2), ADR 0059): the region map
//! decides ([`poly_vm::VmMap::remap`]); this moves the pages.
//!
//! A moved page keeps what it holds without a copy: the memory's own frames
//! are taken out of the page tables and mapped at the new address with the
//! rights they had, parked frames (`PROT_NONE`) and shared frames (copy on
//! write after `fork`, shared anonymous memory) are filed under the new
//! address (a shared frame is mapped there by the next fault), and
//! page-cache pages are let go: the moved region maps the same file pages
//! and faults them in again.

use super::{Inner, Memory, MemoryError, PAGE_SIZE, SharedPage};
use alloc::vec::Vec;
use poly_vm::{Move, RemapPlacement};

impl Memory {
    /// Resize `[old, old + old_len)` to `new_len` bytes, moving it as `how`
    /// allows (`mremap`); returns where it is now. Pages the call removes
    /// (a shrunk end, what a fixed destination replaced) are freed first.
    /// If a page table for a moved page cannot be allocated, that page's
    /// contents are lost and the call fails with
    /// [`MemoryError::OutOfMemory`] after the region map has changed.
    pub fn remap(&self, old: u64, old_len: u64, new_len: u64, how: RemapPlacement) -> Result<u64, MemoryError> {
        let mut inner = self.inner.lock();
        let r = inner.map.remap(old, old_len, new_len, how)?;
        self.release(&mut inner, &r.removed);
        if let Some(mv) = r.moved {
            self.move_pages(&mut inner, mv)?;
        }
        Ok(r.addr)
    }

    fn move_pages(&self, inner: &mut Inner, mv: Move) -> Result<(), MemoryError> {
        let Move { from, to, len } = mv;
        let end = from + len;
        let dest = |page: u64| to + (page - from);
        // Page-cache pages: unmapped before their entries go.
        let cached: Vec<u64> = inner.cached.range(from..end).map(|(&p, _)| p).collect();
        for page in cached {
            self.space.unmap(page, PAGE_SIZE).expect("a mapped user page");
            inner.cached.remove(&page);
        }
        // Shared frames: unmapped here, kept for the new address.
        let shared: Vec<u64> = inner.shared.range(from..end).map(|(&p, _)| p).collect();
        for page in shared {
            let s = inner.shared.remove(&page).expect("listed above");
            if s.mapped {
                self.space.unmap(page, PAGE_SIZE).expect("a mapped user page");
            }
            inner.shared.insert(dest(page), SharedPage { frame: s.frame, mapped: false });
        }
        let parked: Vec<u64> = inner.parked.range(from..end).map(|(&p, _)| p).collect();
        for page in parked {
            let frame = inner.parked.remove(&page).expect("listed above");
            inner.parked.insert(dest(page), frame);
        }
        // The memory's own frames: what is left in the page tables.
        let mut result = Ok(());
        for (page, m) in self.space.mapped_pages(from, end) {
            let Some(frame) = (if m.owned { self.space.take(page) } else { None }) else { continue };
            if self.space.map_owned(dest(page), frame, m.prot).is_err() {
                result = Err(MemoryError::OutOfMemory);
            }
        }
        result
    }
}

crate::kernel_test! {
    fn remap_moves_own_shared_and_cached_pages_without_copies() -> TestResult {
        use super::{AnonShared, FileRef, Layout, MapRequest, Placement, Prot};
        const P: u64 = PAGE_SIZE;
        let m = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let a = m.map_anonymous(Placement::Hint(0), 2 * P, Prot::RW).map_err(|_| "mmap failed")?;
        m.write(a, b"first").map_err(|_| "write failed")?;
        m.write(a + P, b"second").map_err(|_| "write failed")?;
        let frame = m.space.translate(a).ok_or("the page is not mapped")?.phys;
        // Something right after it, so growing must move.
        m.map_anonymous(Placement::Fixed(a + 2 * P), P, Prot::READ).map_err(|_| "mmap failed")?;
        if m.remap(a, 2 * P, 4 * P, RemapPlacement::InPlace).is_ok() {
            return Err("grew in place over another mapping");
        }
        let b = m.remap(a, 2 * P, 4 * P, RemapPlacement::MayMove).map_err(|_| "mremap failed")?;
        let mut buf = [0u8; 6];
        m.read(b, &mut buf[..5]).map_err(|_| "read failed")?;
        if &buf[..5] != b"first" || m.space.translate(b).map(|t| t.phys) != Some(frame) {
            return Err("the first page did not move with its frame");
        }
        m.read(b + P, &mut buf).map_err(|_| "read failed")?;
        m.read(b + 3 * P, &mut buf[..1]).map_err(|_| "read failed")?;
        if m.space.translate(a).is_some() || m.read(a, &mut buf[..1]).is_ok() {
            return Err("the old address still maps a page");
        }
        // Shared anonymous memory keeps its frames, seen by a fork.
        let object = alloc::sync::Arc::new(AnonShared::new(2 * P));
        let req = MapRequest::file(2 * P, Prot::RW, FileRef::new(object.clone()), 0, true);
        let s = m.map(req.at(Placement::Hint(0))).map_err(|_| "shared mmap failed")?;
        m.write(s, b"shared").map_err(|_| "write failed")?;
        let t = m.remap(s, 2 * P, 2 * P, RemapPlacement::Fixed(s - 16 * P)).map_err(|_| "mremap failed")?;
        m.read(t, &mut buf).map_err(|_| "read failed")?;
        if &buf != b"shared" || object.pages() != 1 {
            return Err("shared memory lost its page in a move");
        }
        // Shrinking frees the end.
        let c = m.remap(b, 4 * P, P, RemapPlacement::MayMove).map_err(|_| "mremap failed")?;
        if c != b || m.read(b + P, &mut buf[..1]).is_ok() {
            return Err("shrinking did not remove the end");
        }
        Ok(())
    }
}
