//! Files: [`File`], an open handle closed when dropped, and [`read`] /
//! [`write`] for whole files.

use crate::abi::{Handle, Whence, open};
use crate::{Error, Result, io, sys};
use alloc::vec::Vec;

/// An open file (any handle: a regular file, a device, a pipe), closed
/// when dropped.
#[derive(Debug)]
pub struct File {
    handle: Handle,
}

impl File {
    /// Open `path` for reading.
    pub fn open(path: &str) -> Result<File> {
        File::open_with(path, open::READ, 0)
    }

    /// Create `path` (mode 0644) or cut it to length 0, for writing.
    pub fn create(path: &str) -> Result<File> {
        File::open_with(path, open::WRITE | open::CREATE | open::TRUNCATE, 0o644)
    }

    /// Open `path` with `flags` (`abi::open`) and `mode` for a file it
    /// creates.
    pub fn open_with(path: &str, flags: u32, mode: u16) -> Result<File> {
        sys::open(path.as_bytes(), flags, mode).map(|handle| File { handle })
    }

    /// Take ownership of `handle`: it is closed when the file is dropped.
    pub fn from_handle(handle: Handle) -> File {
        File { handle }
    }

    pub fn handle(&self) -> Handle {
        self.handle
    }

    /// Give up ownership of the handle without closing it.
    pub fn into_handle(self) -> Handle {
        let h = self.handle;
        core::mem::forget(self);
        h
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        sys::read(self.handle, buf)
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        sys::write(self.handle, buf)
    }

    pub fn write_all(&self, buf: &[u8]) -> Result<()> {
        io::write_all(self.handle, buf)
    }

    /// Read until the end of the file, appending to `out`; returns the
    /// bytes read.
    pub fn read_to_end(&self, out: &mut Vec<u8>) -> Result<usize> {
        let start = out.len();
        let mut chunk = [0u8; 4096];
        loop {
            match self.read(&mut chunk) {
                Ok(0) => return Ok(out.len() - start),
                Ok(n) => out.extend_from_slice(&chunk[..n]),
                Err(Error::Interrupted) => {}
                Err(e) => return Err(e),
            }
        }
    }

    pub fn seek(&self, offset: i64, whence: Whence) -> Result<u64> {
        sys::seek(self.handle, offset, whence)
    }

    /// A second handle on the same open file (sharing its position).
    pub fn try_clone(&self) -> Result<File> {
        sys::dup(self.handle).map(File::from_handle)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        let _ = sys::close(self.handle);
    }
}

/// The whole file at `path`.
pub fn read(path: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    File::open(path)?.read_to_end(&mut out)?;
    Ok(out)
}

/// Make the file at `path` hold `data` (created with mode 0644 if needed).
pub fn write(path: &str, data: &[u8]) -> Result<()> {
    File::create(path)?.write_all(data)
}
