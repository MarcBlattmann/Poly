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
//! - [`FuseFs`] asks the server for everything and keeps what it answers
//!   for as long as the reply says (`entry_valid`, `attr_valid`). Every
//!   open of a file or directory is the server's too (`OPEN`, `OPENDIR`,
//!   `CREATE`): its file handle goes back with the reads, writes, `FLUSH`
//!   at each close of a descriptor and `RELEASE` when the open file is
//!   gone. Changes go to the server before the call returns (no writeback
//!   cache).
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;
#[cfg(test)]
mod tests;

use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_vfs::lock::{Lock, RawSpinLock};
use poly_vfs::{
    Attr, DeviceId, DirEntry, Errno, FileSystem, FileType, FsInfo, FsStats, Ino, NewNode, OpenFlags, OpenTrunc,
    PollEvents, RenameFlags, Result, SetAttr, SetTime, Timespec, WaitQueue,
};

/// `FUSE_SUPER_MAGIC` (`linux/magic.h`).
pub const FUSE_SUPER_MAGIC: u64 = 0x6573_5546;
pub const ROOT_ID: u64 = 1;
const IN_HEADER: usize = 40;
const OUT_HEADER: usize = 16;
/// Reads need at least this much room, more once a larger `max_write` is set.
const MIN_READ_BUFFER: usize = 8192;
/// The longest message of a connection: a `WRITE` request or the reply to
/// a `READ` (the protocol's largest transfer is 256 pages) and its headers.
pub const MAX_MESSAGE: usize = 256 * 4096 + 4096;

/// `enum fuse_opcode` (linux/fuse.h).
pub mod op {
    pub const LOOKUP: u32 = 1;
    pub const GETATTR: u32 = 3;
    pub const SETATTR: u32 = 4;
    pub const READLINK: u32 = 5;
    pub const SYMLINK: u32 = 6;
    pub const MKNOD: u32 = 8;
    pub const MKDIR: u32 = 9;
    pub const UNLINK: u32 = 10;
    pub const RMDIR: u32 = 11;
    pub const RENAME: u32 = 12;
    pub const LINK: u32 = 13;
    pub const OPEN: u32 = 14;
    pub const READ: u32 = 15;
    pub const WRITE: u32 = 16;
    pub const STATFS: u32 = 17;
    pub const RELEASE: u32 = 18;
    pub const FSYNC: u32 = 20;
    pub const FLUSH: u32 = 25;
    pub const INIT: u32 = 26;
    pub const OPENDIR: u32 = 27;
    pub const READDIR: u32 = 28;
    pub const RELEASEDIR: u32 = 29;
    pub const FSYNCDIR: u32 = 30;
    pub const CREATE: u32 = 35;
    pub const RENAME2: u32 = 45;
}

/// `FATTR_*` (linux/fuse.h): the fields a `SETATTR` request sets.
pub mod fattr {
    pub const MODE: u32 = 1 << 0;
    pub const UID: u32 = 1 << 1;
    pub const GID: u32 = 1 << 2;
    pub const SIZE: u32 = 1 << 3;
    pub const ATIME: u32 = 1 << 4;
    pub const MTIME: u32 = 1 << 5;
    pub const FH: u32 = 1 << 6;
    pub const ATIME_NOW: u32 = 1 << 7;
    pub const MTIME_NOW: u32 = 1 << 8;
}

/// `FUSE_ASYNC_READ`, `FUSE_ATOMIC_O_TRUNC`, `FUSE_MAX_PAGES` (init flags).
const FUSE_ATOMIC_O_TRUNC: u32 = 1 << 3;
const FUSE_MAX_PAGES: u32 = 1 << 22;
const INIT_FLAGS: u32 = 1 | FUSE_ATOMIC_O_TRUNC | FUSE_MAX_PAGES;
/// The most a `READ` asks for and a `WRITE` carries when the server set no
/// smaller limit (the spec sheet of fuse_init).
const DEFAULT_MAX_READ: u32 = 131_072;
const DEFAULT_MAX_WRITE: u32 = 65_536;
const PAGE: u32 = 4096;

/// Who makes a call.
#[derive(Clone, Copy, Debug, Default)]
pub struct Caller {
    pub uid: u32,
    pub gid: u32,
    pub pid: u32,
    /// Its file mode creation mask (umask(2)): `CREATE`, `MKNOD` and
    /// `MKDIR` carry it.
    pub umask: u32,
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
    waiting: BTreeMap<u64, Slot>,
    /// The `INIT` reply's flags, `max_write` and (with `FUSE_MAX_PAGES`)
    /// `max_pages`.
    init_flags: u32,
    max_write: u32,
    max_pages: u32,
}

/// A request the server has not answered, or its answer.
enum Slot {
    /// A caller sleeps until the answer comes.
    Waiting,
    Answered(i32, Vec<u8>),
    /// Nobody waits (`RELEASE`): the answer is taken and dropped.
    Background,
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
        s.waiting.insert(unique, Slot::Waiting);
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

    /// The file system was unmounted: the server's reads fail with `ENODEV`
    /// (after the requests nobody waits for, a last `RELEASE`).
    pub fn unmounted(&self) {
        {
            let mut guard = self.state.lock();
            let s = &mut *guard;
            s.unmounted = true;
            let waiting = &s.waiting;
            s.pending.retain(|p| matches!(waiting.get(&p.unique), Some(Slot::Background)));
        }
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
    }

    /// The most a `WRITE` request carries: the server's `max_write`, and
    /// its `max_pages` pages if it set them.
    pub fn max_write(&self) -> u32 {
        let s = self.state.lock();
        let pages = if s.max_pages != 0 { s.max_pages.saturating_mul(PAGE) } else { DEFAULT_MAX_WRITE };
        s.max_write.min(pages).max(PAGE)
    }

    /// The most a `READ` request asks for (before the mount's `max_read`).
    pub fn max_read(&self) -> u32 {
        let s = self.state.lock();
        if s.max_pages != 0 { s.max_pages.saturating_mul(PAGE).max(PAGE) } else { DEFAULT_MAX_READ }
    }

    /// Whether the server's `INIT` reply set `FUSE_ATOMIC_O_TRUNC`: an
    /// `OPEN` carrying `O_TRUNC` truncates.
    pub fn atomic_trunc(&self) -> bool {
        self.state.lock().init_flags & FUSE_ATOMIC_O_TRUNC != 0
    }

    /// The mount is made and the server has not answered `INIT` yet.
    pub fn init_pending(&self) -> bool {
        let s = self.state.lock();
        s.bound && !s.init_done && !s.refused && !s.dead
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
                if s.unmounted && s.pending.is_empty() {
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
        match s.waiting.get(&unique) {
            Some(Slot::Waiting) => {}
            Some(Slot::Background) => {
                s.waiting.remove(&unique);
                return Ok(data.len());
            }
            _ => return Err(Errno::ENOENT),
        }
        if unique == s.init_unique {
            s.waiting.remove(&unique);
            // Major 7 and no error; anything else is unusable.
            if error != 0 || body.len() < 8 || u32_at(&body, 0) != 7 {
                s.refused = true;
            } else {
                s.max_write = if body.len() >= 24 { u32_at(&body, 20) } else { 4096 };
                let flags = if body.len() >= 16 { u32_at(&body, 12) } else { 0 };
                s.init_flags = flags;
                s.max_pages = if flags & FUSE_MAX_PAGES != 0 && body.len() >= 30 {
                    u32::from(u16::from_le_bytes([body[28], body[29]]))
                } else {
                    0
                };
                s.init_done = true;
            }
        } else if let Some(slot) = s.waiting.get_mut(&unique) {
            *slot = Slot::Answered(error, body);
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
            s.waiting.insert(unique, Slot::Waiting);
        }
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
        let waited = env.wait(&self.queue, &mut || {
            let s = self.state.lock();
            s.dead || matches!(s.waiting.get(&unique), Some(Slot::Answered(..)))
        });
        let mut s = self.state.lock();
        let answer = match s.waiting.remove(&unique) {
            Some(Slot::Answered(e, body)) => Some((e, body)),
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

    /// Sends a request nobody waits for (`RELEASE`, a `FLUSH` at the end of
    /// a process); the server's answer is dropped. Nothing is sent on a
    /// connection that is not usable. Does not sleep.
    pub fn post(&self, who: Caller, opcode: u32, nodeid: u64, body: &[u8]) {
        {
            let mut s = self.state.lock();
            if !s.init_done || s.dead || s.refused || s.unmounted {
                return;
            }
            let unique = s.next_unique;
            s.next_unique += 2;
            let mut bytes = header(IN_HEADER + body.len(), opcode, unique, nodeid, who);
            bytes.extend_from_slice(body);
            s.pending.push_back(Pending { unique, bytes });
            s.waiting.insert(unique, Slot::Background);
        }
        self.queue.notify(PollEvents::READABLE | PollEvents::ALWAYS);
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

/// One open of a file or directory.
#[derive(Clone, Copy)]
struct Open {
    /// The server's file handle (0 without an `OPEN`).
    fh: u64,
    /// The server answered an `OPEN`: it gets a `RELEASE`.
    opened: bool,
    /// The open flags, as the requests of this file carry them.
    flags: u32,
}

/// A mounted FUSE file system.
pub struct FuseFs {
    conn: Arc<Conn>,
    env: Arc<dyn Env>,
    opts: Options,
    name: &'static str,
    cache: Lock<RawSpinLock, Cache>,
    /// The mount call is still running: the root is answered without the server.
    mounting: AtomicBool,
    /// The open files, by the handle the VFS holds.
    opens: Lock<RawSpinLock, BTreeMap<u64, Open>>,
    next_handle: AtomicU64,
    /// Requests the server answered with `ENOSYS`: not sent again.
    no_open: AtomicBool,
    no_opendir: AtomicBool,
    no_flush: AtomicBool,
    no_fsync: AtomicBool,
    no_fsyncdir: AtomicBool,
    no_create: AtomicBool,
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

/// The bytes of a request body: little-endian words, then names.
#[derive(Default)]
struct Body(Vec<u8>);

impl Body {
    fn u32(mut self, v: u32) -> Body {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u64(mut self, v: u64) -> Body {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    /// A name and its NUL.
    fn name(mut self, name: &[u8]) -> Body {
        self.0.extend_from_slice(name);
        self.0.push(0);
        self
    }
    fn bytes(mut self, data: &[u8]) -> Body {
        self.0.extend_from_slice(data);
        self
    }
}

/// The open flags a request carries: without the ones that only matter
/// while the name is looked up.
fn wire_flags(flags: OpenFlags, keep_create: bool) -> u32 {
    let mut drop = OpenFlags::NOCTTY;
    if !keep_create {
        drop = drop | OpenFlags::CREAT | OpenFlags::EXCL;
    }
    flags.without(drop).bits()
}

/// A device number as `fuse_mknod_in.rdev` and `fuse_attr.rdev` hold it:
/// the minor's low byte, the major from bit 8, the rest of the minor from
/// bit 20 (`major << 8 | minor` for small numbers).
fn rdev_bits(dev: DeviceId) -> u32 {
    (dev.minor & 0xff) | (dev.major << 8) | ((dev.minor & !0xff) << 12)
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
            mounting: AtomicBool::new(true),
            opens: Lock::new(BTreeMap::new()),
            next_handle: AtomicU64::new(1),
            no_open: AtomicBool::new(false),
            no_opendir: AtomicBool::new(false),
            no_flush: AtomicBool::new(false),
            no_fsync: AtomicBool::new(false),
            no_fsyncdir: AtomicBool::new(false),
            no_create: AtomicBool::new(false),
        }
    }

    /// The mount call is over: from now on the server answers for the root.
    pub fn finish_mount(&self) {
        self.mounting.store(false, Ordering::Relaxed);
    }

    fn call(&self, opcode: u32, nodeid: u64, body: &[u8]) -> Result<Vec<u8>> {
        self.conn.call(&*self.env, opcode, nodeid, body)
    }

    /// The root as the mount options describe it (`rootmode`, `user_id`,
    /// `group_id`), before the server said anything about it.
    fn root_attr(&self) -> Attr {
        let t = Timespec::default();
        Attr {
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
        }
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

    /// The attributes of `node` changed at the server: ask again.
    fn stale(&self, node: Ino) {
        if let Some(a) = self.cache.lock().attrs.get_mut(&node) {
            a.until = 0;
        }
    }

    /// The name is no longer what the cache says; returns the node it had.
    fn forget_name(&self, dir: Ino, name: &[u8]) -> Option<Ino> {
        self.cache.lock().entries.remove(&(dir, name.to_vec())).map(|(node, _)| node).filter(|&n| n != 0)
    }

    /// Takes a `struct fuse_entry_out` (the answer to a lookup or to making
    /// a name): the node behind `dir/name` (0: none), its attributes and how
    /// long both are good for.
    fn take_entry(&self, dir: Ino, name: &[u8], r: &[u8]) -> Result<Ino> {
        if r.len() < 128 {
            return Err(Errno::EIO);
        }
        let node = u64_at(r, 0);
        let now = self.env.now_ns();
        let entry_until = valid_until(now, u64_at(r, 16), u32_at(r, 32));
        let mut c = self.cache.lock();
        let key = (dir, name.to_vec());
        if entry_until > now {
            c.entries.insert(key, (node, entry_until));
        } else {
            c.entries.remove(&key);
        }
        if node == 0 {
            return Ok(0);
        }
        let attr = Self::parse_attr(&r[40..128]);
        c.attrs.insert(node, Cached { attr, until: valid_until(now, u64_at(r, 24), u32_at(r, 36)) });
        c.parents.insert(node, dir);
        Ok(node)
    }

    /// The answer to a request that made `dir/name`: its node.
    fn made(&self, dir: Ino, name: &[u8], r: &[u8]) -> Result<Ino> {
        let node = self.take_entry(dir, name, r)?;
        self.stale(dir);
        if node == 0 { Err(Errno::EIO) } else { Ok(node) }
    }

    /// Takes a `struct fuse_attr_out`: the attributes of `node`.
    fn take_attr(&self, node: Ino, r: &[u8]) -> Result<Attr> {
        if r.len() < 16 + 88 {
            return Err(Errno::EIO);
        }
        let attr = Self::parse_attr(&r[16..104]);
        let until = valid_until(self.env.now_ns(), u64_at(r, 0), u32_at(r, 8));
        self.keep_attr(node, attr, until);
        Ok(attr)
    }

    /// Sends `OPEN` or `OPENDIR` for `ino`, unless the server said it has
    /// none (`ENOSYS`, remembered: the file is then open without one).
    fn open_at_server(&self, ino: Ino, flags: OpenFlags, dir: bool) -> Result<Open> {
        let (opcode, skip) = if dir { (op::OPENDIR, &self.no_opendir) } else { (op::OPEN, &self.no_open) };
        let bits = wire_flags(flags, false);
        let mut open = Open { fh: 0, opened: false, flags: bits };
        if !skip.load(Ordering::Relaxed) {
            match self.call(opcode, ino, &Body::default().u32(bits).u32(0).0) {
                Ok(r) if r.len() >= 8 => {
                    open.fh = u64_at(&r, 0);
                    open.opened = true;
                }
                Ok(_) => return Err(Errno::EIO),
                Err(Errno::ENOSYS) => skip.store(true, Ordering::Relaxed),
                Err(e) => return Err(e),
            }
        }
        if flags.contains(OpenFlags::TRUNC) {
            self.stale(ino);
        }
        Ok(open)
    }

    /// The open file is gone: `RELEASE`, which nobody waits for.
    fn release_at_server(&self, ino: Ino, open: Open, dir: bool) {
        if open.opened {
            let body = Body::default().u64(open.fh).u32(open.flags).u32(0).u64(0);
            self.conn.post(self.env.caller(), if dir { op::RELEASEDIR } else { op::RELEASE }, ino, &body.0);
        }
    }

    fn keep_open(&self, open: Open) -> u64 {
        let handle = self.next_handle.fetch_add(1, Ordering::Relaxed);
        self.opens.lock().insert(handle, open);
        handle
    }

    fn open_of(&self, handle: u64) -> Result<Open> {
        self.opens.lock().get(&handle).copied().ok_or(Errno::EBADF)
    }

    /// Reads through `open`: as many `READ` requests as the limits make of
    /// it, until the server answers with less than asked.
    fn read_open(&self, ino: Ino, open: Open, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let mut limit = self.conn.max_read();
        if self.opts.max_read != 0 {
            limit = limit.min(self.opts.max_read);
        }
        let limit = limit.max(1) as usize;
        let mut done = 0;
        while done < buf.len() {
            let want = (buf.len() - done).min(limit);
            let body = Body::default()
                .u64(open.fh)
                .u64(offset + done as u64)
                .u32(want as u32)
                .u32(0)
                .u64(0)
                .u32(open.flags)
                .u32(0);
            let data = match self.call(op::READ, ino, &body.0) {
                Ok(d) => d,
                Err(_) if done > 0 => break,
                Err(e) => return Err(e),
            };
            let n = data.len().min(want);
            buf[done..done + n].copy_from_slice(&data[..n]);
            done += n;
            if n < want {
                break;
            }
        }
        Ok(done)
    }

    /// Writes through `open`: `WRITE` requests of at most the server's
    /// limit at consecutive offsets, until one writes less than it carried.
    fn write_open(&self, ino: Ino, open: Open, offset: u64, buf: &[u8]) -> Result<usize> {
        let limit = self.conn.max_write() as usize;
        let mut done = 0;
        while done < buf.len() {
            let chunk = &buf[done..(done + limit).min(buf.len())];
            let body = Body::default()
                .u64(open.fh)
                .u64(offset + done as u64)
                .u32(chunk.len() as u32)
                .u32(0)
                .u64(0)
                .u32(open.flags)
                .u32(0)
                .bytes(chunk);
            let r = match self.call(op::WRITE, ino, &body.0) {
                Ok(r) => r,
                Err(_) if done > 0 => break,
                Err(e) => return Err(e),
            };
            if r.len() < 4 {
                return Err(Errno::EIO);
            }
            let n = (u32_at(&r, 0) as usize).min(chunk.len());
            done += n;
            if n < chunk.len() {
                break;
            }
        }
        // The file is at least this long now; the rest is the server's.
        let end = offset + done as u64;
        if let Some(a) = self.cache.lock().attrs.get_mut(&ino)
            && a.attr.size < end
        {
            a.attr.size = end;
        }
        Ok(done)
    }

    fn setattr_with(&self, ino: Ino, set: &SetAttr, fh: Option<u64>) -> Result<Attr> {
        let mut valid = 0;
        let now = Timespec::default();
        let mut time = |t: Option<SetTime>, bit: u32, now_bit: u32| match t {
            None => now,
            Some(SetTime::Now) => {
                valid |= bit | now_bit;
                let ns = self.env.now_ns();
                Timespec::new((ns / 1_000_000_000) as i64, (ns % 1_000_000_000) as u32)
            }
            Some(SetTime::At(at)) => {
                valid |= bit;
                at
            }
        };
        let atime = time(set.atime, fattr::ATIME, fattr::ATIME_NOW);
        let mtime = time(set.mtime, fattr::MTIME, fattr::MTIME_NOW);
        let mut mode = 0;
        if let Some(perm) = set.perm {
            valid |= fattr::MODE;
            mode = self.known_attr(ino).map_or(0, |a| a.kind.mode_bits()) | u32::from(perm);
        }
        if set.uid.is_some() {
            valid |= fattr::UID;
        }
        if set.gid.is_some() {
            valid |= fattr::GID;
        }
        if set.size.is_some() {
            valid |= fattr::SIZE;
        }
        if fh.is_some() {
            valid |= fattr::FH;
        }
        let body = Body::default()
            .u32(valid)
            .u32(0)
            .u64(fh.unwrap_or(0))
            .u64(set.size.unwrap_or(0))
            .u64(0)
            .u64(atime.sec as u64)
            .u64(mtime.sec as u64)
            .u64(0)
            .u32(atime.nsec)
            .u32(mtime.nsec)
            .u32(0)
            .u32(mode)
            .u32(0)
            .u32(set.uid.unwrap_or(0))
            .u32(set.gid.unwrap_or(0))
            .u32(0);
        let r = self.call(op::SETATTR, ino, &body.0);
        if r.is_err() {
            self.stale(ino);
        }
        self.take_attr(ino, &r?)
    }

    /// Lists through `open` from `pos`: `READDIR` requests, each from the
    /// `off` of the last entry before.
    fn readdir_open(&self, dir: Ino, open: Open, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        let mut off = pos;
        loop {
            let body = Body::default().u64(open.fh).u64(off).u32(PAGE).u32(0).u64(0).u32(open.flags).u32(0);
            let r = self.call(op::READDIR, dir, &body.0)?;
            if r.is_empty() {
                return Ok(());
            }
            let mut at = 0;
            let mut any = false;
            while at + 24 <= r.len() {
                let (ino, next) = (u64_at(&r, at), u64_at(&r, at + 8));
                let n = u32_at(&r, at + 16) as usize;
                let t = u32_at(&r, at + 20);
                let name = r.get(at + 24..at + 24 + n).ok_or(Errno::EIO)?;
                let kind = FileType::from_mode(t << 12).unwrap_or(FileType::Regular);
                if !emit(&DirEntry { ino, kind, name, next }) {
                    return Ok(());
                }
                any = true;
                off = next;
                at += (24 + n).div_ceil(8) * 8;
            }
            if !any {
                return Ok(());
            }
        }
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
        let r = self.call(op::LOOKUP, dir, &Body::default().name(name).0)?;
        match self.take_entry(dir, name, &r)? {
            0 => Err(Errno::ENOENT),
            node => Ok(node),
        }
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        // The root is a directory of the mounter's from the mount on; the
        // server is asked once it has answered INIT.
        if ino == ROOT_ID && (self.mounting.load(Ordering::Relaxed) || self.conn.init_pending()) {
            return Ok(self.root_attr());
        }
        if let Some(mut a) = self.cached_attr(ino) {
            a.ino = a.ino.max(1);
            return Ok(a);
        }
        let r = self.call(op::GETATTR, ino, &[0u8; 16])?;
        self.take_attr(ino, &r)
    }

    /// A walk needs the type, which a node keeps for life; whether the
    /// caller may search the directory is the server's business unless the
    /// mount has `default_permissions` (then the kernel checks the mode,
    /// with attributes that are still good).
    fn walk_attr(&self, ino: Ino) -> Result<Attr> {
        if self.opts.default_permissions {
            return self.getattr(ino);
        }
        let known = match self.known_attr(ino) {
            Some(a) => a,
            None if ino == ROOT_ID => self.root_attr(),
            None => self.getattr(ino)?,
        };
        Ok(Attr { perm: known.perm | 0o111, ..known })
    }

    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        self.setattr_with(ino, set, None)
    }

    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino> {
        if matches!(node.kind, FileType::Directory | FileType::Symlink) {
            return Err(Errno::EINVAL);
        }
        let mode = node.kind.mode_bits() | u32::from(node.perm);
        let body = Body::default().u32(mode).u32(rdev_bits(node.rdev)).u32(self.env.caller().umask).u32(0).name(name);
        let r = self.call(op::MKNOD, dir, &body.0)?;
        self.made(dir, name, &r)
    }

    fn create_open(&self, dir: Ino, name: &[u8], node: &NewNode, flags: OpenFlags) -> Result<Option<(Ino, u64)>> {
        if node.kind != FileType::Regular || self.no_create.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let bits = wire_flags(flags, true);
        let mode = FileType::Regular.mode_bits() | u32::from(node.perm);
        let body = Body::default().u32(bits).u32(mode).u32(self.env.caller().umask).u32(0).name(name);
        let r = match self.call(op::CREATE, dir, &body.0) {
            Ok(r) => r,
            // A server without CREATE makes the file with MKNOD and opens it.
            Err(Errno::ENOSYS) => {
                self.no_create.store(true, Ordering::Relaxed);
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        if r.len() < 128 + 8 {
            return Err(Errno::EIO);
        }
        let ino = self.made(dir, name, &r)?;
        let open = Open { fh: u64_at(&r, 128), opened: true, flags: wire_flags(flags, false) };
        Ok(Some((ino, self.keep_open(open))))
    }

    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, _uid: u32, _gid: u32) -> Result<Ino> {
        let body = Body::default().u32(u32::from(perm)).u32(self.env.caller().umask).name(name);
        let r = self.call(op::MKDIR, dir, &body.0)?;
        self.made(dir, name, &r)
    }

    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], _uid: u32, _gid: u32) -> Result<Ino> {
        let r = self.call(op::SYMLINK, dir, &Body::default().name(name).name(target).0)?;
        self.made(dir, name, &r)
    }

    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()> {
        let r = self.call(op::LINK, dir, &Body::default().u64(ino).name(name).0)?;
        self.made(dir, name, &r).map(|_| ())
    }

    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()> {
        let r = self.call(op::UNLINK, dir, &Body::default().name(name).0);
        if let Some(node) = self.forget_name(dir, name) {
            self.stale(node);
        }
        self.stale(dir);
        r.map(|_| ())
    }

    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()> {
        let r = self.call(op::RMDIR, dir, &Body::default().name(name).0);
        if let Some(node) = self.forget_name(dir, name) {
            self.stale(node);
        }
        self.stale(dir);
        r.map(|_| ())
    }

    fn rename(&self, od: Ino, on: &[u8], nd: Ino, nn: &[u8], flags: RenameFlags) -> Result<()> {
        // RENAME_NOREPLACE onto a name that exists is the kernel's answer.
        if flags.noreplace {
            match self.lookup(nd, nn) {
                Ok(_) => return Err(Errno::EEXIST),
                Err(Errno::ENOENT) => {}
                Err(e) => return Err(e),
            }
        }
        let r = if flags.noreplace || flags.exchange {
            let bits = if flags.exchange { RenameFlags::LINUX_EXCHANGE } else { RenameFlags::LINUX_NOREPLACE };
            self.call(op::RENAME2, od, &Body::default().u64(nd).u32(bits).u32(0).name(on).name(nn).0)
        } else {
            self.call(op::RENAME, od, &Body::default().u64(nd).name(on).name(nn).0)
        };
        let moved = self.forget_name(od, on);
        let other = self.forget_name(nd, nn);
        for node in [moved, other].into_iter().flatten() {
            self.stale(node);
        }
        self.stale(od);
        self.stale(nd);
        if r.is_ok() {
            let mut c = self.cache.lock();
            if let Some(node) = moved {
                c.parents.insert(node, nd);
            }
            if let (true, Some(node)) = (flags.exchange, other) {
                c.parents.insert(node, od);
            }
        }
        r.map(|_| ())
    }

    fn readlink(&self, ino: Ino) -> Result<Vec<u8>> {
        if let Some(a) = self.known_attr(ino)
            && a.kind != FileType::Symlink
        {
            return Err(Errno::EINVAL);
        }
        self.call(op::READLINK, ino, &[])
    }

    /// The server keeps the nodes it handed out: nothing to pin.
    fn open(&self, _ino: Ino) -> Result<()> {
        Ok(())
    }

    fn open_handle(&self, ino: Ino, flags: OpenFlags, dir: bool) -> Result<Option<u64>> {
        Ok(Some(self.keep_open(self.open_at_server(ino, flags, dir)?)))
    }

    fn open_truncates(&self) -> OpenTrunc {
        if self.conn.atomic_trunc() { OpenTrunc::InOpen } else { OpenTrunc::Always }
    }

    fn close_handle(&self, ino: Ino, handle: u64, _flags: OpenFlags, dir: bool) {
        let open = self.opens.lock().remove(&handle);
        if let Some(open) = open {
            self.release_at_server(ino, open, dir);
        }
    }

    fn flush_handle(&self, ino: Ino, handle: u64, wait: bool) -> Result<()> {
        let open = self.open_of(handle)?;
        if self.no_flush.load(Ordering::Relaxed) {
            return Ok(());
        }
        let body = Body::default().u64(open.fh).u32(0).u32(0).u64(0);
        if !wait {
            self.conn.post(self.env.caller(), op::FLUSH, ino, &body.0);
            return Ok(());
        }
        match self.call(op::FLUSH, ino, &body.0) {
            Err(Errno::ENOSYS) => {
                self.no_flush.store(true, Ordering::Relaxed);
                Ok(())
            }
            r => r.map(|_| ()),
        }
    }

    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // A read without an open file (a mapping, a lower layer): opened
        // for the moment.
        let open = self.open_at_server(ino, OpenFlags::RDONLY, false)?;
        let r = self.read_open(ino, open, offset, buf);
        self.release_at_server(ino, open, false);
        r
    }

    fn read_handle(&self, ino: Ino, handle: u64, offset: u64, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        self.read_open(ino, self.open_of(handle)?, offset, buf)
    }

    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        let open = self.open_at_server(ino, OpenFlags::WRONLY, false)?;
        let r = self.write_open(ino, open, offset, buf);
        self.release_at_server(ino, open, false);
        r
    }

    fn write_handle(&self, ino: Ino, handle: u64, offset: u64, buf: &[u8]) -> Result<usize> {
        self.write_open(ino, self.open_of(handle)?, offset, buf)
    }

    /// The end of the file is what the server last said plus what was
    /// written since.
    fn append_handle(&self, ino: Ino, handle: u64, buf: &[u8]) -> Result<(u64, usize)> {
        let open = self.open_of(handle)?;
        let at = self.getattr(ino)?.size;
        Ok((at, self.write_open(ino, open, at, buf)?))
    }

    fn truncate_handle(&self, ino: Ino, handle: u64, size: u64) -> Result<()> {
        let open = self.open_of(handle)?;
        let set = SetAttr { size: Some(size), ..SetAttr::default() };
        self.setattr_with(ino, &set, Some(open.fh)).map(|_| ())
    }

    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        if let Some(a) = self.known_attr(dir)
            && a.kind != FileType::Directory
        {
            return Err(Errno::ENOTDIR);
        }
        let open = self.open_at_server(dir, OpenFlags::RDONLY | OpenFlags::DIRECTORY, true)?;
        let r = self.readdir_open(dir, open, pos, emit);
        self.release_at_server(dir, open, true);
        r
    }

    fn readdir_handle(
        &self,
        dir: Ino,
        handle: u64,
        pos: u64,
        emit: &mut dyn FnMut(&DirEntry<'_>) -> bool,
    ) -> Result<()> {
        self.readdir_open(dir, self.open_of(handle)?, pos, emit)
    }

    fn sync_handle(&self, ino: Ino, handle: u64, data_only: bool, dir: bool) -> Result<()> {
        let open = self.open_of(handle)?;
        let (opcode, skip) = if dir { (op::FSYNCDIR, &self.no_fsyncdir) } else { (op::FSYNC, &self.no_fsync) };
        if skip.load(Ordering::Relaxed) {
            return Ok(());
        }
        match self.call(opcode, ino, &Body::default().u64(open.fh).u32(u32::from(data_only)).u32(0).0) {
            Err(Errno::ENOSYS) => {
                skip.store(true, Ordering::Relaxed);
                Ok(())
            }
            r => r.map(|_| ()),
        }
    }

    fn statfs(&self) -> Result<FsStats> {
        let r = self.call(op::STATFS, ROOT_ID, &[])?;
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
