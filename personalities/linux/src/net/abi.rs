//! Socket constants and structure layouts of the x86-64 Linux ABI.
//!
//! Values from the public headers: address families, socket types and
//! `MSG_*` flags as the C library's `<sys/socket.h>` (`bits/socket.h`,
//! `bits/socket_type.h`) and the UAPI `linux/net.h` give them (`SOCK_NONBLOCK`
//! = `O_NONBLOCK`, `SOCK_CLOEXEC` = `O_CLOEXEC`); `SOL_SOCKET`, `SO_*` and
//! `SCM_*` from `asm-generic/socket.h` (x86-64 uses it); `IPPROTO_*`,
//! `IP_*` from `linux/in.h`; `IPV6_*` from `linux/in6.h`; `TCP_*` from
//! `linux/tcp.h`; `SOL_NETLINK` and `struct sockaddr_nl` from
//! `linux/netlink.h`; `SIOCINQ` / `SIOCOUTQ` from `linux/sockios.h`.
//! Structure layouts: `struct sockaddr_in` (`linux/in.h`), `struct
//! sockaddr_in6` (`linux/in6.h`), `struct sockaddr_un` (`linux/un.h`),
//! `struct msghdr`, `struct mmsghdr`, `struct cmsghdr`, `struct ucred`,
//! `struct linger` (`linux/socket.h`, `sys/socket.h`), `struct timeval`
//! (`linux/time.h`), with the x86-64 psABI's sizes and alignment.

use crate::errno::Errno;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};

pub const AF_UNSPEC: u16 = 0;
pub const AF_UNIX: u16 = 1;
pub const AF_INET: u16 = 2;
pub const AF_INET6: u16 = 10;
pub const AF_NETLINK: u16 = 16;

pub const SOCK_STREAM: u32 = 1;
pub const SOCK_DGRAM: u32 = 2;
pub const SOCK_RAW: u32 = 3;
pub const SOCK_SEQPACKET: u32 = 5;
/// The type bits of `socket`'s `type` argument; the rest are flags.
pub const SOCK_TYPE_MASK: u32 = 0xf;
pub const SOCK_NONBLOCK: u32 = 0o4000;
pub const SOCK_CLOEXEC: u32 = 0o2000000;

pub const MSG_OOB: u32 = 0x1;
pub const MSG_PEEK: u32 = 0x2;
pub const MSG_DONTROUTE: u32 = 0x4;
pub const MSG_CTRUNC: u32 = 0x8;
pub const MSG_TRUNC: u32 = 0x20;
pub const MSG_DONTWAIT: u32 = 0x40;
pub const MSG_EOR: u32 = 0x80;
pub const MSG_WAITALL: u32 = 0x100;
pub const MSG_CONFIRM: u32 = 0x800;
pub const MSG_ERRQUEUE: u32 = 0x2000;
pub const MSG_NOSIGNAL: u32 = 0x4000;
pub const MSG_MORE: u32 = 0x8000;
pub const MSG_WAITFORONE: u32 = 0x10000;
pub const MSG_CMSG_CLOEXEC: u32 = 0x4000_0000;

pub const SHUT_RD: i32 = 0;
pub const SHUT_WR: i32 = 1;
pub const SHUT_RDWR: i32 = 2;

pub const SOL_SOCKET: i32 = 1;
pub const SOL_IP: i32 = 0;
pub const SOL_TCP: i32 = 6;
pub const SOL_UDP: i32 = 17;
pub const SOL_IPV6: i32 = 41;
pub const SOL_NETLINK: i32 = 270;

pub const SO_DEBUG: i32 = 1;
pub const SO_REUSEADDR: i32 = 2;
pub const SO_TYPE: i32 = 3;
pub const SO_ERROR: i32 = 4;
pub const SO_DONTROUTE: i32 = 5;
pub const SO_BROADCAST: i32 = 6;
pub const SO_SNDBUF: i32 = 7;
pub const SO_RCVBUF: i32 = 8;
pub const SO_KEEPALIVE: i32 = 9;
pub const SO_OOBINLINE: i32 = 10;
pub const SO_PRIORITY: i32 = 12;
pub const SO_LINGER: i32 = 13;
pub const SO_REUSEPORT: i32 = 15;
pub const SO_PASSCRED: i32 = 16;
pub const SO_PEERCRED: i32 = 17;
pub const SO_RCVLOWAT: i32 = 18;
pub const SO_SNDLOWAT: i32 = 19;
/// `SO_RCVTIMEO_OLD`, which `SO_RCVTIMEO` is where `time_t` is 64 bits.
pub const SO_RCVTIMEO: i32 = 20;
pub const SO_SNDTIMEO: i32 = 21;
pub const SO_ACCEPTCONN: i32 = 30;
pub const SO_SNDBUFFORCE: i32 = 32;
pub const SO_RCVBUFFORCE: i32 = 33;
pub const SO_PROTOCOL: i32 = 38;
pub const SO_DOMAIN: i32 = 39;

pub const SCM_RIGHTS: i32 = 1;
pub const SCM_CREDENTIALS: i32 = 2;

pub const IPPROTO_IP: i32 = 0;
pub const IPPROTO_ICMP: i32 = 1;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;
pub const IPPROTO_ICMPV6: i32 = 58;

pub const IP_TOS: i32 = 1;
pub const IP_TTL: i32 = 2;
pub const IPV6_UNICAST_HOPS: i32 = 16;
pub const IPV6_V6ONLY: i32 = 26;

pub const TCP_NODELAY: i32 = 1;
pub const TCP_MAXSEG: i32 = 2;
pub const TCP_KEEPIDLE: i32 = 4;
pub const TCP_KEEPINTVL: i32 = 5;
pub const TCP_KEEPCNT: i32 = 6;

pub const NETLINK_ROUTE: i32 = 0;
pub const NETLINK_ADD_MEMBERSHIP: i32 = 1;
pub const NETLINK_DROP_MEMBERSHIP: i32 = 2;

/// `SIOCINQ` is `FIONREAD`; `SIOCOUTQ` is `TIOCOUTQ` (`linux/sockios.h`).
pub const SIOCINQ: u32 = 0x541B;
pub const SIOCOUTQ: u32 = 0x5411;

/// `sizeof(struct sockaddr_un)`: the family and 108 bytes of path.
pub const SOCKADDR_UN_SIZE: usize = 110;
/// `sizeof(struct sockaddr_in)`.
pub const SOCKADDR_IN_SIZE: usize = 16;
/// `sizeof(struct sockaddr_in6)`.
pub const SOCKADDR_IN6_SIZE: usize = 28;
/// `sizeof(struct sockaddr_nl)`.
pub const SOCKADDR_NL_SIZE: usize = 12;
/// The largest address the calls accept (`sizeof(struct
/// sockaddr_storage)`).
pub const SOCKADDR_MAX: usize = 128;

/// A Unix domain socket address (unix(7), "Address format").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnixAddr {
    /// `addrlen` is `sizeof(sa_family_t)`.
    Unnamed,
    /// A path (without its terminating NUL, never empty).
    Path(Vec<u8>),
    /// A name in the abstract namespace (after the leading NUL).
    Abstract(Vec<u8>),
}

/// A socket address of any supported family.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SockAddr {
    /// `AF_UNSPEC`: dissolve a datagram socket's association (connect(2)).
    Unspec,
    Inet(SocketAddrV4),
    Inet6(SocketAddrV6),
    Unix(UnixAddr),
    /// `struct sockaddr_nl`: port id and multicast groups.
    Netlink {
        pid: u32,
        groups: u32,
    },
}

impl SockAddr {
    /// The family of this address.
    pub fn family(&self) -> u16 {
        match self {
            SockAddr::Unspec => AF_UNSPEC,
            SockAddr::Inet(_) => AF_INET,
            SockAddr::Inet6(_) => AF_INET6,
            SockAddr::Unix(_) => AF_UNIX,
            SockAddr::Netlink { .. } => AF_NETLINK,
        }
    }

    /// Decode an address of `bytes.len()` bytes (the caller's `addrlen`,
    /// already checked against [`SOCKADDR_MAX`]). `EINVAL` when it is too
    /// short for its family (bind(2), connect(2)), `EAFNOSUPPORT` for an
    /// unknown family.
    pub fn decode(bytes: &[u8]) -> Result<SockAddr, Errno> {
        if bytes.len() < 2 {
            return Err(Errno::EINVAL);
        }
        let family = u16::from_le_bytes([bytes[0], bytes[1]]);
        match family {
            AF_UNSPEC => Ok(SockAddr::Unspec),
            AF_INET => {
                if bytes.len() < SOCKADDR_IN_SIZE {
                    return Err(Errno::EINVAL);
                }
                let port = u16::from_be_bytes([bytes[2], bytes[3]]);
                let ip = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
                Ok(SockAddr::Inet(SocketAddrV4::new(ip, port)))
            }
            AF_INET6 => {
                // ipv6(7): `struct sockaddr_in6` is 28 bytes; the 24-byte
                // form without `sin6_scope_id` (RFC 2133) is accepted too.
                if bytes.len() < 24 {
                    return Err(Errno::EINVAL);
                }
                let port = u16::from_be_bytes([bytes[2], bytes[3]]);
                let flow = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                let mut a = [0u8; 16];
                a.copy_from_slice(&bytes[8..24]);
                let scope = if bytes.len() >= 28 {
                    u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]])
                } else {
                    0
                };
                Ok(SockAddr::Inet6(SocketAddrV6::new(Ipv6Addr::from(a), port, flow, scope)))
            }
            AF_UNIX => {
                if bytes.len() > SOCKADDR_UN_SIZE {
                    return Err(Errno::EINVAL);
                }
                let path = &bytes[2..];
                if path.is_empty() {
                    return Ok(SockAddr::Unix(UnixAddr::Unnamed));
                }
                if path[0] == 0 {
                    return Ok(SockAddr::Unix(UnixAddr::Abstract(path[1..].to_vec())));
                }
                // unix(7) BUGS: the path ends at the first NUL, or at the
                // end of the address (Linux adds the NUL).
                let end = path.iter().position(|&b| b == 0).unwrap_or(path.len());
                Ok(SockAddr::Unix(UnixAddr::Path(path[..end].to_vec())))
            }
            AF_NETLINK => {
                if bytes.len() < SOCKADDR_NL_SIZE {
                    return Err(Errno::EINVAL);
                }
                let pid = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
                let groups = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
                Ok(SockAddr::Netlink { pid, groups })
            }
            _ => Err(Errno::EAFNOSUPPORT),
        }
    }

    /// The address as the calls return it, at its actual length (unix(7):
    /// `offsetof(sun_path) + strlen + 1` for a path, `2 + 1 + len` for an
    /// abstract name, 2 for an unnamed socket).
    pub fn encode(&self) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&self.family().to_le_bytes());
        match self {
            SockAddr::Unspec => {}
            SockAddr::Inet(a) => {
                b.extend_from_slice(&a.port().to_be_bytes());
                b.extend_from_slice(&a.ip().octets());
                b.extend_from_slice(&[0; 8]);
            }
            SockAddr::Inet6(a) => {
                b.extend_from_slice(&a.port().to_be_bytes());
                b.extend_from_slice(&a.flowinfo().to_be_bytes());
                b.extend_from_slice(&a.ip().octets());
                b.extend_from_slice(&a.scope_id().to_le_bytes());
            }
            SockAddr::Unix(UnixAddr::Unnamed) => {}
            SockAddr::Unix(UnixAddr::Path(p)) => {
                b.extend_from_slice(p);
                b.push(0);
            }
            SockAddr::Unix(UnixAddr::Abstract(n)) => {
                b.push(0);
                b.extend_from_slice(n);
            }
            SockAddr::Netlink { pid, groups } => {
                b.extend_from_slice(&[0, 0]);
                b.extend_from_slice(&pid.to_le_bytes());
                b.extend_from_slice(&groups.to_le_bytes());
            }
        }
        b
    }
}

/// `struct msghdr` (56 bytes): name, namelen, iov, iovlen, control,
/// controllen, flags.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MsgHdr {
    pub name: u64,
    pub namelen: u32,
    pub iov: u64,
    pub iovlen: u64,
    pub control: u64,
    pub controllen: u64,
    pub flags: u32,
}

impl MsgHdr {
    pub const SIZE: usize = 56;
    /// Offsets of the fields a receive writes back.
    pub const NAMELEN_OFFSET: u64 = 8;
    pub const CONTROLLEN_OFFSET: u64 = 40;
    pub const FLAGS_OFFSET: u64 = 48;

    pub fn from_bytes(b: &[u8; Self::SIZE]) -> MsgHdr {
        let u64_at = |o: usize| u64::from_le_bytes(b[o..o + 8].try_into().expect("8 bytes"));
        let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().expect("4 bytes"));
        MsgHdr {
            name: u64_at(0),
            namelen: u32_at(8),
            iov: u64_at(16),
            iovlen: u64_at(24),
            control: u64_at(32),
            controllen: u64_at(40),
            flags: u32_at(48),
        }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..8].copy_from_slice(&self.name.to_le_bytes());
        b[8..12].copy_from_slice(&self.namelen.to_le_bytes());
        b[16..24].copy_from_slice(&self.iov.to_le_bytes());
        b[24..32].copy_from_slice(&self.iovlen.to_le_bytes());
        b[32..40].copy_from_slice(&self.control.to_le_bytes());
        b[40..48].copy_from_slice(&self.controllen.to_le_bytes());
        b[48..52].copy_from_slice(&self.flags.to_le_bytes());
        b
    }
}

/// `struct mmsghdr`: a `msghdr` and `unsigned int msg_len`, 64 bytes.
pub const MMSGHDR_SIZE: u64 = 64;

/// `struct cmsghdr`'s header: `size_t cmsg_len; int cmsg_level; int
/// cmsg_type;`, 16 bytes; data are aligned to `size_t` (cmsg(3)).
pub const CMSG_HDR: usize = 16;

/// `CMSG_ALIGN`.
pub const fn cmsg_align(n: usize) -> usize {
    (n + 7) & !7
}

/// `CMSG_LEN(n)`: the header and `n` bytes of data.
pub const fn cmsg_len(n: usize) -> usize {
    CMSG_HDR + n
}

/// `CMSG_SPACE(n)`: what one message with `n` bytes of data takes.
pub const fn cmsg_space(n: usize) -> usize {
    CMSG_HDR + cmsg_align(n)
}

/// One control message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cmsg {
    pub level: i32,
    pub ty: i32,
    pub data: Vec<u8>,
}

/// Parse a control buffer as `CMSG_FIRSTHDR` / `CMSG_NXTHDR` walk it:
/// `EINVAL` for a header whose length is below the header's or past the
/// buffer (sendmsg(2): "EINVAL ... an invalid ancillary message").
pub fn parse_cmsgs(buf: &[u8]) -> Result<Vec<Cmsg>, Errno> {
    let mut out = Vec::new();
    let mut at = 0;
    while at + CMSG_HDR <= buf.len() {
        let len = u64::from_le_bytes(buf[at..at + 8].try_into().expect("8 bytes")) as usize;
        if len < CMSG_HDR || len > buf.len() - at {
            return Err(Errno::EINVAL);
        }
        let level = i32::from_le_bytes(buf[at + 8..at + 12].try_into().expect("4 bytes"));
        let ty = i32::from_le_bytes(buf[at + 12..at + 16].try_into().expect("4 bytes"));
        out.push(Cmsg { level, ty, data: buf[at + CMSG_HDR..at + len].to_vec() });
        at += cmsg_align(len);
    }
    Ok(out)
}

/// Append one control message to `out` (header, data, padding to the
/// next `CMSG_SPACE`, but no padding after the last one is needed: the
/// caller trims).
pub fn push_cmsg(out: &mut Vec<u8>, level: i32, ty: i32, data: &[u8]) {
    out.extend_from_slice(&(cmsg_len(data.len()) as u64).to_le_bytes());
    out.extend_from_slice(&level.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(data);
    out.resize(cmsg_align(out.len()), 0);
}

/// `struct ucred` (12 bytes): pid, uid, gid.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ucred {
    pub pid: i32,
    pub uid: u32,
    pub gid: u32,
}

impl Ucred {
    pub const SIZE: usize = 12;

    pub fn from_bytes(b: &[u8]) -> Ucred {
        Ucred {
            pid: i32::from_le_bytes(b[0..4].try_into().expect("4 bytes")),
            uid: u32::from_le_bytes(b[4..8].try_into().expect("4 bytes")),
            gid: u32::from_le_bytes(b[8..12].try_into().expect("4 bytes")),
        }
    }

    pub fn to_bytes(self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.pid.to_le_bytes());
        b[4..8].copy_from_slice(&self.uid.to_le_bytes());
        b[8..12].copy_from_slice(&self.gid.to_le_bytes());
        b
    }
}

/// `struct linger` (8 bytes): `l_onoff`, `l_linger` (seconds).
pub const LINGER_SIZE: usize = 8;
/// `struct timeval` (16 bytes): seconds, microseconds.
pub const TIMEVAL_SIZE: usize = 16;
