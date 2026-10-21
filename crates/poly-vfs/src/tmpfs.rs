//! tmpfs: a file system in memory.
//!
//! Regular files are sparse: data lives in 4 KiB pages allocated when first
//! written, so a hole (from seeking past the end, or growing a file with
//! `truncate`) costs nothing and reads as zeros.
//!
//! Directories list as Linux's tmpfs does (the spec sheet
//! `specs/linux/x86_64/getdents64.yaml`, `directory_order`): `.` and `..`,
//! then the entries newest first. An entry that is created, linked or
//! renamed in goes to the front of the list with a new *cookie* from the
//! directory's counter (3, 4, ...; never reused until the counter wraps);
//! a rename over an existing name puts that name at the front with the
//! cookie it had. A record's `d_off` is the cookie of the entry after it
//! (2147483647 after the last one), and reading from a position starts at
//! the entry with the largest cookie not above it, in list order.
//!
//! Everything sits behind one lock per file system. `readdir` copies a batch
//! of entries out and calls the VFS's callback with the lock released.

use crate::fs::{Clock, DirEntry, FileSystem, FsInfo, NewNode, RenameFlags};
use crate::sync::SpinMutex;
use crate::xattr::{SetMode, XATTR_LIST_MAX, Xattrs};
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
/// A directory's size: this many bytes, plus [`DIRENT_SIZE`] per entry
/// other than `.` and `..`.
const DIR_SIZE: u64 = 40;
const DIRENT_SIZE: u64 = 20;
/// Entries copied out per round of `readdir`.
const READDIR_BATCH: usize = 64;

/// Directory positions (`d_off` values and the offsets lseek sets): `.` is
/// at 0, `..` at 1, the front of the list at 2, and the end is
/// `INT32_MAX`; every other entry is at its cookie.
const POS_DOTDOT: u64 = 1;
const POS_FRONT: u64 = 2;
/// The end of a listing.
pub const DIR_END: u64 = i32::MAX as u64;
/// The cookies a directory hands out, in order (then from the lowest free
/// one at or above the first, once the counter has passed the last).
const FIRST_COOKIE: u64 = 3;
const LAST_COOKIE: u64 = DIR_END - 1;

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
    /// Inodes the file system may hold, if limited (`ENOSPC` beyond).
    inode_limit: Option<u64>,
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
    /// Extended attributes (`user.`, `trusted.`, `security.`).
    xattrs: Xattrs,
    /// Made by `create_tmpfile` and never named: `link` may give it its
    /// first name.
    linkable: bool,
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

/// A directory entry.
#[derive(Clone, Copy)]
struct Entry {
    ino: Ino,
    /// Its position (see [`DIR_END`]).
    cookie: u64,
    /// Its place in the list: the larger, the nearer the front.
    stamp: u64,
}

struct Dir {
    parent: Ino,
    by_name: BTreeMap<Box<[u8]>, Entry>,
    /// Stamp → name: the list, front (newest) last.
    by_stamp: BTreeMap<u64, Box<[u8]>>,
    /// Cookie → stamp, to find where a position reads from.
    by_cookie: BTreeMap<u64, u64>,
    /// The cookie the next added entry gets, if free.
    next_cookie: u64,
    next_stamp: u64,
    /// Removed with `rmdir`; lives on while pinned, empty and closed to
    /// new entries.
    removed: bool,
}

/// One record of a listing: inode, name and `d_off`.
type Record = (Ino, Box<[u8]>, u64);

impl Dir {
    fn new(parent: Ino) -> Dir {
        Dir {
            parent,
            by_name: BTreeMap::new(),
            by_stamp: BTreeMap::new(),
            by_cookie: BTreeMap::new(),
            next_cookie: FIRST_COOKIE,
            next_stamp: 0,
            removed: false,
        }
    }

    fn get(&self, name: &[u8]) -> Option<Ino> {
        self.by_name.get(name).map(|e| e.ino)
    }

    /// Whether a cookie is free for another entry (`ENOSPC` otherwise:
    /// only with 2^31 - 4 entries in the directory).
    fn has_room(&self) -> bool {
        (self.by_cookie.len() as u64) <= LAST_COOKIE - FIRST_COOKIE
    }

    /// The next cookie: the counter's value, and once the counter has
    /// passed [`LAST_COOKIE`] the lowest free one from where it is on
    /// (starting over at [`FIRST_COOKIE`]). Needs [`Dir::has_room`].
    fn new_cookie(&mut self) -> u64 {
        let mut c = self.next_cookie;
        loop {
            if c > LAST_COOKIE {
                c = FIRST_COOKIE;
            }
            if !self.by_cookie.contains_key(&c) {
                break;
            }
            c += 1;
        }
        self.next_cookie = c + 1;
        c
    }

    fn new_stamp(&mut self) -> u64 {
        self.next_stamp += 1;
        self.next_stamp
    }

    /// Adds `name` at the front with a new cookie.
    fn insert(&mut self, name: &[u8], ino: Ino) {
        let cookie = self.new_cookie();
        let stamp = self.new_stamp();
        self.by_name.insert(name.into(), Entry { ino, cookie, stamp });
        self.by_stamp.insert(stamp, name.into());
        self.by_cookie.insert(cookie, stamp);
    }

    /// Points the existing `name` at `ino` and moves it to the front; it
    /// keeps its cookie (rename over a name, `RENAME_EXCHANGE`).
    fn replace(&mut self, name: &[u8], ino: Ino) {
        let stamp = self.new_stamp();
        if let Some(e) = self.by_name.get_mut(name) {
            if let Some(n) = self.by_stamp.remove(&e.stamp) {
                self.by_stamp.insert(stamp, n);
            }
            self.by_cookie.insert(e.cookie, stamp);
            e.ino = ino;
            e.stamp = stamp;
        }
    }

    fn remove(&mut self, name: &[u8]) -> Option<Ino> {
        let e = self.by_name.remove(name)?;
        self.by_stamp.remove(&e.stamp);
        self.by_cookie.remove(&e.cookie);
        Some(e.ino)
    }

    /// The cookie of the entry at `stamp`.
    fn cookie_at(&self, stamp: u64) -> u64 {
        self.by_stamp.get(&stamp).and_then(|n| self.by_name.get(n)).map_or(DIR_END, |e| e.cookie)
    }

    /// Up to `max` records of the listing from position `pos` (directory
    /// `me`), each with its `d_off`: the position of the record after it.
    fn records(&self, me: Ino, pos: u64, max: usize) -> Vec<Record> {
        let mut out = Vec::new();
        let front = self.by_stamp.last_key_value().map(|(&s, _)| s);
        let start = match pos {
            0 | POS_DOTDOT => {
                if pos == 0 {
                    out.push((me, (*b".").into(), POS_DOTDOT));
                }
                out.push((self.parent, (*b"..").into(), front.map_or(DIR_END, |s| self.cookie_at(s))));
                front
            }
            POS_FRONT => front,
            DIR_END => None,
            // The entry with the largest cookie not above the position (so
            // above the end, the largest), else the front.
            _ => self.by_cookie.range(..=pos).next_back().map(|(_, &s)| s).or(front),
        };
        let Some(start) = start else { return out };
        let mut list = self.by_stamp.range(..=start).rev().peekable();
        while out.len() < max {
            let Some((_, name)) = list.next() else { break };
            let next = list.peek().map_or(DIR_END, |&(&s, _)| self.cookie_at(s));
            out.push((self.by_name.get(name).map_or(0, |e| e.ino), name.clone(), next));
        }
        out
    }
}

impl Tmpfs {
    /// An empty tmpfs whose root is owned by root with mode `0755`.
    pub fn new(clock: Arc<dyn Clock>) -> Tmpfs {
        Tmpfs::build(clock, None, None)
    }

    /// An empty tmpfs that holds at most `bytes` of file data (rounded down
    /// to whole pages); writes beyond fail with `ENOSPC`.
    pub fn with_limit(clock: Arc<dyn Clock>, bytes: u64) -> Tmpfs {
        Tmpfs::build(clock, Some(bytes / PAGE), None)
    }

    /// An empty tmpfs that holds at most `bytes` of file data and `inodes`
    /// inodes (the root included), tmpfs(5)'s `size` and `nr_inodes`:
    /// beyond either, writes and new files fail with `ENOSPC`. `statfs`
    /// reports both as the totals.
    pub fn with_limits(clock: Arc<dyn Clock>, bytes: u64, inodes: u64) -> Tmpfs {
        Tmpfs::build(clock, Some(bytes / PAGE), Some(inodes))
    }

    fn build(clock: Arc<dyn Clock>, page_limit: Option<u64>, inode_limit: Option<u64>) -> Tmpfs {
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
            xattrs: Xattrs::new(),
            data: Data::Dir(Dir::new(ROOT)),
            linkable: false,
        };
        let mut nodes = BTreeMap::new();
        nodes.insert(ROOT, root);
        Tmpfs { inner: SpinMutex::new(Inner { nodes, next_ino: ROOT + 1, pages: 0, inode_limit }), clock, page_limit }
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
        let d = self.live_dir(dir)?;
        if d.get(name).is_some() {
            return Err(Errno::EEXIST);
        }
        if !d.has_room() {
            return Err(Errno::ENOSPC);
        }
        match self.inode_limit {
            Some(limit) if self.nodes.len() as u64 >= limit => Err(Errno::ENOSPC),
            _ => Ok(()),
        }
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
            // Linux's tmpfs: 40 bytes plus 20 per entry (the spec sheet
            // specs/linux/apps/ls.yaml, `tmpfs_sizes`); no data pages.
            Data::Dir(d) => (DIR_SIZE + DIRENT_SIZE * d.by_name.len() as u64, 0),
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
            xattrs: Xattrs::new(),
            data,
            linkable: false,
        }
    }

    fn check_space(&self, inner: &Inner, new_pages: u64) -> Result<()> {
        match self.page_limit {
            Some(limit) if inner.pages + new_pages > limit => Err(Errno::ENOSPC),
            _ => Ok(()),
        }
    }

    /// Writes `buf` at `offset`; `touch` sets the modification and change
    /// times (not for a page cache's writeback, which keeps them).
    fn write_locked(&self, inner: &mut Inner, ino: Ino, offset: u64, buf: &[u8], touch: bool) -> Result<usize> {
        let now = self.clock.now();
        let end = offset.checked_add(buf.len() as u64).ok_or(Errno::EFBIG)?;
        if end > MAX_FILE_SIZE {
            return Err(Errno::EFBIG);
        }
        let new_pages = file(inner.node(ino)?)?.new_pages(offset, buf.len());
        self.check_space(inner, new_pages)?;
        let n = inner.node_mut(ino)?;
        file_mut(n)?.write(offset, buf);
        if touch && !buf.is_empty() {
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

    /// Blocks are 4 KiB pages: the limit (or, without one, the pages in
    /// use, none free) and the pages in use; inodes are not limited, so
    /// the count of free ones is the room left in the inode numbers.
    fn statfs(&self) -> Result<crate::FsStats> {
        let inner = self.inner.lock();
        let total = self.page_limit.unwrap_or(inner.pages).max(inner.pages);
        let free = total - inner.pages;
        let used = inner.nodes.len() as u64;
        // Without a limit, as many inodes as an inode count can hold.
        let files = inner.inode_limit.unwrap_or(u64::from(u32::MAX)).max(used);
        Ok(crate::FsStats {
            magic: crate::TMPFS_MAGIC,
            block_size: PAGE,
            blocks: total,
            blocks_free: free,
            blocks_avail: free,
            files,
            files_free: files - used,
            name_max: crate::NAME_MAX as u64,
            fragment_size: PAGE,
            ..crate::FsStats::default()
        })
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

    fn create_tmpfile(&self, dir: Ino, node: &NewNode) -> Result<Ino> {
        if node.kind != FileType::Regular {
            return Err(Errno::EINVAL);
        }
        let mut n = self.new_node(FileType::Regular, node.perm, node.uid, node.gid, Data::File(Pages::default()));
        n.nlink = 0;
        n.linkable = true;
        let mut inner = self.inner.lock();
        inner.live_dir(dir)?;
        if let Some(limit) = inner.inode_limit
            && inner.nodes.len() as u64 >= limit
        {
            return Err(Errno::ENOSPC);
        }
        Ok(inner.add(n))
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
        if target.nlink == 0 && !target.linkable {
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
        t.linkable = false;
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
            // Each name keeps its cookie; both go to the front, the new
            // name first.
            inner.live_dir_mut(old_dir)?.replace(old_name, dst);
            inner.live_dir_mut(new_dir)?.replace(new_name, src);
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
            // The name goes to the front with the cookie it had.
            inner.live_dir_mut(old_dir)?.remove(old_name);
            inner.live_dir_mut(new_dir)?.replace(new_name, src);
            if dst_dir {
                inner.adjust_nlink(new_dir, -1);
            }
            inner.drop_link(dst, now);
        } else {
            if old_dir != new_dir && !inner.live_dir(new_dir)?.has_room() {
                return Err(Errno::ENOSPC);
            }
            // A new name: at the front with a new cookie.
            inner.live_dir_mut(old_dir)?.remove(old_name);
            inner.live_dir_mut(new_dir)?.insert(new_name, src);
        }
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

    fn getxattr(&self, ino: Ino, name: &[u8]) -> Result<Vec<u8>> {
        self.inner.lock().node(ino)?.xattrs.get(name).map(<[u8]>::to_vec)
    }

    /// Every kind of node keeps attributes (the VFS decides which
    /// namespaces apply to it); the names of one inode take at most
    /// `XATTR_LIST_MAX` bytes in a list (`ENOSPC` beyond), so listxattr(2)
    /// can always return them.
    fn setxattr(&self, ino: Ino, name: &[u8], value: &[u8], how: SetMode) -> Result<()> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let node = inner.node_mut(ino)?;
        let exists = node.xattrs.get(name).is_ok();
        how.check(exists)?;
        if !exists && node.xattrs.list().len() + name.len() + 1 > XATTR_LIST_MAX {
            return Err(Errno::ENOSPC);
        }
        node.xattrs.set(name, value, how)?;
        node.ctime = now;
        Ok(())
    }

    fn listxattr(&self, ino: Ino) -> Result<Vec<u8>> {
        Ok(self.inner.lock().node(ino)?.xattrs.list())
    }

    fn removexattr(&self, ino: Ino, name: &[u8]) -> Result<()> {
        let now = self.clock.now();
        let mut inner = self.inner.lock();
        let node = inner.node_mut(ino)?;
        node.xattrs.remove(name)?;
        node.ctime = now;
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
        self.write_locked(&mut inner, ino, offset, buf, true)
    }

    fn write_back(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        let mut inner = self.inner.lock();
        self.write_locked(&mut inner, ino, offset, buf, false)
    }

    fn append(&self, ino: Ino, buf: &[u8]) -> Result<(u64, usize)> {
        let mut inner = self.inner.lock();
        let at = file(inner.node(ino)?)?.size;
        Ok((at, self.write_locked(&mut inner, ino, at, buf, true)?))
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        // Positions and d_off values: see `DIR_END` and `Dir::records`. A
        // batch ends with the d_off of its last record, which is where the
        // next batch reads from. A removed directory (still open) is ENOENT
        // (getdents64(2)); one removed while listing just ends.
        let start = pos;
        let mut pos = pos;
        loop {
            let batch: Vec<(Ino, FileType, Box<[u8]>, u64)> = {
                let inner = self.inner.lock();
                let Data::Dir(d) = &inner.node(dir)?.data else { return Err(Errno::ENOTDIR) };
                if d.removed {
                    return if pos == start { Err(Errno::ENOENT) } else { Ok(()) };
                }
                d.records(dir, pos, READDIR_BATCH)
                    .into_iter()
                    .map(|(ino, name, next)| {
                        let kind = inner.nodes.get(&ino).map_or(FileType::Regular, |n| n.kind);
                        (ino, kind, name, next)
                    })
                    .collect()
            };
            let Some(last) = batch.last() else { return Ok(()) };
            let next_pos = last.3;
            for (ino, kind, name, next) in &batch {
                if !emit(&DirEntry { ino: *ino, kind: *kind, name, next: *next }) {
                    return Ok(());
                }
            }
            if batch.len() < READDIR_BATCH || next_pos == DIR_END {
                return Ok(());
            }
            pos = next_pos;
        }
    }

    /// A listing that has ended stays at [`DIR_END`] (and returns nothing)
    /// until lseek moves it, even if entries are added.
    fn readdir_end(&self) -> Option<u64> {
        Some(DIR_END)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cookie(d: &Dir, name: &[u8]) -> u64 {
        d.by_name[name].cookie
    }

    fn names(recs: &[Record]) -> Vec<&[u8]> {
        recs.iter().map(|r| &r.1[..]).collect()
    }

    /// Cookies count up from 3; once the counter has passed the last one
    /// (2147483646) it goes on from the lowest free cookie at or above 3.
    #[test]
    fn cookies_wrap_to_the_lowest_free_one() {
        let mut d = Dir::new(ROOT);
        for n in [b"x", b"y", b"z"] {
            d.insert(n, 10);
        }
        assert_eq!((cookie(&d, b"x"), cookie(&d, b"y"), cookie(&d, b"z")), (3, 4, 5));
        d.remove(b"y");
        d.next_cookie = LAST_COOKIE;
        for n in [b"p", b"q", b"r", b"s"] {
            d.insert(n, 10);
        }
        assert_eq!(cookie(&d, b"p"), 2147483646);
        assert_eq!(cookie(&d, b"q"), 4, "3 is taken: the lowest free one");
        assert_eq!(cookie(&d, b"r"), 6, "5 is taken");
        assert_eq!(cookie(&d, b"s"), 7);
        // Still newest first, and positions find entries by cookie.
        let all = d.records(ROOT, 0, 64);
        let want: [&[u8]; 8] = [b".", b"..", b"s", b"r", b"q", b"p", b"z", b"x"];
        assert_eq!(names(&all), want);
        assert_eq!(all.iter().map(|r| r.2).collect::<Vec<_>>(), [1, 7, 6, 4, 2147483646, 5, 3, DIR_END]);
        let want: [&[u8]; 5] = [b"r", b"q", b"p", b"z", b"x"];
        assert_eq!(names(&d.records(ROOT, 6, 64)), want);
        assert!(d.has_room());
    }

    #[test]
    fn records_stop_at_max_with_the_next_cookie() {
        let mut d = Dir::new(ROOT);
        for n in [b"a", b"b", b"c"] {
            d.insert(n, 10);
        }
        let r = d.records(7, 0, 3);
        assert_eq!(r.iter().map(|r| (r.0, r.2)).collect::<Vec<_>>(), [(7, 1), (ROOT, 5), (10, 4)]);
        assert_eq!(d.records(7, 4, 1).iter().map(|r| r.2).collect::<Vec<_>>(), [3]);
        assert!(d.records(7, DIR_END, 64).is_empty());
    }
}
