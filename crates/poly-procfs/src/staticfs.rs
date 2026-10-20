//! A read-only file system of fixed directories, symbolic links and files
//! whose contents a function makes on every read: what the kernel's sysfs
//! is (sysfs(5)).

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{Attr, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, RenameFlags, Result};
use poly_vfs::{DeviceId, SetAttr, Timespec};

/// Makes a file's contents.
pub type Contents = Box<dyn Fn() -> Vec<u8> + Send + Sync>;

enum Kind {
    Dir(Vec<(Box<[u8]>, Ino)>),
    File(Contents),
    Link(Box<[u8]>),
}

struct Node {
    parent: Ino,
    kind: Kind,
    perm: u16,
}

/// A tree built once with [`StaticFsBuilder`] and never changed by anyone:
/// every change is `EROFS` (the file system reports itself read-only), and
/// the contents of a file are made afresh on each read.
pub struct StaticFs {
    name: &'static str,
    magic: u64,
    nodes: Vec<Node>,
    time: Timespec,
}

impl core::fmt::Debug for StaticFs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StaticFs").field("name", &self.name).field("nodes", &self.nodes.len()).finish()
    }
}

/// Builds a [`StaticFs`] from paths.
pub struct StaticFsBuilder {
    fs: StaticFs,
}

impl core::fmt::Debug for StaticFsBuilder {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.fs.fmt(f)
    }
}

/// The root's inode number. Inode `n` is entry `n - 1` of the node table.
const ROOT: Ino = 1;

impl StaticFsBuilder {
    /// An empty tree named `name` (its type in `/proc/mounts`), with
    /// `statfs` magic number `magic`, whose nodes all have timestamp `time`.
    pub fn new(name: &'static str, magic: u64, time: Timespec) -> StaticFsBuilder {
        let root = Node { parent: ROOT, kind: Kind::Dir(Vec::new()), perm: 0o755 };
        StaticFsBuilder { fs: StaticFs { name, magic, nodes: alloc::vec![root], time } }
    }

    /// The inode of directory `path` (`/`-separated, relative to the root),
    /// made with its missing parents.
    pub fn dir(&mut self, path: &str) -> Ino {
        let mut at = ROOT;
        for name in path.split('/').filter(|n| !n.is_empty()) {
            at = match self.child(at, name.as_bytes()) {
                Some(ino) => ino,
                None => self.add(at, name.as_bytes(), Kind::Dir(Vec::new()), 0o755),
            };
        }
        at
    }

    fn child(&self, dir: Ino, name: &[u8]) -> Option<Ino> {
        match &self.fs.nodes[(dir - 1) as usize].kind {
            Kind::Dir(entries) => entries.iter().find(|(n, _)| &**n == name).map(|(_, i)| *i),
            _ => None,
        }
    }

    fn add(&mut self, dir: Ino, name: &[u8], kind: Kind, perm: u16) -> Ino {
        let ino = self.fs.nodes.len() as Ino + 1;
        self.fs.nodes.push(Node { parent: dir, kind, perm });
        match &mut self.fs.nodes[(dir - 1) as usize].kind {
            Kind::Dir(entries) => entries.push((name.into(), ino)),
            _ => panic!("StaticFs: a parent that is not a directory"),
        }
        ino
    }

    fn split(path: &str) -> (&str, &str) {
        let path = path.trim_end_matches('/');
        match path.rfind('/') {
            Some(i) => (&path[..i], &path[i + 1..]),
            None => ("", path),
        }
    }

    /// A file at `path` (parents made as needed) whose contents `f` makes,
    /// readable by everyone.
    pub fn file(&mut self, path: &str, f: impl Fn() -> Vec<u8> + Send + Sync + 'static) -> &mut Self {
        let (dir, name) = Self::split(path);
        let dir = self.dir(dir);
        assert!(self.child(dir, name.as_bytes()).is_none(), "StaticFs: {path} made twice");
        self.add(dir, name.as_bytes(), Kind::File(Box::new(f)), 0o444);
        self
    }

    /// A file with fixed text.
    pub fn text(&mut self, path: &str, text: &str) -> &mut Self {
        let bytes: Vec<u8> = text.as_bytes().to_vec();
        self.file(path, move || bytes.clone())
    }

    /// A symbolic link at `path` to `target`.
    pub fn link(&mut self, path: &str, target: &str) -> &mut Self {
        let (dir, name) = Self::split(path);
        let dir = self.dir(dir);
        assert!(self.child(dir, name.as_bytes()).is_none(), "StaticFs: {path} made twice");
        self.add(dir, name.as_bytes(), Kind::Link(target.as_bytes().into()), 0o777);
        self
    }

    pub fn build(self) -> Arc<StaticFs> {
        Arc::new(self.fs)
    }
}

impl StaticFs {
    fn node(&self, ino: Ino) -> Result<&Node> {
        usize::try_from(ino).ok().and_then(|i| i.checked_sub(1)).and_then(|i| self.nodes.get(i)).ok_or(Errno::ENOENT)
    }
}

impl FileSystem for StaticFs {
    fn info(&self) -> FsInfo {
        FsInfo { name: self.name, read_only: true, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        let node = self.node(dir)?;
        let Kind::Dir(entries) = &node.kind else { return Err(Errno::ENOTDIR) };
        if name == b".." {
            return Ok(node.parent);
        }
        entries.iter().find(|(n, _)| &**n == name).map(|(_, i)| *i).ok_or(Errno::ENOENT)
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        let node = self.node(ino)?;
        let (kind, nlink, size) = match &node.kind {
            Kind::Dir(_) => (FileType::Directory, 2, 0),
            Kind::File(_) => (FileType::Regular, 1, 4096),
            Kind::Link(t) => (FileType::Symlink, 1, t.len() as u64),
        };
        Ok(Attr {
            ino,
            kind,
            perm: node.perm,
            nlink,
            uid: 0,
            gid: 0,
            size,
            rdev: DeviceId::default(),
            atime: self.time,
            mtime: self.time,
            ctime: self.time,
            blocks: 0,
            blksize: 4096,
        })
    }

    fn setattr(&self, _: Ino, _: &SetAttr) -> Result<Attr> {
        Err(Errno::EROFS)
    }

    fn create(&self, _: Ino, _: &[u8], _: &NewNode) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn mkdir(&self, _: Ino, _: &[u8], _: u16, _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn symlink(&self, _: Ino, _: &[u8], _: &[u8], _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EROFS)
    }

    fn link(&self, _: Ino, _: &[u8], _: Ino) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn unlink(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn rmdir(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn rename(&self, _: Ino, _: &[u8], _: Ino, _: &[u8], _: RenameFlags) -> Result<()> {
        Err(Errno::EROFS)
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        match &self.node(ino)?.kind {
            Kind::Link(t) => Ok(t.to_vec()),
            _ => Err(Errno::EINVAL),
        }
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        match &self.node(ino)?.kind {
            Kind::File(f) => Ok(crate::copy_at(&f(), offset, buf)),
            Kind::Dir(_) => Err(Errno::EISDIR),
            Kind::Link(_) => Err(Errno::EINVAL),
        }
    }

    fn write(&self, _: Ino, _: u64, _: &[u8]) -> Result<usize> {
        Err(Errno::EROFS)
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        let node = self.node(dir)?;
        let Kind::Dir(entries) = &node.kind else { return Err(Errno::ENOTDIR) };
        let dots = [(&b"."[..], dir), (&b".."[..], node.parent)];
        let all = dots.iter().map(|(n, i)| (*n, *i)).chain(entries.iter().map(|(n, i)| (&**n, *i)));
        for (at, (name, ino)) in all.enumerate().skip(usize::try_from(pos).unwrap_or(usize::MAX)) {
            let kind = self.getattr(ino)?.kind;
            if !emit(&DirEntry { ino, kind, name, next: at as u64 + 1 }) {
                break;
            }
        }
        Ok(())
    }

    fn statfs(&self) -> Result<FsStats> {
        Ok(FsStats { magic: self.magic, block_size: 4096, fragment_size: 4096, name_max: 255, ..FsStats::default() })
    }
}
