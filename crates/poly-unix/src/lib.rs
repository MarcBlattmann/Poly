//! Unix domain sockets (`AF_UNIX`), as unix(7), socket(7) and the
//! section 2 pages of the socket calls describe them (ADR 0048).
//!
//! - [`UnixSocket`] is one socket: `SOCK_STREAM`, `SOCK_DGRAM` or
//!   `SOCK_SEQPACKET`, with its receive queue, connection state, name and
//!   options. Its calls never block: one that would returns `EAGAIN`
//!   (`connect` too, while the listener's backlog is full), and the
//!   socket's [`WaitQueue`] is notified whenever its readiness may have
//!   changed, so the kernel's blocking calls, `poll` and `epoll` work on it
//!   like on a pipe (ADR 0042).
//! - [`UnixFile`] is the [`FileObject`] behind an open socket: `read` /
//!   `write` are `recv` / `send` without flags, dropping it (the last
//!   close of the open file description) closes the socket.
//! - [`Namespace`] holds the names: the abstract namespace (`sun_path[0]
//!   == 0`) and sockets bound to a path, keyed by the socket inode the
//!   bind made in the VFS (`S_IFSOCK`, unix(7): "connecting to a stream
//!   socket object requires write permission on that socket"). It also
//!   counts descriptors in flight and collects unreachable cycles of them
//!   ([`Namespace::collect`], `gc.rs`).
//!
//! # Messages
//!
//! Every send makes one packet: the bytes, the sender's address, its
//! credentials (explicit `SCM_CREDENTIALS` or the sender's own) and the
//! open files of `SCM_RIGHTS` (references to open file descriptions,
//! counted as "in flight" until received). Datagram and sequenced-packet
//! receives take one packet (the rest of a longer one is dropped,
//! `MSG_TRUNC`); stream receives take bytes across packets, but stop after
//! a packet that carried descriptors, the "barrier" unix(7) describes, and
//! before a packet whose credentials differ while `SO_PASSCRED` is on.
//! Descriptors of a packet that is dropped (queue purged at close, a
//! datagram read with `read`, a truncated receive) are released with it.
//!
//! # Points the manual pages leave open (Poly's choices, to be confirmed by
//! the diff runner)
//!
//! - Buffer sizes: [`DEFAULT_BUFFER`] for `SO_SNDBUF` / `SO_RCVBUF`; a
//!   stream or sequenced-packet sender may have that many bytes waiting at
//!   its peer; a datagram receiver holds at most [`MAX_DGRAM_QUEUE`]
//!   datagrams and [`DEFAULT_BUFFER`] bytes (one datagram always fits).
//! - A socket closed with unread data gives its stream peer `ECONNRESET`
//!   once, after the peer has read what was queued for it; otherwise the
//!   peer reads end of file. Writing to a closed or read-shut peer is
//!   `EPIPE`.
//! - Sending a datagram to a socket connected to someone else is `EPERM`;
//!   to a closed connected peer `ECONNREFUSED`; without a destination
//!   `ENOTCONN` (unix(7)'s list).
//! - `read` / `write` through [`UnixFile`] use the credentials the socket
//!   was created with.

#![no_std]

extern crate alloc;

mod gc;
mod sync;
#[cfg(test)]
mod tests;

use crate::sync::SpinMutex;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use poly_vfs::poll::{PollEvents, WaitQueue};
use poly_vfs::{Access, At, Cred, DeviceId, FileObject, FileType, FsContext, OpenFile, Vfs};

pub use poly_vfs::Errno;

/// Every fallible call's result.
pub type Result<T, E = Errno> = core::result::Result<T, E>;

/// Descriptors one `SCM_RIGHTS` message may carry (unix(7): `SCM_MAX_FD`,
/// 253; more is `EINVAL`).
pub const SCM_MAX_FD: usize = 253;

/// `SO_SNDBUF` / `SO_RCVBUF` of a new socket, as `getsockopt` reports them
/// (Poly's choice; socket(7) leaves the default to `wmem_default`).
pub const DEFAULT_BUFFER: usize = 212_992;

/// Datagrams a receiving datagram socket holds.
pub const MAX_DGRAM_QUEUE: usize = 512;

/// The largest listen backlog (`SOMAXCONN`, 4096 on current Linux, listen(2)).
pub const MAX_BACKLOG: usize = 4096;

/// The smallest (doubled) buffer sizes socket(7) gives: 2048 for
/// `SO_SNDBUF`, 256 for `SO_RCVBUF`.
pub const MIN_SNDBUF: usize = 2048;
pub const MIN_RCVBUF: usize = 256;

/// unix(7): a datagram may be at most the (doubled) `SO_SNDBUF` "less 32
/// bytes used for overhead".
const DGRAM_OVERHEAD: usize = 32;

/// `SOCK_STREAM`, `SOCK_DGRAM`, `SOCK_SEQPACKET`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketType {
    Stream,
    Datagram,
    SeqPacket,
}

impl SocketType {
    /// Connection-oriented (stream and sequenced-packet).
    pub const fn connected_kind(self) -> bool {
        !matches!(self, SocketType::Datagram)
    }
}

/// `struct ucred`: a process id and its user and group ids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Credentials {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

/// A socket's address, as `getsockname`, `getpeername`, `accept` and
/// `recvfrom` report it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Address {
    /// Not bound: `addrlen` is `sizeof(sa_family_t)`.
    #[default]
    Unnamed,
    /// Bound to this path (without the terminating NUL).
    Path(Vec<u8>),
    /// Bound in the abstract namespace (the bytes after `sun_path[0]`).
    Abstract(Vec<u8>),
}

/// Ancillary data of a send: `SCM_RIGHTS` and `SCM_CREDENTIALS`.
#[derive(Default)]
pub struct Ancillary {
    pub rights: Vec<Arc<OpenFile>>,
    /// Explicit credentials, already checked by the caller (unix(7): only
    /// privileged processes may name other ids).
    pub creds: Option<Credentials>,
}

impl core::fmt::Debug for Ancillary {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Ancillary").field("rights", &self.rights.len()).field("creds", &self.creds).finish()
    }
}

/// What a receive got.
pub struct Received {
    /// Bytes copied into the buffer.
    pub len: usize,
    /// The packet's full length for datagram and sequenced-packet sockets
    /// (more than `len` when it was truncated); `len` for streams.
    pub full_len: usize,
    /// The sender's address.
    pub from: Address,
    /// `SCM_RIGHTS`: the open files that came with the data, now the
    /// receiver's (to install as descriptors, or drop).
    pub rights: Vec<Arc<OpenFile>>,
    /// `SCM_CREDENTIALS`, when `SO_PASSCRED` is on.
    pub creds: Option<Credentials>,
}

impl core::fmt::Debug for Received {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Received")
            .field("len", &self.len)
            .field("full_len", &self.full_len)
            .field("from", &self.from)
            .field("rights", &self.rights.len())
            .field("creds", &self.creds)
            .finish()
    }
}

/// The socket-level options a system call layer keeps for any socket and
/// reports back, though Unix sockets do not act on them (`SO_REUSEADDR`,
/// `SO_KEEPALIVE`, `SO_LINGER`, ...; socket(7)).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SocketOptions {
    pub reuse_addr: bool,
    pub reuse_port: bool,
    pub keepalive: bool,
    pub broadcast: bool,
    pub oob_inline: bool,
    /// `SO_LINGER`: `None` off, else seconds.
    pub linger: Option<u32>,
    pub priority: i32,
    pub rcv_lowat: i32,
}

/// `shutdown(2)`'s `how`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

/// One message in a receive queue.
struct Packet {
    data: Vec<u8>,
    /// Bytes of `data` a stream receive has taken already.
    off: usize,
    from: Address,
    creds: Credentials,
    rights: Vec<Arc<OpenFile>>,
}

impl Packet {
    fn left(&self) -> usize {
        self.data.len() - self.off
    }
}

/// Connections waiting for `accept`.
struct Listen {
    backlog: usize,
    pending: VecDeque<Arc<UnixSocket>>,
    /// Sockets whose `connect` found the backlog full, to wake when a slot
    /// frees.
    connectors: Vec<Weak<UnixSocket>>,
}

enum Conn {
    /// Neither listening nor connected.
    Fresh,
    Listening(Listen),
    /// A stream or sequenced-packet connection (the peer is in `peer`).
    Connected,
    /// The connection is over: the peer closed (or was never accepted
    /// and its listener went away).
    Disconnected,
}

struct State {
    conn: Conn,
    /// The other end of a connection.
    peer: Option<Arc<UnixSocket>>,
    /// The default destination of a connected datagram socket.
    dgram_peer: Option<Weak<UnixSocket>>,
    name: Address,
    peer_name: Address,
    rx: VecDeque<Packet>,
    rx_bytes: usize,
    /// This side's shutdowns.
    rd_shut: bool,
    wr_shut: bool,
    /// The peer will send nothing more (it shut down writing or closed):
    /// end of file once the queue is empty.
    peer_done: bool,
    /// The peer takes nothing more (it shut down reading or closed):
    /// writes fail with `EPIPE`.
    peer_gone: bool,
    closed: bool,
    /// Reported once by the next receive (`ECONNRESET`).
    error: Option<Errno>,
    passcred: bool,
    /// The owner's credentials, for `read` / `write` and `SO_PEERCRED` of
    /// the peer.
    creds: Credentials,
    /// `SO_PEERCRED`.
    peer_creds: Option<Credentials>,
    sndbuf: usize,
    rcvbuf: usize,
    /// Datagram senders that found this socket's queue full.
    blocked_senders: Vec<Weak<UnixSocket>>,
    /// On the namespace's list of sockets holding descriptors in flight.
    holds_rights: bool,
    /// The inode a path bind made, to drop the name at close.
    path_key: Option<PathKey>,
}

/// One Unix domain socket.
pub struct UnixSocket {
    ns: Arc<Namespace>,
    kind: SocketType,
    queue: Arc<WaitQueue>,
    state: SpinMutex<State>,
    /// `SO_RCVTIMEO` / `SO_SNDTIMEO` in nanoseconds; 0: none.
    rcv_timeout: AtomicU64,
    snd_timeout: AtomicU64,
    options: SpinMutex<SocketOptions>,
}

impl core::fmt::Debug for UnixSocket {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let s = self.state.lock();
        f.debug_struct("UnixSocket")
            .field("kind", &self.kind)
            .field("name", &s.name)
            .field("queued", &s.rx_bytes)
            .field("closed", &s.closed)
            .finish_non_exhaustive()
    }
}

/// Where a path-bound socket's inode is: device and inode number.
type PathKey = (u32, u32, u64);

#[derive(Default)]
struct Names {
    abstract_names: BTreeMap<Vec<u8>, Weak<UnixSocket>>,
    paths: BTreeMap<PathKey, Weak<UnixSocket>>,
    next_auto: u32,
}

/// Socket names and descriptors in flight: one per system (or per network
/// namespace, later).
#[derive(Default)]
pub struct Namespace {
    names: SpinMutex<Names>,
    /// Sockets whose queues hold open files (`SCM_RIGHTS`).
    holding: SpinMutex<Vec<Weak<UnixSocket>>>,
    /// Open files in flight.
    in_flight: AtomicUsize,
    /// One collection at a time.
    collecting: SpinMutex<()>,
}

impl core::fmt::Debug for Namespace {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Namespace").field("in_flight", &self.in_flight()).finish_non_exhaustive()
    }
}

fn key_of(st: &poly_vfs::Stat) -> PathKey {
    (st.dev.major, st.dev.minor, st.ino)
}

/// Readiness may have changed: these queues get these events once every
/// lock is released.
#[derive(Default)]
struct Wakes(Vec<(Arc<WaitQueue>, PollEvents)>);

impl Wakes {
    fn add(&mut self, sock: &UnixSocket, events: PollEvents) {
        self.0.push((Arc::clone(&sock.queue), events));
    }

    fn run(self) {
        for (q, e) in self.0 {
            q.notify(e);
        }
    }
}

const IN: PollEvents = PollEvents::READABLE;
const OUT: PollEvents = PollEvents::WRITABLE;
const HUP_ALL: PollEvents = PollEvents::from_bits(
    PollEvents::READABLE.bits()
        | PollEvents::WRITABLE.bits()
        | PollEvents::HUP.bits()
        | PollEvents::ERR.bits()
        | PollEvents::RDHUP.bits(),
);

impl Namespace {
    pub fn new() -> Arc<Namespace> {
        Arc::new(Namespace::default())
    }

    /// The socket bound to abstract name `name`, if one is.
    pub fn lookup_abstract(&self, name: &[u8]) -> Option<Arc<UnixSocket>> {
        self.names.lock().abstract_names.get(name).and_then(Weak::upgrade)
    }

    /// The socket a path names, for `connect` and `sendto`: `ENOENT` and
    /// the path walk's errors, `ECONNREFUSED` if the path is no socket or
    /// no socket is bound to it any more, `EACCES` without write
    /// permission on the socket inode (unix(7)).
    pub fn lookup_path(
        &self,
        vfs: &Vfs,
        ctx: &FsContext,
        cred: &Cred,
        at: At<'_>,
        path: &[u8],
    ) -> Result<Arc<UnixSocket>> {
        let node = vfs.lookup(ctx, cred, at, path, true)?;
        let st = node.stat()?;
        if st.kind() != Some(FileType::Socket) {
            return Err(Errno::ECONNREFUSED);
        }
        node.access(cred, Access::WRITE)?;
        self.names.lock().paths.get(&key_of(&st)).and_then(Weak::upgrade).ok_or(Errno::ECONNREFUSED)
    }

    /// Open files in flight (sent and not yet received or dropped).
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }

    /// Bound names (both namespaces), for tests and diagnostics.
    pub fn bound_names(&self) -> usize {
        let n = self.names.lock();
        n.abstract_names.values().filter(|w| w.strong_count() > 0).count()
            + n.paths.values().filter(|w| w.strong_count() > 0).count()
    }

    fn note_holding(&self, sock: &Arc<UnixSocket>) {
        self.holding.lock().push(Arc::downgrade(sock));
    }

    fn remove_name(&self, sock: &UnixSocket, name: &Address, key: Option<PathKey>) {
        let mut n = self.names.lock();
        let is_me = |w: &Weak<UnixSocket>| w.upgrade().is_none_or(|s| core::ptr::eq(&*s, sock));
        match name {
            Address::Abstract(a) => {
                if n.abstract_names.get(a).is_some_and(is_me) {
                    n.abstract_names.remove(a);
                }
            }
            Address::Path(_) => {
                if let Some(k) = key
                    && n.paths.get(&k).is_some_and(is_me)
                {
                    n.paths.remove(&k);
                }
            }
            Address::Unnamed => {}
        }
    }
}

impl UnixSocket {
    /// A new, unconnected socket owned by a process with credentials
    /// `creds`.
    pub fn new(ns: &Arc<Namespace>, kind: SocketType, creds: Credentials) -> Arc<UnixSocket> {
        Arc::new(UnixSocket {
            ns: Arc::clone(ns),
            kind,
            queue: Arc::new(WaitQueue::new()),
            state: SpinMutex::new(State {
                conn: Conn::Fresh,
                peer: None,
                dgram_peer: None,
                name: Address::Unnamed,
                peer_name: Address::Unnamed,
                rx: VecDeque::new(),
                rx_bytes: 0,
                rd_shut: false,
                wr_shut: false,
                peer_done: false,
                peer_gone: false,
                closed: false,
                error: None,
                passcred: false,
                creds,
                peer_creds: None,
                sndbuf: DEFAULT_BUFFER,
                rcvbuf: DEFAULT_BUFFER,
                blocked_senders: Vec::new(),
                holds_rights: false,
                path_key: None,
            }),
            rcv_timeout: AtomicU64::new(0),
            snd_timeout: AtomicU64::new(0),
            options: SpinMutex::new(SocketOptions { rcv_lowat: 1, ..SocketOptions::default() }),
        })
    }

    /// socketpair(2): two connected unnamed sockets, each with the other's
    /// (the creator's) credentials as `SO_PEERCRED`.
    pub fn pair(ns: &Arc<Namespace>, kind: SocketType, creds: Credentials) -> (Arc<UnixSocket>, Arc<UnixSocket>) {
        let a = UnixSocket::new(ns, kind, creds);
        let b = UnixSocket::new(ns, kind, creds);
        for (me, other) in [(&a, &b), (&b, &a)] {
            let mut s = me.state.lock();
            s.peer_creds = Some(creds);
            if kind.connected_kind() {
                s.conn = Conn::Connected;
                s.peer = Some(Arc::clone(other));
            } else {
                s.dgram_peer = Some(Arc::downgrade(other));
            }
        }
        (a, b)
    }

    pub fn kind(&self) -> SocketType {
        self.kind
    }

    /// The queue to wait on for this socket's readiness.
    pub fn wait_queue(&self) -> &Arc<WaitQueue> {
        &self.queue
    }

    // ---- names -----------------------------------------------------------

    fn check_bindable(&self) -> Result<()> {
        let s = self.state.lock();
        if s.closed {
            return Err(Errno::EBADF);
        }
        if s.name != Address::Unnamed {
            return Err(Errno::EINVAL);
        }
        Ok(())
    }

    /// bind(2) in the abstract namespace (`EADDRINUSE` if taken, `EINVAL`
    /// if this socket has a name already; a taken name is reported first:
    /// bind's spec sheet).
    pub fn bind_abstract(self: &Arc<Self>, name: &[u8]) -> Result<()> {
        let taken = |n: &Names| n.abstract_names.get(name).is_some_and(|w| w.strong_count() > 0);
        if taken(&*self.ns.names.lock()) {
            return Err(Errno::EADDRINUSE);
        }
        self.check_bindable()?;
        {
            let mut n = self.ns.names.lock();
            if taken(&*n) {
                return Err(Errno::EADDRINUSE);
            }
            n.abstract_names.insert(name.to_vec(), Arc::downgrade(self));
        }
        self.set_name(Address::Abstract(name.to_vec()))
    }

    /// Bind to a name in the abstract namespace the kernel picks: a NUL and
    /// five hexadecimal digits (unix(7), "Autobind feature").
    pub fn autobind(self: &Arc<Self>) -> Result<()> {
        self.check_bindable()?;
        let name = {
            let mut n = self.ns.names.lock();
            let mut found = None;
            for _ in 0..(1u32 << 20) {
                let v = n.next_auto & 0xf_ffff;
                n.next_auto = n.next_auto.wrapping_add(1);
                let mut name = Vec::with_capacity(5);
                for shift in (0..5).rev() {
                    name.push(b"0123456789abcdef"[((v >> (shift * 4)) & 0xf) as usize]);
                }
                if !n.abstract_names.get(&name).is_some_and(|w| w.strong_count() > 0) {
                    found = Some(name);
                    break;
                }
            }
            let name = found.ok_or(Errno::EADDRINUSE)?;
            n.abstract_names.insert(name.clone(), Arc::downgrade(self));
            name
        };
        self.set_name(Address::Abstract(name))
    }

    /// bind(2) to a path: makes the socket inode (mode `0777` less the
    /// umask, unix(7)) with `Vfs::mknod`; `EADDRINUSE` if the path exists,
    /// even for a socket that has a name already (bind's spec sheet: the
    /// socket's own path again), else `EINVAL` for that.
    pub fn bind_path(self: &Arc<Self>, vfs: &Vfs, ctx: &FsContext, cred: &Cred, at: At<'_>, path: &[u8]) -> Result<()> {
        if self.check_bindable().is_err() {
            return match vfs.stat(ctx, cred, at, path, false) {
                Ok(_) => Err(Errno::EADDRINUSE),
                Err(_) => Err(Errno::EINVAL),
            };
        }
        let made = vfs.mknod(ctx, cred, at, path, FileType::Socket, 0o777, DeviceId::default());
        made.map_err(|e| if e == Errno::EEXIST { Errno::EADDRINUSE } else { e })?;
        let st = vfs.stat(ctx, cred, at, path, false)?;
        let key = key_of(&st);
        self.ns.names.lock().paths.insert(key, Arc::downgrade(self));
        self.state.lock().path_key = Some(key);
        self.set_name(Address::Path(path.to_vec()))
    }

    fn set_name(&self, name: Address) -> Result<()> {
        let mut s = self.state.lock();
        if s.name != Address::Unnamed {
            // Raced with another bind of this socket.
            return Err(Errno::EINVAL);
        }
        s.name = name;
        Ok(())
    }

    /// getsockname(2).
    pub fn local_name(&self) -> Address {
        self.state.lock().name.clone()
    }

    /// getpeername(2): `ENOTCONN` unless connected.
    pub fn peer_name(&self) -> Result<Address> {
        let s = self.state.lock();
        if self.kind.connected_kind() {
            match s.conn {
                Conn::Connected | Conn::Disconnected => Ok(s.peer_name.clone()),
                _ => Err(Errno::ENOTCONN),
            }
        } else {
            let peer = s.dgram_peer.as_ref().and_then(Weak::upgrade).ok_or(Errno::ENOTCONN)?;
            drop(s);
            Ok(peer.local_name())
        }
    }

    // ---- connections -------------------------------------------------------

    /// listen(2): `EINVAL` for a connected or an unbound socket (listen's
    /// spec sheet), `EOPNOTSUPP` for a datagram one. A listening socket's
    /// backlog can be changed by listening again; the queue holds
    /// `backlog + 1` pending connections.
    pub fn listen(self: &Arc<Self>, backlog: usize, creds: Credentials) -> Result<()> {
        if !self.kind.connected_kind() {
            return Err(Errno::EOPNOTSUPP);
        }
        if self.state.lock().name == Address::Unnamed {
            return Err(Errno::EINVAL);
        }
        let mut s = self.state.lock();
        let backlog = backlog.min(MAX_BACKLOG);
        match &mut s.conn {
            Conn::Fresh => {
                s.conn = Conn::Listening(Listen { backlog, pending: VecDeque::new(), connectors: Vec::new() });
                s.creds = creds;
                Ok(())
            }
            Conn::Listening(l) => {
                l.backlog = backlog;
                Ok(())
            }
            _ => Err(Errno::EINVAL),
        }
    }

    /// Whether `listen` was called (`SO_ACCEPTCONN`).
    pub fn is_listening(&self) -> bool {
        matches!(self.state.lock().conn, Conn::Listening(_))
    }

    /// connect(2) of a stream or sequenced-packet socket to the listening
    /// socket `target`, or of a datagram socket (its default destination
    /// and only source). `EAGAIN` while the backlog is full (this socket
    /// is woken when a slot frees), `ECONNREFUSED` if `target` does not
    /// listen, `EPROTOTYPE` for a socket of another type, `EISCONN` if
    /// connected already, `EINVAL` on a listening socket.
    pub fn connect(self: &Arc<Self>, target: &Arc<UnixSocket>, creds: Credentials) -> Result<()> {
        if target.kind != self.kind {
            return Err(Errno::EPROTOTYPE);
        }
        if !self.kind.connected_kind() {
            return self.connect_dgram(target);
        }
        let passcred_unnamed = {
            let s = self.state.lock();
            match s.conn {
                Conn::Fresh => {}
                Conn::Listening(_) => return Err(Errno::EINVAL),
                Conn::Connected | Conn::Disconnected => return Err(Errno::EISCONN),
            }
            s.passcred && s.name == Address::Unnamed
        };
        if passcred_unnamed {
            // unix(7): SO_PASSCRED on an unbound socket autobinds it.
            let _ = self.autobind();
        }
        let my_name = self.local_name();
        // The server's end, handed out by accept.
        let server = UnixSocket::new(&self.ns, self.kind, creds);
        let (listener_name, listener_creds) = {
            let mut t = target.state.lock();
            let name = t.name.clone();
            let lcreds = t.creds;
            let Conn::Listening(l) = &mut t.conn else { return Err(Errno::ECONNREFUSED) };
            if l.pending.len() > l.backlog {
                l.connectors.push(Arc::downgrade(self));
                return Err(Errno::EAGAIN);
            }
            {
                let mut e = server.state.lock();
                e.conn = Conn::Connected;
                e.peer = Some(Arc::clone(self));
                e.name = name.clone();
                e.peer_name = my_name;
                e.peer_creds = Some(creds);
                e.creds = lcreds;
            }
            l.pending.push_back(Arc::clone(&server));
            (name, lcreds)
        };
        {
            let mut s = self.state.lock();
            s.conn = Conn::Connected;
            s.peer = Some(server);
            s.peer_name = listener_name;
            s.peer_creds = Some(listener_creds);
            s.creds = creds;
        }
        let mut w = Wakes::default();
        w.add(target, IN);
        w.add(self, OUT);
        w.run();
        Ok(())
    }

    fn connect_dgram(self: &Arc<Self>, target: &Arc<UnixSocket>) -> Result<()> {
        if target.state.lock().closed {
            return Err(Errno::ECONNREFUSED);
        }
        let mut s = self.state.lock();
        s.dgram_peer = Some(Arc::downgrade(target));
        s.peer_creds = Some(target.state.lock().creds);
        Ok(())
    }

    /// connect(2) with `AF_UNSPEC` on a datagram socket: no default
    /// destination any more.
    pub fn disconnect(&self) -> Result<()> {
        if self.kind.connected_kind() {
            return Err(Errno::EINVAL);
        }
        self.state.lock().dgram_peer = None;
        Ok(())
    }

    /// Is this a connected stream or sequenced-packet socket (perhaps with
    /// its peer gone)?
    pub fn is_connected(&self) -> bool {
        matches!(self.state.lock().conn, Conn::Connected | Conn::Disconnected)
    }

    /// accept(2): the next pending connection; `EAGAIN` if none, `EINVAL`
    /// if not listening, `EOPNOTSUPP` for a datagram socket.
    pub fn accept(&self) -> Result<Arc<UnixSocket>> {
        if !self.kind.connected_kind() {
            return Err(Errno::EOPNOTSUPP);
        }
        let (sock, connectors) = {
            let mut s = self.state.lock();
            let Conn::Listening(l) = &mut s.conn else { return Err(Errno::EINVAL) };
            let sock = l.pending.pop_front().ok_or(Errno::EAGAIN)?;
            (sock, core::mem::take(&mut l.connectors))
        };
        // A slot freed: connectors waiting for one try again.
        let mut w = Wakes::default();
        for c in connectors.iter().filter_map(Weak::upgrade) {
            w.add(&c, OUT);
        }
        w.run();
        Ok(sock)
    }

    // ---- data ----------------------------------------------------------------

    /// send(2) / sendmsg(2): queue `data` (with `anc`) at the peer, or at
    /// `to` for a datagram socket. Streams take what fits (at least one
    /// byte) and return how much; datagram and sequenced-packet sockets
    /// take all of it or nothing. `creds` are the sender's, reported to a
    /// receiver with `SO_PASSCRED` unless `anc.creds` names others.
    ///
    /// Errors: `EAGAIN` without room, `EPIPE` on a stream whose peer is
    /// gone or that shut down writing, `ENOTCONN` without a peer or
    /// destination, `EISCONN` with a destination on a connected socket,
    /// `EMSGSIZE` for a datagram above the send buffer, `EINVAL` for more
    /// than [`SCM_MAX_FD`] descriptors, `ECONNREFUSED`, `EPROTOTYPE`,
    /// `EPERM` (see the crate documentation).
    pub fn send(
        self: &Arc<Self>,
        data: &[u8],
        to: Option<&Arc<UnixSocket>>,
        anc: Ancillary,
        creds: Credentials,
    ) -> Result<usize> {
        if anc.rights.len() > SCM_MAX_FD {
            return Err(Errno::EINVAL);
        }
        if self.kind.connected_kind() {
            self.send_connected(data, to, anc, creds)
        } else {
            self.send_dgram(data, to, anc, creds)
        }
    }

    fn send_connected(
        self: &Arc<Self>,
        data: &[u8],
        to: Option<&Arc<UnixSocket>>,
        anc: Ancillary,
        creds: Credentials,
    ) -> Result<usize> {
        let (peer, from, limit) = {
            let s = self.state.lock();
            match s.conn {
                Conn::Fresh | Conn::Listening(_) => {
                    return Err(if to.is_some() { Errno::EOPNOTSUPP } else { Errno::ENOTCONN });
                }
                Conn::Connected | Conn::Disconnected => {}
            }
            if to.is_some() {
                return Err(Errno::EISCONN);
            }
            if s.wr_shut || s.peer_gone {
                return Err(Errno::EPIPE);
            }
            let Some(peer) = s.peer.clone() else { return Err(Errno::EPIPE) };
            (peer, s.name.clone(), s.sndbuf)
        };
        if self.kind == SocketType::SeqPacket && data.len() > limit.saturating_sub(DGRAM_OVERHEAD) {
            return Err(Errno::EMSGSIZE);
        }
        if data.is_empty() && self.kind == SocketType::Stream {
            // Nothing to queue; descriptors need a byte to travel with
            // (unix(7)) and are dropped.
            return Ok(0);
        }
        let n = {
            let mut p = peer.state.lock();
            if p.closed || p.rd_shut {
                return Err(Errno::EPIPE);
            }
            let room = limit.saturating_sub(p.rx_bytes);
            let n = match self.kind {
                SocketType::Stream => data.len().min(room),
                _ if p.rx.is_empty() || data.len() <= room => data.len(),
                _ => 0,
            };
            if n == 0 && !(self.kind == SocketType::SeqPacket && data.is_empty()) {
                drop(p);
                return Err(Errno::EAGAIN);
            }
            let rights = anc.rights;
            let held = !rights.is_empty();
            self.ns.in_flight.fetch_add(rights.len(), Ordering::SeqCst);
            p.rx_bytes += n;
            p.rx.push_back(Packet {
                data: data[..n].to_vec(),
                off: 0,
                from,
                creds: anc.creds.unwrap_or(creds),
                rights,
            });
            let note = held && !p.holds_rights;
            if note {
                p.holds_rights = true;
            }
            drop(p);
            if note {
                self.ns.note_holding(&peer);
            }
            n
        };
        peer.queue.notify(IN);
        Ok(n)
    }

    fn send_dgram(
        self: &Arc<Self>,
        data: &[u8],
        to: Option<&Arc<UnixSocket>>,
        anc: Ancillary,
        creds: Credentials,
    ) -> Result<usize> {
        let (dest, from, limit, passcred_unnamed) = {
            let s = self.state.lock();
            if s.wr_shut {
                return Err(Errno::EPIPE);
            }
            let connected = s.dgram_peer.clone();
            let dest = match (to, connected) {
                (Some(_), Some(_)) => return Err(Errno::EISCONN),
                (Some(t), None) => Arc::clone(t),
                (None, Some(w)) => w.upgrade().ok_or(Errno::ECONNREFUSED)?,
                (None, None) => return Err(Errno::ENOTCONN),
            };
            (dest, s.name.clone(), s.sndbuf, s.passcred && s.name == Address::Unnamed)
        };
        if dest.kind != SocketType::Datagram {
            return Err(Errno::EPROTOTYPE);
        }
        if data.len() > limit.saturating_sub(DGRAM_OVERHEAD) {
            return Err(Errno::EMSGSIZE);
        }
        let from = if passcred_unnamed && self.autobind().is_ok() { self.local_name() } else { from };
        {
            let mut d = dest.state.lock();
            if d.closed {
                return Err(Errno::ECONNREFUSED);
            }
            if let Some(p) = &d.dgram_peer
                && !core::ptr::eq(p.as_ptr(), Arc::as_ptr(self))
            {
                // A connected datagram socket takes datagrams from its peer
                // only (connect(2)).
                return Err(Errno::EPERM);
            }
            if d.rd_shut {
                return Err(Errno::EPIPE);
            }
            let full = !d.rx.is_empty() && (d.rx.len() >= MAX_DGRAM_QUEUE || d.rx_bytes + data.len() > DEFAULT_BUFFER);
            if full {
                d.blocked_senders.push(Arc::downgrade(self));
                return Err(Errno::EAGAIN);
            }
            let rights = anc.rights;
            let held = !rights.is_empty();
            self.ns.in_flight.fetch_add(rights.len(), Ordering::SeqCst);
            d.rx_bytes += data.len();
            d.rx.push_back(Packet { data: data.to_vec(), off: 0, from, creds: anc.creds.unwrap_or(creds), rights });
            let note = held && !d.holds_rights;
            if note {
                d.holds_rights = true;
            }
            drop(d);
            if note {
                self.ns.note_holding(&dest);
            }
        }
        dest.queue.notify(IN);
        Ok(data.len())
    }

    /// recv(2) / recvmsg(2): see the crate documentation for what one
    /// receive takes. `EAGAIN` with nothing queued (unless at end of
    /// file: `len` 0), `EINVAL` on a listening socket, `ENOTCONN` on an
    /// unconnected stream, a pending `ECONNRESET` once the queue is empty.
    pub fn recv(&self, buf: &mut [u8], peek: bool) -> Result<Received> {
        let r = self.recv_locked(buf, peek);
        if let Ok(got) = &r
            && !peek
            && got.full_len > 0
        {
            self.after_read();
        }
        r
    }

    fn recv_locked(&self, buf: &mut [u8], peek: bool) -> Result<Received> {
        let mut s = self.state.lock();
        if let Conn::Listening(_) = s.conn {
            return Err(Errno::EINVAL);
        }
        if s.rx.is_empty() {
            if let Some(e) = s.error.take() {
                return Err(e);
            }
            let eof = s.rd_shut || s.peer_done || matches!(s.conn, Conn::Disconnected);
            if self.kind.connected_kind() && matches!(s.conn, Conn::Fresh) && !s.rd_shut {
                return Err(Errno::ENOTCONN);
            }
            if eof || (!self.kind.connected_kind() && s.rd_shut) {
                return Ok(Received { len: 0, full_len: 0, from: Address::Unnamed, rights: Vec::new(), creds: None });
            }
            return Err(Errno::EAGAIN);
        }
        let passcred = s.passcred;
        if self.kind != SocketType::Stream {
            let p = s.rx.front().expect("not empty");
            let n = p.data.len().min(buf.len());
            buf[..n].copy_from_slice(&p.data[..n]);
            let full_len = p.data.len();
            let from = p.from.clone();
            let creds = passcred.then_some(p.creds);
            let rights = if peek {
                p.rights.clone()
            } else {
                let p = s.rx.pop_front().expect("not empty");
                s.rx_bytes -= p.data.len();
                // The descriptors are the receiver's now.
                self.ns.in_flight.fetch_sub(p.rights.len(), Ordering::SeqCst);
                p.rights
            };
            return Ok(Received { len: n, full_len, from, rights, creds });
        }
        // A stream: bytes across packets, up to the ancillary barriers.
        let mut copied = 0;
        let mut rights = Vec::new();
        let mut first_creds = None;
        let mut from = Address::Unnamed;
        let mut index = 0;
        while copied < buf.len() {
            let Some(p) = s.rx.get(index) else { break };
            if let Some(c) = first_creds
                && passcred
                && c != p.creds
            {
                break;
            }
            if index == 0 {
                first_creds = Some(p.creds);
                from = p.from.clone();
            }
            let n = p.left().min(buf.len() - copied);
            buf[copied..copied + n].copy_from_slice(&p.data[p.off..p.off + n]);
            copied += n;
            let barrier = !p.rights.is_empty();
            if peek {
                if barrier {
                    rights = p.rights.clone();
                }
                index += 1;
            } else {
                let p = s.rx.front_mut().expect("present");
                p.off += n;
                if barrier {
                    rights = core::mem::take(&mut p.rights);
                    self.ns.in_flight.fetch_sub(rights.len(), Ordering::SeqCst);
                }
                if p.left() == 0 {
                    s.rx.pop_front();
                }
                s.rx_bytes -= n;
            }
            if barrier {
                break;
            }
        }
        let creds = if passcred { first_creds } else { None };
        Ok(Received { len: copied, full_len: copied, from, rights, creds })
    }

    /// After a receive took data: the peer (stream) or blocked senders
    /// (datagrams) may write again.
    fn after_read(&self) {
        let (peer, senders) = {
            let mut s = self.state.lock();
            (s.peer.clone(), core::mem::take(&mut s.blocked_senders))
        };
        let mut w = Wakes::default();
        if let Some(p) = peer {
            w.add(&p, OUT);
        }
        for snd in senders.iter().filter_map(Weak::upgrade) {
            w.add(&snd, OUT);
        }
        w.run();
    }

    /// Bytes a read would return (`FIONREAD`): everything queued for a
    /// stream, the next packet's size otherwise. `EINVAL` on a listening
    /// socket (unix(7)).
    pub fn bytes_readable(&self) -> Result<usize> {
        let s = self.state.lock();
        if let Conn::Listening(_) = s.conn {
            return Err(Errno::EINVAL);
        }
        Ok(match self.kind {
            SocketType::Stream => s.rx_bytes,
            _ => s.rx.front().map_or(0, |p| p.data.len()),
        })
    }

    /// Bytes this socket has queued at its peer that the peer has not read
    /// (`SIOCOUTQ`, for connected sockets; 0 otherwise).
    pub fn send_queued(&self) -> usize {
        let peer = self.state.lock().peer.clone();
        peer.map_or(0, |p| p.state.lock().rx_bytes)
    }

    /// shutdown(2). The peer sees end of file (`SHUT_WR`) or gets `EPIPE`
    /// when it writes (`SHUT_RD`). A socket that is not connected accepts
    /// it too (shutdown's spec sheet: an unconnected `AF_UNIX` stream).
    pub fn shutdown(&self, how: Shutdown) -> Result<()> {
        let peer = {
            let mut s = self.state.lock();
            if matches!(how, Shutdown::Read | Shutdown::Both) {
                s.rd_shut = true;
            }
            if matches!(how, Shutdown::Write | Shutdown::Both) {
                s.wr_shut = true;
            }
            s.peer.clone()
        };
        if let Some(p) = &peer {
            let mut ps = p.state.lock();
            if matches!(how, Shutdown::Write | Shutdown::Both) {
                ps.peer_done = true;
            }
            if matches!(how, Shutdown::Read | Shutdown::Both) {
                ps.peer_gone = true;
            }
        }
        let mut w = Wakes::default();
        w.add(self, HUP_ALL);
        if let Some(p) = &peer {
            w.add(p, HUP_ALL);
        }
        w.run();
        Ok(())
    }

    /// The last reference from user space is gone (close(2) of the last
    /// descriptor): drop the name, the queue (and the descriptors in it)
    /// and pending connections, and tell the peer.
    pub fn close(&self) {
        let (peer, queue, pending, name, key, connectors, senders, had_data) = {
            let mut s = self.state.lock();
            if s.closed {
                return;
            }
            s.closed = true;
            let had_data = !s.rx.is_empty();
            let queue = core::mem::take(&mut s.rx);
            s.rx_bytes = 0;
            let (pending, connectors) = match core::mem::replace(&mut s.conn, Conn::Disconnected) {
                Conn::Listening(l) => (l.pending, l.connectors),
                _ => (VecDeque::new(), Vec::new()),
            };
            let key = s.path_key.take();
            (
                s.peer.take(),
                queue,
                pending,
                s.name.clone(),
                key,
                connectors,
                core::mem::take(&mut s.blocked_senders),
                had_data,
            )
        };
        self.ns.remove_name(self, &name, key);
        let mut w = Wakes::default();
        if let Some(p) = &peer {
            let mut ps = p.state.lock();
            ps.peer_done = true;
            ps.peer_gone = true;
            if matches!(ps.conn, Conn::Connected) {
                ps.conn = Conn::Disconnected;
            }
            if had_data {
                ps.error = Some(Errno::ECONNRESET);
            }
            drop(ps);
            w.add(p, HUP_ALL);
        }
        for c in connectors.iter().filter_map(Weak::upgrade) {
            w.add(&c, HUP_ALL);
        }
        for snd in senders.iter().filter_map(Weak::upgrade) {
            w.add(&snd, HUP_ALL);
        }
        w.add(self, HUP_ALL);
        // Connections nobody accepted end: their clients see a reset.
        for e in &pending {
            let peer = e.state.lock().peer.clone();
            if let Some(c) = peer {
                c.state.lock().error = Some(Errno::ECONNRESET);
            }
            e.close();
        }
        let released: usize = queue.iter().map(|p| p.rights.len()).sum();
        self.ns.in_flight.fetch_sub(released, Ordering::SeqCst);
        drop(peer);
        // The queued descriptors go now, with no lock held (closing one may
        // close sockets in turn).
        drop(queue);
        drop(pending);
        w.run();
    }

    /// Whether the socket was closed.
    pub fn is_closed(&self) -> bool {
        self.state.lock().closed
    }

    // ---- readiness and options -------------------------------------------------

    /// What the socket is ready for (poll(2), socket(7)'s table).
    pub fn poll(&self) -> PollEvents {
        let (s_peer, mut e) = {
            let s = self.state.lock();
            let mut e = PollEvents::NONE;
            if s.error.is_some() {
                e = e | PollEvents::ERR;
            }
            match &s.conn {
                Conn::Listening(l) => {
                    if !l.pending.is_empty() {
                        e = e | IN;
                    }
                    return e;
                }
                Conn::Fresh if self.kind.connected_kind() => {
                    // Never connected: nothing to read or write.
                    return e | PollEvents::HUP | OUT;
                }
                _ => {}
            }
            if !s.rx.is_empty() || s.rd_shut || s.peer_done || s.error.is_some() {
                e = e | IN;
            }
            if s.rd_shut || s.peer_done {
                e = e | PollEvents::RDHUP;
            }
            let both = (s.rd_shut || s.peer_done) && (s.wr_shut || s.peer_gone);
            if both || matches!(s.conn, Conn::Disconnected) {
                e = e | PollEvents::HUP;
            }
            let peer = if self.kind.connected_kind() {
                s.peer.clone().map(|p| (p, s.sndbuf))
            } else {
                s.dgram_peer.as_ref().and_then(Weak::upgrade).map(|p| (p, 0))
            };
            if s.wr_shut || s.peer_gone {
                // A write would not block: it fails at once.
                e = e | OUT;
                (None, e)
            } else {
                (Some(peer), e)
            }
        };
        if let Some(peer) = s_peer {
            let writable = match peer {
                None => !self.kind.connected_kind(),
                Some((p, limit)) => {
                    let ps = p.state.lock();
                    if ps.closed {
                        true
                    } else if self.kind.connected_kind() {
                        ps.rx_bytes < limit
                    } else {
                        ps.rx.len() < MAX_DGRAM_QUEUE && ps.rx_bytes < DEFAULT_BUFFER
                    }
                }
            };
            if writable {
                e = e | OUT;
            }
        }
        e
    }

    /// `SO_PASSCRED`.
    pub fn set_passcred(&self, on: bool) {
        self.state.lock().passcred = on;
    }

    pub fn passcred(&self) -> bool {
        self.state.lock().passcred
    }

    /// `SO_PEERCRED`: the peer's credentials as of `connect`, `listen` or
    /// `socketpair` (unix(7)); `None` when not connected.
    pub fn peer_credentials(&self) -> Option<Credentials> {
        self.state.lock().peer_creds
    }

    /// `SO_SNDBUF` / `SO_RCVBUF` as set (already doubled and clamped by the
    /// caller).
    pub fn set_buffers(&self, send: Option<usize>, recv: Option<usize>) {
        let mut s = self.state.lock();
        if let Some(v) = send {
            s.sndbuf = v.max(MIN_SNDBUF);
        }
        if let Some(v) = recv {
            s.rcvbuf = v.max(MIN_RCVBUF);
        }
    }

    /// `(SO_SNDBUF, SO_RCVBUF)`.
    pub fn buffers(&self) -> (usize, usize) {
        let s = self.state.lock();
        (s.sndbuf, s.rcvbuf)
    }

    /// `SO_ERROR`: take the pending error.
    pub fn take_error(&self) -> Option<Errno> {
        self.state.lock().error.take()
    }

    /// `SO_RCVTIMEO` / `SO_SNDTIMEO` in nanoseconds (0: none).
    pub fn set_timeouts(&self, recv: Option<u64>, send: Option<u64>) {
        if let Some(t) = recv {
            self.rcv_timeout.store(t, Ordering::Relaxed);
        }
        if let Some(t) = send {
            self.snd_timeout.store(t, Ordering::Relaxed);
        }
    }

    /// `(SO_RCVTIMEO, SO_SNDTIMEO)` in nanoseconds; 0 for none.
    pub fn timeouts(&self) -> (u64, u64) {
        (self.rcv_timeout.load(Ordering::Relaxed), self.snd_timeout.load(Ordering::Relaxed))
    }

    /// Read or change the generic options.
    pub fn with_options<R>(&self, f: impl FnOnce(&mut SocketOptions) -> R) -> R {
        f(&mut self.options.lock())
    }

    /// The credentials of the owning process, as `read` / `write` use them.
    pub fn owner_credentials(&self) -> Credentials {
        self.state.lock().creds
    }
}

impl Drop for UnixSocket {
    fn drop(&mut self) {
        // A socket that was never closed (an unaccepted connection whose
        // listener is gone without closing it, a test's socket): its
        // queued descriptors leave the in-flight count with it.
        let s = self.state.get_mut();
        let held: usize = s.rx.iter().map(|p| p.rights.len()).sum();
        self.ns.in_flight.fetch_sub(held, Ordering::SeqCst);
    }
}

/// The open file behind a socket descriptor.
pub struct UnixFile {
    sock: Arc<UnixSocket>,
}

impl UnixFile {
    pub fn new(sock: Arc<UnixSocket>) -> UnixFile {
        UnixFile { sock }
    }

    pub fn socket(&self) -> &Arc<UnixSocket> {
        &self.sock
    }
}

impl core::fmt::Debug for UnixFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("UnixFile").field(&self.sock).finish()
    }
}

impl Drop for UnixFile {
    fn drop(&mut self) {
        self.sock.close();
    }
}

fn nonzero(t: u64) -> Option<u64> {
    (t != 0).then_some(t)
}

impl FileObject for UnixFile {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        // Descriptors that came with the data are dropped (read(2) has no
        // way to take them).
        self.sock.recv(buf, false).map(|r| r.len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        let creds = self.sock.owner_credentials();
        self.sock.send(buf, None, Ancillary::default(), creds)
    }

    fn poll(&self) -> PollEvents {
        self.sock.poll()
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.sock.queue)
    }

    fn bytes_readable(&self) -> Option<usize> {
        self.sock.bytes_readable().ok()
    }

    fn timeouts(&self) -> (Option<u64>, Option<u64>) {
        let (r, w) = self.sock.timeouts();
        (nonzero(r), nonzero(w))
    }
}
