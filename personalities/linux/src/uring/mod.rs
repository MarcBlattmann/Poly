//! io_uring (io_uring_setup(2), io_uring_enter(2), io_uring_register(2)):
//! the shared rings, submission and completion bookkeeping, and how each
//! request runs.
//!
//! An instance ([`Instance`]) owns two pieces of memory the kernel makes
//! ([`UringKernel::ring_memory`]) and user space maps from the ring's file
//! descriptor: the rings region (SQ ring fields, CQ ring fields, the CQEs
//! and the SQ index array, one region as `IORING_FEAT_SINGLE_MMAP`
//! promises) and the SQE array. Everything else (requests still waiting,
//! CQEs that did not fit, registered buffers, files and eventfd) is the
//! instance's own state behind a lock.
//!
//! There is no worker thread: requests run in the thread that submits
//! them. One that cannot finish at once (a read of an empty pipe, a poll,
//! a timeout) waits in the instance's pending list and is tried again on
//! every io_uring_enter(2), and also by its process's threads on their way
//! back to user mode after any system call or interrupt
//! ([`exec::background`]), so a request completes soon after its file
//! becomes ready even if the program never enters again, as Linux's
//! asynchronous completion promises. A `GETEVENTS` wait sleeps in a
//! poll(2) of the pending requests' descriptors until one is ready, a
//! timeout is due or a signal comes ([`exec`]). The kernel watches the
//! files of waiting requests ([`UringKernel::watch`]): a wake-up of one
//! kicks the instance, which wakes whoever polls the ring's descriptor.
//!
//! Layouts and constants are those of the UAPI header `linux/io_uring.h`.

pub mod abi;
pub mod exec;
pub mod ring;

#[cfg(test)]
mod tests;

pub use abi::{Cqe, Layout, Params, Sqe};
pub use ring::{Instance, Notifier, Pending, Wait};

use crate::errno::Errno;
use alloc::sync::Arc;
use core::any::Any;

/// A reference to an open file description, held by the kernel's own type
/// (registered files and the registered eventfd keep their files open
/// while the descriptors they came from may be closed).
pub type FileHandle = Arc<dyn Any + Send + Sync>;

/// Memory shared between the kernel and the processes that map a ring.
/// Offsets are in bytes from the start of the region; accesses never
/// cross its end (the instance only touches offsets its [`Layout`] puts
/// inside).
pub trait RingMemory: Send + Sync + core::fmt::Debug {
    /// Size in bytes.
    fn len(&self) -> usize;

    /// Whether it is empty (it never is).
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Copy `buf.len()` bytes at `offset` out.
    fn read(&self, offset: usize, buf: &mut [u8]);

    /// Copy `data` in at `offset`.
    fn write(&self, offset: usize, data: &[u8]);

    /// The aligned `u32` at `offset`, with acquire ordering (user space
    /// publishes the SQ tail and CQ head with release stores).
    fn load_acquire(&self, offset: usize) -> u32;

    /// Store the aligned `u32` at `offset` with release ordering.
    fn store_release(&self, offset: usize, value: u32);

    /// The kernel's own type behind the trait object (to map it).
    fn as_any(&self) -> &dyn Any;
}

/// The kernel's watch on the file of a waiting request: it counts the
/// file's wake-ups (for the events the request asks for) and kicks the
/// instance ([`Instance::kick`]) at each. Dropping it ends the watch.
pub trait Watch: Send + Sync + core::fmt::Debug {
    /// Wake-ups so far.
    fn count(&self) -> u64;
}

/// What io_uring needs from the kernel beyond [`LinuxKernel`]: ring
/// memory, the ring's file descriptor, and references to open files.
///
/// [`LinuxKernel`]: crate::kernel::LinuxKernel
pub trait UringKernel {
    /// New zero-filled memory of at least `len` bytes that the ring's
    /// descriptor maps ([`Instance::region`]). `ENOMEM`.
    fn ring_memory(&mut self, len: usize) -> Result<Arc<dyn RingMemory>, Errno>;

    /// Put `instance` on the lowest free descriptor: an anonymous inode
    /// (`anon_inode:[io_uring]`), `O_RDWR`, close-on-exec. `EMFILE`,
    /// `ENFILE`.
    fn install_ring(&mut self, instance: Arc<Instance>) -> Result<i32, Errno>;

    /// The instance open as `fd`: `EBADF` if `fd` is not open, `None` if it
    /// is not an io_uring.
    fn ring_of(&mut self, fd: i32) -> Result<Option<Arc<Instance>>, Errno>;

    /// A reference to the file open as `fd` (`EBADF`).
    fn file_handle(&mut self, fd: i32) -> Result<FileHandle, Errno>;

    /// Open `handle` again on the lowest free descriptor (close-on-exec),
    /// for one request on a registered file; the caller closes it.
    fn install_handle(&mut self, handle: &FileHandle) -> Result<i32, Errno>;

    /// Whether `fd` is an eventfd (`EBADF` if it is not open).
    fn is_eventfd(&mut self, fd: i32) -> Result<bool, Errno>;

    /// Add 1 to the eventfd `handle` refers to (errors are ignored: a
    /// counter at its maximum stays there).
    fn eventfd_signal(&mut self, handle: &FileHandle);

    /// Whether `handle` refers to an io_uring instance (which cannot be
    /// registered as a file).
    fn handle_is_ring(&mut self, handle: &FileHandle) -> bool;

    /// Watch the file open as `fd` for wake-ups with `events` on behalf of
    /// `instance`; `None` if the file has nothing to watch (a regular file
    /// is always ready) or is not open.
    fn watch(&mut self, fd: i32, events: u16, instance: &Arc<Instance>) -> Option<Arc<dyn Watch>> {
        let _ = (fd, events, instance);
        None
    }
}
