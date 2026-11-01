//! The files memfd_create(2) makes: unnamed regular files on a memory file
//! system that no path reaches, labelled by a name, with file seals
//! (fcntl(2), "File sealing"; ADR 0111).
//!
//! A [`Memfd`] travels with the open file description
//! ([`OpenFile::memfd`](crate::OpenFile::memfd)), which every descriptor
//! (`dup`, `fork`) and every mapping of the file shares, so the seals are
//! those of the file. The rules, from fcntl(2):
//!
//! - `F_ADD_SEALS` adds bits; once `F_SEAL_SEAL` is set nothing more can
//!   be added (`EPERM`); unknown bits are `EINVAL`; `F_SEAL_WRITE` while a
//!   writable shared mapping exists is `EBUSY` (seals unchanged).
//! - `F_SEAL_SHRINK`: no `ftruncate` to a smaller size; `F_SEAL_GROW`: no
//!   `ftruncate` to a larger size and no write past the end;
//!   `F_SEAL_WRITE` and `F_SEAL_FUTURE_WRITE`: no writes and no new
//!   writable shared mapping (all `EPERM`).
//! - `F_SEAL_EXEC`: the execute bits of the mode stay as they are
//!   (`EPERM` for a `chmod` changing any of them).
//!
//! Writable shared mappings are counted by [`WritableMap`] guards: the
//! kernel keeps one with the page source of each such mapping, which all
//! regions split from it (and a fork child's copies) share, so the count
//! drops when the last region of the mapping is gone.

use crate::sync::SpinMutex;
use crate::{Errno, Result};
use alloc::boxed::Box;
use alloc::sync::Arc;

/// Seal bits (`linux/fcntl.h`).
pub mod seal {
    pub const SEAL: u32 = 0x1;
    pub const SHRINK: u32 = 0x2;
    pub const GROW: u32 = 0x4;
    pub const WRITE: u32 = 0x8;
    pub const FUTURE_WRITE: u32 = 0x10;
    pub const EXEC: u32 = 0x20;
    /// Every seal this kernel knows.
    pub const ALL: u32 = SEAL | SHRINK | GROW | WRITE | FUTURE_WRITE | EXEC;
}

/// The execute bits of a mode.
const EXEC_BITS: u16 = 0o111;

#[derive(Debug)]
struct State {
    seals: u32,
    /// Live writable shared mappings ([`WritableMap`]s).
    writable_maps: usize,
}

/// A memfd's label and seals.
#[derive(Debug)]
pub struct Memfd {
    name: Box<[u8]>,
    state: SpinMutex<State>,
}

impl Memfd {
    /// A memfd called `name` starting with `seals`.
    pub fn new(name: &[u8], seals: u32) -> Arc<Memfd> {
        Arc::new(Memfd { name: name.into(), state: SpinMutex::new(State { seals, writable_maps: 0 }) })
    }

    /// The name given to memfd_create(2).
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// What `/proc/<pid>/fd/<n>` and `/proc/<pid>/maps` show:
    /// `/memfd:<name> (deleted)`.
    pub fn path(&self) -> alloc::vec::Vec<u8> {
        let mut p = alloc::vec::Vec::with_capacity(self.name.len() + 17);
        p.extend_from_slice(b"/memfd:");
        p.extend_from_slice(&self.name);
        p.extend_from_slice(b" (deleted)");
        p
    }

    /// `F_GET_SEALS`.
    pub fn seals(&self) -> u32 {
        self.state.lock().seals
    }

    /// `F_ADD_SEALS`.
    pub fn add_seals(&self, add: u32) -> Result<()> {
        if add & !seal::ALL != 0 {
            return Err(Errno::EINVAL);
        }
        let mut s = self.state.lock();
        if s.seals & seal::SEAL != 0 {
            return Err(Errno::EPERM);
        }
        if add & seal::WRITE != 0 && s.seals & seal::WRITE == 0 && s.writable_maps > 0 {
            return Err(Errno::EBUSY);
        }
        s.seals |= add;
        Ok(())
    }

    /// May `len` bytes be written at `offset` of a file of `size` bytes?
    pub fn check_write(&self, offset: u64, len: u64, size: u64) -> Result<()> {
        let seals = self.seals();
        if len == 0 {
            return Ok(());
        }
        if seals & (seal::WRITE | seal::FUTURE_WRITE) != 0 {
            return Err(Errno::EPERM);
        }
        if seals & seal::GROW != 0 && offset.saturating_add(len) > size {
            return Err(Errno::EPERM);
        }
        Ok(())
    }

    /// May the file's size change from `size` to `new`?
    pub fn check_truncate(&self, size: u64, new: u64) -> Result<()> {
        let seals = self.seals();
        if (new < size && seals & seal::SHRINK != 0) || (new > size && seals & seal::GROW != 0) {
            return Err(Errno::EPERM);
        }
        Ok(())
    }

    /// May the mode change from `old` to `new`?
    pub fn check_chmod(&self, old: u16, new: u16) -> Result<()> {
        if self.seals() & seal::EXEC != 0 && (old ^ new) & EXEC_BITS != 0 {
            return Err(Errno::EPERM);
        }
        Ok(())
    }

    /// A new mapping of the file: `EPERM` for a writable shared one while
    /// writes are sealed; otherwise, for a writable shared one, the guard
    /// that counts it until dropped.
    pub fn map(self: &Arc<Self>, shared: bool, writable: bool) -> Result<Option<WritableMap>> {
        if !(shared && writable) {
            return Ok(None);
        }
        let mut s = self.state.lock();
        if s.seals & (seal::WRITE | seal::FUTURE_WRITE) != 0 {
            return Err(Errno::EPERM);
        }
        s.writable_maps += 1;
        Ok(Some(WritableMap(Arc::clone(self))))
    }

    /// Live writable shared mappings.
    pub fn writable_maps(&self) -> usize {
        self.state.lock().writable_maps
    }
}

/// One writable shared mapping of a memfd, counted while it lives.
#[derive(Debug)]
pub struct WritableMap(Arc<Memfd>);

impl Drop for WritableMap {
    fn drop(&mut self) {
        self.0.state.lock().writable_maps -= 1;
    }
}
