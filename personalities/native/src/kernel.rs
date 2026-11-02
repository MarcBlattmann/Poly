//! What the native personality needs from the kernel: [`NativeKernel`],
//! implemented by the kernel for the calling thread (`kernel/src/native`)
//! and by a fake in the host tests.
//!
//! The handlers in [`crate::sys`] have already checked and decoded every
//! argument (flags, lengths, user strings copied in) when they call it,
//! so the kernel side only maps requests onto its objects.

use crate::abi::{Clock, Error, ExitStatus, FileInfo, Handle, PeerInfo, PollEntry, Whence};
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
    /// More handles: the caller's `.0` as the child's `.1` (ABI version 2).
    pub handles: Vec<(Handle, Handle)>,
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
    /// if it is not a child, or was waited for already). With `no_hang`,
    /// [`Error::WouldBlock`] if it has not ended.
    fn wait(&mut self, pid: u32, no_hang: bool) -> Result<ExitStatus, Error>;

    // ---- ABI version 2 ----------------------------------------------------

    /// A device request on `handle`'s open file (its ioctl).
    fn control(&mut self, handle: Handle, request: u32, arg: u64) -> Result<u64, Error>;

    /// Wait until an entry is ready or `timeout_ns` passed (`None`:
    /// forever); fill `revents`. Returns how many entries are ready.
    fn poll(&mut self, entries: &mut [PollEntry], timeout_ns: Option<u64>) -> Result<usize, Error>;

    /// Map `len` bytes (whole pages) of `handle`'s file from `offset`.
    fn map_handle(&mut self, handle: Handle, offset: u64, len: u64, prot: Prot, shared: bool) -> Result<u64, Error>;

    /// A new memory object of `size` bytes that can be sealed.
    fn memory_object(&mut self, size: u64) -> Result<Handle, Error>;

    fn seal(&mut self, handle: Handle, seals: u32) -> Result<(), Error>;

    fn set_nonblocking(&mut self, handle: Handle, on: bool) -> Result<(), Error>;

    fn handle_info(&mut self, handle: Handle) -> Result<FileInfo, Error>;

    fn stat(&mut self, path: &[u8]) -> Result<FileInfo, Error>;

    fn rename(&mut self, from: &[u8], to: &[u8]) -> Result<(), Error>;

    fn remove(&mut self, path: &[u8], directory: bool) -> Result<(), Error>;

    fn make_dir(&mut self, path: &[u8], mode: u16) -> Result<(), Error>;

    fn set_mode(&mut self, path: &[u8], mode: u16) -> Result<(), Error>;

    /// The next entries of the directory `handle` is open on, each given
    /// to `emit` (kind, name) until it returns false (that entry is not
    /// consumed). Returns how many were taken.
    fn read_dir(&mut self, handle: Handle, emit: &mut dyn FnMut(u8, &[u8]) -> bool) -> Result<usize, Error>;

    fn sync(&mut self);

    /// A pipe: its read and write handles.
    fn pipe(&mut self) -> Result<[Handle; 2], Error>;

    fn make_fifo(&mut self, path: &[u8], mode: u16) -> Result<(), Error>;

    fn socket(&mut self, kind: SocketKind, nonblock: bool) -> Result<Handle, Error>;

    fn socket_pair(&mut self, kind: SocketKind, nonblock: bool) -> Result<[Handle; 2], Error>;

    fn bind(&mut self, handle: Handle, name: &SocketName) -> Result<(), Error>;

    fn listen(&mut self, handle: Handle, backlog: u32) -> Result<(), Error>;

    fn accept(&mut self, handle: Handle, nonblock: bool) -> Result<Handle, Error>;

    fn connect(&mut self, handle: Handle, name: &SocketName) -> Result<(), Error>;

    /// Send `data` with `handles` (the caller's, passed on with the first
    /// byte). Returns the bytes sent.
    fn send(&mut self, handle: Handle, data: &[u8], handles: &[Handle]) -> Result<usize, Error>;

    /// Receive into `buf`; handles that came with the bytes become the
    /// caller's (at most `max_handles`; the rest are closed). Returns the
    /// byte count and the new handles.
    fn recv(&mut self, handle: Handle, buf: &mut [u8], max_handles: usize) -> Result<(usize, Vec<Handle>), Error>;

    fn socket_peer(&mut self, handle: Handle) -> Result<PeerInfo, Error>;

    /// A handle readable once child `pid` has ended.
    fn process_handle(&mut self, pid: u32) -> Result<Handle, Error>;

    fn signal(&mut self, pid: u32, what: Signal) -> Result<(), Error>;

    /// The network interfaces: `InterfaceRecord` flags and name.
    fn net_interfaces(&mut self) -> Vec<(u32, Vec<u8>)>;

    fn truncate(&mut self, handle: Handle, len: u64) -> Result<(), Error>;
}

/// The kind of an AF_UNIX socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketKind {
    Stream,
    Datagram,
    SeqPacket,
}

/// A socket name: a path in the file system or a name in the abstract
/// name space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SocketName {
    Path(Vec<u8>),
    Abstract(Vec<u8>),
}

/// What `signal` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    Probe,
    Terminate,
    Kill,
}
