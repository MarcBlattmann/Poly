//! userfaultfd(2) objects (ADR 0122): the file behind the descriptor and
//! what a fault on a registered page does.
//!
//! The object's state (handshake, features, message queue) is the
//! personality's `UffdState`; registrations live in the region map of the
//! memory the object was made for (`poly_vm::Vma::uffd`, by the object's
//! id). A missing page of a registered range makes `user::page_fault`
//! call [`fault`]: the message is queued and the thread waits until a
//! wake covers its page (`UFFDIO_WAKE`, `UFFDIO_COPY` / `UFFDIO_ZEROPAGE`
//! without `DONTWAKE`), the range is unregistered or the object is
//! closed, and then resolves the fault again (or, with
//! `UFFD_FEATURE_SIGBUS`, gets `SIGBUS` at once). A fault the kernel takes
//! inside a system call is not handled by an object made with
//! `UFFD_USER_MODE_ONLY`: the copy fails with `EFAULT`. Waits are on the
//! process event queue; everything that may end one bumps a generation
//! counter and wakes that queue.

use super::Thread;
use super::services::vfs_errno;
use crate::fs::{self, OpenFile, OpenFlags};
use crate::process;
use crate::sync::SpinLock;
use crate::user::FaultError;
use crate::user::memory::FillError;
use crate::user::memory::Memory;
use alloc::collections::BTreeMap;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use poly_personality_linux::errno::Errno;
use poly_personality_linux::sys::userfaultfd::{UffdOp, UffdReply, UffdState, uapi};
use poly_vfs::poll::{PollEvents, WaitQueue};
use poly_vfs::{FileObject, Result as VfsResult};

const PAGE: u64 = crate::user::memory::PAGE_SIZE;

/// Live objects by id, for [`fault`].
static OBJECTS: SpinLock<BTreeMap<u32, Weak<Uffd>>> = SpinLock::new(BTreeMap::new());
/// The next object id (0 means "not registered" in the region map).
static NEXT_ID: AtomicU32 = AtomicU32::new(1);
/// Bumped by everything that may end a fault's wait, on any object.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// One userfaultfd object.
pub struct Uffd {
    id: u32,
    state: SpinLock<UffdState>,
    /// Readers and pollers of the descriptor.
    queue: Arc<WaitQueue>,
    /// The memory whose faults it handles.
    memory: Weak<Memory>,
    /// Made with `O_NONBLOCK`: poll(2) reports `POLLERR` otherwise.
    nonblock: bool,
}

fn errno_vfs(e: Errno) -> poly_vfs::Errno {
    poly_vfs::Errno::from_code(i32::from(e.get())).unwrap_or(poly_vfs::Errno::EINVAL)
}

impl Uffd {
    /// Tell waiting faults to look at their pages again.
    fn kick(&self) {
        GENERATION.fetch_add(1, Ordering::SeqCst);
        process::wake_waiters();
    }

    /// Wake the tasks blocked on pages of `[start, start + len)`.
    fn wake(&self, start: u64, len: u64) {
        self.state.lock().wake(start, len);
        self.kick();
    }

    fn memory(&self) -> Result<Arc<Memory>, Errno> {
        self.memory.upgrade().ok_or(Errno::EINVAL)
    }

    /// Carry out a request from the personality.
    fn op(&self, op: UffdOp<'_>) -> Result<UffdReply, Errno> {
        match op {
            UffdOp::Probe => Ok(UffdReply::Done),
            UffdOp::Ready => self.state.lock().ready().map(|()| UffdReply::Done),
            UffdOp::Api { api, features, privileged } => {
                let (features, ioctls) = self.state.lock().api(api, features, privileged)?;
                Ok(UffdReply::Api { features, ioctls })
            }
            UffdOp::Register { start, len, mode } => {
                self.memory()?.set_userfault(start, len, self.id).map_err(|_| Errno::EINVAL)?;
                let wp = if mode & uapi::UFFDIO_REGISTER_MODE_WP != 0 { 1 << uapi::NR_WRITEPROTECT } else { 0 };
                Ok(UffdReply::Ioctls(uapi::RANGE_IOCTLS | wp))
            }
            UffdOp::Unregister { start, len } => {
                self.memory()?.clear_userfault(start, len, self.id).map_err(|_| Errno::EINVAL)?;
                self.wake(start, len);
                Ok(UffdReply::Done)
            }
            UffdOp::Wake { start, len } => {
                self.wake(start, len);
                Ok(UffdReply::Done)
            }
            UffdOp::Copy { dst, data, wake } => {
                let filled = self.fill(dst, PAGE, Some(data));
                if wake && matches!(filled, Ok(UffdReply::Filled(n)) if n > 0) {
                    self.wake(dst, PAGE);
                }
                filled
            }
            UffdOp::ZeroPage { start, len, wake } => {
                let filled = self.fill(start, len, None);
                if wake
                    && let Ok(UffdReply::Filled(n)) = filled
                    && n > 0
                {
                    self.wake(start, n);
                }
                filled
            }
        }
    }

    /// Fill the missing pages of `[start, start + len)` with `data` (one
    /// page) or zeros, up to the first present one.
    fn fill(&self, start: u64, len: u64, data: Option<&[u8]>) -> Result<UffdReply, Errno> {
        let memory = self.memory()?;
        let mut done = 0;
        while done < len {
            match memory.fill_missing(start + done, data, self.id) {
                Ok(()) => done += PAGE,
                Err(FillError::Present) => break,
                Err(_) if done > 0 => break,
                Err(FillError::NotRegistered) => return Err(Errno::ENOENT),
                Err(FillError::OutOfMemory) => return Err(Errno::ENOMEM),
            }
        }
        Ok(UffdReply::Filled(done))
    }
}

impl Drop for Uffd {
    fn drop(&mut self) {
        OBJECTS.lock().remove(&self.id);
        if let Some(memory) = self.memory.upgrade() {
            memory.forget_userfaults(self.id);
        }
        self.kick();
    }
}

impl FileObject for Uffd {
    fn read(&self, buf: &mut [u8]) -> VfsResult<usize> {
        self.state.lock().read(buf).map_err(errno_vfs)
    }

    fn write(&self, _buf: &[u8]) -> VfsResult<usize> {
        Err(poly_vfs::Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if !self.nonblock {
            return PollEvents::ERR;
        }
        if self.state.lock().pending() { PollEvents::READABLE } else { PollEvents::NONE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[userfaultfd]")
    }
}

/// userfaultfd(2) for thread `t`: a new object for its memory, installed
/// as a descriptor.
pub fn create(t: &Thread<'_>, nonblock: bool, cloexec: bool, user_mode_only: bool) -> Result<i32, Errno> {
    let memory = crate::user::current_memory().ok_or(Errno::EINVAL)?;
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let object = Arc::new(Uffd {
        id,
        state: SpinLock::new(UffdState::with_user_mode_only(user_mode_only)),
        queue: Arc::new(WaitQueue::new()),
        memory: Arc::downgrade(&memory),
        nonblock,
    });
    OBJECTS.lock().insert(id, Arc::downgrade(&object));
    let files = &t.process().files;
    let node = fs::vfs().anon_inode(0o600, &files.cred()).map_err(vfs_errno)?;
    // The description is read-only: F_GETFL shows `O_RDONLY` plus
    // `O_NONBLOCK` if asked for; reads and ioctls need no more.
    let flags = if nonblock { OpenFlags::RDONLY | OpenFlags::NONBLOCK } else { OpenFlags::RDONLY };
    let file = Arc::new(OpenFile::on_object(node, object, flags));
    files.install(file, cloexec).map_err(vfs_errno)
}

/// A request on descriptor `fd` of thread `t`; `None` if it is no
/// userfaultfd object.
pub fn op(t: &Thread<'_>, fd: i32, op: UffdOp<'_>) -> Option<Result<UffdReply, Errno>> {
    let file = t.process().files.file(fd).ok()?;
    let object = file.object_as::<Uffd>()?;
    Some(object.op(op))
}

/// A fault at `addr` (`write` or read) by thread `tid` on a missing page
/// registered with object `id`, in `memory`, taken in user mode or by the
/// kernel inside a system call (`kernel`): report it and wait until a wake
/// covers the page (see the module docs). `Ok` means: resolve the fault
/// again. `SIGBUS` (`EFAULT` for a kernel fault) with
/// `UFFD_FEATURE_SIGBUS`, for a kernel fault with `UFFD_USER_MODE_ONLY`,
/// or when the thread must end (`stop`) while it waits.
pub fn fault(
    memory: &Arc<Memory>,
    id: u32,
    addr: u64,
    write: bool,
    tid: u32,
    kernel: bool,
    stop: &dyn Fn() -> bool,
) -> Result<(), FaultError> {
    let Some(object) = OBJECTS.lock().get(&id).and_then(Weak::upgrade) else {
        // Closed meanwhile: the registration goes, the page faults as usual.
        memory.forget_userfaults(id);
        return Ok(());
    };
    let ticket = {
        let mut state = object.state.lock();
        if !state.handles(kernel) || state.sigbus() {
            return Err(FaultError::Bus);
        }
        state.push_fault(addr, write, tid)
    };
    object.queue.notify(PollEvents::READABLE);
    // Only a weak reference while waiting: closing the descriptor drops
    // the object, which ends the wait.
    let weak = Arc::downgrade(&object);
    drop(object);
    let leave = |result| {
        if let Some(o) = weak.upgrade() {
            o.state.lock().leave(ticket);
        }
        result
    };
    loop {
        let seen = GENERATION.load(Ordering::SeqCst);
        match weak.upgrade() {
            None => return Ok(()),
            // Woken, or the page no longer faults to this object (its
            // range was unregistered or unmapped).
            Some(o) if o.state.lock().woken(ticket) || !memory.userfault_registered(addr, id) => {
                drop(o);
                return leave(Ok(()));
            }
            Some(_) => {}
        }
        // Atomics only: the condition runs with the scheduler locked.
        process::wait_event(|| GENERATION.load(Ordering::SeqCst) != seen || stop());
        if stop() {
            return leave(Err(FaultError::Bus));
        }
    }
}
