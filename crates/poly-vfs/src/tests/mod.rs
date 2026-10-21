//! Host tests for the VFS and tmpfs. Paths and expected errors follow the
//! POSIX and Linux manual pages for each call.

extern crate std;

mod caps;
mod devices;
mod dirorder;
mod fdtable;
mod files;
mod locks;
mod mounts;
mod names;
mod objects;
mod path;
mod path_order;
mod perm;
mod stat;
mod tmpfs;
mod xattrs;

use crate::*;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, Ordering};

/// A clock that moves one second per reading, so every change gets a
/// distinct timestamp.
#[derive(Debug, Default)]
pub(crate) struct TestClock(AtomicI64);

impl Clock for TestClock {
    fn now(&self) -> Timespec {
        Timespec::new(self.0.fetch_add(1, Ordering::Relaxed) + 1_000_000, 0)
    }
}

pub(crate) fn tmpfs() -> Arc<Tmpfs> {
    Arc::new(Tmpfs::new(Arc::new(TestClock::default())))
}

/// A VFS on a fresh tmpfs, a root context and root credentials.
pub(crate) struct Env {
    pub(crate) fs: Arc<Tmpfs>,
    pub(crate) vfs: Vfs,
    pub(crate) ctx: FsContext,
    pub(crate) root: Cred,
}

impl Env {
    pub(crate) fn new() -> Env {
        let fs = tmpfs();
        let vfs = Vfs::new(fs.clone());
        let ctx = vfs.root_context().unwrap();
        Env { fs, vfs, ctx, root: Cred::root() }
    }

    pub(crate) fn mkdir(&self, path: &str) {
        self.vfs.mkdir(&self.ctx, &self.root, At::Cwd, path.as_bytes(), 0o755).unwrap();
    }

    pub(crate) fn write(&self, path: &str, data: &[u8]) {
        let f = self.open(path, OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::TRUNC).unwrap();
        assert_eq!(f.write(data), Ok(data.len()));
    }

    pub(crate) fn read(&self, path: &str) -> Result<Vec<u8>> {
        let f = self.open(path, OpenFlags::RDONLY)?;
        read_all(&f)
    }

    pub(crate) fn symlink(&self, target: &str, path: &str) {
        self.vfs.symlink(&self.ctx, &self.root, target.as_bytes(), At::Cwd, path.as_bytes()).unwrap();
    }

    pub(crate) fn open(&self, path: &str, flags: OpenFlags) -> Result<Arc<OpenFile>> {
        self.vfs.open(&self.ctx, &self.root, At::Cwd, path.as_bytes(), flags, 0o644)
    }

    pub(crate) fn stat(&self, path: &str) -> Result<Stat> {
        self.vfs.stat(&self.ctx, &self.root, At::Cwd, path.as_bytes(), true)
    }

    pub(crate) fn lstat(&self, path: &str) -> Result<Stat> {
        self.vfs.stat(&self.ctx, &self.root, At::Cwd, path.as_bytes(), false)
    }

    /// The inode number `path` resolves to (following links).
    pub(crate) fn ino(&self, path: &str) -> Result<Ino> {
        self.stat(path).map(|s| s.ino)
    }

    pub(crate) fn cd(&mut self, path: &str) -> Result<()> {
        self.vfs.chdir(&mut self.ctx, &self.root, At::Cwd, path.as_bytes())
    }

    pub(crate) fn cwd(&self) -> Vec<u8> {
        self.vfs.getcwd(&self.ctx).unwrap()
    }

    /// Names in a directory, without `.` and `..`, sorted.
    pub(crate) fn ls(&self, path: &str) -> Vec<std::string::String> {
        let f = self.open(path, OpenFlags::RDONLY | OpenFlags::DIRECTORY).unwrap();
        let mut names = Vec::new();
        f.read_dir(&mut |e| {
            if e.name != b"." && e.name != b".." {
                names.push(std::string::String::from_utf8(e.name.to_vec()).unwrap());
            }
            true
        })
        .unwrap();
        names.sort();
        names
    }
}

pub(crate) fn read_all(f: &OpenFile) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 1000];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(out);
        }
        out.extend_from_slice(&buf[..n]);
    }
}
