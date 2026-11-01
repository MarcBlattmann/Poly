//! Frames with several users: [`SharedFrame`] (a user frame behind an
//! `Arc`, freed with its last reference, or a page of device memory such
//! as a DRM buffer, which its owner keeps) and [`AnonShared`], the memory
//! object behind shared anonymous mappings.

use super::super::UserFrame;
use super::{IoError, PAGE_SIZE, PageSource};
use crate::arch;
use crate::sync::SpinLock;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use core::any::Any;

/// A frame that several memories map (after `fork`, or as a page of
/// shared memory). Kept in an `Arc`; a user frame is freed when the last
/// one goes, by the [`UserFrame`] inside. A device frame belongs to the
/// memory object it is part of, which the frame holds a reference on: it
/// is never taken over as a memory's own, so a private mapping copies it on
/// a write.
pub struct SharedFrame(pub(super) Owner);

/// Who frees a shared frame.
pub(super) enum Owner {
    User(UserFrame),
    /// A page at `phys` of memory that `_memory` keeps allocated (held,
    /// never read).
    Device {
        phys: u64,
        _memory: Arc<dyn Any + Send + Sync>,
    },
}

impl core::fmt::Debug for SharedFrame {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match &self.0 {
            Owner::User(u) => f.debug_tuple("SharedFrame").field(u).finish(),
            Owner::Device { phys, .. } => write!(f, "SharedFrame(device {phys:#x})"),
        }
    }
}

impl SharedFrame {
    /// A user frame, freed with the last reference.
    pub(super) fn owned(frame: UserFrame) -> SharedFrame {
        SharedFrame(Owner::User(frame))
    }

    /// The page at `phys` (page-aligned RAM in the direct map) of device
    /// memory that `memory` keeps allocated as long as it lives.
    pub fn device(phys: u64, memory: Arc<dyn Any + Send + Sync>) -> SharedFrame {
        debug_assert_eq!(phys % PAGE_SIZE, 0);
        SharedFrame(Owner::Device { phys, _memory: memory })
    }

    /// Whether this is device memory (never a memory's own frame).
    pub fn is_device(&self) -> bool {
        matches!(self.0, Owner::Device { .. })
    }

    /// The user frame, to take over as a memory's own; a device frame
    /// back as it is.
    pub(super) fn into_owned(self) -> Result<UserFrame, SharedFrame> {
        match self.0 {
            Owner::User(f) => Ok(f),
            device => Err(SharedFrame(device)),
        }
    }

    /// Physical address of the frame.
    pub fn phys(&self) -> u64 {
        match &self.0 {
            Owner::User(f) => f.phys(),
            Owner::Device { phys, .. } => *phys,
        }
    }

    /// Copy the frame's contents to `out`.
    pub fn copy_to(&self, out: &mut [u8; PAGE_SIZE as usize]) {
        let from = arch::phys_to_virt(self.phys()).expect("user frame outside the direct map") as *const u8;
        // SAFETY: the frame stays allocated while `self` lives, and the
        // direct map maps it. Others may be writing it (shared anonymous
        // memory) or reading it; bytes have no invalid values, so a copy
        // that races with a write sees old or new bytes, as a user-mode
        // copy would.
        unsafe { core::ptr::copy_nonoverlapping(from, out.as_mut_ptr(), PAGE_SIZE as usize) };
    }
}

/// The memory behind `MAP_SHARED | MAP_ANONYMOUS`: zero-filled pages
/// created at the first fault, which every mapping of the object (in
/// whichever process, after `fork`) shares. mmap(2): "the mapping is
/// shared ... visible to other processes mapping the same region".
#[derive(Debug)]
pub struct AnonShared {
    len: u64,
    pages: SpinLock<BTreeMap<u64, Arc<SharedFrame>>>,
}

impl AnonShared {
    /// An object of `len` bytes.
    pub fn new(len: u64) -> AnonShared {
        AnonShared { len, pages: SpinLock::new(BTreeMap::new()) }
    }

    /// Pages that exist so far.
    pub fn pages(&self) -> usize {
        self.pages.lock().len()
    }
}

impl PageSource for AnonShared {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        // Only for a private copy, which shared mappings never make; the
        // contents are those of the page, or zero.
        let n = buf.len().min(self.len.saturating_sub(offset) as usize);
        let page = offset / PAGE_SIZE * PAGE_SIZE;
        let at = (offset - page) as usize;
        let n = n.min(PAGE_SIZE as usize - at);
        let frame = self.pages.lock().get(&page).cloned();
        match frame {
            Some(f) => {
                let mut bytes = [0u8; PAGE_SIZE as usize];
                f.copy_to(&mut bytes);
                buf[..n].copy_from_slice(&bytes[at..at + n]);
            }
            None => buf[..n].fill(0),
        }
        Ok(n)
    }

    fn shareable(&self) -> bool {
        true
    }

    fn shared_frame(&self, offset: u64) -> Option<Result<Arc<SharedFrame>, IoError>> {
        let page = offset / PAGE_SIZE * PAGE_SIZE;
        if let Some(f) = self.pages.lock().get(&page) {
            return Some(Ok(Arc::clone(f)));
        }
        // Allocated unlocked; if another fault made the page meanwhile,
        // that one wins and this frame is freed.
        let Some(frame) = UserFrame::zeroed() else { return Some(Err(IoError)) };
        let new = Arc::new(SharedFrame::owned(frame));
        Some(Ok(Arc::clone(self.pages.lock().entry(page).or_insert(new))))
    }
}
