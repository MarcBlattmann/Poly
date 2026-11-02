//! What the native personality needs from the kernel: [`NativeKernel`],
//! implemented by the kernel for the calling thread (`kernel/src/native`)
//! and by a fake in the host tests.
//!
//! The handlers in [`crate::sys`] have already checked and decoded every
//! argument (flags, lengths, user strings copied in) when they call it,
//! so the kernel side only maps requests onto its objects.

use crate::abi::{Clock, Error, ExitStatus, Handle, Whence};
use alloc::vec::Vec;

/// A decoded `open` request.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OpenHow {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub exclusive: bool,
    pub truncate: bool,
    pub append: bool,
    pub nonblock: bool,
    pub directory: bool,
    /// Permission bits of a file it creates (`0o7777` at most).
    pub mode: u16,
}

/// Memory protection, decoded from [`crate::abi::prot`] bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Prot {
    pub read: bool,
    pub write: bool,
    pub exec: bool,
}

/// A decoded `spawn` request: everything copied in from the caller.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Spawn {
    pub path: Vec<u8>,
    pub args: Vec<Vec<u8>>,
    pub env: Vec<Vec<u8>>,
    /// The caller's handles the child gets as 0, 1 and 2 (`None`: none).
    pub stdio: [Option<Handle>; 3],
}

/// The kernel's services for one native thread.
pub trait NativeKernel {
    /// The calling process's id.
    fn pid(&self) -> u32;

    /// Copy user memory at `addr` into `dst` ([`Error::BadAddress`] if it
    /// is not mapped readable user memory).
    fn copy_from_user(&self, addr: u64, dst: &mut [u8]) -> Result<(), Error>;

    /// Copy `src` to user memory at `addr`.
    fn copy_to_user(&mut self, addr: u64, src: &[u8]) -> Result<(), Error>;

    /// End the whole process with exit status `code` once the call returns.
    fn exit(&mut self, code: u8);

    /// Write `buf` to `handle`, waiting while a pipe or socket is full
    /// (unless it is non-blocking). Returns the bytes written.
    fn write(&mut self, handle: Handle, buf: &[u8]) -> Result<usize, Error>;

    /// Read into `buf` from `handle`, waiting until something is there
    /// (unless it is non-blocking). Returns the bytes read.
    fn read(&mut self, handle: Handle, buf: &mut [u8]) -> Result<usize, Error>;

    /// Open the file at `path` (absolute, or relative to the working
    /// directory) on the lowest free handle.
    fn open(&mut self, path: &[u8], how: &OpenHow) -> Result<Handle, Error>;

    fn close(&mut self, handle: Handle) -> Result<(), Error>;

    /// A new handle (the lowest free one) on `handle`'s open file.
    fn dup(&mut self, handle: Handle) -> Result<Handle, Error>;

    fn seek(&mut self, handle: Handle, offset: i64, whence: Whence) -> Result<u64, Error>;

    /// Map `len` bytes (whole pages) of zeroed private memory with `prot`,
    /// at `hint` if that range is free (0: anywhere). Returns the address.
    fn map(&mut self, hint: u64, len: u64, prot: Prot) -> Result<u64, Error>;

    /// Unmap `[addr, addr + len)` (whole pages; holes are fine).
    fn unmap(&mut self, addr: u64, len: u64) -> Result<(), Error>;

    fn protect(&mut self, addr: u64, len: u64, prot: Prot) -> Result<(), Error>;

    /// The time on `clock` in nanoseconds.
    fn clock(&self, clock: Clock) -> u64;

    /// Sleep for `ns` nanoseconds; [`Error::Interrupted`] if the process
    /// is ended meanwhile.
    fn sleep(&mut self, ns: u64) -> Result<(), Error>;

    fn yield_now(&mut self);

    /// Start the program `spawn` describes as a child. Returns its pid.
    fn spawn(&mut self, spawn: &Spawn) -> Result<u32, Error>;

    /// Wait for the child `pid` to end and forget it ([`Error::NoChild`]
    /// if it is not a child, or was waited for already).
    fn wait(&mut self, pid: u32) -> Result<ExitStatus, Error>;
}
