//! POSIX message queues (mq_overview(7)): the named queues of an IPC
//! namespace and what one queue holds.
//!
//! - [`MqTable`]: the names of one IPC namespace (`with_names`), each
//!   naming a [`Queue`]. `mq_unlink` removes only the name: descriptors
//!   already open keep the queue (an `Arc`) and its messages.
//! - [`Queue`]: messages ordered by priority, highest first, FIFO within a
//!   priority; at most `maxmsg` of at most `msgsize` bytes. A receiver that
//!   finds the queue empty or a sender that finds it full gets a ticket and
//!   sleeps; the other side completes the ticket under the queue's lock (a
//!   message handed straight to the first blocked receiver, the first
//!   blocked sender's message queued as soon as there is room), so a
//!   woken caller never races a newcomer for its message.
//! - The one notification registration (`mq_notify`): taken out when a
//!   message reaches an empty queue nobody is blocked on, removed when its
//!   process closes the open file it registered through or exits.
//!
//! Like the System V tables ([`super::msg`]), nothing here blocks or calls
//! the kernel: the handlers (`sys::mqueue`) copy user memory, sleep, wake
//! the threads an operation completed and send the notification signal.

use crate::errno::Errno;
use crate::ns::IpcNs;
use crate::ns::lock::Mutex;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::any::Any;
use core::sync::atomic::{AtomicU64, Ordering};

/// Priorities are below this (`MQ_PRIO_MAX`, `limits.h` / mq_send(3)).
pub const MQ_PRIO_MAX: u32 = 32768;
/// The longest queue name, without the leading slash (`NAME_MAX`).
pub const NAME_MAX: usize = 255;
/// `fs.mqueue.msg_default` / `msgsize_default`: a queue made without an
/// attribute structure (mq_overview(7) defaults).
pub const DEFAULT_MAXMSG: i64 = 10;
pub const DEFAULT_MSGSIZE: i64 = 8192;
/// `fs.mqueue.msg_max` / `msgsize_max`: the most a caller without
/// `CAP_SYS_RESOURCE` may ask for (mq_overview(7) defaults).
pub const MSG_MAX: i64 = 10;
pub const MSGSIZE_MAX: i64 = 8192;
/// The ceilings even with `CAP_SYS_RESOURCE` (mq_overview(7): msg_max
/// "ceiling 65536" (`HARD_MSGMAX`), msgsize_max "ceiling 16777216").
pub const HARD_MSGMAX: i64 = 65536;
pub const HARD_MSGSIZEMAX: i64 = 16 * 1024 * 1024;
/// `fs.mqueue.queues_max` (mq_overview(7) default): queues of a namespace
/// a caller without `CAP_SYS_RESOURCE` may make.
pub const QUEUES_MAX: usize = 256;

/// `sigev_notify` values (`asm-generic/siginfo.h`).
pub const SIGEV_SIGNAL: i32 = 0;
pub const SIGEV_NONE: i32 = 1;
pub const SIGEV_THREAD: i32 = 2;
/// `si_code` of a message queue notification (`asm-generic/siginfo.h`).
pub const SI_MESGQ: i32 = -3;

/// `struct mq_attr` (`linux/mqueue.h`): four `long`s and four reserved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MqAttr {
    pub flags: i64,
    pub maxmsg: i64,
    pub msgsize: i64,
    pub curmsgs: i64,
}

impl MqAttr {
    /// `sizeof(struct mq_attr)`.
    pub const SIZE: usize = 64;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> MqAttr {
        let long = |i: usize| i64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap_or([0; 8]));
        MqAttr { flags: long(0), maxmsg: long(1), msgsize: long(2), curmsgs: long(3) }
    }

    /// The reserved words are zero.
    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        for (i, v) in [self.flags, self.maxmsg, self.msgsize, self.curmsgs].iter().enumerate() {
            b[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        b
    }
}

/// `struct sigevent` (`asm-generic/siginfo.h`), the part mq_notify reads:
/// `sigev_value` (8 bytes), `sigev_signo`, `sigev_notify`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SigEvent {
    pub value: u64,
    pub signo: i32,
    pub notify: i32,
}

impl SigEvent {
    /// `sizeof(struct sigevent)`.
    pub const SIZE: usize = 64;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> SigEvent {
        let int = |o: usize| i32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]]);
        SigEvent { value: u64::from_le_bytes(b[0..8].try_into().unwrap_or([0; 8])), signo: int(8), notify: int(12) }
    }

    /// mq_notify(2): `EINVAL` for an unknown `sigev_notify` and, for
    /// `SIGEV_SIGNAL`, a signal outside 0..=64 (0 is accepted).
    pub fn validate(&self) -> Result<(), Errno> {
        match self.notify {
            SIGEV_SIGNAL if !(0..=64).contains(&self.signo) => Err(Errno::EINVAL),
            SIGEV_SIGNAL | SIGEV_NONE | SIGEV_THREAD => Ok(()),
            _ => Err(Errno::EINVAL),
        }
    }
}

/// The process registered for a queue's notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Registration {
    /// The registered process.
    pub pid: i32,
    /// `sigev_notify`, `sigev_signo`, `sigev_value`.
    pub event: SigEvent,
    /// The open file it registered through ([`new_file_id`]).
    pub file: u64,
}

/// What the kernel attaches to a queue to hear about changes of what it
/// is ready for (its `poll` wait queue).
pub trait QueueWatch: Any + Send + Sync {
    /// The queue's messages changed.
    fn changed(&self);
    /// For the kernel to get its own type back.
    fn as_any(&self) -> &dyn Any;
}

/// What a completed ticket carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Done {
    /// A receiver's message and its priority.
    Received { prio: u32, data: Vec<u8> },
    /// A sender's message was queued (or handed to a receiver).
    Sent,
}

/// What a send did.
#[derive(Debug, PartialEq, Eq)]
pub enum SendStep {
    /// Queued or handed over: wake `woken`; send the notification of
    /// `fired` (it is no longer registered).
    Done { woken: Vec<i32>, fired: Option<Registration> },
    /// The queue is full: sleep on this ticket.
    Sleep(u64),
}

/// What a receive did.
#[derive(Debug, PartialEq, Eq)]
pub enum ReceiveStep {
    /// Took a message; wake `woken` (a sender whose message got the room).
    Done { prio: u32, data: Vec<u8>, woken: Vec<i32> },
    /// The queue is empty: sleep on this ticket.
    Sleep(u64),
}

/// Ticket numbers, unique over all queues.
static TICKETS: AtomicU64 = AtomicU64::new(1);
/// Open file identities ([`Registration::file`]).
static FILE_IDS: AtomicU64 = AtomicU64::new(1);

/// A new identity for an open file of a queue, for [`Queue::file_closed`].
pub fn new_file_id() -> u64 {
    FILE_IDS.fetch_add(1, Ordering::Relaxed)
}

struct Receiver {
    ticket: u64,
    tid: i32,
}

struct Sender {
    ticket: u64,
    tid: i32,
    prio: u32,
    data: Vec<u8>,
}

#[derive(Default)]
struct State {
    /// By priority; each a FIFO.
    messages: BTreeMap<u32, VecDeque<Vec<u8>>>,
    count: usize,
    bytes: usize,
    /// Blocked receivers in the order they came (only while empty).
    receivers: VecDeque<Receiver>,
    /// Blocked senders in the order they came (only while full).
    senders: VecDeque<Sender>,
    /// Completed tickets not yet collected.
    done: BTreeMap<u64, Done>,
    notify: Option<Registration>,
}

impl State {
    fn push(&mut self, prio: u32, data: Vec<u8>) {
        self.count += 1;
        self.bytes += data.len();
        self.messages.entry(prio).or_default().push_back(data);
    }

    fn pop(&mut self) -> Option<(u32, Vec<u8>)> {
        let (&prio, fifo) = self.messages.iter_mut().next_back()?;
        let data = fifo.pop_front()?;
        if fifo.is_empty() {
            self.messages.remove(&prio);
        }
        self.count -= 1;
        self.bytes -= data.len();
        Some((prio, data))
    }
}

/// One message queue.
pub struct Queue {
    /// `mq_maxmsg`, `mq_msgsize`.
    pub maxmsg: usize,
    pub msgsize: usize,
    /// Permission bits (`mode & ~umask`) and owner, fixed at creation.
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    state: Mutex<State>,
    watch: Mutex<Option<Arc<dyn QueueWatch>>>,
}

impl core::fmt::Debug for Queue {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Queue").field("maxmsg", &self.maxmsg).field("msgsize", &self.msgsize).finish()
    }
}

impl Queue {
    pub fn new(maxmsg: usize, msgsize: usize, mode: u16, uid: u32, gid: u32) -> Arc<Queue> {
        Arc::new(Queue {
            maxmsg,
            msgsize,
            mode,
            uid,
            gid,
            state: Mutex::new(State::default()),
            watch: Mutex::new(None),
        })
    }

    /// Messages queued now (`mq_curmsgs`).
    pub fn curmsgs(&self) -> usize {
        self.state.lock().count
    }

    /// Whether a receive (first) or a send (second) would not block.
    pub fn readiness(&self) -> (bool, bool) {
        let s = self.state.lock();
        (s.count > 0, s.count < self.maxmsg)
    }

    /// The watch the kernel attached, attaching `make()` first if none.
    pub fn watch_or_init(&self, make: impl FnOnce() -> Arc<dyn QueueWatch>) -> Arc<dyn QueueWatch> {
        self.watch.lock().get_or_insert_with(make).clone()
    }

    fn changed(&self) {
        let watch = self.watch.lock().clone();
        if let Some(w) = watch {
            w.changed();
        }
    }

    /// Add a message of priority `prio` (below [`MQ_PRIO_MAX`], at most
    /// `msgsize` bytes: the caller checked). With room: handed to the first
    /// blocked receiver if there is one, else queued, taking out the
    /// notification registration if the queue was empty. Full: `EAGAIN`
    /// when `nonblock`, else a ticket for thread `tid`.
    pub fn send(&self, prio: u32, data: Vec<u8>, nonblock: bool, tid: i32) -> Result<SendStep, Errno> {
        let step = {
            let mut s = self.state.lock();
            if s.count >= self.maxmsg || !s.senders.is_empty() {
                if nonblock {
                    return Err(Errno::EAGAIN);
                }
                let ticket = TICKETS.fetch_add(1, Ordering::Relaxed);
                s.senders.push_back(Sender { ticket, tid, prio, data });
                return Ok(SendStep::Sleep(ticket));
            }
            if let Some(r) = s.receivers.pop_front() {
                s.done.insert(r.ticket, Done::Received { prio, data });
                SendStep::Done { woken: alloc::vec![r.tid], fired: None }
            } else {
                let was_empty = s.count == 0;
                s.push(prio, data);
                let fired = if was_empty { s.notify.take() } else { None };
                SendStep::Done { woken: Vec::new(), fired }
            }
        };
        self.changed();
        Ok(step)
    }

    /// Take the oldest message of the highest priority; the first blocked
    /// sender's message then takes the room. Empty: `EAGAIN` when
    /// `nonblock`, else a ticket for thread `tid`.
    pub fn receive(&self, nonblock: bool, tid: i32) -> Result<ReceiveStep, Errno> {
        let step = {
            let mut s = self.state.lock();
            let Some((prio, data)) = s.pop() else {
                if nonblock {
                    return Err(Errno::EAGAIN);
                }
                let ticket = TICKETS.fetch_add(1, Ordering::Relaxed);
                s.receivers.push_back(Receiver { ticket, tid });
                return Ok(ReceiveStep::Sleep(ticket));
            };
            let mut woken = Vec::new();
            if let Some(w) = s.senders.pop_front() {
                s.push(w.prio, w.data);
                s.done.insert(w.ticket, Done::Sent);
                woken.push(w.tid);
            }
            ReceiveStep::Done { prio, data, woken }
        };
        self.changed();
        Ok(step)
    }

    /// The result of `ticket` if a waker completed it.
    pub fn poll(&self, ticket: u64) -> Option<Done> {
        self.state.lock().done.remove(&ticket)
    }

    /// Stop waiting on `ticket` (timeout, signal): its result if it was
    /// completed meanwhile, else `None` and it is forgotten.
    pub fn cancel(&self, ticket: u64) -> Option<Done> {
        let mut s = self.state.lock();
        if let Some(done) = s.done.remove(&ticket) {
            return Some(done);
        }
        s.receivers.retain(|r| r.ticket != ticket);
        s.senders.retain(|w| w.ticket != ticket);
        None
    }

    /// The registered process, if any.
    pub fn registered(&self) -> Option<Registration> {
        self.state.lock().notify
    }

    /// Register `reg`: `EBUSY` if a process (this one or another) is.
    pub fn register(self: &Arc<Self>, reg: Registration) -> Result<(), Errno> {
        {
            let mut s = self.state.lock();
            if s.notify.is_some() {
                return Err(Errno::EBUSY);
            }
            s.notify = Some(reg);
        }
        REGISTERED.lock().push(Arc::downgrade(self));
        Ok(())
    }

    /// Remove the registration of process `pid`, if it is the registered
    /// one (mq_notify with a null `sevp`; the process exits).
    pub fn unregister(&self, pid: i32) {
        let mut s = self.state.lock();
        if s.notify.is_some_and(|r| r.pid == pid) {
            s.notify = None;
        }
    }

    /// The open file `file` is closed for good: a registration made
    /// through it goes.
    pub fn file_closed(&self, file: u64) {
        let mut s = self.state.lock();
        if s.notify.is_some_and(|r| r.file == file) {
            s.notify = None;
        }
    }

    /// What reading a queue descriptor gives (mq_overview(7)): `QSIZE`
    /// (bytes of all queued messages), and the registration's
    /// `sigev_notify`, signal and process (0 when none), in
    /// `"QSIZE:%-10lu NOTIFY:%-5d SIGNO:%-5d NOTIFY_PID:%-6d\n"`.
    pub fn status_line(&self) -> Vec<u8> {
        let (bytes, reg) = {
            let s = self.state.lock();
            (s.bytes, s.notify)
        };
        let (notify, signo, pid) = reg.map_or((0, 0, 0), |r| (r.event.notify, r.event.signo, r.pid));
        let mut out = Vec::with_capacity(60);
        field(&mut out, b"QSIZE:", bytes as i64, 10);
        field(&mut out, b" NOTIFY:", i64::from(notify), 5);
        field(&mut out, b" SIGNO:", i64::from(signo), 5);
        field(&mut out, b" NOTIFY_PID:", i64::from(pid), 6);
        out.push(b'\n');
        out
    }
}

/// `label` then `value` in decimal, left-aligned in `width` columns.
fn field(out: &mut Vec<u8>, label: &[u8], value: i64, width: usize) {
    out.extend_from_slice(label);
    let mut digits = [0u8; 20];
    let mut n = value.unsigned_abs();
    let mut len = 0;
    loop {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let start = out.len();
    if value < 0 {
        out.push(b'-');
    }
    out.extend(digits[..len].iter().rev());
    while out.len() - start < width {
        out.push(b' ');
    }
}

/// Queues that got a registration, for [`process_exit`].
static REGISTERED: Mutex<Vec<Weak<Queue>>> = Mutex::new(Vec::new());

/// Process `pid` exited: its registrations go (mq_notify(3)).
pub fn process_exit(pid: i32) {
    let queues: Vec<Arc<Queue>> = {
        let mut list = REGISTERED.lock();
        list.retain(|w| w.upgrade().is_some_and(|q| q.registered().is_some()));
        list.iter().filter_map(Weak::upgrade).collect()
    };
    for q in queues {
        q.unregister(pid);
    }
}

/// Who opens or unlinks: the effective ids and the capabilities that
/// matter, over the IPC namespace's owner.
#[derive(Clone, Debug, Default)]
pub struct Caller {
    pub euid: u32,
    pub egid: u32,
    pub groups: Arc<[u32]>,
    /// `CAP_DAC_OVERRIDE`: opens pass the permission check.
    pub dac_override: bool,
    /// `CAP_FOWNER`: unlink anyone's queue.
    pub fowner: bool,
    /// `CAP_SYS_RESOURCE`: bigger queues, more of them.
    pub sys_resource: bool,
}

impl Caller {
    /// Whether the mode bits of `q` grant `want` (r 4, w 2) to the caller,
    /// as for a file: owner class, else group class, else others.
    fn may(&self, q: &Queue, want: u16) -> bool {
        if want == 0 || self.dac_override {
            return true;
        }
        let granted = if self.euid == q.uid {
            q.mode >> 6
        } else if self.egid == q.gid || self.groups.contains(&q.gid) {
            q.mode >> 3
        } else {
            q.mode
        } & 7;
        want & !granted == 0
    }
}

/// A name's syntax (the raw call's name has no leading slash): `EACCES`
/// for one with a `/` or that is `.` or `..`, `ENOENT` for an empty one.
/// The length (`ENAMETOOLONG`) was checked while copying it.
pub fn check_name(name: &[u8]) -> Result<(), Errno> {
    if name.is_empty() {
        return Err(Errno::ENOENT);
    }
    if name.contains(&b'/') || name == b"." || name == b".." {
        return Err(Errno::EACCES);
    }
    Ok(())
}

/// `open(2)` flag bits mq_open looks at (`asm-generic/fcntl.h`).
pub mod oflag {
    pub const O_ACCMODE: u32 = 0o3;
    pub const O_WRONLY: u32 = 0o1;
    pub const O_RDWR: u32 = 0o2;
    pub const O_CREAT: u32 = 0o100;
    pub const O_EXCL: u32 = 0o200;
    pub const O_NONBLOCK: u32 = 0o4000;
}

/// What mq_open asks of a namespace's names.
#[derive(Clone, Debug)]
pub struct OpenRequest<'a> {
    pub name: &'a [u8],
    /// `oflag`.
    pub flags: u32,
    /// For a new queue: the permission bits with the umask applied.
    pub mode: u16,
    /// For a new queue: `attr`, if one was given.
    pub attr: Option<MqAttr>,
    /// For a new queue: its owner (effective ids).
    pub uid: u32,
    pub gid: u32,
}

/// The queue names of one IPC namespace.
#[derive(Debug, Default)]
pub struct MqTable {
    names: BTreeMap<Vec<u8>, Arc<Queue>>,
}

impl MqTable {
    pub const fn new() -> MqTable {
        MqTable { names: BTreeMap::new() }
    }

    /// mq_open after the name's syntax was checked: an existing queue is
    /// `EEXIST` with `O_CREAT | O_EXCL`, `EINVAL` for access mode 3,
    /// `EACCES` unless its mode grants the access; a missing one `ENOENT`
    /// without `O_CREAT`, else made from the attributes (`EINVAL` for
    /// sizes of 0 or less, above the caller's limits; `ENOSPC` beyond
    /// [`QUEUES_MAX`]).
    pub fn open(&mut self, req: &OpenRequest<'_>, who: &Caller) -> Result<Arc<Queue>, Errno> {
        use oflag::*;
        let want = match req.flags & O_ACCMODE {
            0 => 4,
            O_WRONLY => 2,
            O_RDWR => 6,
            _ => 0xff,
        };
        if let Some(q) = self.names.get(req.name) {
            if req.flags & (O_CREAT | O_EXCL) == O_CREAT | O_EXCL {
                return Err(Errno::EEXIST);
            }
            if want == 0xff {
                return Err(Errno::EINVAL);
            }
            if !who.may(q, want) {
                return Err(Errno::EACCES);
            }
            return Ok(q.clone());
        }
        if req.flags & O_CREAT == 0 {
            return Err(Errno::ENOENT);
        }
        if want == 0xff {
            return Err(Errno::EINVAL);
        }
        let (maxmsg, msgsize) = match req.attr {
            None => (DEFAULT_MAXMSG, DEFAULT_MSGSIZE),
            Some(a) => {
                let (max, size) =
                    if who.sys_resource { (HARD_MSGMAX, HARD_MSGSIZEMAX) } else { (MSG_MAX, MSGSIZE_MAX) };
                if a.maxmsg <= 0 || a.msgsize <= 0 || a.maxmsg > max || a.msgsize > size {
                    return Err(Errno::EINVAL);
                }
                (a.maxmsg, a.msgsize)
            }
        };
        if !who.sys_resource && self.names.len() >= QUEUES_MAX {
            return Err(Errno::ENOSPC);
        }
        let q = Queue::new(maxmsg as usize, msgsize as usize, req.mode & 0o777, req.uid, req.gid);
        self.names.insert(req.name.to_vec(), q.clone());
        Ok(q)
    }

    /// mq_unlink after the name's syntax was checked: `ENOENT` if there is
    /// no such queue, `EPERM` unless the caller owns it or has
    /// `CAP_FOWNER` (the queue directory is sticky).
    pub fn unlink(&mut self, name: &[u8], who: &Caller) -> Result<(), Errno> {
        let q = self.names.get(name).ok_or(Errno::ENOENT)?;
        if q.uid != who.euid && !who.fowner {
            return Err(Errno::EPERM);
        }
        self.names.remove(name);
        Ok(())
    }

    /// The queue named `name`, if any.
    pub fn get(&self, name: &[u8]) -> Option<Arc<Queue>> {
        self.names.get(name).cloned()
    }
}

/// The names of every IPC namespace, by its inode number (0: the initial
/// one). A namespace's names stay after it is gone: inode numbers are
/// never reused, so nobody sees them again.
static NAMES: Mutex<BTreeMap<u32, MqTable>> = Mutex::new(BTreeMap::new());

/// Run `f` on the queue names of IPC namespace `ns` (`None`: the initial
/// one), locked. `f` must not block or call the kernel.
pub fn with_names<R>(ns: &Option<Arc<IpcNs>>, f: impl FnOnce(&mut MqTable) -> R) -> R {
    let key = ns.as_ref().map_or(0, |n| n.inum());
    f(NAMES.lock().entry(key).or_default())
}

/// An open queue descriptor, as the kernel describes it to the handlers.
#[derive(Clone, Debug)]
pub struct MqFd {
    pub queue: Arc<Queue>,
    /// The open file's identity ([`new_file_id`]).
    pub file: u64,
    /// The open file's access mode and status flags (`F_GETFL`).
    pub flags: u32,
}

impl MqFd {
    pub fn readable(&self) -> bool {
        self.flags & oflag::O_ACCMODE != oflag::O_WRONLY
    }

    pub fn writable(&self) -> bool {
        matches!(self.flags & oflag::O_ACCMODE, oflag::O_WRONLY | oflag::O_RDWR)
    }

    pub fn nonblock(&self) -> bool {
        self.flags & oflag::O_NONBLOCK != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn root() -> Caller {
        Caller { dac_override: true, fowner: true, sys_resource: true, ..Caller::default() }
    }

    fn user(uid: u32) -> Caller {
        Caller { euid: uid, egid: uid, ..Caller::default() }
    }

    fn req(name: &[u8], flags: u32, attr: Option<MqAttr>, uid: u32) -> OpenRequest<'_> {
        OpenRequest { name, flags, mode: 0o600, attr, uid, gid: uid }
    }

    fn sized(maxmsg: i64, msgsize: i64) -> Option<MqAttr> {
        Some(MqAttr { maxmsg, msgsize, ..MqAttr::default() })
    }

    fn recv(q: &Queue) -> (u32, Vec<u8>) {
        match q.receive(true, 1).unwrap() {
            ReceiveStep::Done { prio, data, .. } => (prio, data),
            ReceiveStep::Sleep(_) => panic!("would sleep"),
        }
    }

    #[test]
    fn names() {
        assert_eq!(check_name(b""), Err(Errno::ENOENT));
        assert_eq!(check_name(b"/q"), Err(Errno::EACCES));
        assert_eq!(check_name(b"a/b"), Err(Errno::EACCES));
        assert_eq!(check_name(b"."), Err(Errno::EACCES));
        assert_eq!(check_name(b".."), Err(Errno::EACCES));
        assert_eq!(check_name(b"...q"), Ok(()));
    }

    #[test]
    fn open_rules() {
        use oflag::*;
        let mut t = MqTable::new();
        assert_eq!(t.open(&req(b"q", O_RDWR, None, 0), &root()).unwrap_err(), Errno::ENOENT);
        for bad in [sized(0, 8), sized(4, 0), sized(4, -1), sized(65537, 8)] {
            assert_eq!(t.open(&req(b"q", O_CREAT | O_RDWR, bad, 0), &root()).unwrap_err(), Errno::EINVAL);
        }
        assert_eq!(t.open(&req(b"q", O_CREAT, sized(11, 8), 5), &user(5)).unwrap_err(), Errno::EINVAL);
        assert_eq!(t.open(&req(b"q", O_CREAT, sized(4, 8193), 5), &user(5)).unwrap_err(), Errno::EINVAL);
        let q = t.open(&req(b"q", O_CREAT | O_RDWR, None, 5), &user(5)).unwrap();
        assert_eq!((q.maxmsg, q.msgsize, q.uid), (10, 8192, 5));
        assert_eq!(t.open(&req(b"q", O_CREAT | O_EXCL, None, 5), &user(5)).unwrap_err(), Errno::EEXIST);
        assert_eq!(t.open(&req(b"q", 3, None, 5), &user(5)).unwrap_err(), Errno::EINVAL);
        let again = t.open(&req(b"q", O_CREAT | O_RDWR, sized(0, 0), 5), &user(5)).unwrap();
        assert!(Arc::ptr_eq(&q, &again), "attr ignored for an existing queue");
        assert_eq!(t.open(&req(b"q", 0, None, 7), &user(7)).unwrap_err(), Errno::EACCES);
        assert!(t.open(&req(b"q", O_RDWR, None, 7), &root()).is_ok());
    }

    #[test]
    fn unlink_keeps_open_queues() {
        use oflag::*;
        let mut t = MqTable::new();
        let q = t.open(&req(b"q", O_CREAT | O_RDWR, sized(2, 8), 5), &user(5)).unwrap();
        assert_eq!(t.unlink(b"q", &user(6)), Err(Errno::EPERM));
        assert_eq!(t.unlink(b"q", &user(5)), Ok(()));
        assert_eq!(t.unlink(b"q", &user(5)), Err(Errno::ENOENT));
        let fresh = t.open(&req(b"q", O_CREAT | O_EXCL | O_RDWR, sized(3, 8), 5), &user(5)).unwrap();
        assert!(!Arc::ptr_eq(&q, &fresh));
        q.send(0, b"old".to_vec(), true, 1).unwrap();
        assert_eq!(fresh.curmsgs(), 0);
        assert_eq!(recv(&q).1, b"old");
    }

    #[test]
    fn priority_then_fifo() {
        let q = Queue::new(10, 8, 0o600, 0, 0);
        for (i, p) in [3u32, 0, 3, 7, 0, 7, 1, 3].iter().enumerate() {
            q.send(*p, vec![b'a' + i as u8], true, 1).unwrap();
        }
        let order: Vec<u8> = (0..8).map(|_| recv(&q).1[0]).collect();
        assert_eq!(order, b"dfachgbe");
        assert_eq!(q.receive(true, 1), Err(Errno::EAGAIN));
    }

    #[test]
    fn full_queue() {
        let q = Queue::new(1, 8, 0o600, 0, 0);
        q.send(0, vec![1], true, 1).unwrap();
        assert_eq!(q.send(0, vec![2], true, 1), Err(Errno::EAGAIN));
        let SendStep::Sleep(ticket) = q.send(4, vec![2], false, 9).unwrap() else { panic!("no sleep") };
        assert_eq!(q.readiness(), (true, false));
        let ReceiveStep::Done { data, woken, .. } = q.receive(true, 1).unwrap() else { panic!("no message") };
        assert_eq!((data, woken), (vec![1], vec![9]));
        assert_eq!(q.poll(ticket), Some(Done::Sent));
        assert_eq!(recv(&q), (4, vec![2]));
        let SendStep::Sleep(ticket) = ({
            q.send(0, vec![3], true, 1).unwrap();
            q.send(0, vec![4], false, 9).unwrap()
        }) else {
            panic!("no sleep")
        };
        assert_eq!(q.cancel(ticket), None);
        recv(&q);
        assert_eq!(q.curmsgs(), 0, "a cancelled sender's message is not queued");
    }

    #[test]
    fn handoff_and_notification() {
        let q = Queue::new(4, 8, 0o600, 0, 0);
        let event = SigEvent { value: 7, signo: 12, notify: SIGEV_SIGNAL };
        q.register(Registration { pid: 100, event, file: 1 }).unwrap();
        assert_eq!(q.register(Registration { pid: 100, event, file: 1 }), Err(Errno::EBUSY));
        let ReceiveStep::Sleep(ticket) = q.receive(false, 5).unwrap() else { panic!("no sleep") };
        let step = q.send(2, b"r".to_vec(), false, 6).unwrap();
        assert_eq!(step, SendStep::Done { woken: vec![5], fired: None }, "a blocked receiver suppresses it");
        assert_eq!(q.poll(ticket), Some(Done::Received { prio: 2, data: b"r".to_vec() }));
        let SendStep::Done { fired, .. } = q.send(0, b"s".to_vec(), false, 6).unwrap() else { panic!() };
        assert_eq!(fired.map(|r| r.pid), Some(100));
        assert_eq!(q.registered(), None, "fired once, then gone");
        q.register(Registration { pid: 100, event, file: 1 }).unwrap();
        let SendStep::Done { fired, .. } = q.send(0, b"t".to_vec(), false, 6).unwrap() else { panic!() };
        assert_eq!(fired, None, "nothing fires on a non-empty queue");
        q.file_closed(2);
        assert!(q.registered().is_some());
        q.file_closed(1);
        assert_eq!(q.registered(), None);
        q.register(Registration { pid: 101, event, file: 3 }).unwrap();
        process_exit(101);
        assert_eq!(q.registered(), None);
    }

    #[test]
    fn status_line() {
        let q = Queue::new(4, 64, 0o600, 0, 0);
        assert_eq!(q.status_line(), b"QSIZE:0          NOTIFY:0     SIGNO:0     NOTIFY_PID:0     \n".to_vec());
        q.send(0, b"hello".to_vec(), true, 1).unwrap();
        q.send(2, b"abc".to_vec(), true, 1).unwrap();
        let event = SigEvent { value: 0, signo: 10, notify: SIGEV_SIGNAL };
        q.register(Registration { pid: 4242, event, file: 1 }).unwrap();
        let line = q.status_line();
        assert_eq!(line.len(), 60);
        assert_eq!(line, b"QSIZE:8          NOTIFY:0     SIGNO:10    NOTIFY_PID:4242  \n".to_vec());
    }

    #[test]
    fn sigevent_rules() {
        let e = |notify, signo| SigEvent { value: 0, signo, notify };
        assert_eq!(e(99, 0).validate(), Err(Errno::EINVAL));
        assert_eq!(e(SIGEV_SIGNAL, 65).validate(), Err(Errno::EINVAL));
        assert_eq!(e(SIGEV_SIGNAL, 0).validate(), Ok(()));
        assert_eq!(e(SIGEV_NONE, 999).validate(), Ok(()));
        let attr = MqAttr { flags: 2048, maxmsg: 3, msgsize: 100, curmsgs: 1 };
        assert_eq!(MqAttr::from_bytes(&attr.to_bytes()), attr);
    }
}
