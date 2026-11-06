//! The file system the browser works over.
//!
//! The core never calls an OS itself: the Files app passes an [`Fs`] backed
//! by Poly's native system calls, the file picker (roadmap Phase 8) one
//! backed by whatever world asked for it, and the tests a temp directory or
//! an in-memory fake. Paths are absolute, `/`-separated UTF-8 strings.
//!
//! The trait is small on purpose: listing ([`Fs::read_dir`]), metadata with
//! and without following symbolic links ([`Fs::stat`], [`Fs::lstat`]),
//! reading the first bytes of a file for content sniffing
//! ([`Fs::read_head`]), and the handful of mutations the file operations and
//! the trash need. [`Fs::copy_file`], [`Fs::read_to_string`] and
//! [`Fs::exists`] have default implementations in terms of the others.
//!
//! Poly's native `stat` has no modification time yet, so [`Meta::modified`]
//! is optional and sorting by date puts unknown times first.

use alloc::string::String;
use alloc::vec::Vec;

/// What a directory entry is. Directory listings report the entry itself
/// (a symbolic link is [`Kind::Symlink`]); [`Fs::stat`] reports what it
/// points to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    #[default]
    File,
    Dir,
    Symlink,
    /// Devices, FIFOs, sockets.
    Other,
}

/// One name in a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawEntry {
    pub name: String,
    pub kind: Kind,
}

/// A file's metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Meta {
    pub kind: Kind,
    /// Size in bytes (meaningless for directories).
    pub size: u64,
    /// Permission bits (`0o7777`), e.g. `0o755`.
    pub mode: u16,
    /// The device the file lives on (0 if unknown).
    pub device: u64,
    /// Last modification, Unix seconds, if the system reports it.
    pub modified: Option<u64>,
}

impl Meta {
    /// Whether any execute bit is set.
    pub fn executable(&self) -> bool {
        self.mode & 0o111 != 0
    }
}

/// Why a file system call failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FsError {
    NotFound,
    PermissionDenied,
    AlreadyExists,
    NotADirectory,
    IsADirectory,
    /// A directory to remove is not empty.
    NotEmpty,
    /// A rename across file systems (`EXDEV`).
    CrossDevice,
    NoSpace,
    /// The request makes no sense (trashing the trash, a name with `/`).
    Invalid,
    /// The file system cannot do this (e.g. no permission bits).
    Unsupported,
    Other,
}

impl FsError {
    /// A short sentence for the UI.
    pub fn message(self) -> &'static str {
        match self {
            FsError::NotFound => "No such file or folder",
            FsError::PermissionDenied => "Permission denied",
            FsError::AlreadyExists => "A file or folder with that name already exists",
            FsError::NotADirectory => "Not a folder",
            FsError::IsADirectory => "Is a folder",
            FsError::NotEmpty => "The folder is not empty",
            FsError::CrossDevice => "Cannot move between disks",
            FsError::NoSpace => "The disk is full",
            FsError::Invalid => "Not possible",
            FsError::Unsupported => "Not supported here",
            FsError::Other => "Something went wrong",
        }
    }
}

impl core::fmt::Display for FsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.message())
    }
}

/// A file system. All paths are absolute and normalized
/// ([`crate::path::normalize`]).
pub trait Fs {
    /// The names in directory `path` (without `.` and `..`), in any order,
    /// each with its own kind (symbolic links not followed).
    fn read_dir(&self, path: &str) -> Result<Vec<RawEntry>, FsError>;

    /// Metadata of `path`, following symbolic links.
    fn stat(&self, path: &str) -> Result<Meta, FsError>;

    /// Metadata of `path` itself: a symbolic link is [`Kind::Symlink`].
    /// Recursive deletion relies on this not following links; a system that
    /// cannot tell links apart returns what [`Fs::stat`] returns.
    fn lstat(&self, path: &str) -> Result<Meta, FsError>;

    /// Up to `max` bytes from the start of file `path`.
    fn read_head(&self, path: &str, max: usize) -> Result<Vec<u8>, FsError>;

    /// Creates or truncates file `path` and writes `data`.
    fn write(&self, path: &str, data: &[u8]) -> Result<(), FsError>;

    /// Creates file `path` with `data`, failing with
    /// [`FsError::AlreadyExists`] if anything has that name (`O_EXCL`).
    fn create_new(&self, path: &str, data: &[u8]) -> Result<(), FsError>;

    /// Creates directory `path` (its parent must exist).
    fn create_dir(&self, path: &str) -> Result<(), FsError>;

    /// Moves `from` to `to` on the same file system; may replace an
    /// existing file at `to` (callers check first).
    fn rename(&self, from: &str, to: &str) -> Result<(), FsError>;

    /// Removes a file or a symbolic link (not its target).
    fn remove_file(&self, path: &str) -> Result<(), FsError>;

    /// Removes an empty directory.
    fn remove_dir(&self, path: &str) -> Result<(), FsError>;

    /// Sets the permission bits. The default says the file system has none.
    fn set_mode(&self, path: &str, mode: u16) -> Result<(), FsError> {
        let _ = (path, mode);
        Err(FsError::Unsupported)
    }

    /// The whole file as text (invalid UTF-8 replaced).
    fn read_to_string(&self, path: &str) -> Result<String, FsError> {
        let bytes = self.read_head(path, usize::MAX)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Copies file `from` (links followed) to `to`, creating or truncating
    /// it, and carries the permission bits over where the file system has
    /// them. The default reads the whole file into memory.
    fn copy_file(&self, from: &str, to: &str) -> Result<(), FsError> {
        let meta = self.stat(from)?;
        if meta.kind == Kind::Dir {
            return Err(FsError::IsADirectory);
        }
        let data = self.read_head(from, usize::MAX)?;
        self.write(to, &data)?;
        match self.set_mode(to, meta.mode) {
            Ok(()) | Err(FsError::Unsupported) => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Whether anything (even a broken link) has the name `path`.
    fn exists(&self, path: &str) -> bool {
        self.lstat(path).is_ok()
    }
}

/// Creates directory `path` and any missing parents.
pub fn create_dir_all(fs: &dyn Fs, path: &str) -> Result<(), FsError> {
    let path = crate::path::normalize(path);
    let mut cur = String::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        cur.push('/');
        cur.push_str(seg);
        match fs.stat(&cur) {
            Ok(m) if m.kind == Kind::Dir => continue,
            Ok(_) => return Err(FsError::NotADirectory),
            Err(FsError::NotFound) => match fs.create_dir(&cur) {
                Ok(()) | Err(FsError::AlreadyExists) => {}
                Err(e) => return Err(e),
            },
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfs::MemFs;

    #[test]
    fn create_dir_all_makes_parents_and_tolerates_existing() {
        let fs = MemFs::new();
        create_dir_all(&fs, "/a/b/c").unwrap();
        assert_eq!(fs.stat("/a/b/c").unwrap().kind, Kind::Dir);
        create_dir_all(&fs, "/a/b").unwrap();
        fs.write("/a/f", b"x").unwrap();
        assert_eq!(create_dir_all(&fs, "/a/f/g"), Err(FsError::NotADirectory));
    }

    #[test]
    fn default_copy_and_read() {
        let fs = MemFs::new();
        fs.write("/x", b"hello").unwrap();
        fs.set_mode("/x", 0o755).unwrap();
        fs.copy_file("/x", "/y").unwrap();
        assert_eq!(fs.read_to_string("/y").unwrap(), "hello");
        assert_eq!(fs.stat("/y").unwrap().mode, 0o755);
        assert!(fs.exists("/y"));
        assert!(!fs.exists("/z"));
        assert_eq!(fs.copy_file("/", "/q"), Err(FsError::IsADirectory));
        assert_eq!(FsError::NotEmpty.message(), "The folder is not empty");
    }
}
