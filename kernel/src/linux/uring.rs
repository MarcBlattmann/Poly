//! io_uring in the kernel (the personality's `uring` module does the
//! work): the ring memory, the file object behind a ring's descriptor and
//! its mmap(2) regions, and the file references registered files keep.
//!
//! Ring memory is shared anonymous memory ([`AnonShared`]): every mapping
//! of a region (in whichever process, after `fork`) maps its frames, and
//! the kernel reads and writes the same frames through the direct map.
//! The mappings hold the memory, so the rings outlive the descriptor.
//!
//! The ring file has a wait queue: the instance notifies it when it posts
//! a CQE or is kicked ([`Instance::kick`]), which a [`FileWatch`] on the
//! file of a waiting request does at each wake-up of that file. A kicked
//! instance reads as readable (and `POLLPRI`) until its requests were
//! tried again, which the owner's threads do on their way to user mode.

use super::Thread;
use super::services::vfs_errno;
use crate::arch;
use crate::fs::{self, OpenFile, OpenFlags};
use crate::user::AnonShared;
use crate::user::memory::PageSource;
use alloc::sync::{Arc, Weak};
use core::any::Any;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::uring::{FileHandle, Instance, RingMemory, UringKernel, Watch};
use poly_vfs::eventfd::EventFd;
use poly_vfs::poll::{Registration, Waker};
use poly_vfs::{FileObject, PollEvents, WaitQueue};

const PAGE: usize = 4096;

/// One region of a ring.
#[derive(Debug)]
pub struct Region {
    pages: Arc<AnonShared>,
    len: usize,
}

impl Region {
    /// The direct-map address of byte `offset` (inside the region), with
    /// its page made if it was not yet.
    fn at(&self, offset: usize) -> *mut u8 {
        let page = (offset / PAGE * PAGE) as u64;
        let frame = match self.pages.shared_frame(page) {
            Some(Ok(f)) => f,
            _ => panic!("io_uring: no frame for ring memory"),
        };
        let base = arch::phys_to_virt(frame.phys()).expect("user frame outside the direct map");
        // The frame stays allocated: `self.pages` holds it for as long as
        // the region lives, and the region outlives this call.
        (base as usize + offset % PAGE) as *mut u8
    }

    /// Copy between `offset..offset + len` and a buffer, page by page.
    fn each_page(&self, offset: usize, len: usize, mut f: impl FnMut(*mut u8, usize, usize)) {
        assert!(offset.checked_add(len).is_some_and(|end| end <= self.len), "io_uring: access past the region");
        let mut done = 0;
        while done < len {
            let at = offset + done;
            let n = (PAGE - at % PAGE).min(len - done);
            f(self.at(at), done, n);
            done += n;
        }
    }

    fn word(&self, offset: usize) -> &AtomicU32 {
        assert!(offset.is_multiple_of(4) && offset + 4 <= self.len, "io_uring: unaligned ring word");
        // SAFETY: `at` points at 4 aligned bytes (offset is a multiple of
        // 4, pages are 4096-aligned) inside a frame the region keeps
        // allocated while `self` is borrowed; the frame is only accessed
        // as plain bytes or atomically, and every bit pattern is a valid
        // `u32`.
        unsafe { &*(self.at(offset) as *const AtomicU32) }
    }
}

impl RingMemory for Region {
    fn len(&self) -> usize {
        self.len
    }

    fn read(&self, offset: usize, buf: &mut [u8]) {
        self.each_page(offset, buf.len(), |p, done, n| {
            // SAFETY: `p..p + n` lies inside one frame the region holds
            // (`each_page` splits at page ends); user space may write it
            // meanwhile, and bytes have no invalid values.
            unsafe { core::ptr::copy_nonoverlapping(p, buf[done..].as_mut_ptr(), n) };
        });
    }

    fn write(&self, offset: usize, data: &[u8]) {
        self.each_page(offset, data.len(), |p, done, n| {
            // SAFETY: as in `read`; the destination is ring memory, which
            // only this region and user mappings of it reach.
            unsafe { core::ptr::copy_nonoverlapping(data[done..].as_ptr(), p, n) };
        });
    }

    fn load_acquire(&self, offset: usize) -> u32 {
        self.word(offset).load(Ordering::Acquire)
    }

    fn store_release(&self, offset: usize, value: u32) {
        self.word(offset).store(value, Ordering::Release);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// What a ring's descriptor is open on.
pub struct RingFile {
    instance: Arc<Instance>,
    queue: Arc<WaitQueue>,
}

impl core::fmt::Debug for RingFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RingFile").field("instance", &self.instance).finish()
    }
}

impl FileObject for RingFile {
    fn read(&self, _buf: &mut [u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn write(&self, _buf: &[u8]) -> poly_vfs::Result<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    /// Readable while the CQ ring holds CQEs, or while the instance is
    /// kicked and has requests waiting (one of them is about to complete:
    /// the poller's way back to user mode tries them, see the module
    /// documentation); `POLLPRI` while kicked, which io_uring_enter's own
    /// wait asks for.
    fn poll(&self) -> PollEvents {
        let mut e = PollEvents::WRITABLE;
        let kicked = self.instance.kicked() && self.instance.has_pending();
        if self.instance.cq_ready() > 0 || kicked {
            e = e | PollEvents::READABLE;
        }
        if kicked {
            e = e | PollEvents::PRI;
        }
        e
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[io_uring]")
    }
}

/// mmap(2) of a ring's descriptor: the region file offset `offset` names
/// and where in it the mapping starts (always 0), or `None` if `file` is
/// no ring. `ENOMEM` for an offset that names no region, `EINVAL` for a
/// mapping longer than the region.
pub fn mapping(file: &OpenFile, offset: u64, len: u64) -> Option<Result<Arc<dyn PageSource>, Errno>> {
    let ring = file.object_as::<RingFile>()?;
    let Some(region) = ring.instance.region(offset) else { return Some(Err(Errno::ENOMEM)) };
    let region = region.as_any().downcast_ref::<Region>()?;
    if len > region.pages.len() {
        return Some(Err(Errno::EINVAL));
    }
    let source: Arc<dyn PageSource> = region.pages.clone();
    Some(Ok(source))
}

/// Counts the wake-ups of one file and kicks the instance at each.
struct WatchWaker {
    count: AtomicU64,
    instance: Weak<Instance>,
}

impl Waker for WatchWaker {
    fn wake(&self, _events: PollEvents) {
        self.count.fetch_add(1, Ordering::SeqCst);
        if let Some(i) = self.instance.upgrade() {
            i.kick();
        }
    }
}

/// A waiting request's watch on its file ([`UringKernel::watch`]); the
/// registration ends with it.
pub struct FileWatch {
    waker: Arc<WatchWaker>,
    _registration: Registration,
}

impl core::fmt::Debug for FileWatch {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FileWatch").field("count", &self.waker.count.load(Ordering::Relaxed)).finish()
    }
}

impl Watch for FileWatch {
    fn count(&self) -> u64 {
        self.waker.count.load(Ordering::SeqCst)
    }
}

impl UringKernel for Thread<'_> {
    fn ring_memory(&mut self, len: usize) -> Result<Arc<dyn RingMemory>, Errno> {
        let rounded = len.div_ceil(PAGE).max(1) * PAGE;
        Ok(Arc::new(Region { pages: Arc::new(AnonShared::new(rounded as u64)), len: rounded }))
    }

    fn install_ring(&mut self, instance: Arc<Instance>) -> Result<i32, Errno> {
        let files = &self.process.files;
        let node = fs::vfs().anon_inode(0o600, &files.cred()).map_err(vfs_errno)?;
        let queue = Arc::new(WaitQueue::new());
        let notify = Arc::clone(&queue);
        instance.set_notifier(Arc::new(move || notify.notify(PollEvents::READABLE | PollEvents::PRI)));
        let file = Arc::new(OpenFile::on_object(node, Arc::new(RingFile { instance, queue }), OpenFlags::RDWR));
        files.install(file, true).map_err(vfs_errno)
    }

    fn ring_of(&mut self, fd: i32) -> Result<Option<Arc<Instance>>, Errno> {
        let file = self.process.files.file(fd).map_err(vfs_errno)?;
        Ok(file.object_as::<RingFile>().map(|r| Arc::clone(&r.instance)))
    }

    fn file_handle(&mut self, fd: i32) -> Result<FileHandle, Errno> {
        let file = self.process.files.file(fd).map_err(vfs_errno)?;
        Ok(Arc::new(file))
    }

    fn install_handle(&mut self, handle: &FileHandle) -> Result<i32, Errno> {
        let file = handle.downcast_ref::<Arc<OpenFile>>().ok_or(Errno::EBADF)?;
        self.process.files.install(Arc::clone(file), true).map_err(vfs_errno)
    }

    fn is_eventfd(&mut self, fd: i32) -> Result<bool, Errno> {
        let file = self.process.files.file(fd).map_err(vfs_errno)?;
        Ok(file.object_as::<EventFd>().is_some())
    }

    fn eventfd_signal(&mut self, handle: &FileHandle) {
        if let Some(e) = handle.downcast_ref::<Arc<OpenFile>>().and_then(|f| f.object_as::<EventFd>()) {
            let _ = e.write(&1u64.to_ne_bytes());
        }
    }

    fn handle_is_ring(&mut self, handle: &FileHandle) -> bool {
        handle.downcast_ref::<Arc<OpenFile>>().is_some_and(|f| f.object_as::<RingFile>().is_some())
    }

    fn watch(&mut self, fd: i32, events: u16, instance: &Arc<Instance>) -> Option<Arc<dyn Watch>> {
        let file = self.process.files.file(fd).ok()?;
        let queue = file.wait_queue()?;
        let waker = Arc::new(WatchWaker { count: AtomicU64::new(0), instance: Arc::downgrade(instance) });
        let registration = queue.register(PollEvents::from_bits(u32::from(events)), waker.clone());
        Some(Arc::new(FileWatch { waker, _registration: registration }))
    }
}
