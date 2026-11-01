//! Character devices: the hook through which the kernel provides
//! `/dev/console`, `/dev/null` and friends.
//!
//! A device is registered with [`Vfs::register_char_device`](crate::Vfs::register_char_device)
//! under a [`DeviceId`]. Opening a character device node (made with
//! `mknod`, or by the cpio unpacker) with that number gives an
//! [`OpenFile`](crate::OpenFile) whose reads and writes go to the device;
//! without a registered device, `open` fails with `ENXIO`.
//!
//! The numbers below are the ones the Linux manual pages give for these
//! devices (`null(4)`, `zero(4)` and `full(4)`: major 1; `tty(4)` and
//! `console(4)`: major 5), so device nodes in a Linux initramfs work as is.

use crate::poll::{PollEvents, WaitQueue};
use crate::{DeviceId, Errno, OpenFlags, Result};
use alloc::sync::Arc;

/// A character device.
///
/// Reads and writes get the offset of the open file (always 0 unless
/// [`CharDevice::seekable`]) and whether the file is non-blocking. A device
/// that would block and is called non-blocking returns `EAGAIN`. Devices
/// are called without VFS locks held, so a blocking read may sleep.
pub trait CharDevice: Send + Sync {
    fn read(&self, offset: u64, buf: &mut [u8], nonblock: bool) -> Result<usize>;

    fn write(&self, offset: u64, buf: &[u8], nonblock: bool) -> Result<usize>;

    /// Whether the open file keeps an offset (`lseek` works); otherwise
    /// `lseek` fails with `ESPIPE`.
    fn seekable(&self) -> bool {
        false
    }

    /// Device-specific control; `ENOTTY` when the device has no such
    /// request (which is how `isatty` tells a terminal from anything else).
    fn ioctl(&self, _request: u32, _arg: u64) -> Result<u64> {
        Err(Errno::ENOTTY)
    }

    /// What the device is ready for now (`poll(2)`); always ready to read
    /// and write by default, like the memory devices.
    fn poll(&self) -> PollEvents {
        PollEvents::READABLE | PollEvents::WRITABLE
    }

    /// Where waiters for a change of [`poll`](Self::poll) register (a
    /// terminal with input coming, say); `None` for a device whose
    /// readiness never changes.
    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        None
    }

    /// Called when a node with this device's number is opened with
    /// `flags`, before the open file exists. A device that opens as
    /// another one returns it, and the open file reads, writes and polls
    /// that one instead: a name for whatever device stands behind it
    /// (`/dev/tty`, the caller's terminal), a multiplexer that makes a new
    /// device for every open (`/dev/ptmx`), or a per-open handle that
    /// counts opens and closes (dropped with the open file). An error
    /// fails the `open`. The default opens the device itself.
    fn open(&self, _flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        Ok(None)
    }

    /// For a pseudo-terminal master: the number of its slave
    /// (`/dev/pts/<n>`), which `TIOCGPTPEER` opens. `None` for every
    /// other device.
    fn pty_index(&self) -> Option<u32> {
        None
    }

    /// The device as `Any`, for the kernel to find its own device type
    /// behind an open file where it offers a service the VFS knows nothing
    /// of (mapping a DRM device file, or a sound card's ring buffer).
    /// `None` (the default) for devices nobody looks into.
    fn as_any(&self) -> Option<&(dyn core::any::Any + Send + Sync)> {
        None
    }
}

/// `/dev/null` (1, 3).
pub const NULL: DeviceId = DeviceId::new(1, 3);
/// `/dev/zero` (1, 5).
pub const ZERO: DeviceId = DeviceId::new(1, 5);
/// `/dev/full` (1, 7).
pub const FULL: DeviceId = DeviceId::new(1, 7);
/// `/dev/random` (1, 8).
pub const RANDOM: DeviceId = DeviceId::new(1, 8);
/// `/dev/urandom` (1, 9).
pub const URANDOM: DeviceId = DeviceId::new(1, 9);
/// `/dev/tty`, the controlling terminal (5, 0).
pub const TTY: DeviceId = DeviceId::new(5, 0);
/// `/dev/console` (5, 1).
pub const CONSOLE: DeviceId = DeviceId::new(5, 1);

/// Reads end of file; writes are discarded and succeed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Null;

impl CharDevice for Null {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Ok(0)
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        Ok(buf.len())
    }

    fn seekable(&self) -> bool {
        true
    }
}

/// Reads zeros; writes are discarded and succeed.
#[derive(Clone, Copy, Debug, Default)]
pub struct Zero;

impl CharDevice for Zero {
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> Result<usize> {
        buf.fill(0);
        Ok(buf.len())
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        Ok(buf.len())
    }

    fn seekable(&self) -> bool {
        true
    }
}

/// Reads zeros; writes fail with `ENOSPC`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Full;

impl CharDevice for Full {
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> Result<usize> {
        buf.fill(0);
        Ok(buf.len())
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        if buf.is_empty() { Ok(0) } else { Err(Errno::ENOSPC) }
    }

    fn seekable(&self) -> bool {
        true
    }
}
