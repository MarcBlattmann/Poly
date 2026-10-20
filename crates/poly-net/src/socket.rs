//! Socket-level types: handles, address families, readiness flags, and the
//! datagram (UDP and ICMP echo) socket state. The calls themselves are
//! methods on [`crate::Stack`].

use crate::errno::Errno;
use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::net::{IpAddr, SocketAddr};

/// Address family (`AF_INET`, `AF_INET6`). IPv6 sockets carry only IPv6
/// (as with `IPV6_V6ONLY` set); there are no IPv4-mapped addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Inet,
    Inet6,
}

impl Domain {
    /// The family of an address.
    pub fn of(addr: &SocketAddr) -> Domain {
        if addr.is_ipv4() { Domain::Inet } else { Domain::Inet6 }
    }

    /// The unspecified address of this family with `port`.
    pub fn any(self, port: u16) -> SocketAddr {
        match self {
            Domain::Inet => SocketAddr::new(IpAddr::V4(core::net::Ipv4Addr::UNSPECIFIED), port),
            Domain::Inet6 => SocketAddr::new(IpAddr::V6(core::net::Ipv6Addr::UNSPECIFIED), port),
        }
    }
}

/// Socket type (`SOCK_STREAM`, `SOCK_DGRAM`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SockType {
    Stream,
    Datagram,
}

/// Transport protocol. `Icmp` is the unprivileged "ping" socket
/// (`SOCK_DGRAM` with `IPPROTO_ICMP` or `IPPROTO_ICMPV6`): it sends echo
/// requests and receives the matching replies, the identifier being the
/// socket's port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Tcp,
    Udp,
    Icmp,
}

/// Which directions `shutdown` closes (`SHUT_RD`, `SHUT_WR`, `SHUT_RDWR`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shutdown {
    Read,
    Write,
    Both,
}

/// Names a socket. Stale handles (of a closed socket whose slot was reused)
/// are detected and fail with `EBADF`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SocketHandle {
    pub(crate) index: u32,
    pub(crate) generation: u32,
}

impl SocketHandle {
    /// A number unique among open sockets, for tables keyed by socket.
    pub fn index(self) -> u32 {
        self.index
    }
}

/// Readiness, for `poll` and `epoll`.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Readiness(pub u32);

impl Readiness {
    /// Data (or a connection, or end of stream, or an error) can be read
    /// without blocking.
    pub const READABLE: Readiness = Readiness(1);
    /// A write would not block.
    pub const WRITABLE: Readiness = Readiness(2);
    /// An error is pending (`SO_ERROR`).
    pub const ERROR: Readiness = Readiness(4);
    /// Both directions are closed, or the socket was never connected.
    pub const HANGUP: Readiness = Readiness(8);
    /// The peer closed its sending side.
    pub const READ_HANGUP: Readiness = Readiness(16);

    pub fn contains(self, other: Readiness) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// As Linux `poll` event bits, from the UAPI header
    /// `include/uapi/asm-generic/poll.h`: POLLIN 0x1, POLLOUT 0x4,
    /// POLLERR 0x8, POLLHUP 0x10, POLLRDNORM 0x40, POLLWRNORM 0x100,
    /// POLLRDHUP 0x2000.
    pub fn to_linux_poll(self) -> u16 {
        let mut e = 0;
        if self.contains(Readiness::READABLE) {
            e |= 0x0001 | 0x0040;
        }
        if self.contains(Readiness::WRITABLE) {
            e |= 0x0004 | 0x0100;
        }
        if self.contains(Readiness::ERROR) {
            e |= 0x0008;
        }
        if self.contains(Readiness::HANGUP) {
            e |= 0x0010;
        }
        if self.contains(Readiness::READ_HANGUP) {
            e |= 0x2000;
        }
        e
    }
}

impl core::ops::BitOr for Readiness {
    type Output = Readiness;
    fn bitor(self, o: Readiness) -> Readiness {
        Readiness(self.0 | o.0)
    }
}

impl core::ops::BitOrAssign for Readiness {
    fn bitor_assign(&mut self, o: Readiness) {
        self.0 |= o.0;
    }
}

impl core::fmt::Debug for Readiness {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let names = [(1, "R"), (2, "W"), (4, "ERR"), (8, "HUP"), (16, "RDHUP")];
        let mut first = true;
        write!(f, "Readiness(")?;
        for (bit, name) in names {
            if self.0 & bit != 0 {
                if !first {
                    write!(f, "|")?;
                }
                write!(f, "{name}")?;
                first = false;
            }
        }
        write!(f, ")")
    }
}

/// What `recv_from` returns besides the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecvInfo {
    /// Bytes copied into the caller's buffer.
    pub len: usize,
    /// The datagram's full length (larger than `len` if it was truncated,
    /// as `MSG_TRUNC` reports). Equal to `len` for TCP.
    pub full_len: usize,
    /// Where it came from.
    pub from: SocketAddr,
}

/// Flags for receive calls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecvFlags {
    /// Leave the data queued (`MSG_PEEK`).
    pub peek: bool,
}

/// A queued datagram.
#[derive(Debug)]
pub(crate) struct Datagram {
    pub addr: SocketAddr,
    pub data: Vec<u8>,
}

/// UDP or ICMP echo socket state.
#[derive(Debug)]
pub(crate) struct DgramSocket {
    pub proto: Protocol,
    /// Bound address; the port is the ICMP identifier for ping sockets.
    pub local: Option<SocketAddr>,
    pub remote: Option<SocketAddr>,
    pub rx: VecDeque<Datagram>,
    pub rx_bytes: usize,
    pub rx_cap: usize,
    pub tx: VecDeque<Datagram>,
    pub tx_bytes: usize,
    pub tx_cap: usize,
    pub error: Option<Errno>,
    pub broadcast: bool,
    pub rd_shut: bool,
    pub wr_shut: bool,
    pub rx_dropped: u64,
    /// Per-socket IP options (`IP_TTL`, `IP_TOS`); `None`: the stack's TTL.
    pub ttl: Option<u8>,
    pub tos: u8,
}

impl DgramSocket {
    pub fn new(proto: Protocol, rx_cap: usize, tx_cap: usize) -> DgramSocket {
        DgramSocket {
            proto,
            local: None,
            remote: None,
            rx: VecDeque::new(),
            rx_bytes: 0,
            rx_cap,
            tx: VecDeque::new(),
            tx_bytes: 0,
            tx_cap,
            error: None,
            broadcast: false,
            rd_shut: false,
            wr_shut: false,
            rx_dropped: 0,
            ttl: None,
            tos: 0,
        }
    }

    /// Queue a received datagram if there is room (datagrams that do not
    /// fit are dropped, as UDP allows).
    pub fn deliver(&mut self, addr: SocketAddr, data: &[u8]) -> bool {
        if self.rd_shut || self.rx_bytes + data.len() > self.rx_cap {
            self.rx_dropped += 1;
            return false;
        }
        self.rx_bytes += data.len();
        self.rx.push_back(Datagram { addr, data: data.to_vec() });
        true
    }

    pub fn readiness(&self) -> Readiness {
        let mut r = Readiness::default();
        if !self.rx.is_empty() || self.error.is_some() || self.rd_shut {
            r |= Readiness::READABLE;
        }
        if self.tx_bytes < self.tx_cap || self.error.is_some() {
            r |= Readiness::WRITABLE;
        }
        if self.error.is_some() {
            r |= Readiness::ERROR;
        }
        if self.rd_shut && self.wr_shut {
            r |= Readiness::HANGUP;
        }
        if self.rd_shut {
            r |= Readiness::READ_HANGUP;
        }
        r
    }

    /// Take the next datagram into `out`.
    pub fn recv(&mut self, out: &mut [u8], flags: RecvFlags) -> Result<RecvInfo, Errno> {
        let Some(d) = self.rx.front() else {
            if let Some(e) = self.error.take() {
                return Err(e);
            }
            return if self.rd_shut {
                Ok(RecvInfo { len: 0, full_len: 0, from: self.unspecified() })
            } else {
                Err(Errno::EAGAIN)
            };
        };
        let n = d.data.len().min(out.len());
        out[..n].copy_from_slice(&d.data[..n]);
        let info = RecvInfo { len: n, full_len: d.data.len(), from: d.addr };
        if !flags.peek {
            self.rx_bytes -= d.data.len();
            self.rx.pop_front();
        }
        Ok(info)
    }

    fn unspecified(&self) -> SocketAddr {
        let domain = self.local.map_or(Domain::Inet, |l| Domain::of(&l));
        domain.any(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_maps_to_linux_poll_bits() {
        let r = Readiness::READABLE | Readiness::WRITABLE | Readiness::HANGUP;
        assert_eq!(r.to_linux_poll(), 0x1 | 0x40 | 0x4 | 0x100 | 0x10);
        assert_eq!((Readiness::ERROR | Readiness::READ_HANGUP).to_linux_poll(), 0x8 | 0x2000);
        assert!(Readiness::default().is_empty());
        assert_eq!(alloc::format!("{:?}", r), "Readiness(R|W|HUP)");
    }

    #[test]
    fn datagram_queue_limits_and_truncation() {
        let from: SocketAddr = "10.0.0.2:7".parse().unwrap();
        let mut s = DgramSocket::new(Protocol::Udp, 10, 10);
        assert!(s.deliver(from, b"hello"));
        assert!(s.deliver(from, b"world"));
        assert!(!s.deliver(from, b"!"), "over the receive buffer");
        let mut out = [0u8; 3];
        let i = s.recv(&mut out, RecvFlags { peek: true }).unwrap();
        assert_eq!((i.len, i.full_len, &out), (3, 5, b"hel"));
        let i = s.recv(&mut out, RecvFlags::default()).unwrap();
        assert_eq!(i.full_len, 5);
        let mut big = [0u8; 8];
        assert_eq!(s.recv(&mut big, RecvFlags::default()).unwrap().len, 5);
        assert_eq!(s.recv(&mut big, RecvFlags::default()).unwrap_err(), Errno::EAGAIN);
        s.error = Some(Errno::ECONNREFUSED);
        assert!(s.readiness().contains(Readiness::ERROR | Readiness::READABLE));
        assert_eq!(s.recv(&mut big, RecvFlags::default()).unwrap_err(), Errno::ECONNREFUSED);
    }
}
