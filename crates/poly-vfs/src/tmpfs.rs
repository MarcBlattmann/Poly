//! tmpfs: a file system in memory.
//!
//! Regular files are sparse: data lives in 4 KiB pages allocated when first
//! written, so a hole (from seeking past the end, or growing a file with
//! `truncate`) costs nothing and reads as zeros. Directory entries carry a
//! sequence number in insertion order, which is what `readdir` positions
//! are made of, so a listing continues correctly while entries come and go.
//!
//! Everything sits behind one lock per file system. `readdir` copies a batch
//! of entries out and calls the VFS's callback with the lock released.

use crate::fs::{Clock, DirEntry, FileSystem, FsInfo, NewNode, RenameFlags};
use crate::sync::SpinMutex;
use crate::{Attr, DeviceId, Errno, FileType, Ino, MAX_FILE_SIZE, NAME_MAX, Result, SetAttr, SetTime, Timespec};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

/// Size of a data page.
const PAGE: u64 = 4096;
/// The root directory's inode number.
const ROOT: Ino = 1;
/// Entries copied out per round of `readdir`.
const READDIR_BATCH: usize = 64;

/// An in-memory file system.
pub struct Tmpfs {
    inner: SpinMutex<Inner>,
    clock: Arc<dyn Clock>,
    /// Data pages the file system may hold, if limited (`ENOSPC` beyond).
    page_limit: Option<u64>,
}

impl core::fmt::Debug for Tmpfs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let inner = self.inner.lock();
        f.debug_struct("Tmpfs").field("inodes", &inner.nodes.len()).field("pages", &inner.pages).finish()
    }
}

struct Inner {
    nodes: BTreeMap<Ino, Node>,
    next_ino: Ino,
    /// Data pages held by all files.
    pages: u64,
}

struct Node {
    kind: FileType,
    perm: u16,
    uid: u32,
    gid: u32,
    nlink: u32,
    rdev: DeviceId,
    atime: Timespec,
    mtime: Timespec,
    ctime: Timespec,
    /// Pins taken by `open`.
    pins: u32,
    data: Data,
}

enum Data {
    File(Pages),
    Dir(Dir),
    Symlink(Box<[u8]>),
    /// Device nodes, FIFOs and sockets: attributes only.
    None,
}

#[derive(Default)]
struct Pages {
    size: u64,
    pages: BTreeMap<u64, Box<[u8]>>,
}

struct Dir {
    parent: Ino,
    /// Name → (sequence number, inode).
    by_name: BTreeMap<Box<[u8]>, (u64, Ino)>,
    /// Sequence number → name: the listing order.
    by_seq: BTreeMap<u64, Box<[u8]>>,
    next_seq: u64,
    /// Removed with `rmdir`; lives on while pinned, empty and closed to
    /// new entries.
    removed: bool,
}

impl Dir {
    fn new(parent: Ino) -> Dir {
        Dir { parent, by_name: BTreeMap::new(), by_seq: BTreeMap::new(), next_seq: 0, removed: false }
    }

    fn get(&self, name: &[u8]) -> Option<Ino> {
        self.by_name.get(name).map(|&(_, ino)| ino)
    }

    fn insert(&mut self, name: &[u8], ino: Ino) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.by_name.insert(name.into(), (seq, ino));
        self.by_seq.insert(seq, name.into());
    }

    fn remove(&mut self, name: &[u8]) -> Option<Ino> {
        let (seq, ino) = self.by_name.remove(name)?;
        self.by_seq.remove(&seq);
        Some(ino)
    }
}

impl Tmpfs {
    /// An empty tmpfs whose root is owned by root with mode `0755`.
    pub fn new(clock: Arc<dyn Clock>) -> Tmpfs {
        Tmpfs::build(clock, None)
    }

    /// An empty tmpfs that holds at most `bytes` of file data (rounded down
    /// to whole pages); writes beyond fail with `ENOSPC`.
    pub fn with_limit(clock: Arc<dyn Clock>, bytes: u64) -> Tmpfs {
        Tmpfs::build(clock, Some(bytes / PAGE))
    }

    fn build(clock: Arc<dyn Clock>, page_limit: Option<u64>) -> Tmpfs {
        let now = clock.now();
        let root = Node {
            kind: FileType::Directory,
            perm: 0o755,
            uid: 0,
            gid: 0,
            nlink: 2,
            rdev: DeviceId::default(),
            atime: now,
            mtime: now,
            ctime: now,
            pins: 0,
            data: Data::Dir(Dir::new(ROOT)),
        };
        let mut nodes = BTreeMap::new();
        nodes.insert(ROOT, root);
        Tmpfs { inner: SpinMutex::new(Inner { nodes, next_ino: ROOT + 1, pages: 0 }), clock, page_limit }
    }

    /// Inodes alive, the root included.
    pub fn inode_count(&self) -> usize {
        self.inner.lock().nodes.len()
    }

    /// Bytes of data pages held.
    pub fn bytes_used(&self) -> u64 {
        self.inner.lock().pages * PAGE
    }
}

impl Inner {
    fn node(&self, ino: Ino) -> Result<&Node> {
        self.nodes.get(&ino).ok_or(Errno::ENOENT)
    }

    fn node_mut(&mut self, ino: Ino) -> Result<&mut Node> {
        self.nodes.get_mut(&ino).ok_or(Errno::ENOENT)
    }

    /// A directory that may still get entries.
    fn live_dir(&self, ino: Ino) -> Result<&Dir> {
        match &self.node(ino)?.data {
            Data::Dir(d) if d.removed => Err(Errno::ENOENT),
            Data::Dir(d) => Ok(d),
            _ => Err(Errno::ENOTDIR),
        }
    }

    fn live_dir_mut(&mut self, ino: Ino) -> Result<&mut Dir> {
        match &mut self.node_mut(ino)?.data {
            Data::Dir(d) if d.removed => Err(Errno::ENOENT),
            Data::Dir(d) => Ok(d),
            _ => Err(Errno::ENOTDIR),
        }
    }

    fn is_dir(&self, ino: Ino) -> bool {
        matches!(self.nodes.get(&ino), Some(Node { data: Data::Dir(_), .. }))
    }

    fn is_empty_dir(&self, ino: Ino) -> bool {
        matches!(self.nodes.get(&ino), Some(Node { data: Data::Dir(d), .. }) if d.by_name.is_empty())
    }

    /// Fails with `EEXIST` if `name` exists in live directory `dir`.
    fn check_new(&self, dir: Ino, name: &[u8]) -> Result<()> {
        check_name(name)?;
        if self.live_dir(dir)?.get(name).is_some() { Err(Errno::EEXIST) } else { Ok(()) }
    }

    fn add(&mut self, node: Node) -> Ino {
        let ino = self.next_ino;
        self.next_ino += 1;
        self.nodes.insert(ino, node);
        ino
    }

    fn touch_dir(&mut self, dir: Ino, now: Timespec) {
        if let Some(n) = self.nodes.get_mut(&dir) {
            n.mtime = now;
            n.ctime = now;
        }
    }

    /// Frees `ino` if nothing names or pins it any more.
    fn maybe_free(&mut self, ino: Ino) {
        if ino == ROOT {
            return;
        }
        if let Some(n) = self.nodes.get(&ino)
            && n.nlink == 0
            && n.pins == 0
            && let Some(Node { data: Data::File(p), .. }) = self.nodes.remove(&ino)
        {
            self.pages -= p.pages.len() as u64;
        }
    }

    /// Drops one name of `ino`; a directory loses its last name at once.
    fn drop_link(&mut self, ino: Ino, now: Timespec) {
        if let Some(n) = self.nodes.get_mut(&ino) {
            match &mut n.data {
                Data::Dir(d) => {
                    n.nlink = 0;
                    d.removed = true;
                }
                _ => n.nlink = n.nlink.saturating_sub(1),
            }
            n.ctime = now;
        }
        self.maybe_free(ino);
    }

    fn adjust_nlink(&mut self, ino: Ino, delta: i32) {
        if let Some(n) = self.nodes.get_mut(&ino) {
            n.nlink = n.nlink.saturating_add_signed(delta);
        }
    }

    fn set_parent(&mut self, dir: Ino, parent: Ino) {
        if let Some(Node { data: Data::Dir(d), .. }) = self.nodes.get_mut(&dir) {
            d.parent = parent;
        }
    }

    /// Whether `dir` is `ancestor` or below it.
    fn is_within(&self, mut dir: Ino, ancestor: Ino) -> bool {
        loop {
            if dir == ancestor {
                return true;
            }
            match self.nodes.get(&dir) {
                Some(Node { data: Data::Dir(d), .. }) if d.parent != dir => dir = d.parent,
                _ => return false,
            }
        }
    }

    fn attr(&self, ino: Ino) -> Result<Attr> {
        let n = self.node(ino)?;
        let (size, pages) = match &n.data {
            Data::File(p) => (p.size, p.pages.len() as u64),
            Data::Symlink(t) => (t.len() as u64, 0),
            Data::Dir(_) => (PAGE, 0),
            Data::None => (0, 0),
        };
        Ok(Attr {
            ino,
            kind: n.kind,
            perm: n.perm,
            nlink: n.nlink,
            uid: n.uid,
            gid: n.gid,
            size,
            rdev: n.rdev,
            atime: n.atime,
            mtime: n.mtime,
            ctime: n.ctime,
            blocks: pages * (PAGE / 512),
            blksize: PAGE as u32,
        })
    }
}

fn check_name(name: &[u8]) -> Result<()> {
    if name.len() > NAME_MAX {
        Err(Errno::ENAMETOOLONG)
    } else if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') {
        Err(Errno::EINVAL)
    } else {
        Ok(())
    }
}

fn file(n: &Node) -> Result<&Pages> {
    match &n.data {
        Data::File(p) => Ok(p),
        Data::Dir(_) => Err(Errno::EISDIR),
        _ => Err(Errno::EINVAL),
    }
}

fn file_mut(n: &mut Node) -> Result<&mut Pages> {
    match &mut n.data {
        Data::File(p) => Ok(p),
        Data::Dir(_) => Err(Errno::EISDIR),
        _ => Err(Errno::EINVAL),
    }
}

impl Pages {
    fn read(&self, offset: u64, buf: &mut [u8]) -> usize {
        if offset >= self.size {
            return 0;
        }
        let n = buf.len().min((self.size - offset).min(usize::MAX as u64) as usize);
        let mut done = 0;
        while done < n {
            let pos = offset + done as u64;
            let in_page = (pos % PAGE) as usize;
            let chunk = (PAGE as usize - in_page).min(n - done);
            let dst = &mut buf[done..done + chunk];
            match self.pages.get(&(pos / PAGE)) {
                Some(page) => dst.copy_from_slice(&page[in_page..in_page + chunk]),
                None => dst.fill(0),
            }
            done += chunk;
        }
        n
    }

    /// Pages a write of `len` bytes at `offset` would add.
    fn new_pages(&self, offset: u64, len: usize) -> u64 {
        if len == 0 {
            return 0;
        }
        let first = offset / PAGE;
        let last = (offset + len as u64 - 1) / PAGE;
        (first..=last).filter(|i| !self.pages.contains_key(i)).count() as u64
    }

    fn write(&mut self, offset: u64, buf: &[u8]) {
        let mut done = 0;
        while done < buf.len() {
            let pos = offset + done as u64;
            let in_page = (pos % PAGE) as usize;
            let chunk = (PAGE as usize - in_page).min(buf.len() - done);
            let page = self.pages.entry(pos / PAGE).or_insert_with(|| vec![0; PAGE as usize].into_boxed_slice());
            page[in_page..in_page + chunk].copy_from_slice(&buf[done..done + chunk]);
            done += chunk;
        }
        self.size = self.size.max(offset + buf.len() as u64);
    }

    /// Sets the size; returns how many pages were freed. Bytes past a
    /// smaller size are zeroed so that growing again reads zeros.
    fn truncate(&mut self, size: u64) -> u64 {
        let before = self.pages.len();
        if size < self.size {
            let keep = size.div_ceil(PAGE);
            self.pages.retain(|&i, _| i < keep);
            if !size.is_multiple_of(PAGE)
                && let Some(page) = self.pages.get_mut(&(size / PAGE))
            {
                page[(size % PAGE) as usize..].fill(0);
            }
        }
        self.size = size;
        (before - self.pages.len()) as u64
    }
}

impl Tmpfs {
    fn new_node(&self, kind: FileType, perm: u16, uid: u32, gid: u32, data: Data) -> Node {
        let now = self.clock.now();
        let nlink = if kind == FileType::Directory { 2 } else { 1 };
        Node {
            kind,
            perm: perm & crate::PERM_MASK,
            uid,
            gid,
            nlink,
            rdev: DeviceId::default(),
            atime: now,
            mtime: now,
            ctime: now,
            pins: 0,
            data,
        }
    }

    fn check_space(&self, inner: &Inner, new_pages: u64) -> Result<()> {
        match self.page_limit {
            Some(limit) if inner.pages + new_pages > limit => Err(Errno::ENOSPC),
            _ => Ok(()),
        }
    }

    fn write_locked(&self, inner: &mut Inner, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        let now = self.clock.now();
        let end = offset.checked_add(buf.len() as u64).ok_or(Errno::EFBIG)?;
        if end > MAX_FILE_SIZE {
            return Err(Errno::EFBIG);
        }
        let new_pages = file(inner.node(ino)?)?.new_pages(offset, buf.len());
        self.check_space(inner, new_pages)?;
        let n = inner.node_mut(ino)?;
        file_mut(n)?.write(offset, buf);
        if !buf.is_empty() {
            n.mtime = now;
            n.ctime = now;
        }
        inner.pages += new_pages;
        Ok(buf.len())
    }
}

impl FileSystem for Tmpfs {
    fn info(&self) -> FsInfo {
        FsInfo { name: "tmpfs", read_only: false, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        let inner = self.inner.lock();
        let Data::Dir(d) = &inner.node(dir)?.data else { return Err(Errno::ENOTDIR) };
        match name {
            b".." => Ok(d.parent),
            b"." => Ok(dir),
            _ if d.removed => Err(Errno::ENOENT),
            _ => d.get(name).ok_or(Errno::ENOENT),
        }
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        self.inner.lock().attr(ino)
    }

    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let inner = &mut *inner;
        if let Some(size) = set.size {
            if size > MAX_FILE_SIZE {
                return Err(Errno::EFBIG);
            }
            // Growing allocates nothing: the new part is a hole.
            let freed = file_mut(inner.node_mut(ino)?)?.truncate(size);
            inner.pages -= freed;
        }
        let n = inner.node_mut(ino)?;
        if let Some(perm) = set.perm {
            n.perm = perm & crate::PERM_MASK;
        }
        if let Some(uid) = set.uid {
            n.uid = uid;
        }
        if let Some(gid) = set.gid {
            n.gid = gid;
        }
        let resolve = |t: SetTime| match t {
            SetTime::Now => now,
            SetTime::At(t) => t,
        };
        if let Some(t) = set.atime {
            n.atime = resolve(t);
        }
        if let Some(t) = set.mtime {
            n.mtime = resolve(t);
        }
        if set.size.is_some() {
            n.mtime = now;
        }
        n.ctime = set.ctime.unwrap_or(now);
        inner.attr(ino)
    }

    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        let data = match node.kind {
            FileType::Regular => Data::File(Pages::default()),
            FileType::Directory | FileType::Symlink => return Err(Errno::EINVAL),
            _ => Data::None,
        };
        let mut n = self.new_node(node.kind, node.perm, node.uid, node.gid, data);
        if matches!(node.kind, FileType::CharDevice | FileType::BlockDevice) {
            n.rdev = node.rdev;
        }
        let now = n.ctime;
        let mut inner = self.inner.lock();
        inner.check_new(dir, name)?;
        let ino = inner.add(n);
        inner.live_dir_mut(dir)?.insert(name, ino);
        inner.touch_dir(dir, now);
        Ok(ino)
    }

    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino> {
        let n = self.new_node(FileType::Directory, perm, uid, gid, Data::Dir(Dir::new(dir)));
        let now = n.ctime;
        let mut inner = self.inner.lock();
        inner.check_new(dir, name)?;
        let ino = inner.add(n);
        inner.live_dir_mut(dir)?.insert(name, ino);
        inner.adjust_nlink(dir, 1);
        inner.touch_dir(dir, now);
        Ok(ino)
    }

    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino> {
        if target.is_empty() {
            return Err(Errno::ENOENT);
        }
        let n = self.new_node(FileType::Symlink, 0o777, uid, gid, Data::Symlink(target.into()));
        let now = n.ctime;
        let mut inner = self.inner.lock();
        inner.check_new(dir, name)?;
        let ino = inner.add(n);
        inner.live_dir_mut(dir)?.insert(name, ino);
        inner.touch_dir(dir, now);
        Ok(ino)
    }

    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let target = inner.node(ino)?;
        if target.kind == FileType::Directory {
            return Err(Errno::EPERM);
        }
        if target.nlink == 0 {
            // An unlinked (but open) file cannot get a name back.
            return Err(Errno::ENOENT);
        }
        if target.nlink == u32::MAX {
            return Err(Errno::EMLINK);
        }
        inner.check_new(dir, name)?;
        inner.live_dir_mut(dir)?.insert(name, ino);
        let t = inner.node_mut(ino)?;
        t.nlink += 1;
        t.ctime = now;
        inner.touch_dir(dir, now);
        Ok(())
    }

    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let ino = inner.live_dir(dir)?.get(name).ok_or(Errno::ENOENT)?;
        if inner.is_dir(ino) {
            return Err(Errno::EISDIR);
        }
        inner.live_dir_mut(dir)?.remove(name);
        inner.touch_dir(dir, now);
        inner.drop_link(ino, now);
        Ok(())
    }

    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let ino = inner.live_dir(dir)?.get(name).ok_or(Errno::ENOENT)?;
        if !inner.is_dir(ino) {
            return Err(Errno::ENOTDIR);
        }
        if !inner.is_empty_dir(ino) {
            return Err(Errno::ENOTEMPTY);
        }
        inner.live_dir_mut(dir)?.remove(name);
        inner.adjust_nlink(dir, -1);
        inner.touch_dir(dir, now);
        inner.drop_link(ino, now);
        Ok(())
    }

    fn rename(&self, old_dir: Ino, old_name: &[u8], new_dir: Ino, new_name: &[u8], flags: RenameFlags) -> Result<()> {
        check_name(new_name)?;
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let src = inner.live_dir(old_dir)?.get(old_name).ok_or(Errno::ENOENT)?;
        let dst = inner.live_dir(new_dir)?.get(new_name);
        let src_dir = inner.is_dir(src);
        if src_dir && inner.is_within(new_dir, src) {
            return Err(Errno::EINVAL);
        }
        if flags.exchange {
            let dst = dst.ok_or(Errno::ENOENT)?;
            let dst_dir = inner.is_dir(dst);
            if dst_dir && inner.is_within(old_dir, dst) {
                return Err(Errno::EINVAL);
            }
            if src == dst {
                return Ok(());
            }
            inner.live_dir_mut(old_dir)?.remove(old_name);
            inner.live_dir_mut(new_dir)?.remove(new_name);
            inner.live_dir_mut(old_dir)?.insert(old_name, dst);
            inner.live_dir_mut(new_dir)?.insert(new_name, src);
            if old_dir != new_dir && src_dir != dst_dir {
                // One directory changes parent: link counts follow it.
                let delta = if src_dir { 1 } else { -1 };
                inner.adjust_nlink(new_dir, delta);
                inner.adjust_nlink(old_dir, -delta);
            }
            if src_dir {
                inner.set_parent(src, new_dir);
            }
            if dst_dir {
                inner.set_parent(dst, old_dir);
            }
            for ino in [src, dst] {
                if let Ok(n) = inner.node_mut(ino) {
                    n.ctime = now;
                }
            }
            inner.touch_dir(old_dir, now);
            inner.touch_dir(new_dir, now);
            return Ok(());
        }
        if let Some(dst) = dst {
            if dst == src {
                return Ok(());
            }
            if flags.noreplace {
                return Err(Errno::EEXIST);
            }
            let dst_dir = inner.is_dir(dst);
            match (src_dir, dst_dir) {
                (true, false) => return Err(Errno::ENOTDIR),
                (false, true) => return Err(Errno::EISDIR),
                (true, true) if !inner.is_empty_dir(dst) => return Err(Errno::ENOTEMPTY),
                _ => {}
            }
            inner.live_dir_mut(new_dir)?.remove(new_name);
            if dst_dir {
                inner.adjust_nlink(new_dir, -1);
            }
            inner.drop_link(dst, now);
        }
        inner.live_dir_mut(old_dir)?.remove(old_name);
        inner.live_dir_mut(new_dir)?.insert(new_name, src);
        if src_dir && old_dir != new_dir {
            inner.set_parent(src, new_dir);
            inner.adjust_nlink(old_dir, -1);
            inner.adjust_nlink(new_dir, 1);
        }
        if let Ok(n) = inner.node_mut(src) {
            n.ctime = now;
        }
        inner.touch_dir(old_dir, now);
        inner.touch_dir(new_dir, now);
        Ok(())
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        match &self.inner.lock().node(ino)?.data {
            Data::Symlink(t) => Ok(t.to_vec()),
            _ => Err(Errno::EINVAL),
        }
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let n = inner.node_mut(ino)?;
        let read = file(n)?.read(offset, buf);
        n.atime = now;
        Ok(read)
    }

    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        let mut inner = self.inner.lock();
        self.write_locked(&mut inner, ino, offset, buf)
    }

    fn append(&self, ino: Ino, buf: &[u8]) -> Result<(u64, usize)> {
        let mut inner = self.inner.lock();
        let at = file(inner.node(ino)?)?.size;
        Ok((at, self.write_locked(&mut inner, ino, at, buf)?))
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        // Positions: 0 is ".", 1 is "..", then entry with sequence number s
        // sits at s + 2 (and says the next one is at s + 3).
        let mut pos = pos;
        loop {
            let mut batch: Vec<(Ino, FileType, Box<[u8]>, u64)> = Vec::new();
            {
                let inner = self.inner.lock();
                let Data::Dir(d) = &inner.node(dir)?.data else { return Err(Errno::ENOTDIR) };
                if d.removed {
                    return Ok(());
                }
                if pos == 0 {
                    batch.push((dir, FileType::Directory, (*b".").into(), 1));
                }
                if pos <= 1 {
                    batch.push((d.parent, FileType::Directory, (*b"..").into(), 2));
                }
                for (&seq, name) in d.by_seq.range(pos.saturating_sub(2)..) {
                    if batch.len() >= READDIR_BATCH {
                        break;
                    }
                    let ino = d.get(name).unwrap_or(0);
                    let kind = inner.nodes.get(&ino).map_or(FileType::Regular, |n| n.kind);
                    batch.push((ino, kind, name.clone(), seq + 3));
                }
            }
            let Some(last) = batch.last() else { return Ok(()) };
            let full = batch.len() >= READDIR_BATCH;
            let next_pos = last.3;
            for (ino, kind, name, next) in &batch {
                if !emit(&DirEntry { ino: *ino, kind: *kind, name, next: *next }) {
                    return Ok(());
                }
            }
            if !full {
                return Ok(());
            }
            pos = next_pos;
        }
    }

    fn open(&self, ino: Ino) -> Result<()> {
        let mut inner = self.inner.lock();
        let n = inner.node_mut(ino)?;
        n.pins = n.pins.checked_add(1).ok_or(Errno::ENFILE)?;
        Ok(())
    }

    fn release(&self, ino: Ino) {
        let mut inner = self.inner.lock();
        if let Some(n) = inner.nodes.get_mut(&ino) {
            n.pins = n.pins.saturating_sub(1);
        }
        inner.maybe_free(ino);
    }
}
