//! `poly-files`' file system trait on Poly's native calls (`poly-rt`).
//!
//! Two things the native interface does not say directly:
//!
//! - **Links**: there is no `lstat`; a directory listing's records carry
//!   each name's own kind (not followed), so [`PolyFs::lstat`] looks the
//!   name up in its parent's listing. Recursive deletion relies on that
//!   (it must not follow a link to a folder).
//! - **Moving between disks**: Poly's native errors have no `EXDEV`; the
//!   kernel's other errors map to `Io`. A `rename` that fails with `Io`
//!   between two paths that exist is taken as a move across file systems,
//!   so `poly-files` copies and deletes instead (the copy fails cleanly if
//!   the cause was something else).
//!
//! Modification times: the native `stat` has none yet, so `modified` is
//! always `None` and the Files window has no date column.

use alloc::string::String;
use alloc::vec::Vec;
use poly_files::{Fs, FsError, Kind, Meta, RawEntry};
use poly_rt::Error;
use poly_rt::abi::{kind, open};
use poly_rt::fs::{self, File, OpenOptions};
use poly_rt::io::Read;

/// The native file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct PolyFs;

/// A native error as `poly-files`' error.
pub fn error(e: Error) -> FsError {
    match e {
        Error::NotFound => FsError::NotFound,
        Error::PermissionDenied => FsError::PermissionDenied,
        Error::AlreadyExists => FsError::AlreadyExists,
        Error::NotADirectory => FsError::NotADirectory,
        Error::IsADirectory => FsError::IsADirectory,
        Error::NoSpace => FsError::NoSpace,
        Error::InvalidArgument => FsError::Invalid,
        Error::Unsupported => FsError::Unsupported,
        _ => FsError::Other,
    }
}

/// A native kind byte as `poly-files`' kind.
pub fn kind_of(k: u8) -> Kind {
    match k {
        kind::FILE => Kind::File,
        kind::DIRECTORY => Kind::Dir,
        kind::SYMLINK => Kind::Symlink,
        _ => Kind::Other,
    }
}

impl Fs for PolyFs {
    fn read_dir(&self, path: &str) -> Result<Vec<RawEntry>, FsError> {
        let entries = fs::read_dir(path).map_err(error)?;
        Ok(entries.into_iter().map(|e| RawEntry { name: e.name, kind: kind_of(e.kind) }).collect())
    }

    fn stat(&self, path: &str) -> Result<Meta, FsError> {
        let m = fs::metadata(path).map_err(error)?;
        Ok(Meta { kind: kind_of(m.0.kind), size: m.len(), mode: m.mode(), device: m.device(), modified: None })
    }

    fn lstat(&self, path: &str) -> Result<Meta, FsError> {
        let name = poly_files::path::file_name(path);
        let parent = poly_files::path::parent(path).unwrap_or("/");
        let listed = fs::read_dir(parent).ok().and_then(|es| es.into_iter().find(|e| e.name == name)).map(|e| e.kind);
        match listed.map(kind_of) {
            Some(Kind::Symlink) => {
                let target = self.stat(path).ok();
                Ok(Meta { kind: Kind::Symlink, size: 0, mode: target.map_or(0, |t| t.mode), device: 0, modified: None })
            }
            Some(_) => self.stat(path),
            None if name.is_empty() => self.stat(path),
            None => Err(FsError::NotFound),
        }
    }

    fn read_head(&self, path: &str, max: usize) -> Result<Vec<u8>, FsError> {
        // `max` may be `usize::MAX` (the whole file): read in pieces.
        let mut f = File::open(path).map_err(error)?;
        let mut out = Vec::new();
        let mut buf = alloc::vec![0u8; 64 * 1024];
        while out.len() < max {
            let want = buf.len().min(max - out.len());
            match f.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(k) => out.extend_from_slice(&buf[..k]),
                Err(Error::Interrupted) => {}
                Err(e) => return Err(error(e)),
            }
        }
        Ok(out)
    }

    fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        fs::write(path, data).map_err(error)
    }

    fn create_new(&self, path: &str, data: &[u8]) -> Result<(), FsError> {
        let f = OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(error)?;
        f.write_all(data).map_err(error)
    }

    fn create_dir(&self, path: &str) -> Result<(), FsError> {
        fs::create_dir(path).map_err(error)
    }

    fn rename(&self, from: &str, to: &str) -> Result<(), FsError> {
        match fs::rename(from, to) {
            Ok(()) => Ok(()),
            // See the module documentation.
            Err(Error::Io) if fs::exists(from) && fs::exists(poly_files::path::parent(to).unwrap_or("/")) => {
                Err(FsError::CrossDevice)
            }
            Err(e) => Err(error(e)),
        }
    }

    fn remove_file(&self, path: &str) -> Result<(), FsError> {
        fs::remove_file(path).map_err(error)
    }

    fn remove_dir(&self, path: &str) -> Result<(), FsError> {
        fs::remove_dir(path).map_err(|e| match e {
            Error::Io | Error::Busy => FsError::NotEmpty,
            e => error(e),
        })
    }

    fn set_mode(&self, path: &str, mode: u16) -> Result<(), FsError> {
        fs::set_mode(path, mode).map_err(error)
    }

    /// Copied in 256 KiB pieces (big files do not have to fit in memory),
    /// with the permission bits.
    fn copy_file(&self, from: &str, to: &str) -> Result<(), FsError> {
        let mut src = File::open_with(from, open::READ, 0).map_err(error)?;
        let mode = src.metadata().map(|m| m.mode()).unwrap_or(0o644);
        let dst = OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(to).map_err(error)?;
        let mut buf = alloc::vec![0u8; 256 * 1024];
        loop {
            match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => dst.write_all(&buf[..n]).map_err(error)?,
                Err(Error::Interrupted) => {}
                Err(e) => return Err(error(e)),
            }
        }
        let _ = fs::set_mode(to, mode);
        Ok(())
    }
}

/// The text of `path`, if it can be read.
pub fn read_text(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}
