//! File system events, as inotify(7) describes them: an [`Inotify`]
//! instance is a queue of events plus a set of watches, each watch an
//! inode of a file system and a mask of the events wanted from it.
//!
//! - Watches are numbered per instance from 1 (a watch descriptor, `wd`);
//!   a number is never handed out twice. The same inode, however it was
//!   reached, has one watch per instance; adding it again replaces its
//!   mask, or ORs into it with `IN_MASK_ADD` (`EEXIST` with
//!   `IN_MASK_CREATE`).
//! - A watch does not keep its inode alive. Watches are found by
//!   [`WatchKey`] (the file system and the inode number) in one registry
//!   for all instances; an instance holds only the numbers of its watches.
//!   When the inode goes (its last name and its last reference are gone),
//!   its watches go too: `IN_DELETE_SELF` (if wanted), then `IN_IGNORED`.
//! - The VFS reports what happens through the functions of this module
//!   ([`entry_event`], [`self_event`], [`file_event`], [`inode_may_be_gone`]),
//!   which do nothing while no watch exists anywhere.
//! - Reading takes whole events, oldest first: a 16-byte header `{s32 wd;
//!   u32 mask; u32 cookie; u32 len}` and `len` bytes of name, NUL padded
//!   to a multiple of 16 (`len` 0 for events about the watched inode
//!   itself). A buffer too small for the next event is `EINVAL`; an
//!   empty queue `EAGAIN` (the open file waits on the [`WaitQueue`]
//!   unless non-blocking).
//! - An event equal to the last one queued (same wd, mask, cookie, name)
//!   is merged into it. A queue of [`MAX_QUEUED_EVENTS`] ends with one
//!   `IN_Q_OVERFLOW` event (wd -1) and takes nothing more until read.
//!
//! Lock order: the registry, then an instance's state; neither is held
//! while an instance is dropped or a file system is called.

use crate::file::OpenFile;
use crate::node::Location;
use crate::object::FileObject;
use crate::poll::{PollEvents, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Errno, FileType, Ino, NodeRef, Result};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};

/// Event and flag bits (UAPI `linux/inotify.h`).
pub mod mask {
    pub const IN_ACCESS: u32 = 0x0000_0001;
    pub const IN_MODIFY: u32 = 0x0000_0002;
    pub const IN_ATTRIB: u32 = 0x0000_0004;
    pub const IN_CLOSE_WRITE: u32 = 0x0000_0008;
    pub const IN_CLOSE_NOWRITE: u32 = 0x0000_0010;
    pub const IN_OPEN: u32 = 0x0000_0020;
    pub const IN_MOVED_FROM: u32 = 0x0000_0040;
    pub const IN_MOVED_TO: u32 = 0x0000_0080;
    pub const IN_CREATE: u32 = 0x0000_0100;
    pub const IN_DELETE: u32 = 0x0000_0200;
    pub const IN_DELETE_SELF: u32 = 0x0000_0400;
    pub const IN_MOVE_SELF: u32 = 0x0000_0800;
    /// Every event a watch can ask for.
    pub const IN_ALL_EVENTS: u32 = 0x0000_0fff;
    pub const IN_UNMOUNT: u32 = 0x0000_2000;
    pub const IN_Q_OVERFLOW: u32 = 0x0000_4000;
    pub const IN_IGNORED: u32 = 0x0000_8000;
    pub const IN_ONLYDIR: u32 = 0x0100_0000;
    pub const IN_DONT_FOLLOW: u32 = 0x0200_0000;
    pub const IN_EXCL_UNLINK: u32 = 0x0400_0000;
    pub const IN_MASK_CREATE: u32 = 0x1000_0000;
    pub const IN_MASK_ADD: u32 = 0x2000_0000;
    pub const IN_ISDIR: u32 = 0x4000_0000;
    pub const IN_ONESHOT: u32 = 0x8000_0000;
    /// Every bit `inotify_add_watch` accepts.
    pub const KNOWN: u32 = IN_ALL_EVENTS
        | IN_UNMOUNT
        | IN_Q_OVERFLOW
        | IN_IGNORED
        | IN_ONLYDIR
        | IN_DONT_FOLLOW
        | IN_EXCL_UNLINK
        | IN_MASK_CREATE
        | IN_MASK_ADD
        | IN_ISDIR
        | IN_ONESHOT;
}

use mask::*;

/// The size of an event's header.
pub const EVENT_HEADER: usize = 16;

/// The most events a queue holds before it overflows (Linux's default
/// `fs.inotify.max_queued_events`, inotify(7)).
pub const MAX_QUEUED_EVENTS: usize = 16384;

/// An inode a watch is on: the file system (by identity) and the inode
/// number. Bind mounts of one file system share it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct WatchKey {
    fs: usize,
    ino: Ino,
}

impl WatchKey {
    pub(crate) fn of(loc: &Location) -> WatchKey {
        WatchKey { fs: Arc::as_ptr(loc.mount.fs()) as *const () as usize, ino: loc.ino }
    }

    /// The inode `node` refers to.
    pub fn of_node(node: &NodeRef) -> WatchKey {
        WatchKey::of(node.loc())
    }
}

/// One instance's watch of an inode, as the registry holds it.
struct Watcher {
    instance: Weak<Inotify>,
    id: u64,
    wd: i32,
    mask: u32,
}

/// Every watch of every instance, by inode.
static REGISTRY: SpinMutex<BTreeMap<WatchKey, Vec<Watcher>>> = SpinMutex::new(BTreeMap::new());
/// The number of watches in [`REGISTRY`]: the event functions return at
/// once while it is 0.
static WATCHES: AtomicUsize = AtomicUsize::new(0);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_COOKIE: AtomicU32 = AtomicU32::new(1);

/// Whether any watch exists.
#[inline]
pub(crate) fn active() -> bool {
    WATCHES.load(Ordering::Relaxed) != 0
}

/// A new cookie tying an `IN_MOVED_FROM` to its `IN_MOVED_TO` (never 0).
pub(crate) fn next_cookie() -> u32 {
    loop {
        let c = NEXT_COOKIE.fetch_add(1, Ordering::Relaxed);
        if c != 0 {
            return c;
        }
    }
}

/// One queued event.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Event {
    wd: i32,
    mask: u32,
    cookie: u32,
    name: Option<Box<[u8]>>,
}

impl Event {
    fn new(wd: i32, mask: u32) -> Event {
        Event { wd, mask, cookie: 0, name: None }
    }

    /// The `len` field: the name, its NUL and padding to a multiple of 16.
    fn len_field(&self) -> usize {
        self.name.as_ref().map_or(0, |n| (n.len() + 1).div_ceil(EVENT_HEADER) * EVENT_HEADER)
    }

    fn size(&self) -> usize {
        EVENT_HEADER + self.len_field()
    }

    fn encode(&self, out: &mut [u8]) {
        let len = self.len_field();
        out[0..4].copy_from_slice(&self.wd.to_ne_bytes());
        out[4..8].copy_from_slice(&self.mask.to_ne_bytes());
        out[8..12].copy_from_slice(&self.cookie.to_ne_bytes());
        out[12..16].copy_from_slice(&(len as u32).to_ne_bytes());
        let body = &mut out[EVENT_HEADER..EVENT_HEADER + len];
        body.fill(0);
        if let Some(n) = &self.name {
            body[..n.len()].copy_from_slice(n);
        }
    }
}

#[derive(Default)]
struct State {
    events: VecDeque<Event>,
    /// The bytes the queued events take (`FIONREAD`).
    bytes: usize,
    /// The last watch number handed out.
    last_wd: i32,
    /// This instance's watches.
    watches: BTreeMap<i32, WatchKey>,
}

/// An inotify instance: what `inotify_init1` opens.
pub struct Inotify {
    id: u64,
    state: SpinMutex<State>,
    queue: Arc<WaitQueue>,
}

impl Inotify {
    pub fn new() -> Arc<Inotify> {
        Arc::new(Inotify {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            state: SpinMutex::new(State::default()),
            queue: Arc::new(WaitQueue::new()),
        })
    }

    /// `inotify_add_watch(2)` once the inode is found: the watch of `key`
    /// with `mask` (event bits and `IN_ONESHOT`, `IN_EXCL_UNLINK`;
    /// `IN_MASK_ADD` / `IN_MASK_CREATE` say how). Returns its number.
    pub fn add_watch(self: &Arc<Self>, key: WatchKey, mask: u32) -> Result<i32> {
        let wanted = mask & !(IN_MASK_ADD | IN_MASK_CREATE | IN_ONLYDIR | IN_DONT_FOLLOW);
        let mut reg = REGISTRY.lock();
        let list = reg.entry(key).or_default();
        if let Some(w) = list.iter_mut().find(|w| w.id == self.id) {
            if mask & IN_MASK_CREATE != 0 {
                return Err(Errno::EEXIST);
            }
            w.mask = if mask & IN_MASK_ADD != 0 { w.mask | wanted } else { wanted };
            return Ok(w.wd);
        }
        let wd = {
            let mut st = self.state.lock();
            if st.last_wd == i32::MAX {
                return Err(Errno::ENOSPC);
            }
            st.last_wd += 1;
            let wd = st.last_wd;
            st.watches.insert(wd, key);
            wd
        };
        list.push(Watcher { instance: Arc::downgrade(self), id: self.id, wd, mask: wanted });
        WATCHES.fetch_add(1, Ordering::Relaxed);
        Ok(wd)
    }

    /// `inotify_rm_watch(2)`: removes watch `wd` and queues its
    /// `IN_IGNORED`; `EINVAL` if the instance has no such watch.
    pub fn rm_watch(&self, wd: i32) -> Result<()> {
        {
            let mut reg = REGISTRY.lock();
            let key = self.state.lock().watches.remove(&wd).ok_or(Errno::EINVAL)?;
            if let Some(list) = reg.get_mut(&key) {
                let before = list.len();
                list.retain(|w| !(w.id == self.id && w.wd == wd));
                WATCHES.fetch_sub(before - list.len(), Ordering::Relaxed);
                if list.is_empty() {
                    reg.remove(&key);
                }
            }
        }
        self.push(Event::new(wd, IN_IGNORED));
        Ok(())
    }

    /// The watch numbers in use (for tests and `/proc` one day).
    pub fn watch_count(&self) -> usize {
        self.state.lock().watches.len()
    }

    /// The bytes queued (`FIONREAD`).
    pub fn queued_bytes(&self) -> usize {
        self.state.lock().bytes
    }

    fn forget(&self, wd: i32) {
        self.state.lock().watches.remove(&wd);
    }

    fn push(&self, event: Event) {
        {
            let mut st = self.state.lock();
            if st.events.back() == Some(&event) {
                return;
            }
            let event = if st.events.len() >= MAX_QUEUED_EVENTS {
                let overflow = Event::new(-1, IN_Q_OVERFLOW);
                if st.events.back() == Some(&overflow) {
                    return;
                }
                overflow
            } else {
                event
            };
            st.bytes += event.size();
            st.events.push_back(event);
        }
        self.queue.notify(PollEvents::READABLE);
    }
}

impl Drop for Inotify {
    fn drop(&mut self) {
        let watches = core::mem::take(&mut self.state.lock().watches);
        if watches.is_empty() {
            return;
        }
        let mut reg = REGISTRY.lock();
        for key in watches.values() {
            if let Some(list) = reg.get_mut(key) {
                let before = list.len();
                list.retain(|w| w.id != self.id);
                WATCHES.fetch_sub(before - list.len(), Ordering::Relaxed);
                if list.is_empty() {
                    reg.remove(key);
                }
            }
        }
    }
}

impl core::fmt::Debug for Inotify {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let st = self.state.lock();
        f.debug_struct("Inotify").field("events", &st.events.len()).field("watches", &st.watches.len()).finish()
    }
}

impl FileObject for Inotify {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut st = self.state.lock();
        let Some(first) = st.events.front() else { return Err(Errno::EAGAIN) };
        if first.size() > buf.len() {
            return Err(Errno::EINVAL);
        }
        let mut done = 0;
        while let Some(e) = st.events.front() {
            let size = e.size();
            if done + size > buf.len() {
                break;
            }
            e.encode(&mut buf[done..done + size]);
            done += size;
            st.bytes -= size;
            st.events.pop_front();
        }
        Ok(done)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize> {
        Err(Errno::EBADF)
    }

    fn poll(&self) -> PollEvents {
        if self.state.lock().events.is_empty() { PollEvents::NONE } else { PollEvents::READABLE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn bytes_readable(&self) -> Option<usize> {
        Some(self.queued_bytes())
    }

    fn seek_stays_at_zero(&self) -> bool {
        true
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("inotify")
    }
}

// ---- events ----------------------------------------------------------

/// Queue `mask` (with `cookie` and `name`) for every watch of `key` that
/// wants it; `IN_ONESHOT` watches go after their event.
fn notify(key: WatchKey, mask: u32, cookie: u32, name: Option<&[u8]>) {
    if !active() {
        return;
    }
    let mut hits: Vec<(Arc<Inotify>, i32, bool)> = Vec::new();
    {
        let mut reg = REGISTRY.lock();
        let Some(list) = reg.get_mut(&key) else { return };
        let before = list.len();
        list.retain(|w| {
            if w.mask & mask & IN_ALL_EVENTS == 0 {
                return w.instance.strong_count() != 0;
            }
            let Some(inst) = w.instance.upgrade() else { return false };
            let oneshot = w.mask & IN_ONESHOT != 0;
            hits.push((inst, w.wd, oneshot));
            !oneshot
        });
        WATCHES.fetch_sub(before - list.len(), Ordering::Relaxed);
        if list.is_empty() {
            reg.remove(&key);
        }
    }
    for (inst, wd, oneshot) in hits {
        inst.push(Event { wd, mask, cookie, name: name.map(Box::from) });
        if oneshot {
            inst.forget(wd);
            inst.push(Event::new(wd, IN_IGNORED));
        }
    }
}

/// The inode of `key` is gone: every watch on it ends with
/// `IN_DELETE_SELF` (if wanted) and `IN_IGNORED`.
fn inode_gone(key: WatchKey) {
    let Some(list) = REGISTRY.lock().remove(&key) else { return };
    WATCHES.fetch_sub(list.len(), Ordering::Relaxed);
    for w in list {
        let Some(inst) = w.instance.upgrade() else { continue };
        if w.mask & IN_DELETE_SELF != 0 {
            inst.push(Event::new(w.wd, IN_DELETE_SELF));
        }
        inst.forget(w.wd);
        inst.push(Event::new(w.wd, IN_IGNORED));
    }
}

fn isdir(is_dir: bool) -> u32 {
    if is_dir { IN_ISDIR } else { 0 }
}

/// An event about the entry `name` of directory `dir` (named, with
/// `IN_ISDIR` when the entry is a directory).
pub(crate) fn entry_event(dir: &Location, name: &[u8], mask: u32, is_dir: bool, cookie: u32) {
    if active() {
        notify(WatchKey::of(dir), mask | isdir(is_dir), cookie, Some(name));
    }
}

/// An event about the inode `loc` itself (no name); `IN_ISDIR` for a
/// directory, except on `IN_DELETE_SELF` and `IN_MOVE_SELF`.
pub(crate) fn self_event(loc: &Location, mask: u32, is_dir: bool) {
    if active() {
        let dir = is_dir && mask & (IN_DELETE_SELF | IN_MOVE_SELF) == 0;
        notify(WatchKey::of(loc), mask | isdir(dir), 0, None);
    }
}

/// An event about the file `file` is open on: for its own watches and,
/// named, for the watches of the directory it was opened in.
pub(crate) fn file_event(file: &OpenFile, mask: u32) {
    if !active() {
        return;
    }
    let is_dir = file.kind() == FileType::Directory;
    self_event(file.node().loc(), mask, is_dir);
    if let Some(origin) = file.origin()
        && let Some(mount) = origin.mount.upgrade()
    {
        entry_event(&Location { mount, ino: origin.dir }, &origin.name, mask, is_dir, 0);
    }
}

/// Called once something that may have freed `loc`'s inode is done (its
/// last name removed, a reference dropped): if the inode is watched and
/// gone, its watches end.
pub(crate) fn inode_may_be_gone(loc: &Location) {
    if !active() {
        return;
    }
    let key = WatchKey::of(loc);
    if !REGISTRY.lock().contains_key(&key) {
        return;
    }
    if loc.attr().is_err() {
        inode_gone(key);
    }
}

/// After the name `name` of `victim` in `dir` was removed (`unlink`,
/// `rmdir`): `IN_ATTRIB` for the inode if it lives on (its link count
/// changed), `IN_DELETE` for the directory, and the end of the inode's
/// watches if it is gone.
pub(crate) fn removed(dir: &Location, name: &[u8], victim: &Location, is_dir: bool) {
    if !active() {
        return;
    }
    let alive = victim.attr().is_ok();
    if alive && !is_dir {
        self_event(victim, IN_ATTRIB, false);
    }
    entry_event(dir, name, IN_DELETE, is_dir, 0);
    if !alive {
        inode_gone(WatchKey::of(victim));
    }
}
