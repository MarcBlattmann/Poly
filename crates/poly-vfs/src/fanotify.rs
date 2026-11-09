//! File system events for fanotify(7): a [`Fanotify`] group is a queue of
//! events plus marks, each mark an inode, a mount or a whole file system
//! with the events wanted from it (and the events to ignore).
//!
//! - Marks are found in one registry for all groups, by [`MarkKey`]; a
//!   group holds the keys of its marks. A mark does not keep its object
//!   alive. Dropping the group (its last descriptor closed) removes them.
//! - The VFS reports what happens through the same calls as for inotify
//!   (`inotify.rs` forwards them here): an event on an open file
//!   ([`file_event`]: the file's own marks, its directory's marks that ask
//!   for `FAN_EVENT_ON_CHILD`, its mount's and its file system's), an
//!   event on an inode ([`self_event`]) and a directory entry event
//!   ([`entry_event`]: `FAN_CREATE`, `FAN_DELETE`, `FAN_MOVED_*`, which
//!   only groups that report file ids can ask for). Events on a
//!   directory itself are only reported to marks that have `FAN_ONDIR`.
//! - An event that equals the last one queued in its object and acting
//!   process is merged into it (masks or-ed), as fanotify_mark's spec
//!   sheet observed for open + modify + close.
//! - Reading takes whole events: `struct fanotify_event_metadata` (24
//!   bytes, UAPI `linux/fanotify.h`), with `fd` a new descriptor for the
//!   object in the reader's process (opened with the group's
//!   `event_f_flags`, without causing events itself) or, in a group that
//!   reports file ids, `FAN_NOFD` and info records naming the object by a
//!   file handle. The descriptors come from the kernel's [`Hooks`].
//!
//! Lock order: the registry, then a group's state; neither is held while a
//! group is dropped, a file is opened for an event or a file system is
//! called for anything but attributes.

use crate::file::{Backend, OpenFile, OpenFlags};
use crate::inotify::WatchKey;
use crate::mount::MountId;
use crate::node::Location;
use crate::object::FileObject;
use crate::poll::{PollEvents, WaitQueue};
use crate::sync::SpinMutex;
use crate::{Errno, FileType, NodeRef, Result, Vfs};
use alloc::boxed::Box;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// Flags of `fanotify_init` and bits of a mark's mask (UAPI
/// `linux/fanotify.h`).
pub mod bits {
    pub const FAN_CLOEXEC: u32 = 0x0000_0001;
    pub const FAN_NONBLOCK: u32 = 0x0000_0002;
    pub const FAN_CLASS_NOTIF: u32 = 0x0000_0000;
    pub const FAN_CLASS_CONTENT: u32 = 0x0000_0004;
    pub const FAN_CLASS_PRE_CONTENT: u32 = 0x0000_0008;
    pub const FAN_ALL_CLASS_BITS: u32 = FAN_CLASS_CONTENT | FAN_CLASS_PRE_CONTENT;
    pub const FAN_UNLIMITED_QUEUE: u32 = 0x0000_0010;
    pub const FAN_UNLIMITED_MARKS: u32 = 0x0000_0020;
    pub const FAN_ENABLE_AUDIT: u32 = 0x0000_0040;
    pub const FAN_REPORT_PIDFD: u32 = 0x0000_0080;
    pub const FAN_REPORT_TID: u32 = 0x0000_0100;
    pub const FAN_REPORT_FID: u32 = 0x0000_0200;
    pub const FAN_REPORT_DIR_FID: u32 = 0x0000_0400;
    pub const FAN_REPORT_NAME: u32 = 0x0000_0800;
    pub const FAN_REPORT_TARGET_FID: u32 = 0x0000_1000;
    pub const FAN_REPORT_FD_ERROR: u32 = 0x0000_2000;
    pub const FAN_REPORT_MNT: u32 = 0x0000_4000;

    pub const FAN_ACCESS: u32 = 0x0000_0001;
    pub const FAN_MODIFY: u32 = 0x0000_0002;
    pub const FAN_ATTRIB: u32 = 0x0000_0004;
    pub const FAN_CLOSE_WRITE: u32 = 0x0000_0008;
    pub const FAN_CLOSE_NOWRITE: u32 = 0x0000_0010;
    pub const FAN_OPEN: u32 = 0x0000_0020;
    pub const FAN_MOVED_FROM: u32 = 0x0000_0040;
    pub const FAN_MOVED_TO: u32 = 0x0000_0080;
    pub const FAN_CREATE: u32 = 0x0000_0100;
    pub const FAN_DELETE: u32 = 0x0000_0200;
    pub const FAN_DELETE_SELF: u32 = 0x0000_0400;
    pub const FAN_MOVE_SELF: u32 = 0x0000_0800;
    pub const FAN_OPEN_EXEC: u32 = 0x0000_1000;
    pub const FAN_Q_OVERFLOW: u32 = 0x0000_4000;
    pub const FAN_FS_ERROR: u32 = 0x0000_8000;
    pub const FAN_OPEN_PERM: u32 = 0x0001_0000;
    pub const FAN_ACCESS_PERM: u32 = 0x0002_0000;
    pub const FAN_OPEN_EXEC_PERM: u32 = 0x0004_0000;
    pub const FAN_EVENT_ON_CHILD: u32 = 0x0800_0000;
    pub const FAN_RENAME: u32 = 0x1000_0000;
    pub const FAN_ONDIR: u32 = 0x4000_0000;

    /// Directory entry events: only groups reporting file ids get them.
    pub const DIRENT_EVENTS: u32 = FAN_MOVED_FROM | FAN_MOVED_TO | FAN_CREATE | FAN_DELETE;
    /// Permission events: only content classes may ask for them.
    pub const PERM_EVENTS: u32 = FAN_OPEN_PERM | FAN_ACCESS_PERM | FAN_OPEN_EXEC_PERM;
    /// Every event a notification can be about (not the modifiers).
    pub const EVENTS: u32 = 0x0000_1fff;

    pub const FANOTIFY_METADATA_VERSION: u8 = 3;
    pub const FAN_NOFD: i32 = -1;
    pub const FAN_EVENT_INFO_TYPE_FID: u8 = 1;
    pub const FAN_EVENT_INFO_TYPE_DFID_NAME: u8 = 2;
    pub const FAN_EVENT_INFO_TYPE_DFID: u8 = 3;
}

use bits::*;

/// The size of `struct fanotify_event_metadata`.
pub const METADATA_LEN: usize = 24;

/// `FILEID_INO32_GEN` (UAPI `linux/exportfs.h`): the handle type of the
/// file handles in info records: a 32-bit inode number and a generation.
pub const FILEID_INO32_GEN: i32 = 1;

/// The most events a queue holds without `FAN_UNLIMITED_QUEUE`
/// (fanotify(7): "16384 events").
pub const MAX_QUEUED_EVENTS: usize = 16384;

/// What the kernel provides: who is acting (the pid events carry) and how
/// to give the reader a descriptor for an event's object.
#[derive(Clone, Copy, Debug)]
pub struct Hooks {
    /// The process id of the running thread, as events report it.
    pub actor_pid: fn() -> i32,
    /// Put an open file on `node` with `flags` on the lowest free
    /// descriptor of the running process (`O_CLOEXEC` in `flags` for the
    /// descriptor flag); the file must cause no events
    /// ([`Vfs::open_node_quiet`]).
    pub open_event_fd: fn(&NodeRef, OpenFlags) -> Result<i32>,
}

static HOOKS: SpinMutex<Option<Hooks>> = SpinMutex::new(None);

/// Tell the VFS how to report the acting process and open event files.
pub fn set_hooks(hooks: Hooks) {
    *HOOKS.lock() = Some(hooks);
}

fn hooks() -> Option<Hooks> {
    *HOOKS.lock()
}

/// What a mark is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MarkKey {
    Inode(WatchKey),
    /// A mount, by its id and the identity of the file system it shows
    /// (ids are only unique within one VFS; host tests run several).
    Mount(MountId, usize),
    /// A file system, by the identity of its instance.
    Filesystem(usize),
}

impl MarkKey {
    /// The inode `node` refers to.
    pub fn inode(node: &NodeRef) -> MarkKey {
        MarkKey::Inode(WatchKey::of_node(node))
    }

    /// The mount `node` was reached through (`FAN_MARK_MOUNT`).
    pub fn mount(node: &NodeRef) -> MarkKey {
        MarkKey::Mount(node.mount().id(), Arc::as_ptr(node.mount().fs()) as *const () as usize)
    }

    /// The file system `node` is on (`FAN_MARK_FILESYSTEM`).
    pub fn filesystem(node: &NodeRef) -> MarkKey {
        MarkKey::Filesystem(Arc::as_ptr(node.mount().fs()) as *const () as usize)
    }
}

/// How `fanotify_mark` changes a mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkOp {
    Add,
    Remove,
}

/// One group's mark on an object.
struct Mark {
    group: Weak<Fanotify>,
    id: u64,
    mask: u32,
    ignored: u32,
}

static REGISTRY: SpinMutex<BTreeMap<MarkKey, Vec<Mark>>> = SpinMutex::new(BTreeMap::new());
/// Marks in [`REGISTRY`]: the event functions return at once while 0.
static MARKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Whether any mark exists.
#[inline]
pub(crate) fn active() -> bool {
    MARKS.load(Ordering::Relaxed) != 0
}

/// A file id info record's contents.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Fid {
    info_type: u8,
    fsid: u64,
    /// `struct file_handle`'s `handle_type` and bytes: the handle
    /// name_to_handle_at(2) gives for the object (fanotify_init's spec
    /// sheet), or for a file system without handles the 32-bit inode
    /// number with generation 0 ([`FILEID_INO32_GEN`]).
    handle_type: i32,
    handle: Box<[u8]>,
    name: Option<Box<[u8]>>,
}

impl Fid {
    fn of(loc: &Location, info_type: u8, name: Option<&[u8]>) -> Fid {
        let dev = loc.mount.dev();
        let (handle_type, handle) = match loc.mount.fs().handle(loc.ino) {
            Ok(h) => (h.kind, Box::from(h.bytes())),
            Err(_) => {
                let mut b = [0u8; 8];
                b[..4].copy_from_slice(&(loc.ino as u32).to_le_bytes());
                (FILEID_INO32_GEN, Box::from(&b[..]))
            }
        };
        Fid {
            info_type,
            // statfs(2)'s `f_fsid`: the device number, major then minor.
            fsid: u64::from(dev.major) | (u64::from(dev.minor) << 32),
            handle_type,
            handle,
            name: name.map(Box::from),
        }
    }

    /// `{u8 info_type; u8 pad; u16 len}`, the 8-byte fsid, `struct
    /// file_handle {u32 handle_bytes; s32 handle_type}` and the handle,
    /// then for a `DFID_NAME` record the NUL-terminated name; the record
    /// is padded to a multiple of 4.
    fn size(&self) -> usize {
        let base = 4 + 8 + 8 + self.handle.len();
        match &self.name {
            Some(n) => (base + n.len() + 1).next_multiple_of(4),
            None => base.next_multiple_of(4),
        }
    }

    fn encode(&self, out: &mut [u8]) {
        let len = self.size();
        out[..len].fill(0);
        out[0] = self.info_type;
        out[2..4].copy_from_slice(&(len as u16).to_le_bytes());
        out[4..12].copy_from_slice(&self.fsid.to_le_bytes());
        out[12..16].copy_from_slice(&(self.handle.len() as u32).to_le_bytes());
        out[16..20].copy_from_slice(&self.handle_type.to_le_bytes());
        let end = 20 + self.handle.len();
        out[20..end].copy_from_slice(&self.handle);
        if let Some(n) = &self.name {
            out[end..end + n.len()].copy_from_slice(n);
        }
    }
}

/// One queued event.
struct Event {
    mask: u32,
    pid: i32,
    /// What merging compares: the object (and, for entry events, the
    /// directory with the name).
    key: WatchKey,
    /// The object, for groups that hand out descriptors.
    node: Option<NodeRef>,
    fids: Vec<Fid>,
}

impl Event {
    fn size(&self) -> usize {
        METADATA_LEN + self.fids.iter().map(Fid::size).sum::<usize>()
    }

    fn encode(&self, fd: i32, out: &mut [u8]) {
        let len = self.size();
        out[0..4].copy_from_slice(&(len as u32).to_le_bytes());
        out[4] = FANOTIFY_METADATA_VERSION;
        out[5] = 0;
        out[6..8].copy_from_slice(&(METADATA_LEN as u16).to_le_bytes());
        out[8..16].copy_from_slice(&u64::from(self.mask).to_le_bytes());
        out[16..20].copy_from_slice(&fd.to_le_bytes());
        out[20..24].copy_from_slice(&self.pid.to_le_bytes());
        let mut at = METADATA_LEN;
        for f in &self.fids {
            f.encode(&mut out[at..]);
            at += f.size();
        }
    }
}

#[derive(Default)]
struct State {
    events: VecDeque<Event>,
    /// The keys of this group's marks.
    marks: Vec<MarkKey>,
}

/// A fanotify notification group: what `fanotify_init` opens.
pub struct Fanotify {
    id: u64,
    flags: u32,
    event_flags: OpenFlags,
    privileged: bool,
    state: SpinMutex<State>,
    queue: Arc<WaitQueue>,
}

impl Fanotify {
    /// A group with `fanotify_init`'s (validated) `flags` and
    /// `event_f_flags`; `privileged` if its creator had `CAP_SYS_ADMIN`.
    pub fn new(flags: u32, event_flags: OpenFlags, privileged: bool) -> Arc<Fanotify> {
        Arc::new(Fanotify {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            flags,
            event_flags,
            privileged,
            state: SpinMutex::new(State::default()),
            queue: Arc::new(WaitQueue::new()),
        })
    }

    /// `fanotify_init`'s flags.
    pub fn flags(&self) -> u32 {
        self.flags
    }

    /// Whether the group was made with `CAP_SYS_ADMIN` (only such a group
    /// may mark mounts and file systems).
    pub fn privileged(&self) -> bool {
        self.privileged
    }

    /// Whether events name their objects by file ids rather than
    /// descriptors.
    pub fn reports_fid(&self) -> bool {
        self.flags & (FAN_REPORT_FID | FAN_REPORT_DIR_FID) != 0
    }

    /// `fanotify_mark` with `FAN_MARK_ADD` / `FAN_MARK_REMOVE` once the
    /// object is found: `mask` goes into (or out of) the mark's event mask,
    /// or with `ignored` its ignored mask. Removing from a mark the group
    /// does not have is `ENOENT`; a mark left with nothing goes.
    pub fn mark(self: &Arc<Self>, key: MarkKey, op: MarkOp, mask: u32, ignored: bool) -> Result<()> {
        let mut reg = REGISTRY.lock();
        let list = reg.entry(key).or_default();
        let pos = list.iter().position(|m| m.id == self.id);
        match (op, pos) {
            (MarkOp::Add, Some(i)) => {
                let m = &mut list[i];
                if ignored {
                    m.ignored |= mask;
                } else {
                    m.mask |= mask;
                }
            }
            (MarkOp::Add, None) => {
                let (m, i) = if ignored { (0, mask) } else { (mask, 0) };
                list.push(Mark { group: Arc::downgrade(self), id: self.id, mask: m, ignored: i });
                MARKS.fetch_add(1, Ordering::Relaxed);
                self.state.lock().marks.push(key);
            }
            (MarkOp::Remove, Some(i)) => {
                let m = &mut list[i];
                if ignored {
                    m.ignored &= !mask;
                } else {
                    m.mask &= !mask;
                }
                if m.mask & !(FAN_ONDIR | FAN_EVENT_ON_CHILD) == 0 && m.ignored & EVENTS == 0 {
                    list.remove(i);
                    MARKS.fetch_sub(1, Ordering::Relaxed);
                    self.state.lock().marks.retain(|k| *k != key);
                }
            }
            (MarkOp::Remove, None) => {
                if list.is_empty() {
                    reg.remove(&key);
                }
                return Err(Errno::ENOENT);
            }
        }
        if reg.get(&key).is_some_and(Vec::is_empty) {
            reg.remove(&key);
        }
        Ok(())
    }

    /// `FAN_MARK_FLUSH`: remove the group's inode marks, or with `mounts`
    /// its mount marks, or with `filesystems` its file system marks.
    pub fn flush(&self, mounts: bool, filesystems: bool) {
        let mut reg = REGISTRY.lock();
        let mut st = self.state.lock();
        st.marks.retain(|key| {
            let hit = match key {
                MarkKey::Inode(_) => !mounts && !filesystems,
                MarkKey::Mount(..) => mounts,
                MarkKey::Filesystem(_) => filesystems,
            };
            if hit && let Some(list) = reg.get_mut(key) {
                let before = list.len();
                list.retain(|m| m.id != self.id);
                MARKS.fetch_sub(before - list.len(), Ordering::Relaxed);
                if list.is_empty() {
                    reg.remove(key);
                }
            }
            !hit
        });
    }

    /// The number of marks the group has.
    pub fn mark_count(&self) -> usize {
        self.state.lock().marks.len()
    }

    /// The number of events queued.
    pub fn queued(&self) -> usize {
        self.state.lock().events.len()
    }

    fn push(&self, event: Event) {
        {
            let mut st = self.state.lock();
            if let Some(last) = st.events.back_mut()
                && last.key == event.key
                && last.pid == event.pid
                && last.fids == event.fids
            {
                last.mask |= event.mask;
                return;
            }
            if self.flags & FAN_UNLIMITED_QUEUE == 0 && st.events.len() >= MAX_QUEUED_EVENTS {
                if st.events.back().is_some_and(|e| e.mask == FAN_Q_OVERFLOW) {
                    return;
                }
                let key = event.key;
                st.events.push_back(Event { mask: FAN_Q_OVERFLOW, pid: 0, key, node: None, fids: Vec::new() });
            } else {
                st.events.push_back(event);
            }
        }
        self.queue.notify(PollEvents::READABLE);
    }
}

impl Drop for Fanotify {
    fn drop(&mut self) {
        let marks = core::mem::take(&mut self.state.lock().marks);
        if marks.is_empty() {
            return;
        }
        let mut reg = REGISTRY.lock();
        for key in &marks {
            if let Some(list) = reg.get_mut(key) {
                let before = list.len();
                list.retain(|m| m.id != self.id);
                MARKS.fetch_sub(before - list.len(), Ordering::Relaxed);
                if list.is_empty() {
                    reg.remove(key);
                }
            }
        }
    }
}

impl core::fmt::Debug for Fanotify {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let st = self.state.lock();
        f.debug_struct("Fanotify").field("events", &st.events.len()).field("marks", &st.marks.len()).finish()
    }
}

impl FileObject for Fanotify {
    /// Whole events, oldest first; `EINVAL` if the first does not fit,
    /// `EAGAIN` if there is none. In a descriptor-reporting group each
    /// event's object is opened for the reader first; if that fails for
    /// the first event the error is the read's.
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut done = 0;
        loop {
            let event = {
                let mut st = self.state.lock();
                let Some(first) = st.events.front() else {
                    return if done == 0 { Err(Errno::EAGAIN) } else { Ok(done) };
                };
                let size = first.size();
                if done + size > buf.len() {
                    return if done == 0 { Err(Errno::EINVAL) } else { Ok(done) };
                }
                st.events.pop_front()
            };
            let Some(event) = event else { return Ok(done) };
            let fd = match &event.node {
                Some(node) if !self.reports_fid() => {
                    let open = hooks().map(|h| h.open_event_fd).ok_or(Errno::ENOSYS);
                    let flags = self.event_flags;
                    match open.and_then(|f| f(node, flags)) {
                        Ok(fd) => fd,
                        Err(e) => {
                            // Put it back for the next read.
                            self.state.lock().events.push_front(event);
                            return if done == 0 { Err(e) } else { Ok(done) };
                        }
                    }
                }
                _ => FAN_NOFD,
            };
            let size = event.size();
            event.encode(fd, &mut buf[done..done + size]);
            done += size;
        }
    }

    /// fanotify_init's spec sheet: "write is EINVAL".
    fn write(&self, _buf: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn poll(&self) -> PollEvents {
        if self.state.lock().events.is_empty() { PollEvents::NONE } else { PollEvents::READABLE }
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn bytes_readable(&self) -> Option<usize> {
        Some(self.state.lock().events.iter().map(Event::size).sum())
    }

    fn anon_name(&self) -> Option<&'static str> {
        Some("[fanotify]")
    }
}

// ---- events ----------------------------------------------------------

/// What one group's marks say about an event.
#[derive(Default)]
struct Hit {
    mask: u32,
    ignored: u32,
    /// The marks that wanted it have `FAN_ONDIR`.
    ondir: bool,
}

/// Queue `mask` about `obj` (a directory if `is_dir`) for every group whose
/// marks want it. `parent` is the directory and name the object was found
/// by, whose marks with `FAN_EVENT_ON_CHILD` also count; `entry` says the
/// event is about the entry `name` of directory `obj`.
fn dispatch(obj: &Location, is_dir: bool, parent: Option<(&Location, &[u8])>, entry: Option<&[u8]>, mask: u32) {
    if !active() || mask & EVENTS == 0 {
        return;
    }
    let mut hits: BTreeMap<u64, (Arc<Fanotify>, Hit)> = BTreeMap::new();
    {
        let reg = REGISTRY.lock();
        let mut take = |key: MarkKey, child_rule: bool| {
            let Some(list) = reg.get(&key) else { return };
            for m in list {
                if child_rule && m.mask & FAN_EVENT_ON_CHILD == 0 {
                    continue;
                }
                let Some(group) = m.group.upgrade() else { continue };
                let h = &mut hits.entry(m.id).or_insert_with(|| (group, Hit::default())).1;
                if m.mask & mask != 0 {
                    h.mask |= m.mask & mask;
                    h.ondir |= m.mask & FAN_ONDIR != 0;
                }
                if !child_rule {
                    h.ignored |= m.ignored;
                }
            }
        };
        take(MarkKey::Inode(WatchKey::of(obj)), false);
        if let Some((dir, _)) = parent
            && entry.is_none()
        {
            take(MarkKey::Inode(WatchKey::of(dir)), true);
        }
        take(MarkKey::Mount(obj.mount.id, Arc::as_ptr(obj.mount.fs()) as *const () as usize), false);
        take(MarkKey::Filesystem(Arc::as_ptr(obj.mount.fs()) as *const () as usize), false);
    }
    if hits.is_empty() {
        return;
    }
    let pid = hooks().map_or(0, |h| (h.actor_pid)());
    for (_, (group, hit)) in hits {
        let wanted = hit.mask & !hit.ignored & EVENTS;
        if wanted == 0 || (is_dir && !hit.ondir) {
            continue;
        }
        let fids = if group.reports_fid() { fids_for(&group, obj, parent, entry) } else { Vec::new() };
        let node = if group.reports_fid() { None } else { obj.clone().pin().ok() };
        if node.is_none() && !group.reports_fid() {
            continue;
        }
        let mask = if group.reports_fid() && is_dir { wanted | FAN_ONDIR } else { wanted };
        group.push(Event { mask, pid, key: WatchKey::of(obj), node, fids });
    }
}

/// The info records of an event for a group that reports file ids.
fn fids_for(group: &Fanotify, obj: &Location, parent: Option<(&Location, &[u8])>, entry: Option<&[u8]>) -> Vec<Fid> {
    let mut out = Vec::new();
    let flags = group.flags;
    if flags & FAN_REPORT_DIR_FID != 0 {
        let (dir, name) = match (entry, parent) {
            (Some(name), _) => (obj, Some(name)),
            (None, Some((dir, name))) => (dir, Some(name)),
            (None, None) => (obj, None),
        };
        if flags & FAN_REPORT_NAME != 0 && name.is_some() {
            out.push(Fid::of(dir, FAN_EVENT_INFO_TYPE_DFID_NAME, name));
        } else {
            out.push(Fid::of(dir, FAN_EVENT_INFO_TYPE_DFID, None));
        }
        if flags & FAN_REPORT_FID != 0 && entry.is_none() && parent.is_some() {
            out.push(Fid::of(obj, FAN_EVENT_INFO_TYPE_FID, None));
        }
    } else {
        // FAN_REPORT_FID alone: entry events name the directory.
        out.push(Fid::of(obj, FAN_EVENT_INFO_TYPE_FID, None));
    }
    out
}

/// An event on the open file `file`: for the marks of its inode, of the
/// directory it was opened in (`FAN_EVENT_ON_CHILD`), its mount and its
/// file system.
pub(crate) fn file_event(file: &OpenFile, mask: u32) {
    if !active() {
        return;
    }
    let is_dir = file.kind() == FileType::Directory;
    let loc = file.node().loc();
    let origin = file.origin().and_then(|o| o.mount.upgrade().map(|m| (Location { mount: m, ino: o.dir }, &o.name)));
    match &origin {
        Some((dir, name)) => dispatch(loc, is_dir, Some((dir, name)), None, mask),
        None => dispatch(loc, is_dir, None, None, mask),
    }
}

/// An event on the inode `loc` itself.
pub(crate) fn self_event(loc: &Location, mask: u32, is_dir: bool) {
    if active() {
        dispatch(loc, is_dir, None, None, mask);
    }
}

/// A directory entry event (`FAN_CREATE`, `FAN_DELETE`, `FAN_MOVED_*`)
/// about `name` in `dir`; `is_dir` if the entry is a directory.
pub(crate) fn entry_event(dir: &Location, name: &[u8], mask: u32, is_dir: bool) {
    if !active() || mask & DIRENT_EVENTS == 0 {
        return;
    }
    // The directory is the object; whether the entry is a directory decides
    // `FAN_ONDIR`.
    dispatch_entry(dir, name, mask & DIRENT_EVENTS, is_dir);
}

fn dispatch_entry(dir: &Location, name: &[u8], mask: u32, entry_is_dir: bool) {
    // `dispatch` checks `FAN_ONDIR` against its `is_dir`, which for entry
    // events is the entry's kind.
    dispatch(dir, entry_is_dir, None, Some(name), mask);
}

impl Vfs {
    /// An open file on `node` with `flags` that causes no events (neither
    /// its opening nor its use or close): the descriptors fanotify hands
    /// to its readers. No permission check: the group's creator was
    /// allowed to watch the object.
    pub fn open_node_quiet(&self, node: &NodeRef, flags: OpenFlags) -> Result<Arc<OpenFile>> {
        let attr = node.attr()?;
        let backend = match attr.kind {
            FileType::Regular | FileType::Directory => Backend::Inode,
            FileType::CharDevice => Backend::Device(self.char_device(attr.rdev).ok_or(Errno::ENXIO)?),
            _ => return Err(Errno::ENXIO),
        };
        let flags = flags.without(OpenFlags::CREAT | OpenFlags::TRUNC | OpenFlags::EXCL | OpenFlags::CLOEXEC);
        Ok(Arc::new(OpenFile::new(node.clone(), attr.kind, backend, flags).quiet()))
    }
}
