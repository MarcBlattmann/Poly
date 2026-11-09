//! FUSE, file systems in user space (ADR 0168): the request protocol of
//! `/dev/fuse` (linux/fuse.h, protocol 7) and a [`poly_vfs::FileSystem`]
//! that asks the server for everything.
//!
//! - [`Conn`] is one connection: requests queue for the server's `read`,
//!   replies come with its `write`, and the thread of a file system call
//!   sleeps ([`Env::wait`]) until its reply arrives. The kernel sends
//!   `FUSE_INIT` by itself after the mount; calls wait for the server's
//!   answer to it. A server that went away makes every call `ENOTCONN`; an
//!   unusable `INIT` reply makes them `ECONNREFUSED`; an unmounted file
//!   system makes the server's next `read` `ENODEV`.
//! - [`FuseFs`] speaks lookup, getattr, readlink, readdir, read and statfs
//!   to the server and keeps what it answers for as long as the reply
//!   says (`entry_valid`, `attr_valid`). Changes are not sent yet.
#![no_std]
extern crate alloc;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::lock::{Lock, RawSpinLock};
use poly_vfs::{
    Attr, DeviceId, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, PollEvents, RenameFlags,
    Result, SetAttr, Timespec, WaitQueue,
};

/// `FUSE_SUPER_MAGIC` (`linux/magic.h`).
pub const FUSE_SUPER_MAGIC: u64 = 0x6573_5546;
pub const ROOT_ID: u64 = 1;
const IN_HEADER: usize = 40;
const OUT_HEADER: usize = 16;
/// Reads need at least this much room, more once a larger `max_write` is set.
const MIN_READ_BUFFER: usize = 8192;

pub mod op {
    pub const LOOKUP: u32 = 1;
    pub const GETATTR: u32 = 3;
    pub const READLINK: u32 = 5;
    pub const OPEN: u32 = 14;
    pub const READ: u32 = 15;
    pub const STATFS: u32 = 17;
    pub const RELEASE: u32 = 18;
    pub const INIT: u32 = 26;
    pub const OPENDIR: u32 = 27;
    pub const READDIR: u32 = 28;
    pub const RELEASEDIR: u32 = 29;
}

/// `FUSE_ASYNC_READ`, `FUSE_ATOMIC_O_TRUNC`, `FUSE_MAX_PAGES` (init flags).
const INIT_FLAGS: u32 = 1 | (1 << 3) | (1 << 22);

/// Who makes a call.
#[derive(Clone, Copy, Debug, Default)]
pub struct Caller {
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
}

/// What the kernel provides: the clock, sleeping and the caller.
pub trait Env: Send + Sync {
    fn now_ns(&self) -> u64;
    /// Sleeps the calling thread until `ready` is true; `queue` is notified
    /// when it may have become true. `EINTR` if interrupted.
    fn wait(&self, queue: &Arc<WaitQueue>, ready: &mut dyn FnMut() -> bool) -> Result<()>;
    fn caller(&self) -> Caller;
}

struct Pending {
    unique: u64,
    bytes: Vec<u8>,
}

#[derive(Default)]
struct State {
    bound: bool,
    /// The server closed its descriptor.
    dead: bool,
    /// The file system was unmounted.
    unmounted: bool,
    init_unique: u64,
    init_done: bool,
    /// The `INIT` reply cannot be used.
    refused: bool,
    next_unique: u64,
    pending: VecDeque<Pending>,
    /// Requests sent and not answered, with the answer once it came.
    waiting: BTreeMap<u64, Option<(i32, Vec<u8>)>>,
    max_write: u32,
    max_pages: u32,
}

/// One connection to a server.
pub struct Conn {
    state: Lock<RawSpinLock, State>,
    queue: Arc<WaitQueue>,
}

impl core::fmt::Debug for Conn {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Conn").finish_non_exhaustive()
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or([0; 4]))
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or([0; 8]))
}

fn header(len: usize, opcode: u32, unique: u64, nodeid: u64, who: Caller) -> Vec<u8> {
    let mut h = Vec::with_capacity(len);
    h.extend_from_slice(&(len as u32).to_le_bytes());
    h.extend_from_slice(&opcode.to_le_bytes());
    h.extend_from_slice(&unique.to_le_bytes());
    h.extend_from_slice(&nodeid.to_le_bytes());
    h.extend_from_slice(&who.uid.to_le_bytes());
    h.extend_from_slice(&who.gid.to_le_bytes());
    h.extend_from_slice(&who.pid.to_le_bytes());
    h.extend_from_slice(&[0u8; 4]);
    h
}

impl Conn {
    pub fn new() -> Arc<Conn> {
        Arc::new(Conn {
            state: Lock::new(State { next_unique: 2, ..State::default() }),
            queue: Arc::new(WaitQueue::new()),
        })
    }

    /// Where the server's readiness waiters register.
    pub fn queue(&self) -> &Arc<WaitQueue> {
        &self.queue
    }

    /// Joins a mount to the connection and queues the `INIT` request made
    /// by `who`. `EINVAL` if the descriptor is bound already.
    pub fn bind(&self, who: Caller) -> Result<()> {
        let mut s = self.state.lock();
        if s.bound {
            return Err(Errno::EINVAL);
        }
        s.bound = true;
        let unique = s.next_unique;
        s.next_unique += 2;
        s.init_unique = unique;
        let mut bytes = header(IN_HEADER + 64, op::INIT, unique, 0, who);
        bytes.extend_from_slice(&7u32.to_le_bytes());
        bytes.extend_from_slice(&31u32.to_le_bytes());
        bytes.extend_from_slice(&131_072u32.to_le_bytes());
        bytes.extend_from_slice(&INIT_FLAGS.to_le_bytes());
        bytes.resize(IN_HEADER + 64, 0);
        s.pending.push_back(Pending { unique, bytes });
        s.waiting.insert(unique, None);
        drop(s);
        self.queue.notify(PollEvents::READABLE);
        Ok(())
    }

    pub fn is_bound(&self) -> bool {
        self.state.lock().bound
    }

    /// The server closed `/dev/fuse`: calls fail with `ENOTCONN`.
    pub fn server_gone(&self) {
        self.state.lock().dead = true;
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
    }

    /// The file system was unmounted: the server's reads fail with `ENODEV`.
    pub fn unmounted(&self) {
        self.state.lock().unmounted = true;
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
    }

    /// The most a `WRITE` request carries (the server's `max_write`).
    pub fn max_write(&self) -> u32 {
        self.state.lock().max_write
    }

    /// Readable: a request waits, or the connection ended.
    pub fn readable(&self) -> bool {
        let s = self.state.lock();
        !s.pending.is_empty() || s.unmounted
    }

    /// The server reads one request.
    pub fn read(&self, env: &dyn Env, buf: &mut [u8], nonblock: bool) -> Result<usize> {
        if !self.is_bound() {
            return Err(Errno::EPERM);
        }
        loop {
            {
                let mut s = self.state.lock();
                if s.unmounted {
                    return Err(Errno::ENODEV);
                }
                // Room for the whole request (a WRITE is as long as max_write).
                let min = MIN_READ_BUFFER;
                if let Some(front) = s.pending.front() {
                    if buf.len() < min || buf.len() < front.bytes.len() {
                        return Err(Errno::EINVAL);
                    }
                    let p = s.pending.pop_front().ok_or(Errno::EIO)?;
                    buf[..p.bytes.len()].copy_from_slice(&p.bytes);
                    return Ok(p.bytes.len());
                }
                if nonblock {
                    return Err(Errno::EAGAIN);
                }
            }
            env.wait(&self.queue, &mut || self.readable())?;
        }
    }

    /// The server writes a reply.
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        if !self.is_bound() {
            return Err(Errno::EPERM);
        }
        if data.len() < OUT_HEADER {
            return Err(Errno::EINVAL);
        }
        let len = u32_at(data, 0) as usize;
        let error = u32_at(data, 4) as i32;
        let unique = u64_at(data, 8);
        if error > 0 || len < OUT_HEADER || len > data.len() {
            return Err(Errno::EINVAL);
        }
        let body = data[OUT_HEADER..len].to_vec();
        let mut s = self.state.lock();
        // Only a request that is waiting and not answered yet takes a reply.
        if !matches!(s.waiting.get(&unique), Some(None)) {
            return Err(Errno::ENOENT);
        }
        if unique == s.init_unique {
            s.waiting.remove(&unique);
            // Major 7 and no error; anything else is unusable.
            if error != 0 || body.len() < 8 || u32_at(&body, 0) != 7 {
                s.refused = true;
            } else {
                s.max_write = if body.len() >= 24 { u32_at(&body, 20) } else { 4096 };
                let flags = if body.len() >= 16 { u32_at(&body, 12) } else { 0 };
                s.max_pages = if flags & (1 << 22) != 0 && body.len() >= 30 {
                    u32::from(u16::from_le_bytes([body[28], body[29]]))
                } else {
                    0
                };
                s.init_done = true;
            }
        } else if let Some(slot) = s.waiting.get_mut(&unique) {
            *slot = Some((error, body));
        }
        drop(s);
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
        Ok(data.len())
    }

    /// Sends a request and waits for its reply body.
    pub fn call(&self, env: &dyn Env, opcode: u32, nodeid: u64, body: &[u8]) -> Result<Vec<u8>> {
        env.wait(&self.queue, &mut || {
            let s = self.state.lock();
            s.init_done || s.refused || s.dead
        })?;
        let unique;
        {
            let mut s = self.state.lock();
            if s.dead {
                return Err(Errno::ENOTCONN);
            }
            if s.refused {
                return Err(Errno::ECONNREFUSED);
            }
            unique = s.next_unique;
            s.next_unique += 2;
            let mut bytes = header(IN_HEADER + body.len(), opcode, unique, nodeid, env.caller());
            bytes.extend_from_slice(body);
            s.pending.push_back(Pending { unique, bytes });
            s.waiting.insert(unique, None);
        }
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
        let waited = env.wait(&self.queue, &mut || {
            let s = self.state.lock();
            s.dead || matches!(s.waiting.get(&unique), Some(Some(_)))
        });
        let mut s = self.state.lock();
        let answer = match s.waiting.remove(&unique) {
            Some(Some(a)) => Some(a),
            _ => None,
        };
        if answer.is_none() {
            s.pending.retain(|p| p.unique != unique);
        }
        drop(s);
        waited?;
        match answer {
            None => Err(Errno::ENOTCONN),
            Some((0, body)) => Ok(body),
            Some((e, _)) if e < 0 => Err(Errno::from_code(-e).unwrap_or(Errno::EIO)),
            Some(_) => Err(Errno::EIO),
        }
    }
}

/// The mount options (`fd=` is the caller's).
#[derive(Clone, Debug, Default)]
pub struct Options {
    pub root_mode: u32,
    pub user_id: u32,
    pub group_id: u32,
    pub max_read: u32,
    pub default_permissions: bool,
    pub allow_other: bool,
}

struct Cached {
    attr: Attr,
    until: u64,
}

#[derive(Default)]
struct Cache {
    attrs: BTreeMap<Ino, Cached>,
    /// `(dir, name)` to the node (0: known not to exist), until.
    entries: BTreeMap<(Ino, Vec<u8>), (u64, u64)>,
    parents: BTreeMap<Ino, Ino>,
}

/// A mounted FUSE file system.
pub struct FuseFs {
    conn: Arc<Conn>,
    env: Arc<dyn Env>,
    opts: Options,
    name: &'static str,
    cache: Lock<RawSpinLock, Cache>,
    /// The mount call is still running: the root is answered without the server.
    mounting: core::sync::atomic::AtomicBool,
}

impl core::fmt::Debug for FuseFs {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FuseFs").finish_non_exhaustive()
    }
}

impl Drop for FuseFs {
    fn drop(&mut self) {
        self.conn.unmounted();
    }
}

fn kind_of(mode: u32) -> FileType {
    FileType::from_mode(mode).unwrap_or(FileType::Regular)
}

fn valid_until(now: u64, secs: u64, nsec: u32) -> u64 {
    now.saturating_add(secs.saturating_mul(1_000_000_000)).saturating_add(u64::from(nsec))
}

impl FuseFs {
    /// The file system of `conn`, whose type shows as `name`.
    pub fn new(conn: Arc<Conn>, env: Arc<dyn Env>, opts: Options, name: &'static str) -> FuseFs {
        FuseFs {
            conn,
            env,
            opts,
            name,
            cache: Lock::new(Cache::default()),
            mounting: core::sync::atomic::AtomicBool::new(true),
        }
    }

    /// The mount call is over: from now on the server answers for the root.
    pub fn finish_mount(&self) {
        self.mounting.store(false, core::sync::atomic::Ordering::Relaxed);
    }

    fn parse_attr(b: &[u8]) -> Attr {
        let t = |s: u64, n: u32| Timespec::new(s as i64, n);
        let mode = u32_at(b, 60);
        Attr {
            ino: u64_at(b, 0),
            kind: kind_of(mode),
            perm: (mode & 0o7777) as u16,
            nlink: u32_at(b, 64),
            uid: u32_at(b, 68),
            gid: u32_at(b, 72),
            size: u64_at(b, 8),
            rdev: DeviceId::new(
                (u32_at(b, 76) >> 8) & 0xfff,
                (u32_at(b, 76) & 0xff) | ((u32_at(b, 76) >> 12) & 0xfff00),
            ),
            atime: t(u64_at(b, 24), u32_at(b, 48)),
            mtime: t(u64_at(b, 32), u32_at(b, 52)),
            ctime: t(u64_at(b, 40), u32_at(b, 56)),
            blocks: u64_at(b, 16),
            blksize: u32_at(b, 80).max(1),
        }
    }

    fn keep_attr(&self, node: Ino, attr: Attr, until: u64) {
        self.cache.lock().attrs.insert(node, Cached { attr, until });
    }

    fn cached_attr(&self, node: Ino) -> Option<Attr> {
        let now = self.env.now_ns();
        let c = self.cache.lock();
        c.attrs.get(&node).filter(|a| a.until > now).map(|a| a.attr)
    }

    /// The attributes of `node` as last known, even if no longer valid.
    fn known_attr(&self, node: Ino) -> Option<Attr> {
        self.cache.lock().attrs.get(&node).map(|a| a.attr)
    }
}

impl FileSystem for FuseFs {
    fn info(&self) -> FsInfo {
        FsInfo { name: self.name, read_only: false, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT_ID
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        if name == b".." {
            return Ok(self.cache.lock().parents.get(&dir).copied().unwrap_or(ROOT_ID));
        }
        if let Some(a) = self.known_attr(dir)
            && a.kind != FileType::Directory
        {
            return Err(Errno::ENOTDIR);
        }
        let now = self.env.now_ns();
        if let Some(&(node, until)) = self.cache.lock().entries.get(&(dir, name.to_vec()))
            && until > now
        {
            return if node == 0 { Err(Errno::ENOENT) } else { Ok(node) };
        }
        let mut body = name.to_vec();
        body.push(0);
        let r = self.conn.call(&*self.env, op::LOOKUP, dir, &body)?;
        if r.len() < 128 {
            return Err(Errno::EIO);
        }
        let node = u64_at(&r, 0);
        let now = self.env.now_ns();
        let entry_until = valid_until(now, u64_at(&r, 16), u32_at(&r, 32));
        if node == 0 {
            self.cache.lock().entries.insert((dir, name.to_vec()), (0, entry_until));
            return Err(Errno::ENOENT);
        }
        let attr = Self::parse_attr(&r[40..128]);
        self.keep_attr(node, attr, valid_until(now, u64_at(&r, 24), u32_at(&r, 36)));
        let mut c = self.cache.lock();
        c.entries.insert((dir, name.to_vec()), (node, entry_until));
        c.parents.insert(node, dir);
        Ok(node)
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        if ino == ROOT_ID && self.mounting.load(core::sync::atomic::Ordering::Relaxed) {
            let t = Timespec::default();
            return Ok(Attr {
                ino: ROOT_ID,
                kind: FileType::Directory,
                perm: (self.opts.root_mode & 0o7777) as u16,
                nlink: 2,
                uid: self.opts.user_id,
                gid: self.opts.group_id,
                size: 0,
                rdev: DeviceId::default(),
                atime: t,
                mtime: t,
                ctime: t,
                blocks: 0,
                blksize: 4096,
            });
        }
        if let Some(mut a) = self.cached_attr(ino) {
            a.ino = a.ino.max(1);
            return Ok(a);
        }
        let r = self.conn.call(&*self.env, op::GETATTR, ino, &[0u8; 16])?;
        if r.len() < 16 + 88 {
            return Err(Errno::EIO);
        }
        let attr = Self::parse_attr(&r[16..104]);
        let until = valid_until(self.env.now_ns(), u64_at(&r, 0), u32_at(&r, 8));
        self.keep_attr(ino, attr, until);
        Ok(attr)
    }

    fn setattr(&self, _ino: Ino, _set: &SetAttr) -> Result<Attr> {
        Err(Errno::EOPNOTSUPP)
    }

    fn create(&self, _dir: Ino, _name: &[u8], _node: &NewNode) -> Result<Ino> {
        Err(Errno::EOPNOTSUPP)
    }

    fn mkdir(&self, _dir: Ino, _name: &[u8], _perm: u16, _uid: u32, _gid: u32) -> Result<Ino> {
        Err(Errno::EOPNOTSUPP)
    }

    fn symlink(&self, _dir: Ino, _name: &[u8], _target: &[u8], _uid: u32, _gid: u32) -> Result<Ino> {
        Err(Errno::EOPNOTSUPP)
    }

    fn link(&self, _dir: Ino, _name: &[u8], _ino: Ino) -> Result<()> {
        Err(Errno::EOPNOTSUPP)
    }

    fn unlink(&self, _dir: Ino, _name: &[u8]) -> Result<()> {
        Err(Errno::EOPNOTSUPP)
    }

    fn rmdir(&self, _dir: Ino, _name: &[u8]) -> Result<()> {
        Err(Errno::EOPNOTSUPP)
    }

    fn rename(&self, _od: Ino, _on: &[u8], _nd: Ino, _nn: &[u8], _flags: RenameFlags) -> Result<()> {
        Err(Errno::EOPNOTSUPP)
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        if let Some(a) = self.known_attr(ino)
            && a.kind != FileType::Symlink
        {
            return Err(Errno::EINVAL);
        }
        self.conn.call(&*self.env, op::READLINK, ino, &[])
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let size = self.getattr(ino)?.size;
        if offset >= size || buf.is_empty() {
            return Ok(0);
        }
        let mut open = [0u8; 8];
        open[0..4].copy_from_slice(&0u32.to_le_bytes());
        let fh = match self.conn.call(&*self.env, op::OPEN, ino, &open) {
            Ok(r) if r.len() >= 8 => u64_at(&r, 0),
            Ok(_) => return Err(Errno::EIO),
            Err(Errno::ENOSYS) => 0,
            Err(e) => return Err(e),
        };
        let want = (buf.len() as u64).min(size - offset).min(if self.opts.max_read != 0 {
            u64::from(self.opts.max_read)
        } else {
            131_072
        });
        let mut body = Vec::with_capacity(40);
        body.extend_from_slice(&fh.to_le_bytes());
        body.extend_from_slice(&offset.to_le_bytes());
        body.extend_from_slice(&(want as u32).to_le_bytes());
        body.extend_from_slice(&[0u8; 20]);
        let r = self.conn.call(&*self.env, op::READ, ino, &body);
        if fh != 0 {
            let mut rel = Vec::with_capacity(24);
            rel.extend_from_slice(&fh.to_le_bytes());
            rel.extend_from_slice(&[0u8; 16]);
            let _ = self.conn.call(&*self.env, op::RELEASE, ino, &rel);
        }
        let data = r?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        Ok(n)
    }

    fn write(&self, _ino: Ino, _offset: u64, _buf: &[u8]) -> Result<usize> {
        Err(Errno::EOPNOTSUPP)
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        if let Some(a) = self.known_attr(dir)
            && a.kind != FileType::Directory
        {
            return Err(Errno::ENOTDIR);
        }
        let mut open = [0u8; 8];
        open[0..4].copy_from_slice(&0u32.to_le_bytes());
        let fh = match self.conn.call(&*self.env, op::OPENDIR, dir, &open) {
            Ok(r) if r.len() >= 8 => u64_at(&r, 0),
            Ok(_) => return Err(Errno::EIO),
            Err(Errno::ENOSYS) => 0,
            Err(e) => return Err(e),
        };
        let mut off = pos;
        let result = (|| -> Result<()> {
            loop {
                let mut body = Vec::with_capacity(40);
                body.extend_from_slice(&fh.to_le_bytes());
                body.extend_from_slice(&off.to_le_bytes());
                body.extend_from_slice(&4096u32.to_le_bytes());
                body.extend_from_slice(&[0u8; 20]);
                let r = self.conn.call(&*self.env, op::READDIR, dir, &body)?;
                if r.is_empty() {
                    return Ok(());
                }
                let mut at = 0;
                while at + 24 <= r.len() {
                    let (ino, next) = (u64_at(&r, at), u64_at(&r, at + 8));
                    let n = u32_at(&r, at + 16) as usize;
                    let t = u32_at(&r, at + 20);
                    let name = r.get(at + 24..at + 24 + n).ok_or(Errno::EIO)?;
                    let kind = FileType::from_mode(t << 12).unwrap_or(FileType::Regular);
                    if !emit(&DirEntry { ino, kind, name, next }) {
                        return Ok(());
                    }
                    off = next;
                    at += (24 + n).div_ceil(8) * 8;
                }
            }
        })();
        if fh != 0 {
            let mut rel = Vec::with_capacity(24);
            rel.extend_from_slice(&fh.to_le_bytes());
            rel.extend_from_slice(&[0u8; 16]);
            let _ = self.conn.call(&*self.env, op::RELEASEDIR, dir, &rel);
        }
        result
    }

    fn statfs(&self) -> Result<FsStats> {
        let r = self.conn.call(&*self.env, op::STATFS, ROOT_ID, &[])?;
        if r.len() < 80 {
            return Err(Errno::EIO);
        }
        Ok(FsStats {
            magic: FUSE_SUPER_MAGIC,
            blocks: u64_at(&r, 0),
            blocks_free: u64_at(&r, 8),
            blocks_avail: u64_at(&r, 16),
            files: u64_at(&r, 24),
            files_free: u64_at(&r, 32),
            block_size: u64::from(u32_at(&r, 40)),
            name_max: u64::from(u32_at(&r, 44)),
            fragment_size: u64::from(u32_at(&r, 48)),
            ..FsStats::default()
        })
    }
}
