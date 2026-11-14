//! Sockets for the Linux personality (ADR 0048): `LinuxNet` for the
//! running thread, over four kinds of socket files.
//!
//! | Family | Open file's object | State |
//! |---|---|---|
//! | `AF_INET`, `AF_INET6` | [`InetSocket`] (`net/file.rs`) | the `poly-net` stack of its network namespace behind `net::call_in` |
//! | `AF_UNIX` | `poly_unix::UnixFile` | `poly-unix`, one [`Namespace`] for the system, abstract names per network namespace |
//! | `AF_NETLINK` | [`NetlinkFile`] (`netlink.rs`) | the personality's `NETLINK_ROUTE` over the devices of its network namespace |
//! | raw (`SOCK_RAW`, `AF_PACKET`) | [`RawSocket`] | none: it can be made and closed, no packet goes through it |
//!
//! A socket belongs to the network namespace of the process that made it
//! (ADR 0178): an Internet socket is in that namespace's stack
//! ([`stack_of`]), a Unix socket's abstract names are that namespace's, and
//! the interface requests of ioctl(2) and rtnetlink on any socket act on
//! that namespace's devices.
//!
//! Every socket has a wait queue its readiness changes are notified on,
//! so blocking calls here wait with `poly_vfs::poll::wait_until_done` and
//! the thread's [`ThreadBlocker`] (a signal ends the wait: `EINTR`, which
//! the personality turns into a restart or not), `O_NONBLOCK` and
//! `MSG_DONTWAIT` make them try once, and `SO_RCVTIMEO` / `SO_SNDTIMEO`
//! give them a deadline. `read`, `write`, `poll` and `epoll` reach the
//! same objects through the VFS.
//!
//! Descriptors in flight (`SCM_RIGHTS`) are references to open file
//! descriptions held by the receiving socket's queue; [`after_close`]
//! collects cycles of them that no process can reach any more.

mod netlink;

pub use netlink::{NetlinkFile, interfaces, interfaces_of};

use super::Thread;
use super::services::vfs_errno;
use crate::fs::poll::ThreadBlocker;
use crate::fs::{self, FileType, OpenFile, OpenFlags};
use crate::net::file::{InetSocket, errno as net_errno};
use crate::net::{Domain, NsStack, Protocol, Shutdown as NetShutdown, SockType as NetType};
use crate::sync::{SetOnce, SpinLock};
use crate::time;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::any::Any;
use core::net::{SocketAddr, SocketAddrV6};
use core::time::Duration;
use poly_personality_linux::LinuxKernel;
use poly_personality_linux::errno::Errno;
use poly_personality_linux::fileio::NewFd;
use poly_personality_linux::net::netlink::Interfaces;
use poly_personality_linux::net::{
    Family, Kind, LinuxNet, OptName, OptValue, RecvMsg, RecvReq, SendMsg, Shutdown, SockAddr, SockOpt, SockType, Ucred,
    UnixAddr,
};
use poly_personality_linux::ns::{self, NetNs};
use poly_unix::{Address, Ancillary, Credentials, Namespace, SocketType, UnixFile, UnixSocket};
use poly_vfs::poll::{self, Timeout, WaitQueue};
use poly_vfs::{At, FileObject, PollEvents};

/// The system's Unix socket names and descriptors in flight.
pub(crate) fn namespace() -> &'static Arc<Namespace> {
    static NS: SetOnce<Arc<Namespace>> = SetOnce::new();
    if let Some(ns) = NS.get() {
        return ns;
    }
    match NS.set(Namespace::new()) {
        Ok(ns) => ns,
        Err(_) => NS.get().expect("set by someone else"),
    }
}

/// After a descriptor was closed: with descriptors in flight, collect the
/// ones only unreachable sockets hold (`poly_unix::Namespace::collect`).
pub fn after_close() {
    let ns = namespace();
    if ns.in_flight() > 0 {
        ns.collect();
    }
}

/// Descriptors in flight (for tests).
pub fn in_flight() -> usize {
    namespace().in_flight()
}

/// The stack of network namespace `ns` (`None`, the initial one: the
/// interface's, also `None`): made the first time a socket or a device
/// request of the namespace needs it, with its loopback as the
/// namespace's devices say.
pub(crate) fn stack_of(ns: &Option<Arc<NetNs>>) -> Option<Arc<NsStack>> {
    let n = ns.as_ref()?;
    let mut made = false;
    let state = n.kernel_state(|| {
        made = true;
        NsStack::new()
    });
    let stack = state.downcast::<NsStack>().ok()?;
    if made {
        let up = n.with_devices(|d| d.lo_up());
        crate::net::call_in(Some(&stack), |s| s.set_loopback(up));
    }
    Some(stack)
}

/// The stack of network namespace `ns` if it has one already.
pub(crate) fn existing_stack(ns: &Option<Arc<NetNs>>) -> Option<Arc<NsStack>> {
    ns.as_ref()?.kernel_state_if_made()?.downcast::<NsStack>().ok()
}

/// The number that stands for network namespace `ns` among the scopes of
/// abstract Unix socket names: its inode number, 0 for the initial one.
fn scope_of(ns: &Option<Arc<NetNs>>) -> u32 {
    ns.as_ref().map_or(0, |n| n.inum())
}

/// Whether the running thread may change the devices of network
/// namespace `ns`: `CAP_NET_ADMIN` in the user namespace that owns it.
pub(crate) fn net_admin(ns: &Option<Arc<NetNs>>) -> bool {
    super::cred::current_creds()
        .is_some_and(|c| ns::user::ns_capable(&c, &ns::net_owner(ns), poly_personality_linux::cred::cap::NET_ADMIN))
}

/// A raw socket (`SOCK_RAW` of `AF_INET` / `AF_INET6`, or `AF_PACKET`;
/// raw(7), packet(7)): sandboxes and their tests make one to see whether
/// they hold `CAP_NET_RAW`. It can be made, bound, given options and
/// closed; nothing is carried through it: a send finds no route
/// (`ENETUNREACH`) and a receive waits for packets that never come.
pub struct RawSocket {
    kind: Kind,
    ns: Option<Arc<NetNs>>,
    queue: Arc<WaitQueue>,
    options: SpinLock<poly_unix::SocketOptions>,
    /// `SO_RCVTIMEO` / `SO_SNDTIMEO` in nanoseconds; 0: none.
    timeouts: SpinLock<(u64, u64)>,
}

impl RawSocket {
    fn new(kind: Kind, ns: Option<Arc<NetNs>>) -> Arc<RawSocket> {
        Arc::new(RawSocket {
            kind,
            ns,
            queue: Arc::new(WaitQueue::new()),
            options: SpinLock::new(poly_unix::SocketOptions { rcv_lowat: 1, ..poly_unix::SocketOptions::default() }),
            timeouts: SpinLock::new((0, 0)),
        })
    }
}

impl FileObject for RawSocket {
    fn read(&self, _buf: &mut [u8]) -> Result<usize, poly_vfs::Errno> {
        Err(poly_vfs::Errno::EAGAIN)
    }

    fn write(&self, _buf: &[u8]) -> Result<usize, poly_vfs::Errno> {
        Err(net_errno(poly_net::Errno::ENETUNREACH))
    }

    fn poll(&self) -> PollEvents {
        PollEvents::WRITABLE
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn timeouts(&self) -> (Option<u64>, Option<u64>) {
        let (r, w) = *self.timeouts.lock();
        ((r != 0).then_some(r), (w != 0).then_some(w))
    }
}

/// The four kinds of socket an open file can be.
enum Sock {
    Inet(Arc<InetSocket>),
    Unix(Arc<UnixFile>),
    Netlink(Arc<NetlinkFile>),
    Raw(Arc<RawSocket>),
}

fn sock_of(file: &OpenFile) -> Result<Sock, Errno> {
    if let Some(s) = file.object_as::<InetSocket>() {
        return Ok(Sock::Inet(s));
    }
    if let Some(s) = file.object_as::<UnixFile>() {
        return Ok(Sock::Unix(s));
    }
    if let Some(s) = file.object_as::<NetlinkFile>() {
        return Ok(Sock::Netlink(s));
    }
    if let Some(s) = file.object_as::<RawSocket>() {
        return Ok(Sock::Raw(s));
    }
    Err(Errno::ENOTSOCK)
}

fn verr(e: poly_vfs::Errno) -> Errno {
    vfs_errno(e)
}

fn nerr(e: poly_net::Errno) -> Errno {
    vfs_errno(net_errno(e))
}

/// A socket address of the stack's as the personality's.
fn inet_addr(a: SocketAddr) -> SockAddr {
    match a {
        SocketAddr::V4(v) => SockAddr::Inet(v),
        SocketAddr::V6(v) => SockAddr::Inet6(SocketAddrV6::new(*v.ip(), v.port(), 0, 0)),
    }
}

fn unix_addr(a: Address) -> SockAddr {
    SockAddr::Unix(match a {
        Address::Unnamed => UnixAddr::Unnamed,
        Address::Path(p) => UnixAddr::Path(p),
        Address::Abstract(n) => UnixAddr::Abstract(n),
    })
}

/// The address an Internet socket of `domain` takes (`EAFNOSUPPORT` for
/// another family, as bind(2) / connect(2) on Linux report it; to be
/// confirmed by the diff runner).
fn to_inet(domain: Domain, a: &SockAddr) -> Result<SocketAddr, Errno> {
    match (domain, a) {
        (Domain::Inet, SockAddr::Inet(v)) => Ok(SocketAddr::V4(*v)),
        (Domain::Inet6, SockAddr::Inet6(v)) => Ok(SocketAddr::V6(SocketAddrV6::new(*v.ip(), v.port(), 0, 0))),
        _ => Err(Errno::EAFNOSUPPORT),
    }
}

/// How long a call may wait: not at all for a non-blocking one, until the
/// socket's timeout (`ns`, 0: none), or for ever.
fn timeout(nonblock: bool, ns: u64) -> Timeout {
    if nonblock {
        Timeout::Now
    } else if ns == 0 {
        Timeout::Never
    } else {
        Timeout::At(time::now_ns().saturating_add(ns))
    }
}

/// `SO_PEERCRED` of a socket without a peer: pid 0, ids -1.
const NO_CREDS: Ucred = Ucred { pid: 0, uid: u32::MAX, gid: u32::MAX };

impl Thread<'_> {
    /// The network namespace of the calling process (`None`: the initial
    /// one): where its new sockets go.
    fn own_netns(&self) -> Option<Arc<NetNs>> {
        self.linux.with(|s| s.ns.net.clone())
    }

    fn sock_file(&self, fd: i32) -> Result<(Arc<OpenFile>, Sock), Errno> {
        let file = self.process.files.file(fd).map_err(verr)?;
        let sock = sock_of(&file)?;
        Ok((file, sock))
    }

    /// The calling process's id and real ids (unix(7): what a receiver
    /// with `SO_PASSCRED` gets unless the sender names others).
    fn unix_creds(&self) -> Credentials {
        let c = LinuxKernel::credentials(self);
        Credentials { pid: self.process.pid as i32, uid: c.uid, gid: c.gid }
    }

    /// Open `object` on a new socket inode and give it the lowest free
    /// descriptor.
    fn install_socket(&self, object: Arc<dyn FileObject>, new: NewFd) -> Result<i32, Errno> {
        let files = &self.process.files;
        let node = fs::vfs().anon_node(FileType::Socket, 0o777, &files.cred()).map_err(verr)?;
        let flags = if new.nonblock { OpenFlags::RDWR | OpenFlags::NONBLOCK } else { OpenFlags::RDWR };
        files.install(Arc::new(OpenFile::on_object(node, object, flags)), new.cloexec).map_err(verr)
    }

    /// Run `attempt` until it stops failing with `EAGAIN`, waiting on
    /// `queue` for `interest` as `wait` allows.
    fn wait_for<T>(
        &self,
        queue: &Arc<WaitQueue>,
        interest: PollEvents,
        wait: Timeout,
        attempt: impl FnMut() -> Result<T, poly_vfs::Errno>,
    ) -> Result<T, Errno> {
        let mut blocker = ThreadBlocker::new(self.process, Arc::clone(&self.flags));
        poll::wait_until_done(queue, interest, &mut blocker, wait, attempt).map_err(verr)
    }

    /// The Unix socket `addr` names, for connect and sendto of a socket
    /// made in `scope`: an abstract name is looked up among that network
    /// namespace's (unix(7)), a path by whoever can reach it.
    fn unix_target(&self, scope: u32, addr: &SockAddr) -> Result<Arc<UnixSocket>, Errno> {
        let ns = namespace();
        match addr {
            SockAddr::Unix(UnixAddr::Path(p)) => {
                let files = &self.process.files;
                ns.lookup_path(fs::vfs(), &files.context(), &files.cred(), At::Cwd, p).map_err(verr)
            }
            SockAddr::Unix(UnixAddr::Abstract(n)) => ns.lookup_abstract_in(scope, n).ok_or(Errno::ECONNREFUSED),
            _ => Err(Errno::EINVAL),
        }
    }

    /// Received open files as descriptors: at most `max` (and as many as
    /// the descriptor limit allows); the rest are closed with the
    /// references dropped here. Whether any were left out.
    fn install_rights(&self, rights: Vec<Arc<OpenFile>>, max: usize, cloexec: bool) -> (Vec<i32>, bool) {
        let mut fds = Vec::new();
        let mut truncated = rights.len() > max;
        for f in rights.into_iter().take(max) {
            match self.process.files.install(f, cloexec) {
                Ok(fd) => fds.push(fd),
                Err(_) => truncated = true,
            }
        }
        (fds, truncated)
    }

    fn inet_send(&self, s: &InetSocket, nonblock: bool, msg: &SendMsg<'_>) -> Result<usize, Errno> {
        // SCM_RIGHTS / SCM_CREDENTIALS mean nothing to an Internet socket:
        // accepted and ignored, the data is sent (sendmsg's spec sheet).
        let to = match &msg.to {
            None => None,
            // A connected stream ignores the destination (sendto(2)).
            Some(_) if s.ty == NetType::Stream => None,
            Some(a) => Some(to_inet(s.domain, a)?),
        };
        let wait = timeout(nonblock || msg.dontwait, s.state().snd_timeout);
        let queue = Arc::clone(s.queue());
        if s.ty != NetType::Stream {
            return self.wait_for(&queue, PollEvents::WRITABLE, wait, || s.send(msg.data, to));
        }
        // A blocking stream send takes everything unless interrupted or
        // timed out after some bytes (send(2)).
        let mut done = 0;
        loop {
            match self.wait_for(&queue, PollEvents::WRITABLE, wait, || s.send(&msg.data[done..], None)) {
                Ok(n) => {
                    done += n;
                    if done >= msg.data.len() || wait == Timeout::Now || n == 0 {
                        return Ok(done);
                    }
                }
                Err(_) if done > 0 => return Ok(done),
                // A TCP socket that never connected: EPIPE (sendto's spec
                // sheet), as for one whose connection is gone.
                Err(Errno::ENOTCONN) => return Err(Errno::EPIPE),
                Err(e) => return Err(e),
            }
        }
    }

    fn inet_recv(&self, s: &InetSocket, nonblock: bool, buf: &mut [u8], req: &RecvReq) -> Result<RecvMsg, Errno> {
        let wait = timeout(nonblock || req.dontwait, s.state().rcv_timeout);
        let queue = Arc::clone(s.queue());
        let stream = s.ty == NetType::Stream;
        let first = self.wait_for(&queue, PollEvents::READABLE, wait, || s.recv(buf, req.peek))?;
        let mut len = first.len;
        if stream && req.waitall && !req.peek && wait != Timeout::Now {
            // MSG_WAITALL: until the buffer is full, end of stream, an
            // error or a signal (recv(2)).
            while len < buf.len() && first.len > 0 {
                match self.wait_for(&queue, PollEvents::READABLE, wait, || s.recv(&mut buf[len..], false)) {
                    Ok(i) if i.len > 0 => len += i.len,
                    _ => break,
                }
            }
        }
        Ok(RecvMsg {
            len,
            full_len: if stream { len } else { first.full_len },
            from: (!stream).then(|| inet_addr(first.from)),
            ..RecvMsg::default()
        })
    }

    fn inet_connect(&self, s: &InetSocket, nonblock: bool, addr: &SockAddr) -> Result<(), Errno> {
        if *addr == SockAddr::Unspec {
            return s.with(|st, h| st.disconnect(h)).map_err(nerr);
        }
        let to = to_inet(s.domain, addr)?;
        let pending = s.state().connecting;
        match s.with(|st, h| st.connect(h, to)) {
            Ok(()) => return Ok(()),
            Err(poly_net::Errno::EINPROGRESS) => {}
            Err(poly_net::Errno::EALREADY) if pending && !nonblock => {}
            Err(poly_net::Errno::EISCONN) if pending => {
                s.update(|st| st.connecting = false);
                return Ok(());
            }
            Err(e) => return Err(nerr(e)),
        }
        if nonblock {
            return Err(Errno::EINPROGRESS);
        }
        let wait = timeout(false, s.state().snd_timeout);
        let done = poly_net::Readiness::WRITABLE | poly_net::Readiness::ERROR | poly_net::Readiness::HANGUP;
        let r = self.wait_for(s.queue(), PollEvents::WRITABLE, wait, || {
            let ready = s.with(|st, h| st.readiness(h)).map_err(net_errno)?;
            if ready.0 & done.0 == 0 {
                return Err(poly_vfs::Errno::EAGAIN);
            }
            if let Some(e) = s.with(|st, h| st.take_error(h)).map_err(net_errno)? {
                return Err(net_errno(e));
            }
            match s.with(|st, h| st.tcp_state(h)).map_err(net_errno)? {
                poly_net::TcpState::Established | poly_net::TcpState::CloseWait => Ok(()),
                _ => Err(poly_vfs::Errno::ECONNREFUSED),
            }
        });
        match r {
            Ok(()) => {
                s.update(|st| st.connecting = false);
                Ok(())
            }
            Err(Errno::EINTR) => {
                s.update(|st| st.connecting = true);
                Err(Errno::EINTR)
            }
            // socket(7): a connect whose SO_SNDTIMEO ran out reports
            // EINPROGRESS.
            Err(Errno::EAGAIN) => {
                s.update(|st| st.connecting = true);
                Err(Errno::EINPROGRESS)
            }
            Err(e) => {
                s.update(|st| st.connecting = false);
                Err(e)
            }
        }
    }
}

fn net_type(ty: SockType) -> Result<NetType, Errno> {
    match ty {
        SockType::Stream => Ok(NetType::Stream),
        SockType::Datagram => Ok(NetType::Datagram),
        _ => Err(Errno::ESOCKTNOSUPPORT),
    }
}

fn unix_type(ty: SockType) -> Result<SocketType, Errno> {
    match ty {
        SockType::Stream => Ok(SocketType::Stream),
        SockType::Datagram => Ok(SocketType::Datagram),
        SockType::SeqPacket => Ok(SocketType::SeqPacket),
        SockType::Raw => Err(Errno::ESOCKTNOSUPPORT),
    }
}

fn unix_sock_type(ty: SocketType) -> SockType {
    match ty {
        SocketType::Stream => SockType::Stream,
        SocketType::Datagram => SockType::Datagram,
        SocketType::SeqPacket => SockType::SeqPacket,
    }
}

fn net_protocol(p: Protocol) -> i32 {
    match p {
        Protocol::Tcp => 6,
        Protocol::Udp => 17,
        Protocol::Icmp => 1,
    }
}

fn bool_opt(v: bool) -> OptValue {
    OptValue::Int(i32::from(v))
}

/// The generic options every kind of socket answers.
fn generic_get(o: &poly_unix::SocketOptions, name: OptName) -> Option<OptValue> {
    Some(match name {
        OptName::ReuseAddr => bool_opt(o.reuse_addr),
        OptName::ReusePort => bool_opt(o.reuse_port),
        OptName::KeepAlive => bool_opt(o.keepalive),
        OptName::Broadcast => bool_opt(o.broadcast),
        OptName::OobInline => bool_opt(o.oob_inline),
        OptName::Linger => OptValue::Linger { onoff: o.linger.is_some(), seconds: o.linger.unwrap_or(0) },
        OptName::Priority => OptValue::Int(o.priority),
        OptName::RcvLowat => OptValue::Int(o.rcv_lowat.max(1)),
        OptName::SndLowat => OptValue::Int(1),
        _ => return None,
    })
}

/// Apply a generic option; false if `opt` is not one.
fn generic_set(o: &mut poly_unix::SocketOptions, opt: SockOpt) -> bool {
    match opt {
        SockOpt::ReuseAddr(v) => o.reuse_addr = v,
        SockOpt::ReusePort(v) => o.reuse_port = v,
        SockOpt::KeepAlive(v) => o.keepalive = v,
        SockOpt::Broadcast(v) => o.broadcast = v,
        SockOpt::OobInline(v) => o.oob_inline = v,
        SockOpt::Linger(v) => o.linger = v,
        SockOpt::Priority(v) => o.priority = v,
        SockOpt::RcvLowat(v) => o.rcv_lowat = v,
        _ => return false,
    }
    true
}

impl LinuxNet for Thread<'_> {
    fn socket(&mut self, kind: Kind, new: NewFd) -> Result<i32, Errno> {
        let ns = self.own_netns();
        let object: Arc<dyn FileObject> = match kind.family {
            Family::Packet => RawSocket::new(kind, ns),
            Family::Inet | Family::Inet6 if kind.ty == SockType::Raw => RawSocket::new(kind, ns),
            Family::Inet | Family::Inet6 => {
                let domain = if kind.family == Family::Inet { Domain::Inet } else { Domain::Inet6 };
                let protocol = match kind.protocol {
                    6 => Protocol::Tcp,
                    17 => Protocol::Udp,
                    _ => Protocol::Icmp,
                };
                let stack = stack_of(&ns);
                let home = ns.map(|n| -> Arc<dyn Any + Send + Sync> { n });
                InetSocket::new_in(stack, home, domain, net_type(kind.ty)?, protocol).map_err(nerr)?
            }
            Family::Unix => {
                let sock = UnixSocket::new_in(namespace(), scope_of(&ns), unix_type(kind.ty)?, self.unix_creds());
                Arc::new(UnixFile::new(sock))
            }
            Family::Netlink => NetlinkFile::new(kind.ty, self.process.pid, ns),
        };
        self.install_socket(object, new)
    }

    fn netns(&mut self, fd: i32) -> Result<Option<Arc<NetNs>>, Errno> {
        Ok(match self.sock_file(fd)?.1 {
            Sock::Inet(s) => s.home().and_then(|h| Arc::clone(h).downcast::<NetNs>().ok()),
            Sock::Unix(u) => match u.socket().scope() {
                0 => None,
                // A namespace nobody is in any more has no devices to ask
                // about.
                inum => match ns::lookup(inum, ns::Kind::Net) {
                    Some(ns::Handle::Net(Some(n))) => Some(n),
                    _ => return Err(Errno::ENOTSOCK),
                },
            },
            Sock::Netlink(n) => n.netns().clone(),
            Sock::Raw(r) => r.ns.clone(),
        })
    }

    fn interfaces(&mut self, ns: &Option<Arc<NetNs>>) -> Interfaces {
        interfaces_of(ns)
    }

    fn set_link_up(&mut self, ns: &Option<Arc<NetNs>>, index: u32, up: bool) -> Result<(), Errno> {
        netlink::set_link_up(ns, index, up)
    }

    fn socketpair(&mut self, kind: Kind, new: NewFd) -> Result<[i32; 2], Errno> {
        if kind.family != Family::Unix {
            return Err(Errno::EOPNOTSUPP);
        }
        let scope = scope_of(&self.own_netns());
        let (a, b) = UnixSocket::pair_in(namespace(), scope, unix_type(kind.ty)?, self.unix_creds());
        let fa = self.install_socket(Arc::new(UnixFile::new(a)), new)?;
        match self.install_socket(Arc::new(UnixFile::new(b)), new) {
            Ok(fb) => Ok([fa, fb]),
            Err(e) => {
                let _ = self.process.files.close(fa);
                Err(e)
            }
        }
    }

    fn bind(&mut self, fd: i32, addr: &SockAddr) -> Result<(), Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => {
                let a = to_inet(s.domain, addr)?;
                s.with(|st, h| st.bind(h, a)).map_err(nerr)
            }
            Sock::Unix(u) => {
                let sock = u.socket();
                match addr {
                    SockAddr::Unix(UnixAddr::Unnamed) => sock.autobind().map_err(verr),
                    SockAddr::Unix(UnixAddr::Abstract(n)) => sock.bind_abstract(n).map_err(verr),
                    SockAddr::Unix(UnixAddr::Path(p)) => {
                        let files = &self.process.files;
                        sock.bind_path(fs::vfs(), &files.context(), &files.cred(), At::Cwd, p).map_err(verr)
                    }
                    _ => Err(Errno::EINVAL),
                }
            }
            Sock::Netlink(n) => match addr {
                SockAddr::Netlink { pid, groups } => n.bind(*pid, *groups),
                _ => Err(Errno::EINVAL),
            },
            // Nothing arrives on a raw socket whatever it is bound to.
            Sock::Raw(_) => Ok(()),
        }
    }

    fn listen(&mut self, fd: i32, backlog: u32) -> Result<(), Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => {
                s.with(|st, h| st.listen(h, backlog as usize)).map_err(nerr)?;
                s.update(|st| st.listening = true);
                Ok(())
            }
            Sock::Unix(u) => u.socket().listen(backlog as usize, self.unix_creds()).map_err(verr),
            Sock::Netlink(_) | Sock::Raw(_) => Err(Errno::EOPNOTSUPP),
        }
    }

    fn accept(&mut self, fd: i32, new: NewFd) -> Result<(i32, SockAddr), Errno> {
        let (file, sock) = self.sock_file(fd)?;
        let nonblock = file.flags().contains(OpenFlags::NONBLOCK);
        match sock {
            Sock::Inet(s) => {
                if s.ty != NetType::Stream {
                    return Err(Errno::EOPNOTSUPP);
                }
                let wait = timeout(nonblock, s.state().rcv_timeout);
                let (h, peer) = self.wait_for(s.queue(), PollEvents::READABLE, wait, || {
                    s.with(|st, h| st.accept(h)).map_err(net_errno)
                })?;
                let conn = s.accepted(h);
                let fd = self.install_socket(conn, new)?;
                Ok((fd, inet_addr(peer)))
            }
            Sock::Unix(u) => {
                let sock = Arc::clone(u.socket());
                let wait = timeout(nonblock, sock.timeouts().0);
                let conn = self.wait_for(sock.wait_queue(), PollEvents::READABLE, wait, || sock.accept())?;
                let peer = conn.peer_name().map(unix_addr).unwrap_or(SockAddr::Unix(UnixAddr::Unnamed));
                let fd = self.install_socket(Arc::new(UnixFile::new(conn)), new)?;
                Ok((fd, peer))
            }
            Sock::Netlink(_) | Sock::Raw(_) => Err(Errno::EOPNOTSUPP),
        }
    }

    fn connect(&mut self, fd: i32, addr: &SockAddr) -> Result<(), Errno> {
        let (file, sock) = self.sock_file(fd)?;
        let nonblock = file.flags().contains(OpenFlags::NONBLOCK);
        match sock {
            Sock::Inet(s) => self.inet_connect(&s, nonblock, addr),
            Sock::Unix(u) => {
                let sock = Arc::clone(u.socket());
                if *addr == SockAddr::Unspec {
                    return sock.disconnect().map_err(verr);
                }
                let target = self.unix_target(sock.scope(), addr)?;
                let creds = self.unix_creds();
                let wait = timeout(nonblock, sock.timeouts().1);
                self.wait_for(sock.wait_queue(), PollEvents::WRITABLE, wait, || sock.connect(&target, creds))
            }
            Sock::Netlink(n) => match addr {
                SockAddr::Netlink { pid: 0, .. } | SockAddr::Unspec => Ok(()),
                SockAddr::Netlink { .. } => {
                    let _ = n;
                    Err(Errno::ECONNREFUSED)
                }
                _ => Err(Errno::EINVAL),
            },
            // The default destination of a raw socket: remembered by
            // nobody, since nothing is sent.
            Sock::Raw(_) => Ok(()),
        }
    }

    fn name(&mut self, fd: i32, peer: bool) -> Result<SockAddr, Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => {
                let a = if peer { s.with(|st, h| st.peer_addr(h)) } else { s.with(|st, h| st.local_addr(h)) };
                a.map(inet_addr).map_err(nerr)
            }
            Sock::Unix(u) => {
                let sock = u.socket();
                if peer { sock.peer_name().map(unix_addr).map_err(verr) } else { Ok(unix_addr(sock.local_name())) }
            }
            Sock::Netlink(n) => Ok(if peer { SockAddr::Netlink { pid: 0, groups: 0 } } else { n.local() }),
            Sock::Raw(_) if peer => Err(Errno::ENOTCONN),
            Sock::Raw(r) => match r.kind.family {
                Family::Inet => Ok(SockAddr::Inet(core::net::SocketAddrV4::new(core::net::Ipv4Addr::UNSPECIFIED, 0))),
                Family::Inet6 => Ok(SockAddr::Inet6(SocketAddrV6::new(core::net::Ipv6Addr::UNSPECIFIED, 0, 0, 0))),
                _ => Err(Errno::EOPNOTSUPP),
            },
        }
    }

    fn send(&mut self, fd: i32, msg: &SendMsg<'_>) -> Result<usize, Errno> {
        let (file, sock) = self.sock_file(fd)?;
        let nonblock = file.flags().contains(OpenFlags::NONBLOCK);
        match sock {
            Sock::Inet(s) => self.inet_send(&s, nonblock, msg),
            Sock::Unix(u) => {
                let sock = Arc::clone(u.socket());
                let to = match &msg.to {
                    None => None,
                    // A stream or sequenced-packet socket never looks an
                    // address up (sendto's spec sheet: EISCONN on a
                    // connected AF_UNIX stream, whatever the address).
                    Some(_) if sock.kind() != SocketType::Datagram => {
                        return Err(if sock.is_connected() { Errno::EISCONN } else { Errno::EOPNOTSUPP });
                    }
                    Some(a) => Some(self.unix_target(sock.scope(), a)?),
                };
                let mut rights = Vec::with_capacity(msg.rights.len());
                for &r in &msg.rights {
                    rights.push(self.process.files.file(r).map_err(verr)?);
                }
                let creds = self.unix_creds();
                let anc_creds = msg.creds.map(|c| Credentials { pid: c.pid, uid: c.uid, gid: c.gid });
                let wait = timeout(nonblock || msg.dontwait, sock.timeouts().1);
                let mut anc = Some(Ancillary { rights, creds: anc_creds });
                let stream = sock.kind() == SocketType::Stream;
                let mut done = 0;
                loop {
                    let r = self.wait_for(sock.wait_queue(), PollEvents::WRITABLE, wait, || {
                        // The ancillary data go with the first bytes; an
                        // attempt that fails keeps them for the next.
                        let a = anc.take().unwrap_or_default();
                        let held = Ancillary { rights: a.rights.clone(), creds: a.creds };
                        let r = sock.send(&msg.data[done..], to.as_ref(), held, creds);
                        if r.is_err() {
                            anc = Some(a);
                        }
                        r
                    });
                    match r {
                        Ok(n) => {
                            done += n;
                            if !stream || done >= msg.data.len() || wait == Timeout::Now || n == 0 {
                                return Ok(done);
                            }
                        }
                        Err(_) if done > 0 => return Ok(done),
                        Err(e) => return Err(e),
                    }
                }
            }
            Sock::Netlink(n) => {
                match &msg.to {
                    None | Some(SockAddr::Netlink { pid: 0, .. }) => {}
                    Some(SockAddr::Netlink { .. }) => return Err(Errno::ECONNREFUSED),
                    Some(_) => return Err(Errno::EINVAL),
                }
                n.request(msg.data);
                Ok(msg.data.len())
            }
            Sock::Raw(_) => Err(Errno::ENETUNREACH),
        }
    }

    fn recv(&mut self, fd: i32, buf: &mut [u8], req: &RecvReq) -> Result<RecvMsg, Errno> {
        let (file, sock) = self.sock_file(fd)?;
        let nonblock = file.flags().contains(OpenFlags::NONBLOCK);
        match sock {
            Sock::Inet(s) => self.inet_recv(&s, nonblock, buf, req),
            Sock::Unix(u) => {
                let sock = Arc::clone(u.socket());
                let wait = timeout(nonblock || req.dontwait, sock.timeouts().0);
                let got = self.wait_for(sock.wait_queue(), PollEvents::READABLE, wait, || sock.recv(buf, req.peek))?;
                let stream = sock.kind() == SocketType::Stream;
                let mut len = got.len;
                let no_rights = got.rights.is_empty();
                let (rights, truncated) = self.install_rights(got.rights, req.max_rights, req.cloexec);
                if stream && req.waitall && !req.peek && no_rights && wait != Timeout::Now {
                    while len < buf.len() && got.len > 0 {
                        let more = self.wait_for(sock.wait_queue(), PollEvents::READABLE, wait, || {
                            sock.recv(&mut buf[len..], false)
                        });
                        match more {
                            Ok(r) if r.len > 0 && r.rights.is_empty() => len += r.len,
                            Ok(r) if r.len > 0 => {
                                len += r.len;
                                break;
                            }
                            _ => break,
                        }
                    }
                }
                Ok(RecvMsg {
                    len,
                    full_len: if stream { len } else { got.full_len },
                    from: (!stream).then(|| unix_addr(got.from)),
                    rights,
                    rights_truncated: truncated,
                    creds: got.creds.map(|c| Ucred { pid: c.pid, uid: c.uid, gid: c.gid }),
                })
            }
            Sock::Netlink(n) => {
                let wait = timeout(nonblock || req.dontwait, n.timeouts().0);
                let (len, full_len) = self.wait_for(n.queue(), PollEvents::READABLE, wait, || n.read(buf, req.peek))?;
                Ok(RecvMsg { len, full_len, from: Some(SockAddr::Netlink { pid: 0, groups: 0 }), ..RecvMsg::default() })
            }
            Sock::Raw(r) => {
                let wait = timeout(nonblock || req.dontwait, r.timeouts.lock().0);
                self.wait_for(&r.queue, PollEvents::READABLE, wait, || Err::<RecvMsg, _>(poly_vfs::Errno::EAGAIN))
            }
        }
    }

    fn shutdown(&mut self, fd: i32, how: Shutdown) -> Result<(), Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => {
                let how = match how {
                    Shutdown::Read => NetShutdown::Read,
                    Shutdown::Write => NetShutdown::Write,
                    Shutdown::Both => NetShutdown::Both,
                };
                s.with_wake(|st, h| st.shutdown(h, how)).map_err(nerr)
            }
            Sock::Unix(u) => {
                let how = match how {
                    Shutdown::Read => poly_unix::Shutdown::Read,
                    Shutdown::Write => poly_unix::Shutdown::Write,
                    Shutdown::Both => poly_unix::Shutdown::Both,
                };
                u.socket().shutdown(how).map_err(verr)
            }
            Sock::Netlink(_) => Err(Errno::EOPNOTSUPP),
            Sock::Raw(_) => Ok(()),
        }
    }

    fn setsockopt(&mut self, fd: i32, opt: SockOpt) -> Result<(), Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => inet_setsockopt(&s, opt),
            Sock::Unix(u) => {
                let sock = u.socket();
                match opt {
                    SockOpt::PassCred(v) => sock.set_passcred(v),
                    SockOpt::SndBuf(v) => sock.set_buffers(Some(v), None),
                    SockOpt::RcvBuf(v) => sock.set_buffers(None, Some(v)),
                    SockOpt::RcvTimeout(t) => sock.set_timeouts(Some(t), None),
                    SockOpt::SndTimeout(t) => sock.set_timeouts(None, Some(t)),
                    o => {
                        if !sock.with_options(|g| generic_set(g, o)) {
                            return Err(Errno::ENOPROTOOPT);
                        }
                    }
                }
                Ok(())
            }
            Sock::Netlink(n) => n.setsockopt(opt),
            Sock::Raw(r) => {
                match opt {
                    SockOpt::RcvTimeout(t) => r.timeouts.lock().0 = t,
                    SockOpt::SndTimeout(t) => r.timeouts.lock().1 = t,
                    SockOpt::SndBuf(_) | SockOpt::RcvBuf(_) | SockOpt::PassCred(_) => {}
                    o => {
                        if !generic_set(&mut r.options.lock(), o) {
                            return Err(Errno::ENOPROTOOPT);
                        }
                    }
                }
                Ok(())
            }
        }
    }

    fn getsockopt(&mut self, fd: i32, name: OptName) -> Result<OptValue, Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => inet_getsockopt(&s, name),
            Sock::Unix(u) => {
                let sock = u.socket();
                Ok(match name {
                    OptName::Type => OptValue::Int(unix_sock_type(sock.kind()).to_linux() as i32),
                    OptName::Domain => OptValue::Int(i32::from(Family::Unix.to_linux())),
                    OptName::Protocol => OptValue::Int(0),
                    OptName::Error => OptValue::Int(sock.take_error().map_or(0, |e| e as i32)),
                    OptName::AcceptConn => bool_opt(sock.is_listening()),
                    OptName::PeerCred => OptValue::Ucred(sock.peer_credentials().map_or(NO_CREDS, |c| Ucred {
                        pid: c.pid,
                        uid: c.uid,
                        gid: c.gid,
                    })),
                    OptName::PassCred => bool_opt(sock.passcred()),
                    OptName::SndBuf => OptValue::Int(sock.buffers().0 as i32),
                    OptName::RcvBuf => OptValue::Int(sock.buffers().1 as i32),
                    OptName::RcvTimeout => OptValue::Timeval(sock.timeouts().0),
                    OptName::SndTimeout => OptValue::Timeval(sock.timeouts().1),
                    n => sock.with_options(|g| generic_get(g, n)).ok_or(Errno::ENOPROTOOPT)?,
                })
            }
            Sock::Netlink(n) => n.getsockopt(name),
            Sock::Raw(r) => Ok(match name {
                OptName::Type => OptValue::Int(r.kind.ty.to_linux() as i32),
                OptName::Domain => OptValue::Int(i32::from(r.kind.family.to_linux())),
                OptName::Protocol => OptValue::Int(r.kind.protocol),
                OptName::Error | OptName::AcceptConn => OptValue::Int(0),
                OptName::PeerCred => OptValue::Ucred(NO_CREDS),
                OptName::PassCred => bool_opt(false),
                OptName::SndBuf | OptName::RcvBuf => OptValue::Int(poly_unix::DEFAULT_BUFFER as i32),
                OptName::RcvTimeout => OptValue::Timeval(r.timeouts.lock().0),
                OptName::SndTimeout => OptValue::Timeval(r.timeouts.lock().1),
                n => generic_get(&r.options.lock(), n).ok_or(Errno::ENOPROTOOPT)?,
            }),
        }
    }

    fn timed(&mut self, fd: i32, send: bool) -> bool {
        let Ok((_, sock)) = self.sock_file(fd) else { return false };
        let (r, w) = match sock {
            Sock::Inet(s) => {
                let st = s.state();
                (st.rcv_timeout, st.snd_timeout)
            }
            Sock::Unix(u) => u.socket().timeouts(),
            Sock::Netlink(n) => n.timeouts(),
            Sock::Raw(r) => *r.timeouts.lock(),
        };
        if send { w != 0 } else { r != 0 }
    }

    fn send_queued(&mut self, fd: i32) -> Result<usize, Errno> {
        match self.sock_file(fd)?.1 {
            Sock::Inet(s) => s.with(|st, h| st.send_queued(h)).map_err(nerr),
            Sock::Unix(u) => Ok(u.socket().send_queued()),
            Sock::Netlink(_) | Sock::Raw(_) => Ok(0),
        }
    }

    fn keep_error(&mut self, fd: i32, e: Errno) {
        if let Ok((_, Sock::Unix(u))) = self.sock_file(fd)
            && let Some(e) = poly_vfs::Errno::from_code(i32::from(e.get()))
        {
            u.socket().keep_error(e);
        }
    }
}

fn inet_setsockopt(s: &InetSocket, opt: SockOpt) -> Result<(), Errno> {
    let tcp = s.protocol == Protocol::Tcp;
    let v4 = s.domain == Domain::Inet;
    let secs = |v: u32| Duration::from_secs(u64::from(v));
    match opt {
        SockOpt::ReuseAddr(v) => s.with(|st, h| st.set_reuse_addr(h, v)).map_err(nerr)?,
        SockOpt::ReusePort(v) => {
            s.with(|st, h| st.set_reuse_port(h, v)).map_err(nerr)?;
            s.update(|st| st.options.reuse_port = v);
        }
        SockOpt::KeepAlive(v) => {
            // The stack acts on it for TCP; every socket reads it back
            // (setsockopt's visible test: a UDP socket too).
            if tcp {
                s.with(|st, h| st.set_keepalive(h, v)).map_err(nerr)?;
            }
            s.update(|st| st.options.keepalive = v);
        }
        SockOpt::Broadcast(v) => s.with(|st, h| st.set_broadcast(h, v)).map_err(nerr)?,
        SockOpt::SndBuf(v) => {
            // A connected TCP socket keeps its buffers (the stack sizes
            // them at the handshake); the value is still reported back.
            let _ = s.with(|st, h| st.set_buffer_sizes(h, Some(v), None));
            s.update(|st| st.sndbuf = Some(v));
        }
        SockOpt::RcvBuf(v) => {
            let _ = s.with(|st, h| st.set_buffer_sizes(h, None, Some(v)));
            s.update(|st| st.rcvbuf = Some(v));
        }
        SockOpt::RcvTimeout(t) => s.update(|st| st.rcv_timeout = t),
        SockOpt::SndTimeout(t) => s.update(|st| st.snd_timeout = t),
        SockOpt::TcpNoDelay(v) if tcp => s.with(|st, h| st.set_nodelay(h, v)).map_err(nerr)?,
        SockOpt::TcpKeepIdle(v) if tcp => {
            s.with(|st, h| st.set_keepalive_params(h, Some(secs(v)), None, None)).map_err(nerr)?
        }
        SockOpt::TcpKeepIntvl(v) if tcp => {
            s.with(|st, h| st.set_keepalive_params(h, None, Some(secs(v)), None)).map_err(nerr)?
        }
        SockOpt::TcpKeepCnt(v) if tcp => {
            s.with(|st, h| st.set_keepalive_params(h, None, None, Some(v))).map_err(nerr)?
        }
        SockOpt::TcpMaxSeg(v) if tcp => s.update(|st| st.maxseg = Some(v)),
        SockOpt::IpTtl(t) if v4 => {
            let (_, tos) = s.with(|st, h| st.ip_options(h)).map_err(nerr)?;
            s.with(|st, h| st.set_ip_options(h, t, tos)).map_err(nerr)?;
        }
        SockOpt::Ipv6UnicastHops(t) if !v4 => {
            let (_, tos) = s.with(|st, h| st.ip_options(h)).map_err(nerr)?;
            s.with(|st, h| st.set_ip_options(h, t, tos)).map_err(nerr)?;
        }
        SockOpt::IpTos(tos) if v4 => {
            let (ttl, _) = s.with(|st, h| st.ip_options(h)).map_err(nerr)?;
            s.with(|st, h| st.set_ip_options(h, ttl, tos)).map_err(nerr)?;
        }
        SockOpt::Ipv6V6Only(v) if !v4 => s.update(|st| st.v6only = v),
        SockOpt::IpRecvErr(v) if v4 => s.update(|st| st.recverr = v),
        SockOpt::Ipv6RecvErr(v) if !v4 => s.update(|st| st.recverr = v),
        // SO_PASSCRED means nothing to an Internet socket; accepted.
        SockOpt::PassCred(_) => {}
        o => {
            if !s.update(|st| generic_set(&mut st.options, o)) {
                return Err(Errno::ENOPROTOOPT);
            }
        }
    }
    Ok(())
}

fn inet_getsockopt(s: &InetSocket, name: OptName) -> Result<OptValue, Errno> {
    let tcp = s.protocol == Protocol::Tcp;
    let v4 = s.domain == Domain::Inet;
    let flags = s.with(|st, h| st.flags(h)).map_err(nerr)?;
    let st = s.state();
    Ok(match name {
        OptName::Type => OptValue::Int(match s.ty {
            NetType::Stream => 1,
            NetType::Datagram => 2,
        }),
        OptName::Domain => OptValue::Int(if v4 { 2 } else { 10 }),
        OptName::Protocol => OptValue::Int(net_protocol(s.protocol)),
        OptName::Error => {
            let e = s.with(|x, h| x.take_error(h)).map_err(nerr)?;
            OptValue::Int(e.map_or(0, |e| i32::from(e.0)))
        }
        OptName::AcceptConn => bool_opt(st.listening),
        OptName::PeerCred => OptValue::Ucred(NO_CREDS),
        OptName::ReuseAddr => bool_opt(flags.reuse_addr),
        OptName::KeepAlive => bool_opt(st.options.keepalive),
        OptName::Broadcast => bool_opt(flags.broadcast),
        OptName::SndBuf | OptName::RcvBuf => {
            let (snd, rcv) = s.with(|x, h| x.buffer_sizes(h)).map_err(nerr)?;
            let v = if name == OptName::SndBuf { st.sndbuf.unwrap_or(snd) } else { st.rcvbuf.unwrap_or(rcv) };
            OptValue::Int(v.min(i32::MAX as usize) as i32)
        }
        OptName::RcvTimeout => OptValue::Timeval(st.rcv_timeout),
        OptName::SndTimeout => OptValue::Timeval(st.snd_timeout),
        OptName::PassCred => bool_opt(false),
        OptName::TcpNoDelay if tcp => bool_opt(flags.nodelay),
        OptName::TcpKeepIdle | OptName::TcpKeepIntvl | OptName::TcpKeepCnt if tcp => {
            let (idle, intvl, cnt) = s.with(|x, h| x.keepalive_params(h)).map_err(nerr)?;
            OptValue::Int(match name {
                OptName::TcpKeepIdle => idle.as_secs() as i32,
                OptName::TcpKeepIntvl => intvl.as_secs() as i32,
                _ => cnt as i32,
            })
        }
        OptName::TcpMaxSeg if tcp => {
            let mss = s.with(|x, h| x.tcp_info(h)).map(|i| i.send_mss as u32).unwrap_or(536);
            OptValue::Int(st.maxseg.unwrap_or(mss) as i32)
        }
        OptName::IpTtl if v4 => {
            let (ttl, _) = s.with(|x, h| x.ip_options(h)).map_err(nerr)?;
            OptValue::Int(i32::from(ttl.unwrap_or_else(|| s.with(|x, _| x.default_ttl()))))
        }
        OptName::Ipv6UnicastHops if !v4 => {
            let (ttl, _) = s.with(|x, h| x.ip_options(h)).map_err(nerr)?;
            OptValue::Int(i32::from(ttl.unwrap_or_else(|| s.with(|x, _| x.default_ttl()))))
        }
        OptName::IpTos if v4 => OptValue::Int(i32::from(s.with(|x, h| x.ip_options(h)).map_err(nerr)?.1)),
        OptName::Ipv6V6Only if !v4 => bool_opt(st.v6only),
        OptName::IpRecvErr if v4 => bool_opt(st.recverr),
        OptName::Ipv6RecvErr if !v4 => bool_opt(st.recverr),
        n => generic_get(&st.options, n).ok_or(Errno::ENOPROTOOPT)?,
    })
}
