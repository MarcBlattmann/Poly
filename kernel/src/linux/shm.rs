//! System V shared memory in the kernel: the memory of a segment and its
//! attachments (shmat(2), shmdt(2)). The personality keeps the segments
//! themselves (`poly_personality_linux::ipc::shm`).
//!
//! A segment's memory is a [`ShmPages`]: shared anonymous memory
//! ([`AnonShared`], zero-filled pages made at the first fault) that knows
//! its serial. An attachment is a shared mapping of it, so `fork` gives
//! the child the same pages and every attachment sees every write. The
//! personality's [`Segment`] holds one `Arc` of the pages and every
//! mapping holds another: the attachments are the others (a mapping that
//! `mprotect` split in two counts twice). The pages are freed when the
//! segment is gone and its last mapping too.

use crate::user::memory::{Backing, IoError, PageSource, SharedFrame};
use crate::user::{AnonShared, FileRef, Memory, MemoryError};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::ipc::shm::ShmMemory;

/// The serial of the next segment's memory.
static SERIALS: AtomicU64 = AtomicU64::new(1);

/// The pages of a segment.
#[derive(Debug)]
pub struct ShmPages {
    pages: AnonShared,
    serial: u64,
}

impl PageSource for ShmPages {
    fn len(&self) -> u64 {
        self.pages.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        self.pages.read_at(offset, buf)
    }

    fn shareable(&self) -> bool {
        true
    }

    fn shared_frame(&self, offset: u64) -> Option<Result<Arc<SharedFrame>, IoError>> {
        self.pages.shared_frame(offset)
    }

    fn sysv_shm(&self) -> Option<u64> {
        Some(self.serial)
    }
}

/// What the personality's segment holds: the pages.
#[derive(Debug)]
pub struct Segment {
    pages: Arc<ShmPages>,
}

impl ShmMemory for Segment {
    fn attachments(&self) -> u64 {
        Arc::strong_count(&self.pages) as u64 - 1
    }

    fn serial(&self) -> u64 {
        self.pages.serial
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// The memory of a new segment of `len` bytes (whole pages). No frame is
/// taken until a page is touched.
pub fn create(len: u64) -> Arc<dyn ShmMemory> {
    let serial = SERIALS.fetch_add(1, Ordering::Relaxed);
    Arc::new(Segment { pages: Arc::new(ShmPages { pages: AnonShared::new(len), serial }) })
}

/// Attach `segment` to `memory`: its first `len` bytes, shared, with
/// `prot` (and never more than that, but for `PROT_EXEC`), at
/// `placement`. A fixed placement over a mapping is `EINVAL` (shmat(2)),
/// one below the lowest mappable address `EPERM` (as mmap(2)), no room or
/// a range past the end of user space `ENOMEM`.
pub fn attach(
    memory: &Memory,
    segment: &dyn ShmMemory,
    placement: poly_vm::Placement,
    len: u64,
    prot: poly_vm::Prot,
) -> Result<u64, Errno> {
    let segment = segment.as_any().downcast_ref::<Segment>().ok_or(Errno::EINVAL)?;
    let source: Arc<dyn PageSource> = segment.pages.clone();
    let max = prot | poly_vm::Prot::READ | poly_vm::Prot::EXEC;
    let req = poly_vm::MapRequest::file(len, prot, FileRef::new(source), 0, true).with_max_prot(max).at(placement);
    memory.map(req).map_err(|e| match e {
        MemoryError::OutOfMemory => Errno::ENOMEM,
        MemoryError::Vm(poly_vm::Error::NoMemory) => Errno::ENOMEM,
        MemoryError::Vm(poly_vm::Error::Permission) => Errno::EPERM,
        _ => Errno::EINVAL,
    })
}

/// The serials of the attachments in `memory` with pages in
/// `[addr, addr + len)`, one per attachment.
pub fn mapped(memory: &Memory, addr: u64, len: u64) -> Vec<u64> {
    let end = addr.saturating_add(len);
    attachments(memory).into_iter().filter(|a| a.0 < end && addr < a.1).map(|a| a.2).collect()
}

/// The attachments of `memory`: start, end and serial of each run of
/// adjacent regions mapping one segment's pages in order.
fn attachments(memory: &Memory) -> Vec<(u64, u64, u64)> {
    let mut out: Vec<(u64, u64, u64)> = Vec::new();
    let mut next_offset = 0;
    for v in memory.regions() {
        let Backing::File { file, offset, shared: true } = &v.backing else { continue };
        let Some(serial) = file.source().sysv_shm() else { continue };
        match out.last_mut() {
            Some(last) if last.1 == v.start && last.2 == serial && *offset == next_offset => last.1 = v.end,
            _ => out.push((v.start, v.end, serial)),
        }
        next_offset = offset + v.size();
    }
    out
}

/// shmdt(2): unmap the attachment that starts at `addr` and return its
/// segment's serial; `EINVAL` if none starts there.
pub fn detach(memory: &Memory, addr: u64) -> Result<u64, Errno> {
    let (start, end, serial) = attachments(memory).into_iter().find(|a| a.0 == addr).ok_or(Errno::EINVAL)?;
    memory.unmap(start, end - start).map_err(|_| Errno::EINVAL)?;
    Ok(serial)
}

/// Unmap every attachment of `memory` (its process is exiting); their
/// serials.
pub fn detach_all(memory: &Memory) -> Vec<u64> {
    let all = attachments(memory);
    for (start, end, _) in &all {
        let _ = memory.unmap(*start, end - start);
    }
    all.into_iter().map(|a| a.2).collect()
}
