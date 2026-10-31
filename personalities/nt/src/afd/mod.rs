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
    fn set_option(&self, opt: SockOption) -> Result<(), Errno>;
    fn get_option(&self, opt: OptQuery) -> Result<u32, Errno>;
    /// The handle is closed: calls waiting on it end (`EINTR`). The
    /// socket itself closes when its last reference goes.
    fn release(&self) {}
    /// For the kernel: its own type behind the trait.
    fn as_any(&self) -> &dyn core::any::Any;
}

/// An overlapped send or receive the kernel finishes later
/// ([`crate::kernel::NtKernel::socket_pend`]): the waiting request packet
/// and what to do. The kernel completes it with
/// [`crate::io::IoState::complete`] (a receive's data written to `buf`).
#[derive(Debug)]
pub struct PendIo {
    pub irp: u64,
    pub sock: Arc<dyn NtSocket>,
    pub op: PendOp,
}

#[derive(Debug)]
pub enum PendOp {
    Recv { buf: u64, len: usize, peek: bool },
    Send { data: Vec<u8>, to: Option<SocketAddr> },
}

/// How a request ended: its status, `Information`, and the data to write
/// into the caller's memory (address and bytes).
pub type PendResult = (NtStatus, u64, Option<(u64, Vec<u8>)>);

impl PendIo {
    /// One attempt that does not wait: `None` while it would block, else
    /// the request's status, `Information` and data to write.
    pub fn attempt(&self) -> Option<PendResult> {
        match &self.op {
            PendOp::Recv { buf, len, peek } => {
                let mut data = alloc::vec![0u8; (*len).min(crate::io::irp::MAX_ASYNC_READ)];
                match self.sock.recv(&mut data, *peek, SockWait::Now) {
                    Err(Errno::EAGAIN) => None,
                    Err(e) => Some((status_of(e), 0, None)),
                    Ok(info) => {
                        data.truncate(info.len);
                        let status = if self.sock.kind() == SockType::Datagram && info.full_len > info.len {
                            NtStatus(poly_winsock::error::STATUS_BUFFER_OVERFLOW)
                        } else {
                            poly_ntobj::status::STATUS_SUCCESS
                        };
                        Some((status, info.len as u64, Some((*buf, data))))
                    }
                }
            }
            PendOp::Send { data, to } => match self.sock.send(data, *to, SockWait::Now) {
                Err(Errno::EAGAIN) => None,
                Err(e) => Some((status_of(e), 0, None)),
                Ok(n) => Some((poly_ntobj::status::STATUS_SUCCESS, n as u64, None)),
            },
        }
    }
}

/// A stack error as the status a request completes with.
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
