//! The character devices that stand for terminals: a terminal's own node
//! (`/dev/ttyS0`, `/dev/tty1`), `/dev/console` and `/dev/tty`. Each open
//! goes to a terminal ([`Tty::open`]); the node's device only decides
//! which one, and whether the open may make it a controlling terminal.

use crate::tty::{Tty, Ttys};
use alloc::sync::Arc;
use poly_vfs::lock::{Lock, RawSpinLock};
use poly_vfs::{CharDevice, DeviceId, Errno, OpenFlags, Result};

/// `/dev/tty` (tty(4): major 5, minor 0).
pub const TTY: DeviceId = DeviceId::new(5, 0);
/// `/dev/console` (console(4): major 5, minor 1).
pub const CONSOLE: DeviceId = DeviceId::new(5, 1);

/// The node of terminal `n` of a virtual console (console(4): `/dev/tty1`
/// and on are major 4, minors 1 to 63).
pub const fn virtual_console(n: u32) -> DeviceId {
    DeviceId::new(4, n)
}

/// The node of serial port `n` (ttyS(4): `/dev/ttyS0` and on are major 4,
/// minor 64 and on).
pub const fn serial(n: u32) -> DeviceId {
    DeviceId::new(4, 64 + n)
}

/// A terminal's own node: an open may make it the caller's controlling
/// terminal.
pub struct TtyDevice {
    tty: Arc<Tty>,
}

impl TtyDevice {
    pub fn new(tty: Arc<Tty>) -> TtyDevice {
        TtyDevice { tty }
    }
}

impl core::fmt::Debug for TtyDevice {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TtyDevice").field("tty", &self.tty.name()).finish()
    }
}

impl CharDevice for TtyDevice {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        Ok(Some(self.tty.open(flags, true)?))
    }
}

/// `/dev/console`: the terminal the kernel chose as its console. Opening
/// it never makes that terminal a controlling terminal (POSIX leaves it
/// to the implementation; a shell started on the console gets job
/// control by taking the terminal with `TIOCSCTTY`). Without a console
/// terminal, opens fail with `ENXIO`.
#[derive(Default)]
pub struct Console {
    tty: Lock<RawSpinLock, Option<Arc<Tty>>>,
}

impl Console {
    pub fn new(tty: Option<Arc<Tty>>) -> Console {
        Console { tty: Lock::new(tty) }
    }

    /// The terminal behind the console.
    pub fn tty(&self) -> Option<Arc<Tty>> {
        self.tty.lock().clone()
    }

    pub fn set_tty(&self, tty: Option<Arc<Tty>>) {
        *self.tty.lock() = tty;
    }
}

impl core::fmt::Debug for Console {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let name = self.tty().map(|t| alloc::string::String::from(t.name()));
        f.debug_struct("Console").field("tty", &name).finish()
    }
}

impl CharDevice for Console {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        let tty = self.tty().ok_or(Errno::ENXIO)?;
        Ok(Some(tty.open(flags, false)?))
    }
}

/// `/dev/tty`: the caller's controlling terminal (tty(4)); `ENXIO` without
/// one (to be confirmed by the diff runner).
pub struct ControllingTty {
    ttys: Arc<Ttys>,
}

impl ControllingTty {
    pub fn new(ttys: Arc<Ttys>) -> ControllingTty {
        ControllingTty { ttys }
    }
}

impl core::fmt::Debug for ControllingTty {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ControllingTty").finish_non_exhaustive()
    }
}

impl CharDevice for ControllingTty {
    fn read(&self, _: u64, _: &mut [u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn write(&self, _: u64, _: &[u8], _: bool) -> Result<usize> {
        Err(Errno::ENXIO)
    }

    fn open(&self, flags: OpenFlags) -> Result<Option<Arc<dyn CharDevice>>> {
        let tty = self.ttys.controlling().ok_or(Errno::ENXIO)?;
        Ok(Some(tty.open(flags, false)?))
    }
}
