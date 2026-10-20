//! Unpacking an archive into a file system, the way the kernel unpacks an
//! initramfs into its root.
//!
//! Everything goes through the [`Vfs`] as root, so it works on any
//! [`FileSystem`] and gets path resolution, symbolic links in paths and
//! permission rules for free:
//!
//! - Names lose leading `/` and `./`; the entry `.` (as `find . | cpio`
//!   writes it) sets the target directory's own owner and mode. A name with
//!   a `..` component is refused (`EINVAL`): an archive only writes below
//!   its target.
//! - Missing parent directories are made (mode `0755`), so archives that
//!   list only files work.
//! - An existing name is replaced: removed first unless both are
//!   directories, in which case only the owner, mode and time change.
//! - Owner, group, mode and modification time come from the header, exactly
//!   (no umask). Directory times are set at the end, since adding entries to
//!   a directory changes its time.
//! - Hard links (same device and inode, `nlink > 1`) become links to the
//!   first name seen, whichever entry carries the data.
//! - Symbolic links in paths are followed; an absolute one resolves against
//!   the context's root, so unpack into the root of a VFS (as an initramfs
//!   is) for absolute links inside the archive to point inside it.

use crate::{Entry, Error, entries};
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{At, Cred, Errno, FileSystem, FileType, FsContext, OpenFlags, SetTime, Timespec, Vfs};

/// What an unpack made.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnpackStats {
    /// Directories made, or already there and updated.
    pub dirs: usize,
    pub files: usize,
    pub symlinks: usize,
    /// Device nodes, FIFOs and sockets.
    pub nodes: usize,
    /// Names made as hard links to an earlier entry.
    pub hard_links: usize,
    /// File data written.
    pub bytes: u64,
}

/// Why an unpack stopped. What was unpacked before stays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnpackError {
    /// The archive is malformed.
    Archive(Error),
    /// The file system refused an entry, or the entry has an unknown file
    /// type or a `..` in its name (`EINVAL`).
    Entry { name: Vec<u8>, errno: Errno },
}

/// Unpacks `data` into the root of `fs`, through a VFS of its own. For a
/// file system that is already mounted, use [`unpack`] on the VFS it is
/// mounted in instead.
pub fn unpack_fs(data: &[u8], fs: Arc<dyn FileSystem>) -> Result<UnpackStats, UnpackError> {
    let vfs = Vfs::new(fs);
    let ctx = vfs.root_context().map_err(|errno| UnpackError::Entry { name: b"/".to_vec(), errno })?;
    unpack(data, &vfs, &ctx, At::Cwd)
}

/// Unpacks `data` below `at` (relative to `ctx`'s working directory for
/// [`At::Cwd`]), as root.
pub fn unpack(data: &[u8], vfs: &Vfs, ctx: &FsContext, at: At<'_>) -> Result<UnpackStats, UnpackError> {
    let mut ctx = ctx.clone();
    ctx.set_umask(0);
    let mut u = Unpacker {
        vfs,
        ctx: &ctx,
        cred: Cred::root(),
        at,
        stats: UnpackStats::default(),
        links: BTreeMap::new(),
        dir_times: Vec::new(),
    };
    for entry in entries(data) {
        let entry = entry.map_err(UnpackError::Archive)?;
        u.entry(&entry).map_err(|errno| UnpackError::Entry { name: entry.name.to_vec(), errno })?;
    }
    u.finish().map_err(|(name, errno)| UnpackError::Entry { name, errno })?;
    Ok(u.stats)
}

struct Unpacker<'a> {
    vfs: &'a Vfs,
    ctx: &'a FsContext,
    cred: Cred,
    at: At<'a>,
    stats: UnpackStats,
    /// (device major, device minor, inode) → the first name it got.
    links: BTreeMap<(u32, u32, u32), Vec<u8>>,
    /// Directories and the modification times to give them at the end.
    dir_times: Vec<(Vec<u8>, u32)>,
}

/// The path an archive name stands for, relative to the target; `None` for
/// the target itself.
fn clean_name(name: &[u8]) -> Result<Option<Vec<u8>>, Errno> {
    let mut path = Vec::with_capacity(name.len());
    for comp in name.split(|&c| c == b'/').filter(|c| !c.is_empty() && *c != b".") {
        if comp == b".." {
            return Err(Errno::EINVAL);
        }
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(comp);
    }
    Ok(if path.is_empty() { None } else { Some(path) })
}

impl Unpacker<'_> {
    fn entry(&mut self, e: &Entry<'_>) -> Result<(), Errno> {
        let kind = e.file_type().ok_or(Errno::EINVAL)?;
        let Some(path) = clean_name(e.name)? else {
            if kind != FileType::Directory {
                return Err(Errno::EINVAL);
            }
            self.set_meta(b".", e, kind)?;
            self.dir_times.push((b".".to_vec(), e.header.mtime));
            return Ok(());
        };
        self.make_parents(&path)?;
        let (vfs, ctx, cred, at) = (self.vfs, self.ctx, &self.cred, self.at);
        match vfs.stat(ctx, cred, at, &path, false) {
            Ok(st) => {
                let is_dir = st.kind() == Some(FileType::Directory);
                if is_dir && kind == FileType::Directory {
                    self.stats.dirs += 1;
                    self.set_meta(&path, e, kind)?;
                    self.dir_times.push((path, e.header.mtime));
                    return Ok(());
                }
                if is_dir {
                    vfs.rmdir(ctx, cred, at, &path)?;
                } else {
                    vfs.unlink(ctx, cred, at, &path)?;
                }
            }
            Err(Errno::ENOENT) => {}
            Err(err) => return Err(err),
        }
        match kind {
            FileType::Directory => {
                vfs.mkdir(ctx, cred, at, &path, e.perm())?;
                self.stats.dirs += 1;
                self.dir_times.push((path.clone(), e.header.mtime));
            }
            FileType::Regular => {
                let key = (e.header.dev_major, e.header.dev_minor, e.header.ino);
                let first = if e.header.nlink > 1 { self.links.get(&key).cloned() } else { None };
                if let Some(first) = first {
                    vfs.link(ctx, cred, at, &first, at, &path, false)?;
                    self.stats.hard_links += 1;
                    if !e.data.is_empty() {
                        self.write(&path, e.data, OpenFlags::WRONLY | OpenFlags::TRUNC, e.perm())?;
                    }
                } else {
                    self.write(&path, e.data, OpenFlags::WRONLY | OpenFlags::CREAT | OpenFlags::EXCL, e.perm())?;
                    self.stats.files += 1;
                    if e.header.nlink > 1 {
                        self.links.insert(key, path.clone());
                    }
                }
                self.stats.bytes += e.data.len() as u64;
            }
            FileType::Symlink => {
                vfs.symlink(ctx, cred, e.data, at, &path)?;
                self.stats.symlinks += 1;
            }
            FileType::CharDevice | FileType::BlockDevice | FileType::Fifo | FileType::Socket => {
                vfs.mknod(ctx, cred, at, &path, kind, e.perm(), e.rdev())?;
                self.stats.nodes += 1;
            }
        }
        self.set_meta(&path, e, kind)
    }

    fn make_parents(&self, path: &[u8]) -> Result<(), Errno> {
        for (i, _) in path.iter().enumerate().filter(|&(_, &c)| c == b'/') {
            match self.vfs.mkdir(self.ctx, &self.cred, self.at, &path[..i], 0o755) {
                Ok(()) | Err(Errno::EEXIST) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn write(&self, path: &[u8], data: &[u8], flags: OpenFlags, perm: u16) -> Result<(), Errno> {
        let file = self.vfs.open(self.ctx, &self.cred, self.at, path, flags, perm)?;
        let mut done = 0;
        while done < data.len() {
            match file.write(&data[done..])? {
                0 => return Err(Errno::EIO),
                n => done += n,
            }
        }
        Ok(())
    }

    /// Owner, mode and (except for directories, see `finish`) time.
    fn set_meta(&self, path: &[u8], e: &Entry<'_>, kind: FileType) -> Result<(), Errno> {
        let node = self.vfs.lookup(self.ctx, &self.cred, self.at, path, false)?;
        // chown before chmod: chown clears set-user-ID and set-group-ID.
        node.chown(&self.cred, Some(e.header.uid), Some(e.header.gid))?;
        if kind != FileType::Symlink {
            node.chmod(&self.cred, e.perm())?;
        }
        if kind != FileType::Directory {
            let t = Some(SetTime::At(Timespec::new(e.header.mtime.into(), 0)));
            node.set_times(&self.cred, t, t)?;
        }
        Ok(())
    }

    fn finish(&mut self) -> Result<(), (Vec<u8>, Errno)> {
        // Deepest last-made first does not matter: setting a directory's
        // time does not change its parent's.
        for (path, mtime) in &self.dir_times {
            let t = Some(SetTime::At(Timespec::new((*mtime).into(), 0)));
            match self.vfs.lookup(self.ctx, &self.cred, self.at, path, false) {
                Ok(node) => node.set_times(&self.cred, t, t).map_err(|e| (path.clone(), e))?,
                // Replaced or removed by a later entry.
                Err(Errno::ENOENT | Errno::ENOTDIR) => {}
                Err(e) => return Err((path.clone(), e)),
            }
        }
        Ok(())
    }
}
