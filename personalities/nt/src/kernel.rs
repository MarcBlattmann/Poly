//! What the NT services need from the kernel, as a trait the kernel
//! implements for the calling thread (one value per system call), so the
//! services run on the host against a fake.

use crate::backend::Backend;
use crate::system::{NtProcess, NtSystem};
use poly_ntobj::{Caller, NtStatus};

/// A user-memory access failed (the address is not mapped user memory
/// with the needed rights).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserFault;

/// The calling thread's view of the kernel.
pub trait NtKernel {
    /// Copy from the caller's memory.
    fn read_user(&mut self, addr: u64, buf: &mut [u8]) -> Result<(), UserFault>;

    /// Copy to the caller's memory.
    fn write_user(&mut self, addr: u64, data: &[u8]) -> Result<(), UserFault>;

    /// Who is calling (process, thread, previous mode).
    fn caller(&self) -> Caller;

    /// Run `f` with the system-wide NT state, the calling process's state
    /// and the kernel's side of its memory, all locked. Nothing in `f`
    /// touches user memory through [`read_user`](Self::read_user) or
    /// [`write_user`](Self::write_user).
    fn with_state<R>(&mut self, f: impl FnOnce(&mut NtSystem, &mut NtProcess, &mut dyn Backend) -> R) -> R;

    /// End the calling process with `status` once the current call
    /// returns (`NtTerminateProcess`).
    fn terminate_process(&mut self, status: NtStatus);

    /// Log three values for a test program (`PolyTrace`).
    fn trace(&mut self, values: [u64; 3]);
}
