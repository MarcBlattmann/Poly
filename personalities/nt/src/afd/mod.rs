//! `\Device\Afd`: sockets for `ws2_32` (ADR 0095).
//!
//! A socket is a file object on `\Device\Afd\Endpoint`; its
//! [`FileRecord`](crate::io::FileRecord) target is an [`Endpoint`], which
//! holds the kernel's socket once [`poly_winsock::afd::SOCKET`] made it,
//! and the state Winsock keeps per socket that the stack does not (the
//! non-blocking mode, `SO_RCVTIMEO` / `SO_SNDTIMEO`, options that are only
//! reported back). The requests are Poly's own `NtDeviceIoControlFile`
//! codes ([`poly_winsock::afd`], decoded in [`services`]); `NtReadFile`
//! and `NtWriteFile` on a socket handle receive and send, as on Windows.
//!
//! The sockets themselves are the kernel's (`poly-net` behind its network
//! thread, ADR 0028), reached through [`NtSocket`], which the kernel
//! implements over the same Internet sockets the Linux personality uses
//! (ADR 0048), and host tests over a `poly-net` stack of their own. A call
//! that may wait (connect, accept, a blocking send or receive) runs in the
//! calling thread with no NT lock held; the kernel blocks it on the
//! socket's wait queue.
//!
//! **Overlapped requests.** A send or receive with
//! [`poly_winsock::afd::FLAG_OVERLAPPED`] that cannot finish at once
//! becomes an I/O request packet kept waiting ([`crate::io::Irp`]) and is
//! handed to the kernel ([`NtKernel::socket_pend`]), whose worker
//! completes it when the socket is ready — through the event, the status
//! block and the APC, like every asynchronous request of ADR 0067.
//! Closing the socket or cancelling the I/O completes it with
//! `STATUS_CANCELLED`. A kernel without that worker (host fakes) runs the
//! request in the calling thread instead.
//!
//! [`NtKernel::socket_pend`]: crate::kernel::NtKernel::socket_pend

pub mod services;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::SocketAddr;
use poly_ntobj::NtStatus;

pub use poly_iphlp::{IfInfo, Kind as IfKind};
pub use poly_net::{Domain, Errno, Protocol, Readiness, RecvInfo, Shutdown, SockType};

/// How long a socket call may wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SockWait {
    /// Try once (a non-blocking socket, an overlapped attempt).
    Now,
    /// Until it can finish.
    Forever,
    /// At most this many milliseconds (`SO_RCVTIMEO` / `SO_SNDTIMEO`);
    /// then it fails with `EAGAIN`, which a blocking call reports as a
    /// timeout.
    Millis(u64),
}

/// Options the stack acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SockOption {
    ReuseAddr(bool),
    KeepAlive(bool),
    Broadcast(bool),
    NoDelay(bool),
    SndBuf(usize),
    RcvBuf(usize),
    /// `SO_LINGER`: `Some(0)` closes with a reset.
    Linger(Option<u32>),
    /// IPv4 TTL / IPv6 unicast hop limit.
    Ttl(u8),
    KeepIdle(u32),
    KeepIntvl(u32),
    KeepCnt(u32),
    /// Winsock's `shutdown(SD_RECEIVE)`: data that is queued or arrives later resets the
    /// connection (set before the shutdown itself).
    AbortOnReadShutdown(bool),
}

/// Options read back from the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptQuery {
    ReuseAddr,
    Broadcast,
    NoDelay,
    SndBuf,
    RcvBuf,
    Ttl,
}

/// A kernel socket, as the personality uses it. Every call either
/// finishes, fails, or (for the ones with a [`SockWait`]) waits as long as
/// it allows and then fails with `EAGAIN`; a wait the kernel interrupts
/// (the thread or process is ending, the socket was released) fails with
/// `EINTR`.
pub trait NtSocket: Send + Sync + core::fmt::Debug {
    fn domain(&self) -> Domain;
    fn kind(&self) -> SockType;
    fn protocol(&self) -> Protocol;
    fn bind(&self, addr: SocketAddr) -> Result<(), Errno>;
    fn listen(&self, backlog: usize) -> Result<(), Errno>;
    fn accept(&self, wait: SockWait) -> Result<(Arc<dyn NtSocket>, SocketAddr), Errno>;
    /// Connect; with [`SockWait::Now`] an attempt in progress is
    /// `EINPROGRESS` (and a second one `EALREADY`).
    fn connect(&self, addr: SocketAddr, wait: SockWait) -> Result<(), Errno>;
    /// Send at least one byte (a stream) or the whole datagram.
    fn send(&self, data: &[u8], to: Option<SocketAddr>, wait: SockWait) -> Result<usize, Errno>;
    fn recv(&self, buf: &mut [u8], peek: bool, wait: SockWait) -> Result<RecvInfo, Errno>;
    fn shutdown(&self, how: Shutdown) -> Result<(), Errno>;
    fn local_addr(&self) -> Result<SocketAddr, Errno>;
    fn peer_addr(&self) -> Result<SocketAddr, Errno>;
    fn readiness(&self) -> Readiness;
    /// `SO_ERROR`: the pending error, cleared.
    fn take_error(&self) -> Option<Errno>;
    fn bytes_readable(&self) -> usize;
    /// Dissolve a datagram socket's association (`connect` with `AF_UNSPEC`).
    fn disconnect(&self) -> Result<(), Errno> {
        Err(Errno::EOPNOTSUPP)
    }
    /// Reset the connection (a RST to the peer) and drop it; the socket stays open.
    fn abort(&self) {}
    /// Is the socket connected to a peer now: a TCP connection past its handshake (still there
    /// when the peer closed its side), a datagram socket with a destination?
    fn established(&self) -> bool {
        self.peer_addr().is_ok()
    }
    fn set_option(&self, opt: SockOption) -> Result<(), Errno>;
    fn get_option(&self, opt: OptQuery) -> Result<u32, Errno>;
    /// The handle is closed: calls waiting on it end (`EINTR`). The
    /// socket itself closes when its last reference goes.
    fn release(&self) {}
    /// For the kernel: its own type behind the trait.
    fn as_any(&self) -> &dyn core::any::Any;
}

//// An overlapped request the kernel finishes later
/// ([`crate::kernel::NtKernel::socket_pend`]): the waiting request packet
/// and what to do. The kernel completes it with
/// [`crate::io::IoState::complete`] (a receive's data written to `buf`).
#[derive(Debug)]
pub struct PendIo {
    pub irp: u64,
    /// The cookie of the request's file object (the endpoint).
    pub file: u64,
    pub sock: Arc<dyn NtSocket>,
    pub op: PendOp,
    /// What a finished `AcceptEx` / `ConnectEx` puts into its socket's endpoint
    /// ([`PendIo::take_placement`]).
    placement: Option<Placement>,
}

/// An overlapped request's work.
#[derive(Debug)]
pub enum PendOp {
    Recv { buf: u64, len: usize, peek: bool },
    /// A send of `data` (a stream's: `done` bytes of it already went; the request completes when all did).
    Send { data: Vec<u8>, to: Option<SocketAddr>, done: usize },
    /// `AcceptEx`: the listener is the request's socket; `target` is the file object of the
    /// endpoint that takes the connection; the buffer is `recv_len` data bytes, then the local and
    /// the remote address in `local_len` and `remote_len` bytes each.
    Accept {
        target: u64,
        buf: u64,
        recv_len: usize,
        local_len: usize,
        remote_len: usize,
        conn: Option<(Arc<dyn NtSocket>, SocketAddr)>,
    },
    /// `ConnectEx`: connect the request's socket to `addr`, then send `data`.
    Connect { target: u64, addr: SocketAddr, started: bool, data: Vec<u8> },
}

/// What a finished `AcceptEx` or `ConnectEx` changes in an endpoint: the connection (for `AcceptEx`,
/// the one that was accepted becomes the endpoint's socket); a socket made by either does not tell
/// its addresses until `SO_UPDATE_ACCEPT_CONTEXT` / `SO_UPDATE_CONNECT_CONTEXT` (Learn: "AcceptEx").
#[derive(Debug)]
pub struct Placement {
    pub target: u64,
    pub sock: Option<Arc<dyn NtSocket>>,
    /// A receive or send ended the connection (reset, aborted): every later one fails the same way.
    pub broken: Option<NtStatus>,
}

/// The status Winsock keeps reporting for a connection the stack ended with `e` (a reset by the peer,
/// a reset after the peer had closed its side, a timeout): the same one for every later receive and
/// send, where the stack reports it once.
pub fn sticky_status(e: Errno, sock: &Arc<dyn NtSocket>) -> Option<NtStatus> {
    match e {
        // A reset after the peer had closed its side (its FIN came) is "connection aborted",
        // as one we sent ourselves; one out of the blue is "connection reset".
        Errno::ECONNRESET if sock.readiness().contains(Readiness::READ_HANGUP) => {
            Some(NtStatus(poly_winsock::error::STATUS_CONNECTION_ABORTED))
        }
        Errno::ECONNRESET => Some(NtStatus(poly_winsock::error::STATUS_CONNECTION_RESET)),
        Errno::EPIPE | Errno::ECONNABORTED => Some(NtStatus(poly_winsock::error::STATUS_CONNECTION_ABORTED)),
        Errno::ETIMEDOUT => Some(NtStatus(poly_winsock::error::STATUS_IO_TIMEOUT)),
        _ => None,
    }
}

/// How a request ended: its status, `Information`, and the data to write
/// into the caller's memory (address and bytes).
pub type PendResult = (NtStatus, u64, Option<(u64, Vec<u8>)>);

impl PendIo {
    pub fn new(irp: u64, file: u64, sock: Arc<dyn NtSocket>, op: PendOp) -> PendIo {
        PendIo { irp, file, sock, op, placement: None }
    }

    /// What the request that just finished (it returned `Some` from [`attempt`](Self::attempt))
    /// puts into its endpoint; the kernel applies it with [`place`] under the NT lock, before the
    /// request completes.
    pub fn take_placement(&mut self) -> Option<Placement> {
        self.placement.take()
    }

    /// One attempt that does not wait: `None` while it would block, else
    /// the request's status, `Information` and data to write.
    pub fn attempt(&mut self) -> Option<PendResult> {
        let sock = self.sock.clone();
        match &mut self.op {
            PendOp::Recv { buf, len, peek } => {
                let mut data = alloc::vec![0u8; (*len).min(crate::io::irp::MAX_ASYNC_READ)];
                match sock.recv(&mut data, *peek, SockWait::Now) {
                    Err(Errno::EAGAIN) => None,
                    Err(e) => {
                        let broken = if sock.kind() == SockType::Stream { sticky_status(e, &sock) } else { None };
                        if broken.is_some() {
                            self.placement = Some(Placement { target: self.file, sock: None, broken });
                        }
                        Some((broken.unwrap_or_else(|| status_of(e)), 0, None))
                    }
                    Ok(info) => {
                        data.truncate(info.len);
                        let status = if sock.kind() == SockType::Datagram && info.full_len > info.len {
                            NtStatus(poly_winsock::error::STATUS_BUFFER_OVERFLOW)
                        } else {
                            poly_ntobj::status::STATUS_SUCCESS
                        };
                        Some((status, info.len as u64, Some((*buf, data))))
                    }
                }
            }
            PendOp::Send { data, to, done } => loop {
                if *done >= data.len() && !data.is_empty() {
                    return Some((poly_ntobj::status::STATUS_SUCCESS, *done as u64, None));
                }
                match sock.send(&data[*done..], *to, SockWait::Now) {
                    Err(Errno::EAGAIN) => return None,
                    Err(e) => return Some((status_of(e), *done as u64, None)),
                    Ok(n) => {
                        *done += n;
                        // A datagram goes whole; a stream until all of it went.
                        if sock.kind() == SockType::Datagram || *done >= data.len() {
                            return Some((poly_ntobj::status::STATUS_SUCCESS, *done as u64, None));
                        }
                    }
                }
            },
            PendOp::Accept { target, buf, recv_len, local_len, remote_len, conn } => {
                if conn.is_none() {
                    match sock.accept(SockWait::Now) {
                        Err(Errno::EAGAIN) => return None,
                        Err(e) => return Some((status_of(e), 0, None)),
                        Ok(c) => *conn = Some(c),
                    }
                }
                let (c, peer) = conn.as_ref()?.clone();
                let mut data = alloc::vec![0u8; *recv_len];
                let mut got = 0;
                if *recv_len > 0 {
                    // The request completes with the first data (or the end of the stream).
                    match c.recv(&mut data, false, SockWait::Now) {
                        Err(Errno::EAGAIN) => return None,
                        Err(e) => return Some((status_of(e), 0, None)),
                        Ok(i) => got = i.len,
                    }
                }
                let mut out = alloc::vec![0u8; *recv_len + *local_len + *remote_len];
                out[..got].copy_from_slice(&data[..got]);
                let put = |out: &mut [u8], at: usize, room: usize, a: Option<SocketAddr>| {
                    if let Some(a) = a {
                        let (b, n) = poly_winsock::addr::encode(&a);
                        let n = n.min(room);
                        out[at..at + n].copy_from_slice(&b[..n]);
                    }
                };
                put(&mut out, *recv_len, *local_len, c.local_addr().ok());
                put(&mut out, *recv_len + *local_len, *remote_len, Some(peer));
                self.placement = Some(Placement { target: *target, sock: Some(c), broken: None });
                Some((poly_ntobj::status::STATUS_SUCCESS, got as u64, Some((*buf, out))))
            }
            PendOp::Connect { target, addr, started, data } => {
                if !*started {
                    match sock.connect(*addr, SockWait::Now) {
                        Ok(()) => {}
                        Err(Errno::EINPROGRESS) => {
                            *started = true;
                            return None;
                        }
                        Err(e) => return Some((status_of(e), 0, None)),
                    }
                } else if !sock.established() {
                    // Not there yet, or it failed (the stack keeps the error).
                    return sock.take_error().map(|e| (status_of(e), 0, None));
                }
                let mut sent = 0;
                if !data.is_empty() {
                    match sock.send(data, None, SockWait::Now) {
                        Err(Errno::EAGAIN) => return None,
                        Err(e) => return Some((status_of(e), 0, None)),
                        Ok(n) => sent = n,
                    }
                }
                self.placement = Some(Placement { target: *target, sock: None, broken: None });
                Some((poly_ntobj::status::STATUS_SUCCESS, sent as u64, None))
            }
        }
    }
}

// A stack error as the status a request completes with.
pub fn status_of(e: Errno) -> NtStatus {
    NtStatus(poly_winsock::error::status_of_errno(e.0))
}

/// What a socket file object is open on.
#[derive(Debug, Default)]
pub struct Endpoint {
    /// The kernel's socket, once made.
    pub sock: Option<Arc<dyn NtSocket>>,
    /// The `socket` arguments (Winsock's values).
    pub family: i32,
    pub ty: i32,
    pub protocol: i32,
    /// `FIONBIO`.
    pub nonblocking: bool,
    /// `SO_RCVTIMEO` / `SO_SNDTIMEO` in milliseconds (0: none).
    pub rcv_timeout: u32,
    pub snd_timeout: u32,
    pub listening: bool,
    /// Options only stored and reported back, by (level, name).
    pub stored: BTreeMap<(i32, i32), Vec<u8>>,
    /// `IPV6_V6ONLY` of an `AF_INET6` socket (Winsock's default is on; Chrome turns it off).
    pub v6only: bool,
    /// `sock` is an IPv4 stack socket that stands in for an `AF_INET6` one with `IPV6_V6ONLY` off
    /// (a dual-stack socket that connected or bound to an IPv4-mapped address): the addresses it
    /// reports are IPv4-mapped.
    pub mapped: bool,
    /// The IPv4 half of a dual-stack listener (the stack's IPv6 sockets carry only IPv6).
    pub twin: Option<Arc<dyn NtSocket>>,
    /// Winsock keeps a connection that was reset or aborted failing the same way for every later
    /// receive and send, where the stack reports it once.
    pub broken: Option<NtStatus>,
    /// Why a connect failed (`SO_ERROR` keeps reading it, as Winsock does).
    pub conn_error: Option<u32>,
    /// A connection exists (or existed): `connect` finished or `accept` made it.
    pub connected: bool,
    /// The peer's address once known (`getpeername` still answers after the connection ended).
    pub peer: Option<SocketAddr>,
    /// When the last non-blocking connect started (system time, 100 ns units; 0: none). Windows
    /// answers a second `connect` made soon after with `WSAEALREADY`, where the stack on a loopback
    /// may have refused the first already.
    pub connect_at: u64,
    /// The socket `AcceptEx` / `ConnectEx` made: its addresses are not told until the context is
    /// updated (`SO_UPDATE_ACCEPT_CONTEXT` / `SO_UPDATE_CONNECT_CONTEXT`).
    pub needs_update: bool,
    /// `shutdown(SD_RECEIVE)` / `shutdown(SD_SEND)` ran.
    pub shut_rd: bool,
    pub shut_wr: bool,
}

impl Endpoint {
    /// How long a receive (or accept) may wait.
    pub fn recv_wait(&self) -> SockWait {
        wait_for(self.nonblocking, self.rcv_timeout)
    }

    /// How long a send (or connect) may wait.
    pub fn send_wait(&self) -> SockWait {
        wait_for(self.nonblocking, self.snd_timeout)
    }
}

fn wait_for(nonblocking: bool, ms: u32) -> SockWait {
    if nonblocking {
        SockWait::Now
    } else if ms == 0 {
        SockWait::Forever
    } else {
        SockWait::Millis(u64::from(ms))
    }
}
