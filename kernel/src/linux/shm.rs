//! System V shared memory in the kernel: the memory of a segment and its
//! attachments (shmat(2), shmdt(2)). The personality keeps the segments
//! themselves (`poly_personality_linux::ipc::shm`).
//!
//! A segment's memory is a [`ShmPages`]: shared anonymous memory
//! ([`AnonShared`], zero-filled pages made at the first fault) that knows
//! its serial. An attachment is a shared mapping of an `Attachment`, a
//! source of its own over those pages: every piece mprotect(2) or
//! munmap(2) leaves of it maps that one value, which knows the address
//! shmat(2) returned, so shmdt(2) of that address detaches all the pieces
//! (also once the first page is gone) and the pieces count as one
//! attachment (shmdt's spec sheet). `fork` gives the child the same pages
//! through attachments of its own, so every attachment sees every write
//! and the child's count separately. The personality's [`Segment`] holds
//! one `Arc` of the pages and every attachment another: the attachments
//! are the others. The pages are freed when the segment is gone and its
//! last attachment too.

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

/// One attachment: the source every region of one shmat(2) maps, so the
/// pieces mprotect(2) or munmap(2) leave of it are still one attachment,
/// named by the address shmat returned ([`PageSource::sysv_attach_addr`]).
/// It holds the segment's pages; fork gives the child an attachment of its
/// own ([`PageSource::fork_copy`]).
#[derive(Debug)]
struct Attachment {
    pages: Arc<ShmPages>,
    /// The attach address; 0 until the mapping is placed.
    addr: AtomicU64,
}

impl PageSource for Attachment {
    fn len(&self) -> u64 {
        self.pages.len()
    }

    fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<usize, IoError> {
        self.pages.read_at(offset, buf)
    }

    /// The segment's pages, so a futex in them has one key in every
    /// attachment of every process.
    fn object_id(&self) -> Option<u64> {
        Some(Arc::as_ptr(&self.pages) as *const () as u64)
    }

    fn shareable(&self) -> bool {
        true
    }

    fn shared_frame(&self, offset: u64) -> Option<Result<Arc<SharedFrame>, IoError>> {
        self.pages.shared_frame(offset)
    }

    fn sysv_shm(&self) -> Option<u64> {
        Some(self.pages.serial)
    }

    fn sysv_attach_addr(&self) -> Option<u64> {
        Some(self.addr.load(Ordering::Relaxed))
    }

    fn fork_copy(&self) -> Option<Arc<dyn PageSource>> {
        Some(Arc::new(Attachment {
            pages: self.pages.clone(),
            addr: AtomicU64::new(self.addr.load(Ordering::Relaxed)),
        }))
    }
}

/// What the personality's segment holds: the pages.
#[derive(Debug)]
pub struct Segment {
    pages: Arc<ShmPages>,
}

impl ShmMemory for Segment {
    /// The live attachments: every one holds the pages once, whatever
    /// number of pieces it is in now.
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
    let attachment = Arc::new(Attachment { pages: segment.pages.clone(), addr: AtomicU64::new(0) });
    let source: Arc<dyn PageSource> = attachment.clone();
    let max = prot | poly_vm::Prot::READ | poly_vm::Prot::EXEC;
    let req = poly_vm::MapRequest::file(len, prot, FileRef::new(source), 0, true).with_max_prot(max).at(placement);
    let addr = memory.map(req).map_err(|e| match e {
        MemoryError::OutOfMemory => Errno::ENOMEM,
        MemoryError::Vm(poly_vm::Error::NoMemory) => Errno::ENOMEM,
        MemoryError::Vm(poly_vm::Error::Permission) => Errno::EPERM,
        _ => Errno::EINVAL,
    })?;
    attachment.addr.store(addr, Ordering::Relaxed);
    Ok(addr)
}

/// A piece of an attachment: start, end, attach address and serial.
type Piece = (u64, u64, u64, u64);

/// Every region of `memory` that maps an attachment.
fn pieces(memory: &Memory) -> Vec<Piece> {
    let mut out = Vec::new();
    for v in memory.regions() {
        let Backing::File { file, shared: true, .. } = &v.backing else { continue };
        let (Some(serial), Some(at)) = (file.source().sysv_shm(), file.source().sysv_attach_addr()) else { continue };
        out.push((v.start, v.end, at, serial));
    }
    out
}

/// The serials of the attachments in `memory` with pages in
/// `[addr, addr + len)`, one per attachment.
pub fn mapped(memory: &Memory, addr: u64, len: u64) -> Vec<u64> {
    let end = addr.saturating_add(len);
    let mut seen: Vec<(u64, u64)> = Vec::new();
    for (start, stop, at, serial) in pieces(memory) {
        if start < end && addr < stop && !seen.contains(&(at, serial)) {
            seen.push((at, serial));
        }
    }
    seen.into_iter().map(|(_, serial)| serial).collect()
}

/// Unmap `pieces`.
fn unmap_all(memory: &Memory, pieces: &[Piece]) {
    for (start, end, _, _) in pieces {
        let _ = memory.unmap(*start, end - start);
    }
}

/// shmdt(2): unmap every piece of the attachment shmat placed at `addr`
/// (also when its first page is gone) and return its segment's serial;
/// `EINVAL` if no attachment has that address.
pub fn detach(memory: &Memory, addr: u64) -> Result<u64, Errno> {
    let mine: Vec<Piece> = pieces(memory).into_iter().filter(|p| p.2 == addr).collect();
    let serial = mine.first().ok_or(Errno::EINVAL)?.3;
    unmap_all(memory, &mine);
    Ok(serial)
}

/// Unmap every attachment of `memory` (its process is exiting); their
/// serials, one per attachment.
pub fn detach_all(memory: &Memory) -> Vec<u64> {
    let all = pieces(memory);
    unmap_all(memory, &all);
    let mut seen: Vec<(u64, u64)> = Vec::new();
    for (_, _, at, serial) in all {
        if !seen.contains(&(at, serial)) {
            seen.push((at, serial));
        }
    }
    seen.into_iter().map(|(_, serial)| serial).collect()
}
