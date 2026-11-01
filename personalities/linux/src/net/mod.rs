//! Sockets (ADR 0048): what the socket system calls need from the kernel
//! ([`LinuxNet`], reached through
//! [`LinuxKernel::net`](crate::LinuxKernel::net)), the types it speaks in,
//! the ABI's structures ([`abi`]) and the `NETLINK_ROUTE` protocol
//! ([`netlink`]).
//!
//! The split follows ADR 0042's: the personality decodes and checks
//! arguments (families, types, flags, `sockaddr`s, `msghdr`s and control
//! messages, option values), gathers and scatters the data between user
//! memory and kernel buffers, raises `SIGPIPE` and decides how a call a
//! signal interrupted ends; the kernel owns the sockets, waits, and
//! installs received descriptors.

pub mod abi;
mod errno;
pub mod netlink;

pub use abi::{SockAddr, Ucred, UnixAddr};

use crate::errno::Errno;
use crate::fileio::NewFd;
use alloc::vec::Vec;

/// Address families the kernel supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Unix,
    Inet,
    Inet6,
    Netlink,
}

impl Family {
    pub const fn from_linux(af: u16) -> Option<Family> {
        match af {
            abi::AF_UNIX => Some(Family::Unix),
            abi::AF_INET => Some(Family::Inet),
            abi::AF_INET6 => Some(Family::Inet6),
            abi::AF_NETLINK => Some(Family::Netlink),
            _ => None,
        }
    }

    pub const fn to_linux(self) -> u16 {
        match self {
            Family::Unix => abi::AF_UNIX,
            Family::Inet => abi::AF_INET,
            Family::Inet6 => abi::AF_INET6,
            Family::Netlink => abi::AF_NETLINK,
        }
    }
}

/// Socket types.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SockType {
    Stream,
    Datagram,
    SeqPacket,
    Raw,
}

impl SockType {
    pub const fn from_linux(ty: u32) -> Option<SockType> {
        match ty {
            abi::SOCK_STREAM => Some(SockType::Stream),
            abi::SOCK_DGRAM => Some(SockType::Datagram),
            abi::SOCK_SEQPACKET => Some(SockType::SeqPacket),
            abi::SOCK_RAW => Some(SockType::Raw),
            _ => None,
        }
    }

    pub const fn to_linux(self) -> u32 {
        match self {
            SockType::Stream => abi::SOCK_STREAM,
            SockType::Datagram => abi::SOCK_DGRAM,
            SockType::SeqPacket => abi::SOCK_SEQPACKET,
            SockType::Raw => abi::SOCK_RAW,
        }
    }
}

/// A checked `(family, type, protocol)` triple (socket(2)): `protocol` is
/// the one the socket reports as `SO_PROTOCOL`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kind {
    pub family: Family,
    pub ty: SockType,
    pub protocol: i32,
}

/// `shutdown(2)`'s `how`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

/// What a send carries besides the data.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendMsg<'a> {
    pub data: &'a [u8],
    /// `sendto`'s destination (`None`: the connected peer).
    pub to: Option<SockAddr>,
    /// Do not block (`MSG_DONTWAIT`).
    pub dontwait: bool,
    /// `SCM_RIGHTS`: descriptors of the caller to pass (Unix sockets).
    pub rights: Vec<i32>,
    /// `SCM_CREDENTIALS`, checked by the personality.
    pub creds: Option<Ucred>,
}

/// How to receive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecvReq {
    pub peek: bool,
    pub dontwait: bool,
    /// `MSG_WAITALL`: a blocking stream receive waits for the whole buffer
    /// (or end of file, an error or a signal).
    pub waitall: bool,
    /// Descriptors the caller's control buffer can take; the kernel
    /// installs at most that many and drops the rest
    /// ([`RecvMsg::rights_truncated`]).
    pub max_rights: usize,
    /// `MSG_CMSG_CLOEXEC`.
    pub cloexec: bool,
}

/// What a receive got.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecvMsg {
    /// Bytes in the buffer.
    pub len: usize,
    /// The datagram's full length (more than `len` when truncated).
    pub full_len: usize,
    /// The sender (datagram and sequenced-packet sockets; `None` where
    /// the protocol gives none, as for streams).
    pub from: Option<SockAddr>,
    /// Descriptors installed for the caller (`SCM_RIGHTS`).
    pub rights: Vec<i32>,
    /// More descriptors came than [`RecvReq::max_rights`] (or than the
    /// descriptor limit allowed); the rest were closed.
    pub rights_truncated: bool,
    /// `SCM_CREDENTIALS` (with `SO_PASSCRED`).
    pub creds: Option<Ucred>,
}

/// A socket option to set, decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SockOpt {
    ReuseAddr(bool),
    /// Accepted and reported; the stack does not share ports among such
    /// sockets (ADR 0048).
    ReusePort(bool),
    KeepAlive(bool),
    Broadcast(bool),
    /// Bytes as `getsockopt` will report them (doubled, clamped).
    SndBuf(usize),
    RcvBuf(usize),
    /// Nanoseconds; 0: none.
    RcvTimeout(u64),
    SndTimeout(u64),
    /// `None`: off; `Some(seconds)`.
    Linger(Option<u32>),
    PassCred(bool),
    OobInline(bool),
    Priority(i32),
    RcvLowat(i32),
    TcpNoDelay(bool),
    TcpKeepIdle(u32),
    TcpKeepIntvl(u32),
    TcpKeepCnt(u32),
    TcpMaxSeg(u32),
    /// `None`: the default.
    IpTtl(Option<u8>),
    IpTos(u8),
    Ipv6V6Only(bool),
    Ipv6UnicastHops(Option<u8>),
    NetlinkAddMembership(u32),
    NetlinkDropMembership(u32),
}

/// A socket option to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptName {
    Type,
    Domain,
    Protocol,
    Error,
    AcceptConn,
    PeerCred,
    ReuseAddr,
    ReusePort,
    KeepAlive,
    Broadcast,
    SndBuf,
    RcvBuf,
    RcvTimeout,
    SndTimeout,
    Linger,
    PassCred,
    OobInline,
    Priority,
    RcvLowat,
    SndLowat,
    TcpNoDelay,
    TcpKeepIdle,
    TcpKeepIntvl,
    TcpKeepCnt,
    TcpMaxSeg,
    IpTtl,
    IpTos,
    Ipv6V6Only,
    Ipv6UnicastHops,
}

/// An option's value, as `getsockopt` encodes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptValue {
    Int(i32),
    /// `struct linger`.
    Linger {
        onoff: bool,
        seconds: u32,
    },
    /// `struct timeval`, from nanoseconds.
    Timeval(u64),
    /// `struct ucred`.
    Ucred(Ucred),
}

/// The kernel's sockets for the calling thread. Descriptors that are not
/// open are `EBADF`, ones that are not sockets `ENOTSOCK`. Calls that
/// block do so unless the descriptor is `O_NONBLOCK` (or the call says
/// `dontwait`), honour the socket's `SO_RCVTIMEO` / `SO_SNDTIMEO`
/// (`EAGAIN`, `EINPROGRESS` for `connect`, when it runs out) and return
/// `EINTR` when the thread's signal notification ends the wait; the
/// personality then decides between `EINTR` and a restart
/// ([`timed`](Self::timed)).
pub trait LinuxNet {
    /// socket(2) for a checked kind.
    fn socket(&mut self, kind: Kind, fd: NewFd) -> Result<i32, Errno>;

    /// socketpair(2) (Unix sockets only).
    fn socketpair(&mut self, kind: Kind, fd: NewFd) -> Result<[i32; 2], Errno>;

    fn bind(&mut self, fd: i32, addr: &SockAddr) -> Result<(), Errno>;

    /// listen(2) with a backlog already clamped to `0..=SOMAXCONN`.
    fn listen(&mut self, fd: i32, backlog: u32) -> Result<(), Errno>;

    /// accept4(2): the new descriptor and the peer's address.
    fn accept(&mut self, fd: i32, new: NewFd) -> Result<(i32, SockAddr), Errno>;

    /// connect(2); `EINPROGRESS` for a non-blocking TCP connect.
    fn connect(&mut self, fd: i32, addr: &SockAddr) -> Result<(), Errno>;

    /// getsockname(2) (`peer` false) or getpeername(2).
    fn name(&mut self, fd: i32, peer: bool) -> Result<SockAddr, Errno>;

    /// sendto(2) / sendmsg(2) of one message (a datagram, or up to all of
    /// a stream chunk); returns the bytes taken. A blocking stream send
    /// takes everything unless interrupted or timed out after some bytes.
    fn send(&mut self, fd: i32, msg: &SendMsg<'_>) -> Result<usize, Errno>;

    /// recvfrom(2) / recvmsg(2) into `buf`.
    fn recv(&mut self, fd: i32, buf: &mut [u8], req: &RecvReq) -> Result<RecvMsg, Errno>;

    fn shutdown(&mut self, fd: i32, how: Shutdown) -> Result<(), Errno>;

    /// setsockopt(2) of a decoded option; `ENOPROTOOPT` for one this kind
    /// of socket does not have.
    fn setsockopt(&mut self, fd: i32, opt: SockOpt) -> Result<(), Errno>;

    /// getsockopt(2); `ENOPROTOOPT` as for `setsockopt`.
    fn getsockopt(&mut self, fd: i32, name: OptName) -> Result<OptValue, Errno>;

    /// Whether the socket has a receive (`send` false) or send timeout:
    /// signal(7) says such calls fail with `EINTR` instead of restarting.
    fn timed(&mut self, fd: i32, send: bool) -> bool;

    /// `SIOCOUTQ`: bytes sent and not yet taken by the peer.
    fn send_queued(&mut self, fd: i32) -> Result<usize, Errno>;

    /// Keep `e` on the socket: its next receive fails with it, once
    /// (recvmmsg(2) with an error after the first message). Sockets that
    /// cannot keep one ignore it.
    fn keep_error(&mut self, fd: i32, e: Errno) {
        let _ = (fd, e);
    }
}
