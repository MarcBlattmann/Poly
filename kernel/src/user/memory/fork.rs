//! `fork`: a copy of a memory that shares its pages until either side
//! writes (copy on write), and what faults and `mprotect` do with shared
//! frames. See the module docs of `memory` and ADR 0041.
//!
//! | Page of the parent | After `fork` |
//! |---|---|
//! | a frame of its own, private region | a [`SharedFrame`], read-only in both |
//! | a [`SharedFrame`] already, private region | one more user, read-only in the child |
//! | parked (`PROT_NONE`) | a [`SharedFrame`] that neither maps until the region is accessible |
//! | a page-cache page, private region | the same page, read-only in the child |
//! | any page of a shared region (`MAP_SHARED`) | nothing: the child's faults find the same page through the file or the [`AnonShared`](super::AnonShared) object |
//!
//! A write to a shared frame in a private region copies it into a frame of
//! the writer's own, unless the writer is its last user: then it takes the
//! frame back ([`AddressSpace::adopt`](super::AddressSpace::adopt)), so a
//! child that exits, or execs, leaves the parent its pages without a copy.

use super::{
    FaultError, Inner, MapError, Memory, MemoryError, PAGE_SIZE, Prot, SharedFrame, SharedPage, UserFrame, map_error,
    page_prot, read_only,
};
use crate::sync::Mutex;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Syncs the TLBs for `flush` when dropped.
struct SyncOnDrop<'a> {
    space: &'a super::AddressSpace,
    flush: crate::arch::user::Flush,
}

impl Drop for SyncOnDrop<'_> {
    fn drop(&mut self) {
        self.space.sync_tlb(core::mem::take(&mut self.flush));
    }
}

impl Memory {
    /// A copy of this memory for a child process (fork(2): "The child
    /// process is created with a single thread ... The entire virtual
    /// address space of the parent is replicated in the child"): the same
    /// regions, break and layout in a new address space, private pages
    /// shared copy-on-write, shared mappings shared. On failure (out of
    /// memory) the parent is unchanged, apart from pages that became
    /// shared frames, which it keeps.
    pub fn fork(&self) -> Result<Memory, MemoryError> {
        let space = super::AddressSpace::new().map_err(|_| MemoryError::OutOfMemory)?;
        let mut inner = self.inner.lock();
        let mut child =
            Inner { map: inner.map.clone(), parked: BTreeMap::new(), cached: BTreeMap::new(), shared: BTreeMap::new() };
        // userfaultfd(2) registrations stay with the parent (no
        // UFFD_FEATURE_EVENT_FORK); mseal(2) seals are inherited.
        child.map.forget_userfaults(None);
        // MADV_WIPEONFORK regions: the child's copy starts zero-filled, so
        // their pages are not shared with it.
        let private: Vec<(u64, u64, Prot)> = inner
            .map
            .regions()
            .filter(|v| !v.is_shared() && !v.wipe_on_fork)
            .map(|v| (v.start, v.end, v.prot))
            .collect();
        let wiped = |inner: &Inner, page: u64| inner.map.find(page).is_some_and(|v| v.wipe_on_fork);
        // The pages made read-only below are synced with the other CPUs
        // (which may run threads of this memory) in one shootdown, before
        // this returns, on every path.
        let mut sync = SyncOnDrop { space: &self.space, flush: crate::arch::user::Flush::None };
        for (start, end, prot) in private {
            let ro = page_prot(read_only(prot));
            for (page, m) in self.space.mapped_pages(start, end) {
                if m.owned {
                    // SAFETY: the frame goes into an `Arc` that both
                    // memories keep in `shared` while they map it; every
                    // path that drops such an entry unmaps the page first.
                    let Some(frame) = (unsafe { self.space.disown(page, ro, &mut sync.flush) }) else { continue };
                    let frame = Arc::new(SharedFrame::owned(frame));
                    inner.shared.insert(page, SharedPage { frame: Arc::clone(&frame), mapped: true });
                    map_shared_into(&space, &mut child, page, frame, ro)?;
                } else if let Some(s) = inner.shared.get(&page) {
                    let frame = Arc::clone(&s.frame);
                    // A frame its last user had made writable must not be
                    // written any more by either side.
                    self.space.protect(page, PAGE_SIZE, ro).map_err(|_| MemoryError::OutOfMemory)?;
                    map_shared_into(&space, &mut child, page, frame, ro)?;
                } else if let Some(c) = inner.cached.get(&page) {
                    let mapping = super::CachedMapping { page: c.page.clone(), shared: false, writable: false };
                    // SAFETY: as in `map_cached`: the child keeps the page
                    // in `cached` while it maps it, read-only.
                    unsafe { space.map_frames(page, mapping.page.frame().phys(), PAGE_SIZE, ro) }
                        .map_err(|_| MemoryError::OutOfMemory)?;
                    child.cached.insert(page, mapping);
                }
            }
        }
        // Parked pages (regions that are PROT_NONE now) are shared too,
        // mapped by neither side until the region is accessible again;
        // shared frames that are parked already just get one more user.
        for (&page, s) in inner.shared.iter().filter(|(_, s)| !s.mapped) {
            if !inner.map.find(page).is_some_and(|v| v.is_shared() || v.wipe_on_fork) {
                child.shared.insert(page, SharedPage { frame: Arc::clone(&s.frame), mapped: false });
            }
        }
        let parked: Vec<u64> = inner.parked.keys().copied().filter(|&p| !wiped(&inner, p)).collect();
        for page in parked {
            let frame = Arc::new(SharedFrame::owned(inner.parked.remove(&page).expect("listed above")));
            inner.shared.insert(page, SharedPage { frame: Arc::clone(&frame), mapped: false });
            child.shared.insert(page, SharedPage { frame, mapped: false });
        }
        let membarrier = core::sync::atomic::AtomicU32::new(self.membarrier_registrations());
        Ok(Memory { space: Arc::new(space), inner: Mutex::new(child), membarrier })
    }

    /// Map shared frame `frame` (a page of shared anonymous memory) at
    /// `page` with `prot`.
    pub(super) fn map_shared(
        &self,
        inner: &mut Inner,
        page: u64,
        frame: Arc<SharedFrame>,
        prot: Prot,
    ) -> Result<(), FaultError> {
        // SAFETY: `frame` goes into `inner.shared`, which keeps it while it
        // is mapped (every path that drops an entry unmaps first); it is
        // user memory of the object this region maps.
        unsafe { self.space.map_frames(page, frame.phys(), PAGE_SIZE, page_prot(prot)) }.map_err(map_error)?;
        inner.shared.insert(page, SharedPage { frame, mapped: true });
        Ok(())
    }

    /// Give the shared frame at `page` the rights `prot` (a fault that
    /// needs no copy). In a private region its last user takes it back as
    /// its own instead.
    pub(super) fn shared_permissions(
        &self,
        inner: &mut Inner,
        page: u64,
        prot: Prot,
        region_shared: bool,
    ) -> Result<(), FaultError> {
        if !region_shared && let Some(done) = self.take_back(inner, page, prot) {
            return done;
        }
        let s = inner.shared.get_mut(&page).expect("the caller checked");
        let rights = page_prot(if region_shared { prot } else { read_only(prot) });
        if s.mapped {
            return self.space.protect(page, PAGE_SIZE, rights).map_err(map_error);
        }
        // SAFETY: as in `map_shared`.
        unsafe { self.space.map_frames(page, s.frame.phys(), PAGE_SIZE, rights) }.map_err(map_error)?;
        s.mapped = true;
        Ok(())
    }

    /// A write to the shared frame at `page` of a private region: copy it
    /// into a frame of this memory's own (mapped with `prot`), or take it
    /// back if nobody else uses it any more.
    pub(super) fn copy_shared(&self, inner: &mut Inner, page: u64, prot: Prot) -> Result<(), FaultError> {
        if let Some(done) = self.take_back(inner, page, prot) {
            return done;
        }
        let mut copy = UserFrame::zeroed().ok_or(FaultError::OutOfMemory)?;
        let s = inner.shared.get(&page).expect("the caller checked");
        s.frame.copy_to(copy.bytes_mut());
        if s.mapped {
            self.space.unmap(page, PAGE_SIZE).map_err(map_error)?;
        }
        inner.shared.remove(&page);
        self.space.map_owned(page, copy, page_prot(prot)).map_err(map_error)
    }

    /// If this memory is the last user of the shared frame at `page`, make
    /// the frame its own again, mapped with `prot`. `None` if others still
    /// use it.
    fn take_back(&self, inner: &mut Inner, page: u64, prot: Prot) -> Option<Result<(), FaultError>> {
        let s = inner.shared.remove(&page)?;
        // Device memory is never taken over: it is copied instead.
        let frame = match Arc::try_unwrap(s.frame).map(SharedFrame::into_owned) {
            Ok(Ok(frame)) => frame,
            Ok(Err(device)) => {
                inner.shared.insert(page, SharedPage { frame: Arc::new(device), mapped: s.mapped });
                return None;
            }
            Err(frame) => {
                inner.shared.insert(page, SharedPage { frame, mapped: s.mapped });
                return None;
            }
        };
        let rights = page_prot(prot);
        let r = if s.mapped {
            match self.space.adopt(page, frame, rights) {
                Ok(()) => Ok(()),
                // Not mapped as it should be: start over with the frame.
                Err(frame) => {
                    let _ = self.space.unmap(page, PAGE_SIZE);
                    self.space.map_owned(page, frame, rights)
                }
            }
        } else {
            self.space.map_owned(page, frame, rights)
        };
        Some(r.map_err(map_error))
    }

    /// `mprotect` of a page holding a shared frame: unmapped for
    /// `PROT_NONE` (the entry keeps the frame), else mapped with `prot`,
    /// read-only in a private region (a write copies it).
    pub(super) fn protect_shared(&self, inner: &mut Inner, page: u64, prot: Prot) -> Result<(), MemoryError> {
        let region_shared = inner.map.find(page).is_some_and(|v| v.is_shared());
        let s = inner.shared.get_mut(&page).expect("the caller checked");
        if prot == Prot::NONE {
            if s.mapped {
                self.space.unmap(page, PAGE_SIZE).map_err(|_| MemoryError::OutOfMemory)?;
                s.mapped = false;
            }
            return Ok(());
        }
        let rights = page_prot(if region_shared { prot } else { read_only(prot) });
        let r = if s.mapped {
            self.space.protect(page, PAGE_SIZE, rights)
        } else {
            // SAFETY: as in `map_shared`.
            unsafe { self.space.map_frames(page, s.frame.phys(), PAGE_SIZE, rights) }
        };
        r.map_err(|_: MapError| MemoryError::OutOfMemory)?;
        s.mapped = true;
        Ok(())
    }

    /// Pages of this memory that share a frame with another memory or a
    /// shared anonymous object.
    pub fn shared_pages(&self) -> u64 {
        self.inner.lock().shared.len() as u64
    }
}

/// Map `frame` at `page` of a child's new address space and record it.
fn map_shared_into(
    space: &super::AddressSpace,
    child: &mut Inner,
    page: u64,
    frame: Arc<SharedFrame>,
    rights: super::PageProt,
) -> Result<(), MemoryError> {
    // SAFETY: the child keeps `frame` in `shared` while it maps it, and
    // drops entries only after unmapping them; it holds the parent's page,
    // which the child may see.
    unsafe { space.map_frames(page, frame.phys(), PAGE_SIZE, rights) }.map_err(|_| MemoryError::OutOfMemory)?;
    child.shared.insert(page, SharedPage { frame, mapped: true });
    Ok(())
}

/// Wait (polling, up to a generous deadline) until the system holds
/// `expected` user frames: frames of threads that ended a moment ago are
/// freed when the scheduler reaps them, so the count may still move.
fn frames_reach(expected: u64) -> Result<(), &'static str> {
    let deadline = crate::time::now_ns() + 30 * crate::time::NANOS_PER_SEC;
    while super::super::user_frames() != expected {
        if crate::time::now_ns() > deadline {
            crate::kprintln!("fork test: {} user frames, expected {expected}", super::super::user_frames());
            return Err("the user frames are not what copy on write should leave");
        }
        crate::thread::sleep(core::time::Duration::from_millis(2));
    }
    Ok(())
}

/// The user frame count once it has not changed for a while, and the
/// threads of processes that ended (earlier tests') are gone with their
/// frames.
fn settled_frames() -> u64 {
    // Bounded: if they never go, the frame checks report it.
    let deadline = crate::time::now_ns() + 30 * crate::time::NANOS_PER_SEC;
    while crate::thread::dying_user_threads() != 0 && crate::time::now_ns() < deadline {
        crate::thread::sleep(core::time::Duration::from_millis(2));
    }
    let mut last = super::super::user_frames();
    loop {
        crate::thread::sleep(core::time::Duration::from_millis(20));
        let now = super::super::user_frames();
        if now == last {
            return now;
        }
        last = now;
    }
}

crate::kernel_test! {
    fn fork_shares_pages_until_written_and_frees_them_with_the_last_user() -> TestResult {
        use super::{Layout, Placement};
        const P: u64 = PAGE_SIZE;
        let before = settled_frames();
        let parent = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
        let a = parent.map_anonymous(Placement::Hint(0), 3 * P, Prot::RW).map_err(|_| "mmap failed")?;
        let w = |m: &Memory, at: u64, s: &[u8]| m.write(at, s).map_err(|_| "a write failed");
        let r = |m: &Memory, at: u64| {
            let mut b = [0u8; 6];
            m.read(at, &mut b).map(|()| b).map_err(|_| "a read failed")
        };
        w(&parent, a, b"parent")?;
        w(&parent, a + P, b"second")?;
        frames_reach(before + 2)?;
        let child = parent.fork().map_err(|_| "fork failed")?;
        if parent.shared_pages() != 2 || child.shared_pages() != 2 {
            return Err("fork did not share the two resident pages");
        }
        frames_reach(before + 2)?;
        // The child writes: a copy of its own; the parent keeps its page.
        w(&child, a, b"child!")?;
        if r(&child, a)? != *b"child!" || r(&parent, a)? != *b"parent" {
            return Err("a write after fork reached the other side");
        }
        frames_reach(before + 3)?;
        // The parent writes the other page: a copy too, the child keeps the
        // old one.
        w(&parent, a + P, b"PARENT")?;
        if r(&child, a + P)? != *b"second" {
            return Err("the parent's write reached the child");
        }
        frames_reach(before + 4)?;
        // Untouched pages are zero on both sides.
        if r(&child, a + 2 * P)? != [0; 6] {
            return Err("an untouched page is not zero in the child");
        }
        drop(child);
        // The child's copy, its zero page and its last reference to the old
        // second page are gone; the parent's first page is a shared frame
        // with one user, which a write takes back without a copy.
        frames_reach(before + 2)?;
        w(&parent, a, b"again!")?;
        frames_reach(before + 2)?;
        if parent.shared_pages() != 0 || r(&parent, a)? != *b"again!" {
            return Err("the last user did not take its shared frame back");
        }
        drop(parent);
        frames_reach(before)
    }
}

crate::kernel_test! {
    fn fork_keeps_shared_anonymous_memory_and_parked_pages() -> TestResult {
        use super::{AnonShared, FileRef, Layout, MapRequest, Placement};
        const P: u64 = PAGE_SIZE;
        let before = settled_frames();
        {
            let parent = Memory::new(Layout::default()).map_err(|_| "no memory map")?;
            let object = Arc::new(AnonShared::new(2 * P));
            let req = MapRequest::file(2 * P, Prot::RW, FileRef::new(object.clone()), 0, true);
            let s = parent.map(req.at(Placement::Hint(0))).map_err(|_| "shared mmap failed")?;
            let a = parent.map_anonymous(Placement::Hint(0), P, Prot::RW).map_err(|_| "mmap failed")?;
            parent.write(s, b"one").map_err(|_| "write failed")?;
            parent.write(a, b"hidden").map_err(|_| "write failed")?;
            parent.protect(a, P, Prot::NONE).map_err(|_| "mprotect failed")?;
            let child = parent.fork().map_err(|_| "fork failed")?;
            let mut b = [0u8; 3];
            child.read(s, &mut b).map_err(|_| "read failed")?;
            if &b != b"one" {
                return Err("the child does not see the shared page");
            }
            child.write(s + P, b"two").map_err(|_| "write failed")?;
            parent.read(s + P, &mut b).map_err(|_| "read failed")?;
            if &b != b"two" || object.pages() != 2 {
                return Err("the child's write to shared memory did not reach the parent");
            }
            if child.read(a, &mut b).is_ok() {
                return Err("a PROT_NONE page is readable in the child");
            }
            child.protect(a, P, Prot::RW).map_err(|_| "mprotect failed")?;
            let mut h = [0u8; 6];
            child.read(a, &mut h).map_err(|_| "read failed")?;
            child.write(a, b"CHILD!").map_err(|_| "write failed")?;
            parent.protect(a, P, Prot::READ).map_err(|_| "mprotect failed")?;
            let mut p = [0u8; 6];
            parent.read(a, &mut p).map_err(|_| "read failed")?;
            if &h != b"hidden" || &p != b"hidden" {
                return Err("a parked page did not survive fork on both sides");
            }
        }
        frames_reach(before)
    }
}
