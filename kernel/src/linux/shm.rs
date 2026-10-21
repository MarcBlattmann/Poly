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
use poly_personality_linux::ipc::shm::{ShmMemory, ShmPiece, attachment_at, attachments_in};

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
    attachments_in(&pieces(memory), addr, len)
}

/// The regions of `memory` that map segments' pages, in address order.
fn pieces(memory: &Memory) -> Vec<ShmPiece> {
    let mut out = Vec::new();
    for v in memory.regions() {
        let Backing::File { file, offset, shared: true } = &v.backing else { continue };
        let source = file.source();
        let Some(serial) = source.sysv_shm() else { continue };
        out.push(ShmPiece { start: v.start, end: v.end, offset: *offset, serial, len: source.len() });
    }
    out
}

/// shmdt(2): unmap every piece of the attachment whose offset 0 is (or
/// was) at `addr` ([`attachment_at`]) and return its segment's serial;
/// `EINVAL` if there is none.
pub fn detach(memory: &Memory, addr: u64) -> Result<u64, Errno> {
    let (serial, ranges) = attachment_at(&pieces(memory), addr).ok_or(Errno::EINVAL)?;
    for (start, end) in ranges {
        memory.unmap(start, end - start).map_err(|_| Errno::EINVAL)?;
    }
    Ok(serial)
}

/// Unmap every attachment of `memory` (its process is exiting); their
/// serials.
pub fn detach_all(memory: &Memory) -> Vec<u64> {
    let all = pieces(memory);
    for p in &all {
        let _ = memory.unmap(p.start, p.end - p.start);
    }
    attachments_in(&all, 0, u64::MAX)
}
