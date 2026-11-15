//! The socket calls (ADR 0048) against a fake [`LinuxNet`]: argument
//! rules, `sockaddr` and `msghdr` encodings, control messages, options,
//! chunked stream sends, `SIGPIPE`, and how interrupted calls end. The
//! sockets themselves are tested in `poly-unix`, `poly-net` and the
//! kernel.

extern crate std;

use super::fake::*;
use crate::abi::Iovec;
use crate::abi::signal::{SIGCHLD, SIGPIPE, SIGUSR1};
use crate::args::Args;
use crate::errno::Errno;
use crate::fileio::NewFd;
use crate::net::abi::{self, MsgHdr, SockAddr, Ucred, UnixAddr};
use crate::net::{
    Family, Kind, LinuxNet, Membership, OptName, OptValue, RecvMsg, RecvReq, SendMsg, Shutdown, SockOpt, SockType,
};
use crate::nr;
use crate::sys::socket;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::collections::{BTreeMap, VecDeque};
use std::vec;
use std::vec::Vec;

/// A socket of the fake.
#[derive(Clone, Debug)]
pub struct FakeSock {
    pub kind: Kind,
    pub new: NewFd,
    pub opts: Vec<SockOpt>,
}

/// One send the fake took: fd, data, destination, dontwait, rights, creds.
pub type Sent = (i32, Vec<u8>, Option<SockAddr>, bool, Vec<i32>, Option<Ucred>);

/// Scripted sockets: records what the personality asked for and answers
/// from queues.
#[derive(Debug, Default)]
pub struct FakeNet {
    pub next_fd: i32,
    pub socks: BTreeMap<i32, FakeSock>,
    pub binds: Vec<(i32, SockAddr)>,
    pub connects: Vec<(i32, SockAddr)>,
    pub listens: Vec<(i32, u32)>,
    pub shutdowns: Vec<(i32, Shutdown)>,
    /// Every send: fd, data, destination, dontwait, rights, creds.
    pub sent: Vec<Sent>,
    /// Bytes each send takes (all when empty), or its error.
    pub send_results: VecDeque<Result<usize, Errno>>,
    /// What receives get: the data and the rest of the answer.
    pub recvs: VecDeque<Result<(Vec<u8>, RecvMsg), Errno>>,
    pub recv_reqs: Vec<(i32, usize, RecvReq)>,
    pub accepts: VecDeque<Result<(i32, SockAddr), Errno>>,
    pub accept_flags: Vec<NewFd>,
    pub connect_result: Option<Errno>,
    pub timed: bool,
    pub names: BTreeMap<(i32, bool), SockAddr>,
    pub outq: usize,
    /// The network namespace its sockets are in.
    pub netns: Option<std::sync::Arc<crate::ns::NetNs>>,
}

impl FakeNet {
    pub fn new() -> FakeNet {
        FakeNet { next_fd: 10, ..FakeNet::default() }
    }

    fn sock(&self, fd: i32) -> Result<&FakeSock, Errno> {
        if fd < 3 {
            return Err(Errno::ENOTSOCK);
        }
        self.socks.get(&fd).ok_or(Errno::EBADF)
    }

    /// A socket of `kind` on the next descriptor.
    pub fn add(&mut self, family: Family, ty: SockType) -> i32 {
        let fd = self.next_fd;
        self.next_fd += 1;
        let kind = Kind { family, ty, protocol: 0 };
        self.socks.insert(fd, FakeSock { kind, new: NewFd::default(), opts: Vec::new() });
        fd
    }
}

impl LinuxNet for FakeNet {
    fn socket(&mut self, kind: Kind, fd: NewFd) -> Result<i32, Errno> {
        let n = self.next_fd;
        self.next_fd += 1;
        self.socks.insert(n, FakeSock { kind, new: fd, opts: Vec::new() });
        Ok(n)
    }

    fn socketpair(&mut self, kind: Kind, fd: NewFd) -> Result<[i32; 2], Errno> {
        Ok([self.socket(kind, fd)?, self.socket(kind, fd)?])
    }

    fn bind(&mut self, fd: i32, addr: &SockAddr) -> Result<(), Errno> {
        self.sock(fd)?;
        self.binds.push((fd, addr.clone()));
        Ok(())
    }

    fn listen(&mut self, fd: i32, backlog: u32) -> Result<(), Errno> {
        self.sock(fd)?;
        self.listens.push((fd, backlog));
        Ok(())
    }

    fn accept(&mut self, fd: i32, new: NewFd) -> Result<(i32, SockAddr), Errno> {
        self.sock(fd)?;
        self.accept_flags.push(new);
        self.accepts.pop_front().unwrap_or(Err(Errno::EAGAIN))
    }

    fn connect(&mut self, fd: i32, addr: &SockAddr) -> Result<(), Errno> {
        self.sock(fd)?;
        self.connects.push((fd, addr.clone()));
        self.connect_result.map_or(Ok(()), Err)
    }

    fn name(&mut self, fd: i32, peer: bool) -> Result<SockAddr, Errno> {
        self.sock(fd)?;
        self.names.get(&(fd, peer)).cloned().ok_or(Errno::ENOTCONN)
    }

    fn send(&mut self, fd: i32, msg: &SendMsg<'_>) -> Result<usize, Errno> {
        self.sock(fd)?;
        let r = self.send_results.pop_front().unwrap_or(Ok(msg.data.len()));
        let taken = r.map(|n| n.min(msg.data.len()));
        if let Ok(n) = taken {
            self.sent.push((fd, msg.data[..n].to_vec(), msg.to.clone(), msg.dontwait, msg.rights.clone(), msg.creds));
        }
        taken
    }

    fn recv(&mut self, fd: i32, buf: &mut [u8], req: &RecvReq) -> Result<RecvMsg, Errno> {
        self.sock(fd)?;
        self.recv_reqs.push((fd, buf.len(), *req));
        let (data, mut msg) = self.recvs.pop_front().unwrap_or(Err(Errno::EAGAIN))?;
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        msg.len = n;
        msg.full_len = msg.full_len.max(data.len());
        if msg.rights.len() > req.max_rights {
            msg.rights.truncate(req.max_rights);
            msg.rights_truncated = true;
        }
        Ok(msg)
    }

    fn shutdown(&mut self, fd: i32, how: Shutdown) -> Result<(), Errno> {
        self.sock(fd)?;
        self.shutdowns.push((fd, how));
        Ok(())
    }

    fn setsockopt(&mut self, fd: i32, opt: SockOpt) -> Result<(), Errno> {
        self.sock(fd)?;
        self.socks.get_mut(&fd).expect("checked").opts.push(opt);
        Ok(())
    }

    fn getsockopt(&mut self, fd: i32, name: OptName) -> Result<OptValue, Errno> {
        let s = self.sock(fd)?;
        Ok(match name {
            OptName::Type => OptValue::Int(s.kind.ty.to_linux() as i32),
            OptName::Domain => OptValue::Int(i32::from(s.kind.family.to_linux())),
            OptName::RcvTimeout => OptValue::Timeval(1_500_000_000),
            OptName::Linger => OptValue::Linger { onoff: true, seconds: 7 },
            OptName::PeerCred => OptValue::Ucred(Ucred { pid: 5, uid: 6, gid: 7 }),
            OptName::SndBuf => OptValue::Int(4096),
            // The options of ip(7), ipv6(7) and tcp(7): defaults, as a
            // socket that never set them reports.
            OptName::Stored(o) => OptValue::Int(o.default_value()),
            OptName::Ipv6Tclass => OptValue::Int(0),
            OptName::IpMtu => OptValue::Int(1500),
            OptName::IpMulticastIf => OptValue::bytes(&[10, 0, 2, 15]),
            OptName::Ipv6PktInfo => OptValue::bytes(&[0; 20]),
            OptName::TcpCongestion => OptValue::bytes(b"cubic\0"),
            _ => return Err(Errno::ENOPROTOOPT),
        })
    }

    fn timed(&mut self, _fd: i32, _send: bool) -> bool {
        self.timed
    }

    fn send_queued(&mut self, fd: i32) -> Result<usize, Errno> {
        self.sock(fd)?;
        Ok(self.outq)
    }

    fn netns(&mut self, fd: i32) -> Result<Option<std::sync::Arc<crate::ns::NetNs>>, Errno> {
        self.sock(fd)?;
        Ok(self.netns.clone())
    }
}

fn kernel() -> FakeKernel {
    let mut k = FakeKernel::new();
    k.net = Some(FakeNet::new());
    k
}

fn net(k: &mut FakeKernel) -> &mut FakeNet {
    k.net.as_mut().expect("a fake network")
}

fn sockaddr_in(ip: [u8; 4], port: u16) -> Vec<u8> {
    let mut b = vec![2, 0];
    b.extend_from_slice(&port.to_be_bytes());
    b.extend_from_slice(&ip);
    b.extend_from_slice(&[0; 8]);
    b
}

const S: u64 = scratch(0);

#[test]
fn socket_checks_domain_type_and_protocol() {
    let ok = |d, t, p| socket::check_kind(d, t, p).map(|(k, _)| k);
    let inet = |ty, protocol| Ok(Kind { family: Family::Inet, ty, protocol });
    assert_eq!(ok(2, 1, 0), inet(SockType::Stream, 6));
    assert_eq!(ok(2, 2, 17), inet(SockType::Datagram, 17));
    assert_eq!(ok(2, 2, 1), inet(SockType::Datagram, 1), "a ping socket");
    assert_eq!(ok(10, 2, 58).map(|k| k.protocol), Ok(58));
    assert_eq!(ok(2, 1, 17), Err(Errno::EPROTONOSUPPORT));
    assert_eq!(ok(2, 5, 0), Err(Errno::ESOCKTNOSUPPORT));
    // Raw sockets (raw(7), packet(7)).
    assert_eq!(ok(2, 3, 255), Ok(Kind { family: Family::Inet, ty: SockType::Raw, protocol: 255 }));
    assert_eq!(ok(10, 3, 58).map(|k| k.protocol), Ok(58));
    assert_eq!(ok(2, 3, 0), Err(Errno::EPROTONOSUPPORT));
    assert_eq!(ok(2, 3, 256), Err(Errno::EPROTONOSUPPORT));
    assert_eq!(ok(17, 3, 0x300), Ok(Kind { family: Family::Packet, ty: SockType::Raw, protocol: 0x300 }));
    assert_eq!(ok(17, 2, 0).map(|k| k.ty), Ok(SockType::Datagram));
    assert_eq!(ok(17, 1, 0), Err(Errno::ESOCKTNOSUPPORT));
    assert_eq!(ok(1, 5, 0).map(|k| k.ty), Ok(SockType::SeqPacket));
    assert_eq!(ok(1, 1, 1).map(|k| k.protocol), Ok(0), "PF_UNIX as protocol");
    assert_eq!(ok(1, 1, 6), Err(Errno::EPROTONOSUPPORT));
    assert_eq!(ok(1, 3, 0), Err(Errno::ESOCKTNOSUPPORT));
    assert_eq!(ok(16, 3, 0).map(|k| k.family), Ok(Family::Netlink));
    assert_eq!(ok(16, 3, 15), Err(Errno::EPROTONOSUPPORT));
    assert_eq!(ok(16, 1, 0), Err(Errno::ESOCKTNOSUPPORT));
    assert_eq!(ok(99, 1, 0), Err(Errno::EAFNOSUPPORT));
    assert_eq!(ok(2, 9, 0), Err(Errno::EINVAL));
    assert_eq!(ok(2, 1 | 0x10, 0), Err(Errno::EINVAL), "unknown flag bits");
    let (_, new) = socket::check_kind(2, 1 | abi::SOCK_NONBLOCK | abi::SOCK_CLOEXEC, 0).unwrap();
    assert_eq!(new, NewFd { nonblock: true, cloexec: true });
    // Through the dispatcher.
    let mut k = kernel();
    assert_eq!(sys(&mut k, nr::SOCKET, &[1, 2 | u64::from(abi::SOCK_CLOEXEC), 0]), Ok(10));
    assert_eq!(net(&mut k).socks[&10].new, NewFd { nonblock: false, cloexec: true });
    let mut bare = FakeKernel::new();
    assert_eq!(sys(&mut bare, nr::SOCKET, &[1, 1, 0]), Err(Errno::ENOSYS));
}

#[test]
fn socketpair_writes_both_descriptors() {
    let mut k = kernel();
    assert_eq!(sys(&mut k, nr::SOCKETPAIR, &[1, 1, 0, S]), Ok(0));
    assert_eq!((k.peek_u32(S), k.peek_u32(S + 4)), (10, 11));
    assert_eq!(sys(&mut k, nr::SOCKETPAIR, &[2, 1, 0, S]), Err(Errno::EOPNOTSUPP));
    assert_eq!(sys(&mut k, nr::SOCKETPAIR, &[1, 1, 0, UNMAPPED]), Err(Errno::EFAULT));
}

#[test]
fn addresses_are_decoded_per_family() {
    let mut k = kernel();
    let fd = u64::try_from(net(&mut k).add(Family::Inet, SockType::Stream)).unwrap();
    k.poke(S, &sockaddr_in([127, 0, 0, 1], 8080));
    assert_eq!(sys(&mut k, nr::BIND, &[fd, S, 16]), Ok(0));
    assert_eq!(net(&mut k).binds[0].1, SockAddr::Inet(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)));
    assert_eq!(sys(&mut k, nr::BIND, &[fd, S, 15]), Err(Errno::EINVAL), "too short for sockaddr_in");
    assert_eq!(sys(&mut k, nr::BIND, &[fd, S, 129]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::BIND, &[fd, S, (-1i64) as u64]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::BIND, &[fd, UNMAPPED, 16]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::BIND, &[u64::MAX, S, 16]), Err(Errno::EBADF));
    k.poke(S, &[77, 0, 0, 0]);
    assert_eq!(sys(&mut k, nr::CONNECT, &[fd, S, 16]), Err(Errno::EAFNOSUPPORT));
    // sockaddr_in6: port, flow, address, scope.
    let mut six = vec![10, 0];
    six.extend_from_slice(&443u16.to_be_bytes());
    six.extend_from_slice(&0u32.to_be_bytes());
    six.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
    six.extend_from_slice(&3u32.to_le_bytes());
    k.poke(S, &six);
    assert_eq!(sys(&mut k, nr::CONNECT, &[fd, S, 28]), Ok(0));
    let want = SockAddr::Inet6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 443, 0, 3));
    assert_eq!(net(&mut k).connects[0].1, want);
    assert_eq!(SockAddr::decode(&want.encode()), Ok(want.clone()));
    assert_eq!(want.encode().len(), abi::SOCKADDR_IN6_SIZE);
    // sockaddr_un: a path (to the first NUL), an abstract name (every
    // byte counted by addrlen), unnamed.
    k.poke(S, b"\x01\x00/tmp/s\x00garbage");
    assert_eq!(sys(&mut k, nr::CONNECT, &[fd, S, 17]), Ok(0));
    assert_eq!(net(&mut k).connects[1].1, SockAddr::Unix(UnixAddr::Path(b"/tmp/s".to_vec())));
    k.poke(S, b"\x01\x00\x00ab\x00c");
    assert_eq!(sys(&mut k, nr::CONNECT, &[fd, S, 7]), Ok(0));
    assert_eq!(net(&mut k).connects[2].1, SockAddr::Unix(UnixAddr::Abstract(b"ab\0c".to_vec())));
    assert_eq!(sys(&mut k, nr::BIND, &[fd, S, 2]), Ok(0));
    assert_eq!(net(&mut k).binds.last().unwrap().1, SockAddr::Unix(UnixAddr::Unnamed));
    assert_eq!(sys(&mut k, nr::BIND, &[fd, S, 111]), Err(Errno::EINVAL), "longer than sockaddr_un");
    // Encodings unix(7) gives for the returned lengths.
    assert_eq!(SockAddr::Unix(UnixAddr::Path(b"/a".to_vec())).encode(), b"\x01\x00/a\x00");
    assert_eq!(SockAddr::Unix(UnixAddr::Abstract(b"x".to_vec())).encode(), b"\x01\x00\x00x");
    assert_eq!(SockAddr::Unix(UnixAddr::Unnamed).encode().len(), 2);
    assert_eq!(SockAddr::Netlink { pid: 9, groups: 1 }.encode().len(), abi::SOCKADDR_NL_SIZE);
}

#[test]
fn names_are_truncated_to_the_callers_room() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Unix, SockType::Stream);
    let long = SockAddr::Unix(UnixAddr::Path(b"/run/a-long-name".to_vec()));
    net(&mut k).names.insert((fd, false), long.clone());
    let fd = fd as u64;
    k.poke(S + 0x100, &8u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::GETSOCKNAME, &[fd, S, S + 0x100]), Ok(0));
    assert_eq!(k.peek(S, 8), &long.encode()[..8]);
    assert_eq!(k.peek_u32(S + 0x100) as usize, long.encode().len(), "the real length");
    assert_eq!(sys(&mut k, nr::GETPEERNAME, &[fd, S, S + 0x100]), Err(Errno::ENOTCONN));
    k.poke(S + 0x100, &(-1i32).to_le_bytes());
    assert_eq!(sys(&mut k, nr::GETSOCKNAME, &[fd, S, S + 0x100]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::GETSOCKNAME, &[fd, 0, S + 0x100]), Err(Errno::EFAULT));
    assert_eq!(sys(&mut k, nr::GETSOCKNAME, &[1, S, S + 0x100]), Err(Errno::ENOTSOCK));
}

#[test]
fn listen_caps_the_backlog_and_shutdown_checks_how() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    sys(&mut k, nr::LISTEN, &[fd, 5]).unwrap();
    sys(&mut k, nr::LISTEN, &[fd, 100_000]).unwrap();
    sys(&mut k, nr::LISTEN, &[fd, (-1i64) as u64]).unwrap();
    assert_eq!(net(&mut k).listens, [(fd as i32, 5), (fd as i32, 4096), (fd as i32, 4096)]);
    assert_eq!(sys(&mut k, nr::SHUTDOWN, &[fd, 3]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::SHUTDOWN, &[fd, 1]), Ok(0));
    assert_eq!(net(&mut k).shutdowns, [(fd as i32, Shutdown::Write)]);
}

#[test]
fn accept4_flags_peer_address_and_interruptions() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    let peer = SockAddr::Inet(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 1), 4000));
    net(&mut k).accepts.push_back(Ok((20, peer.clone())));
    k.poke(S + 0x100, &16u32.to_le_bytes());
    let flags = u64::from(abi::SOCK_NONBLOCK);
    assert_eq!(sys(&mut k, nr::ACCEPT4, &[fd, S, S + 0x100, flags]), Ok(20));
    assert_eq!(k.peek(S, 16), peer.encode());
    assert_eq!(net(&mut k).accept_flags[0], NewFd { nonblock: true, cloexec: false });
    assert_eq!(sys(&mut k, nr::ACCEPT4, &[fd, 0, 0, 0x10]), Err(Errno::EINVAL));
    k.poke(S + 0x100, &(-5i32).to_le_bytes());
    assert_eq!(sys(&mut k, nr::ACCEPT, &[fd, S, S + 0x100]), Err(Errno::EINVAL));
    // Interrupted: a restart under SA_RESTART, unless SO_RCVTIMEO is set:
    // then EINTR for a signal that acts, on for one that is ignored.
    net(&mut k).accepts.push_back(Err(Errno::EINTR));
    let args = Args([fd, 0, 0, 0, 0, 0]);
    assert_eq!(socket::accept(&mut k, &args), Err(Errno::RESTART_IF_SA_RESTART));
    net(&mut k).timed = true;
    k.me().traced = true;
    pend(&mut k, SIGCHLD);
    net(&mut k).accepts.push_back(Err(Errno::EINTR));
    assert_eq!(socket::accept(&mut k, &args), Err(Errno::RESTART_IF_NO_HANDLER), "ignored SIGCHLD");
    pend(&mut k, SIGUSR1);
    net(&mut k).accepts.push_back(Err(Errno::EINTR));
    assert_eq!(socket::accept(&mut k, &args), Err(Errno::EINTR));
}

/// Make `signal` pending for the calling thread (pid and tid 100).
fn pend(k: &mut FakeKernel, signal: u32) {
    assert_eq!(sys(k, nr::TKILL, &[100, u64::from(signal)]), Ok(0));
}

#[test]
fn connect_restarts_unless_timed() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    k.poke(S, &sockaddr_in([10, 0, 2, 2], 80));
    net(&mut k).connect_result = Some(Errno::EINTR);
    let args = Args([fd, S, 16, 0, 0, 0]);
    assert_eq!(socket::connect(&mut k, &args), Err(Errno::RESTART_IF_SA_RESTART));
    net(&mut k).timed = true;
    assert_eq!(socket::connect(&mut k, &args), Err(Errno::RESTART_IF_NO_HANDLER), "no signal acts");
    pend(&mut k, SIGUSR1);
    assert_eq!(socket::connect(&mut k, &args), Err(Errno::EINTR));
    net(&mut k).connect_result = Some(Errno::EINPROGRESS);
    assert_eq!(socket::connect(&mut k, &args), Err(Errno::EINPROGRESS));
}

#[test]
fn sendto_epipe_raises_sigpipe_unless_msg_nosignal() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Unix, SockType::Datagram) as u64;
    k.poke(S, b"data");
    net(&mut k).send_results.push_back(Err(Errno::EPIPE));
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, S, 4, u64::from(abi::MSG_NOSIGNAL), 0, 0]), Err(Errno::EPIPE));
    let tid = k.tid;
    assert!(!k.me().pending(tid).contains(SIGPIPE));
    net(&mut k).send_results.push_back(Err(Errno::EPIPE));
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, S, 4, 0, 0, 0]), Err(Errno::EPIPE));
    assert!(k.me().pending(tid).contains(SIGPIPE));
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, S, 4, u64::from(abi::MSG_OOB), 0, 0]), Err(Errno::EOPNOTSUPP));
    // A destination and MSG_DONTWAIT reach the kernel.
    k.poke(S + 0x100, &sockaddr_in([1, 2, 3, 4], 5));
    let flags = u64::from(abi::MSG_DONTWAIT);
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, S, 4, flags, S + 0x100, 16]), Ok(4));
    let sent = net(&mut k).sent.last().cloned().unwrap();
    assert_eq!((sent.1.as_slice(), sent.3), (b"data".as_slice(), true));
    assert_eq!(sent.2, Some(SockAddr::Inet(SocketAddrV4::new(Ipv4Addr::new(1, 2, 3, 4), 5))));
}

#[test]
fn stream_sends_go_in_chunks_and_stop_at_a_short_one() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    let big = 0x10_0000;
    k.map_range(0x100_0000, 0x4_0000, RW);
    let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
    k.poke(0x100_0000, &data);
    let _ = big;
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, 0x100_0000, 200_000, 0, 0, 0]), Ok(200_000));
    let sizes: Vec<usize> = net(&mut k).sent.iter().map(|s| s.1.len()).collect();
    assert_eq!(sizes, [65536, 65536, 65536, 3392]);
    let joined: Vec<u8> = net(&mut k).sent.iter().flat_map(|s| s.1.clone()).collect();
    assert_eq!(joined, data);
    // The second chunk is taken in part: the call returns what went.
    net(&mut k).sent.clear();
    net(&mut k).send_results.extend([Ok(65536), Ok(100)]);
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, 0x100_0000, 200_000, 0, 0, 0]), Ok(65636));
    // An error after some bytes: the count; before any: the error.
    net(&mut k).send_results.extend([Ok(65536), Err(Errno::EAGAIN)]);
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, 0x100_0000, 200_000, 0, 0, 0]), Ok(65536));
    net(&mut k).send_results.push_back(Err(Errno::EAGAIN));
    assert_eq!(sys(&mut k, nr::SENDTO, &[fd, 0x100_0000, 10, 0, 0, 0]), Err(Errno::EAGAIN));
}

/// Store a msghdr at `at` with `iovs` (stored at `at + 0x100`), a name
/// buffer and a control buffer.
fn msghdr(k: &mut FakeKernel, at: u64, iovs: &[Iovec], name: (u64, u32), control: (u64, u64)) {
    let iov_at = at + 0x100;
    for (i, v) in iovs.iter().enumerate() {
        k.poke(iov_at + i as u64 * 16, &v.to_bytes());
    }
    let m = MsgHdr {
        name: name.0,
        namelen: name.1,
        iov: iov_at,
        iovlen: iovs.len() as u64,
        control: control.0,
        controllen: control.1,
        flags: 0,
    };
    k.poke(at, &m.to_bytes());
}

fn cmsg(level: i32, ty: i32, data: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    abi::push_cmsg(&mut v, level, ty, data);
    v
}

#[test]
fn sendmsg_passes_rights_and_checked_credentials() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Unix, SockType::Stream) as u64;
    k.poke(S + 0x800, b"hello");
    k.poke(S + 0x900, b" world");
    let iovs = [Iovec { base: S + 0x800, len: 5 }, Iovec { base: S + 0x900, len: 6 }];
    let mut control = cmsg(abi::SOL_SOCKET, abi::SCM_RIGHTS, &[3, 0, 0, 0, 4, 0, 0, 0]);
    let me = Ucred { pid: k.pid, uid: k.creds().uid, gid: k.creds().egid };
    control.extend(cmsg(abi::SOL_SOCKET, abi::SCM_CREDENTIALS, &me.to_bytes()));
    k.poke(S + 0xa00, &control);
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xa00, control.len() as u64));
    assert_eq!(sys(&mut k, nr::SENDMSG, &[fd, S, 0]), Ok(11));
    let sent = net(&mut k).sent.pop().unwrap();
    assert_eq!(
        (sent.1.as_slice(), sent.4.as_slice(), sent.5),
        (b"hello world".as_slice(), [3, 4].as_slice(), Some(me))
    );
    // Someone else's process id needs privilege (unix(7)).
    let other = Ucred { pid: 1, ..me };
    let bad = cmsg(abi::SOL_SOCKET, abi::SCM_CREDENTIALS, &other.to_bytes());
    k.poke(S + 0xa00, &bad);
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xa00, bad.len() as u64));
    assert_eq!(sys(&mut k, nr::SENDMSG, &[fd, S, 0]), Err(Errno::EPERM));
    // Broken control messages.
    let mut short = cmsg(abi::SOL_SOCKET, abi::SCM_RIGHTS, &[3, 0, 0, 0]);
    short[0] = 8;
    k.poke(S + 0xa00, &short);
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xa00, short.len() as u64));
    assert_eq!(sys(&mut k, nr::SENDMSG, &[fd, S, 0]), Err(Errno::EINVAL));
    let mut twice = cmsg(abi::SOL_SOCKET, abi::SCM_RIGHTS, &[3, 0, 0, 0]);
    twice.extend(cmsg(abi::SOL_SOCKET, abi::SCM_RIGHTS, &[4, 0, 0, 0]));
    k.poke(S + 0xa00, &twice);
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xa00, twice.len() as u64));
    assert_eq!(sys(&mut k, nr::SENDMSG, &[fd, S, 0]), Err(Errno::EINVAL));
    let unknown = cmsg(abi::SOL_SOCKET, 99, &[0; 4]);
    k.poke(S + 0xa00, &unknown);
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xa00, unknown.len() as u64));
    assert_eq!(sys(&mut k, nr::SENDMSG, &[fd, S, 0]), Err(Errno::EINVAL));
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xa00, 1 << 20));
    assert_eq!(sys(&mut k, nr::SENDMSG, &[fd, S, 0]), Err(Errno::ENOBUFS));
}

#[test]
fn recvmsg_builds_control_messages_and_flags() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Unix, SockType::Datagram) as u64;
    let from = SockAddr::Unix(UnixAddr::Abstract(b"peer".to_vec()));
    let creds = Ucred { pid: 9, uid: 8, gid: 7 };
    let answer = RecvMsg {
        full_len: 12,
        from: Some(from.clone()),
        rights: vec![5, 6],
        creds: Some(creds),
        ..RecvMsg::default()
    };
    net(&mut k).recvs.push_back(Ok((b"twelve bytes".to_vec(), answer.clone())));
    // Two iovecs of 4 and 3 bytes: 7 bytes arrive, the datagram was 12.
    let iovs = [Iovec { base: S + 0x800, len: 4 }, Iovec { base: S + 0x900, len: 3 }];
    let room = abi::cmsg_space(8) + abi::cmsg_space(Ucred::SIZE);
    msghdr(&mut k, S, &iovs, (S + 0xa00, 110), (S + 0xb00, room as u64));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, u64::from(abi::MSG_CMSG_CLOEXEC)]), Ok(7));
    assert_eq!((k.peek(S + 0x800, 4), k.peek(S + 0x900, 3)), (b"twel".to_vec(), b"ve ".to_vec()));
    let req = net(&mut k).recv_reqs[0].2;
    assert!(req.cloexec && req.max_rights >= 2);
    let m = MsgHdr::from_bytes(&k.peek(S, MsgHdr::SIZE).try_into().unwrap());
    assert_eq!(m.flags, abi::MSG_TRUNC);
    assert_eq!(m.namelen as usize, from.encode().len());
    assert_eq!(k.peek(S + 0xa00, m.namelen as usize), from.encode());
    let control = k.peek(S + 0xb00, m.controllen as usize);
    let msgs = abi::parse_cmsgs(&control).unwrap();
    assert_eq!(msgs.len(), 2);
    assert_eq!((msgs[0].level, msgs[0].ty, msgs[0].data.as_slice()), (1, 1, [5, 0, 0, 0, 6, 0, 0, 0].as_slice()));
    assert_eq!((msgs[1].ty, Ucred::from_bytes(&msgs[1].data)), (abi::SCM_CREDENTIALS, creds));
    // MSG_TRUNC: the datagram's real length.
    net(&mut k).recvs.push_back(Ok((b"twelve bytes".to_vec(), RecvMsg::default())));
    msghdr(&mut k, S, &iovs, (0, 0), (0, 0));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, u64::from(abi::MSG_TRUNC)]), Ok(12));
    // Room for one descriptor only: the rest are closed, MSG_CTRUNC.
    net(&mut k).recvs.push_back(Ok((b"x".to_vec(), RecvMsg { rights: vec![5, 6, 7], ..RecvMsg::default() })));
    msghdr(&mut k, S, &iovs, (0, 0), (S + 0xb00, abi::cmsg_len(4) as u64));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, 0]), Ok(1));
    assert_eq!(net(&mut k).recv_reqs.last().unwrap().2.max_rights, 1);
    let m = MsgHdr::from_bytes(&k.peek(S, MsgHdr::SIZE).try_into().unwrap());
    assert_eq!((m.flags, m.controllen as usize), (abi::MSG_CTRUNC, abi::cmsg_len(4)));
    // No control buffer at all: no descriptors asked for.
    net(&mut k).recvs.push_back(Ok((b"x".to_vec(), RecvMsg::default())));
    msghdr(&mut k, S, &iovs, (0, 0), (0, 64));
    sys(&mut k, nr::RECVMSG, &[fd, S, 0]).unwrap();
    assert_eq!(net(&mut k).recv_reqs.last().unwrap().2.max_rights, 0);
    // Flags.
    // recvmmsg's spec sheet: MSG_OOB on a Unix datagram socket is
    // EOPNOTSUPP; a Unix socket has no error queue, MSG_ERRQUEUE reads the
    // normal one. Elsewhere EINVAL and EAGAIN (nothing is queued there).
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, u64::from(abi::MSG_OOB)]), Err(Errno::EOPNOTSUPP));
    net(&mut k).recvs.push_back(Ok((b"e".to_vec(), RecvMsg::default())));
    msghdr(&mut k, S, &iovs, (0, 0), (0, 0));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, u64::from(abi::MSG_ERRQUEUE)]), Ok(1));
    let tcp = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    assert_eq!(sys(&mut k, nr::RECVMSG, &[tcp, S, u64::from(abi::MSG_OOB)]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[tcp, S, u64::from(abi::MSG_ERRQUEUE)]), Err(Errno::EAGAIN));
}

#[test]
fn recvfrom_reports_the_sender_or_no_address() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    net(&mut k).recvs.push_back(Ok((b"abc".to_vec(), RecvMsg::default())));
    k.poke(S + 0x100, &16u32.to_le_bytes());
    let flags = u64::from(abi::MSG_PEEK | abi::MSG_WAITALL);
    assert_eq!(sys(&mut k, nr::RECVFROM, &[fd, S, 100, flags, S + 0x200, S + 0x100]), Ok(3));
    assert_eq!(k.peek_u32(S + 0x100), 0, "a stream gives no address");
    let req = net(&mut k).recv_reqs[0];
    assert!(req.2.peek && req.2.waitall && !req.2.dontwait && req.2.max_rights == 0);
    assert_eq!(req.1, 100);
    let from = SockAddr::Inet(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 9));
    net(&mut k).recvs.push_back(Ok((b"d".to_vec(), RecvMsg { from: Some(from.clone()), ..RecvMsg::default() })));
    k.poke(S + 0x100, &16u32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::RECVFROM, &[fd, S, 100, 0, S + 0x200, S + 0x100]), Ok(1));
    assert_eq!(k.peek(S + 0x200, 16), from.encode());
    // Interrupted with a receive timeout: EINTR (by a signal that acts),
    // else a restart.
    net(&mut k).recvs.push_back(Err(Errno::EINTR));
    let args = Args([fd, S, 10, 0, 0, 0]);
    assert_eq!(socket::recvfrom(&mut k, &args), Err(Errno::RESTART_IF_SA_RESTART));
    net(&mut k).timed = true;
    pend(&mut k, SIGUSR1);
    net(&mut k).recvs.push_back(Err(Errno::EINTR));
    assert_eq!(socket::recvfrom(&mut k, &args), Err(Errno::EINTR));
}

#[test]
fn mmsg_calls_count_messages() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    k.poke(S + 0x800, b"0123456789");
    for i in 0..3u64 {
        let iov = [Iovec { base: S + 0x800, len: 3 + i }];
        msghdr(&mut k, S + i * 0x200, &iov, (0, 0), (0, 0));
    }
    // msghdrs are 64 bytes apart in an mmsghdr array: copy them there.
    for i in 0..3u64 {
        let m = k.peek(S + i * 0x200, MsgHdr::SIZE);
        k.poke(S + 0x1000 + i * abi::MMSGHDR_SIZE, &m);
    }
    assert_eq!(sys(&mut k, nr::SENDMMSG, &[fd, S + 0x1000, 3, 0]), Ok(3));
    let lens: Vec<usize> = net(&mut k).sent.iter().map(|s| s.1.len()).collect();
    assert_eq!(lens, [3, 4, 5]);
    assert_eq!(k.peek_u32(S + 0x1000 + abi::MMSGHDR_SIZE + 56), 4, "msg_len");
    // The second send fails: one message went.
    net(&mut k).send_results.extend([Ok(3), Err(Errno::EAGAIN)]);
    assert_eq!(sys(&mut k, nr::SENDMMSG, &[fd, S + 0x1000, 3, 0]), Ok(1));
    // Receives: MSG_WAITFORONE makes the later ones non-blocking.
    net(&mut k).recvs.extend([Ok((b"a".to_vec(), RecvMsg::default())), Ok((b"bc".to_vec(), RecvMsg::default()))]);
    let flags = u64::from(abi::MSG_WAITFORONE);
    assert_eq!(sys(&mut k, nr::RECVMMSG, &[fd, S + 0x1000, 3, flags, 0]), Ok(2));
    let reqs = &net(&mut k).recv_reqs;
    assert_eq!(reqs.iter().map(|r| r.2.dontwait).collect::<Vec<_>>(), [false, true, true]);
    assert_eq!(k.peek_u32(S + 0x1000 + abi::MMSGHDR_SIZE + 56), 2);
    assert_eq!(sys(&mut k, nr::RECVMMSG, &[fd, S + 0x1000, 3, 0, 0]), Err(Errno::EAGAIN));
}

#[test]
fn options_are_decoded_and_encoded() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    let set = |k: &mut FakeKernel, level: i32, name: i32, val: &[u8]| {
        k.poke(S, val);
        sys(k, nr::SETSOCKOPT, &[fd, level as u64, name as u64, S, val.len() as u64])
    };
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_REUSEADDR, &1i32.to_le_bytes()), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_SNDBUF, &5000i32.to_le_bytes()), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_RCVBUF, &10i32.to_le_bytes()), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_RCVBUF, &(1i32 << 30).to_le_bytes()), Ok(0));
    let mut tv = [0u8; 16];
    tv[..8].copy_from_slice(&2i64.to_le_bytes());
    tv[8..].copy_from_slice(&500_000i64.to_le_bytes());
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_RCVTIMEO, &tv), Ok(0));
    tv[8..].copy_from_slice(&1_000_000i64.to_le_bytes());
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_SNDTIMEO, &tv), Err(Errno::EDOM));
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_LINGER, &[1, 0, 0, 0, 0, 0, 0, 0]), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_TCP, abi::TCP_KEEPIDLE, &0i32.to_le_bytes()), Err(Errno::EINVAL));
    assert_eq!(set(&mut k, abi::SOL_TCP, abi::TCP_NODELAY, &[1, 0]), Err(Errno::EINVAL), "short optlen");
    assert_eq!(set(&mut k, abi::SOL_IP, abi::IP_TTL, &(-1i32).to_le_bytes()), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_IP, abi::IP_TTL, &256i32.to_le_bytes()), Err(Errno::EINVAL));
    assert_eq!(set(&mut k, abi::SOL_IP, abi::IP_RECVERR, &1i32.to_le_bytes()), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_IPV6, abi::IPV6_RECVERR, &0i32.to_le_bytes()), Ok(0));
    assert_eq!(set(&mut k, abi::SOL_SOCKET, abi::SO_TYPE, &1i32.to_le_bytes()), Err(Errno::ENOPROTOOPT));
    assert_eq!(set(&mut k, 999, 1, &1i32.to_le_bytes()), Err(Errno::ENOPROTOOPT));
    let opts = net(&mut k).socks[&(fd as i32)].opts.clone();
    assert_eq!(
        opts,
        [
            SockOpt::ReuseAddr(true),
            SockOpt::SndBuf(10_000),
            SockOpt::RcvBuf(256),
            SockOpt::RcvBuf(2 * 212_992),
            SockOpt::RcvTimeout(2_500_000_000),
            SockOpt::Linger(Some(0)),
            SockOpt::IpTtl(None),
            SockOpt::IpRecvErr(true),
            SockOpt::Ipv6RecvErr(false),
        ]
    );
    // getsockopt: values, truncation, lengths.
    let get = |k: &mut FakeKernel, level: i32, name: i32, room: i32| {
        k.poke(S + 0x100, &room.to_le_bytes());
        let r = sys(k, nr::GETSOCKOPT, &[fd, level as u64, name as u64, S, S + 0x100]);
        r.map(|_| k.peek(S, k.peek_u32(S + 0x100) as usize))
    };
    assert_eq!(get(&mut k, abi::SOL_SOCKET, abi::SO_TYPE, 4), Ok(1i32.to_le_bytes().to_vec()));
    assert_eq!(get(&mut k, abi::SOL_SOCKET, abi::SO_DOMAIN, 4), Ok(2i32.to_le_bytes().to_vec()));
    let tv = get(&mut k, abi::SOL_SOCKET, abi::SO_RCVTIMEO, 16).unwrap();
    assert_eq!(
        (i64::from_le_bytes(tv[..8].try_into().unwrap()), i64::from_le_bytes(tv[8..].try_into().unwrap())),
        (1, 500_000)
    );
    assert_eq!(get(&mut k, abi::SOL_SOCKET, abi::SO_LINGER, 8), Ok([1, 0, 0, 0, 7, 0, 0, 0].to_vec()));
    assert_eq!(
        get(&mut k, abi::SOL_SOCKET, abi::SO_PEERCRED, 12).map(|b| Ucred::from_bytes(&b)),
        Ok(Ucred { pid: 5, uid: 6, gid: 7 })
    );
    assert_eq!(get(&mut k, abi::SOL_SOCKET, abi::SO_SNDBUF, 2), Ok(vec![0, 16]), "cut to the caller's room");
    assert_eq!(get(&mut k, abi::SOL_SOCKET, abi::SO_SNDBUF, -1), Err(Errno::EINVAL));
    assert_eq!(get(&mut k, abi::SOL_SOCKET, 999, 4), Err(Errno::ENOPROTOOPT));
    assert_eq!(get(&mut k, abi::SOL_SOCKET, abi::SO_ERROR, 4), Err(Errno::ENOPROTOOPT), "the fake has no SO_ERROR");
    k.poke(S + 0x100, &4i32.to_le_bytes());
    assert_eq!(sys(&mut k, nr::GETSOCKOPT, &[1, 1, 3, S, S + 0x100]), Err(Errno::ENOTSOCK));
    assert_eq!(socket::encode_value(OptValue::Timeval(1_000)), [0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn ioctl_siocoutq_on_sockets() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream);
    net(&mut k).outq = 1234;
    // The fake kernel's fstat needs a file for the descriptor.
    k.files.insert(fd, File::new(super::fake::Kind::Pipe));
    assert_eq!(sys(&mut k, nr::IOCTL, &[fd as u64, u64::from(abi::SIOCOUTQ), S]), Ok(0));
    assert_eq!(k.peek_u32(S), 1234);
}

/// The netns_loopback sheet's ownership rules: raw sockets need
/// `CAP_NET_RAW` in the user namespace that owns the caller's network
/// namespace.
#[test]
fn raw_sockets_need_cap_net_raw_in_the_namespaces_owner() {
    use crate::task::clone::{CLONE_NEWNET, CLONE_NEWUSER};
    let raw_inet = |k: &mut FakeKernel| sys(k, nr::SOCKET, &[2, 3, 255]);
    let raw_packet = |k: &mut FakeKernel| sys(k, nr::SOCKET, &[17, 3, 0x300]);
    // No capabilities in the initial user namespace.
    let mut k = kernel();
    assert_eq!(raw_inet(&mut k), Err(Errno::EPERM));
    assert_eq!(raw_packet(&mut k), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::SOCKET, &[17, 2, 0]), Err(Errno::EPERM), "a packet socket of any type");
    assert!(sys(&mut k, nr::SOCKET, &[2, 2, 0]).is_ok(), "datagram sockets work");
    assert!(sys(&mut k, nr::SOCKET, &[16, 3, 0]).is_ok(), "and rtnetlink");
    assert_eq!(sys(&mut k, nr::SOCKET, &[2, 3, 0]), Err(Errno::EPROTONOSUPPORT), "the kind is checked first");
    // A new user namespace alone: the network namespace keeps its owner.
    assert_eq!(sys(&mut k, nr::UNSHARE, &[CLONE_NEWUSER]), Ok(0));
    assert!(k.creds().capable(crate::cred::cap::NET_RAW), "root in the new user namespace");
    assert_eq!(raw_inet(&mut k), Err(Errno::EPERM));
    assert_eq!(raw_packet(&mut k), Err(Errno::EPERM));
    // With a network namespace of its own: owned by the new user namespace.
    let mut k = kernel();
    assert_eq!(sys(&mut k, nr::UNSHARE, &[CLONE_NEWUSER | CLONE_NEWNET]), Ok(0));
    assert!(raw_inet(&mut k).is_ok());
    assert!(raw_packet(&mut k).is_ok());
    // The capability in the initial namespace.
    let mut k = kernel();
    k.creds_mut().caps = crate::cred::Caps::ROOT;
    assert!(raw_inet(&mut k).is_ok());
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(raw_inet(&mut k), Err(Errno::EPERM));
}

/// The interface requests on a socket of a new network namespace
/// (netdevice(7), the netns_loopback sheet).
#[test]
fn interface_ioctls_act_on_the_sockets_network_namespace() {
    use crate::net::dev::*;
    use crate::task::clone::{CLONE_NEWNET, CLONE_NEWUSER};
    let mut k = kernel();
    assert_eq!(sys(&mut k, nr::UNSHARE, &[CLONE_NEWUSER | CLONE_NEWNET]), Ok(0));
    let ns = k.me().ns.net.clone();
    assert!(ns.is_some());
    net(&mut k).netns = ns.clone();
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    let ifreq = |k: &mut FakeKernel, request: u32, name: &[u8], value: u16| {
        let mut r = [0u8; 40];
        r[..name.len()].copy_from_slice(name);
        r[16..18].copy_from_slice(&value.to_le_bytes());
        k.poke(S, &r);
        sys(k, nr::IOCTL, &[fd, u64::from(request), S]).map(|_| k.peek(S, 40))
    };
    let flags = |k: &mut FakeKernel| ifreq(k, SIOCGIFFLAGS, b"lo", 0).map(|r| u16::from_le_bytes([r[16], r[17]]));
    assert_eq!(flags(&mut k), Ok(0x8), "down");
    assert_eq!(ifreq(&mut k, SIOCGIFADDR, b"lo", 0), Err(Errno::EADDRNOTAVAIL));
    assert_eq!(ifreq(&mut k, SIOCGIFFLAGS, b"eth0", 0), Err(Errno::ENODEV));
    assert_eq!(ifreq(&mut k, SIOCGIFMTU, b"lo", 0).map(|r| r[16..20].to_vec()), Ok(65536i32.to_le_bytes().to_vec()));
    // SIOCGIFCONF: nothing while lo is down; ifc_len comes back as 0.
    let conf = |k: &mut FakeKernel, len: i32, buf: u64| {
        let mut c = [0u8; 16];
        c[0..4].copy_from_slice(&len.to_le_bytes());
        c[8..16].copy_from_slice(&buf.to_le_bytes());
        k.poke(S + 0x100, &c);
        sys(k, nr::IOCTL, &[fd, u64::from(SIOCGIFCONF), S + 0x100]).map(|_| k.peek_u32(S + 0x100))
    };
    assert_eq!(conf(&mut k, 400, S + 0x200), Ok(0));
    assert_eq!(conf(&mut k, -1, S + 0x200), Err(Errno::EINVAL));
    // Up, as bubblewrap does it.
    assert!(ifreq(&mut k, SIOCSIFFLAGS, b"lo", 0x49).is_ok());
    assert!(ns.as_ref().unwrap().with_devices(|d| d.lo_up()));
    assert_eq!(flags(&mut k), Ok(0x49));
    assert_eq!(ifreq(&mut k, SIOCGIFADDR, b"lo", 0).map(|r| r[20..24].to_vec()), Ok(vec![127, 0, 0, 1]));
    assert_eq!(conf(&mut k, 400, S + 0x200), Ok(40));
    assert_eq!(k.peek(S + 0x200, 3), [b'l', b'o', 0]);
    assert_eq!(k.peek(S + 0x200 + 20, 4), [127, 0, 0, 1]);
    assert_eq!(conf(&mut k, 0, 0), Ok(40), "a null buffer asks for the size");
    assert_eq!(ifreq(&mut k, SIOCSIFFLAGS, b"eth0", 1), Err(Errno::ENODEV));
    assert_eq!(ifreq(&mut k, SIOCSIFMTU, b"lo", 1500), Err(Errno::EOPNOTSUPP));
    // Down again.
    assert!(ifreq(&mut k, SIOCSIFFLAGS, b"lo", 0).is_ok());
    assert_eq!(flags(&mut k), Ok(0x8));
    assert_eq!(sys(&mut k, nr::IOCTL, &[fd, u64::from(SIOCGIFFLAGS), UNMAPPED]), Err(Errno::EFAULT));

    // Without CAP_NET_ADMIN in the owner of the socket's namespace: the
    // getters work, the setters are EPERM, before the device is looked up.
    k.creds_mut().caps = crate::cred::Caps::NONE;
    assert_eq!(flags(&mut k), Ok(0x8));
    assert_eq!(ifreq(&mut k, SIOCSIFFLAGS, b"lo", 0x49), Err(Errno::EPERM));
    assert_eq!(ifreq(&mut k, SIOCSIFFLAGS, b"eth0", 0x49), Err(Errno::EPERM));
    assert!(!ns.as_ref().unwrap().with_devices(|d| d.lo_up()));

    // A socket of the initial namespace held by root of a new user
    // namespace: its devices are not that root's to change.
    let mut k = kernel();
    assert_eq!(sys(&mut k, nr::UNSHARE, &[CLONE_NEWUSER]), Ok(0));
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    let mut r = [0u8; 40];
    r[..2].copy_from_slice(b"lo");
    k.poke(S, &r);
    assert_eq!(sys(&mut k, nr::IOCTL, &[fd, u64::from(SIOCSIFFLAGS), S]), Err(Errno::EPERM));
    assert_eq!(sys(&mut k, nr::IOCTL, &[fd, u64::from(SIOCGIFFLAGS), S]), Ok(0));
    let got = k.peek(S + 16, 2);
    assert_eq!(u16::from_le_bytes([got[0], got[1]]), 0x49, "the initial lo is up");
    // Not a socket: the request is not an interface request to answer.
    assert_ne!(sys(&mut k, nr::IOCTL, &[1, u64::from(SIOCGIFFLAGS), S]), Ok(0));
}

// ---- IP, IPv6 and TCP options (ip(7), ipv6(7), tcp(7)) ----------------------

/// setsockopt on `fd` with `val` as the argument.
fn set_opt(k: &mut FakeKernel, fd: u64, level: i32, name: i32, val: &[u8]) -> Result<u64, Errno> {
    k.poke(S, val);
    sys(k, nr::SETSOCKOPT, &[fd, level as u64, name as u64, S, val.len() as u64])
}

/// getsockopt on `fd` with `room` bytes of room: the bytes it stored.
fn get_opt(k: &mut FakeKernel, fd: u64, level: i32, name: i32, room: i32) -> Result<Vec<u8>, Errno> {
    k.poke(S + 0x100, &room.to_le_bytes());
    sys(k, nr::GETSOCKOPT, &[fd, level as u64, name as u64, S, S + 0x100])?;
    Ok(k.peek(S, k.peek_u32(S + 0x100) as usize))
}

fn last_opt(k: &mut FakeKernel, fd: u64) -> SockOpt {
    *net(k).socks[&(fd as i32)].opts.last().expect("an option was set")
}

#[test]
fn ip_flag_and_range_options_are_decoded() {
    use crate::net::StoredOpt::*;
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    let int = |v: i32| v.to_le_bytes();
    // Flags: IP_PKTINFO, IP_RECVTOS, IP_RECVTTL, IP_RECVOPTS, IP_RETOPTS, IP_FREEBIND,
    // IP_TRANSPARENT, IP_BIND_ADDRESS_NO_PORT: any non-zero value is on.
    for (name, opt) in [
        (abi::IP_PKTINFO, IpPktInfo),
        (abi::IP_RECVTOS, IpRecvTos),
        (abi::IP_RECVTTL, IpRecvTtl),
        (abi::IP_RECVOPTS, IpRecvOpts),
        (abi::IP_RETOPTS, IpRetOpts),
        (abi::IP_FREEBIND, IpFreebind),
        (abi::IP_TRANSPARENT, IpTransparent),
        (abi::IP_BIND_ADDRESS_NO_PORT, IpBindAddressNoPort),
        (abi::IP_MULTICAST_LOOP, IpMulticastLoop),
    ] {
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, name, &int(1)), Ok(0), "{opt:?}");
        assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(opt, 1));
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, name, &int(0)), Ok(0));
        assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(opt, 0));
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, name, &int(-3)), Ok(0));
        assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(opt, 1), "non-zero is on");
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, name, &[]), Err(Errno::EINVAL), "{opt:?}: no argument");
    }
    // ip(7): the IP_* int options also take an unsigned char.
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_TTL, &[200]), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(IpMulticastTtl, 200));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_TTL, &int(0)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(IpMulticastTtl, 0));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_TTL, &int(256)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_TTL, &int(-1)), Err(Errno::EINVAL));
    // IP_MTU_DISCOVER: IP_PMTUDISC_DONT (0) to IP_PMTUDISC_OMIT (5).
    for v in 0..=5 {
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MTU_DISCOVER, &int(v)), Ok(0));
        assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(IpMtuDiscover, v));
    }
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MTU_DISCOVER, &int(6)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MTU_DISCOVER, &int(-1)), Err(Errno::EINVAL));
    // IP_MTU is only read.
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MTU, &int(1500)), Err(Errno::ENOPROTOOPT));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, 200, &int(1)), Err(Errno::ENOPROTOOPT));
}

#[test]
fn ipv6_options_are_decoded() {
    use crate::net::StoredOpt::*;
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet6, SockType::Datagram) as u64;
    let int = |v: i32| v.to_le_bytes();
    for (name, opt) in [
        (abi::IPV6_RECVPKTINFO, Ipv6RecvPktInfo),
        (abi::IPV6_RECVTCLASS, Ipv6RecvTclass),
        (abi::IPV6_RECVHOPLIMIT, Ipv6RecvHopLimit),
        (abi::IPV6_DONTFRAG, Ipv6DontFrag),
    ] {
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, name, &int(1)), Ok(0), "{opt:?}");
        assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(opt, 1));
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, name, &int(0)), Ok(0));
        assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(opt, 0));
        // ipv6(7): an int is required.
        assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, name, &[1]), Err(Errno::EINVAL), "{opt:?}: one byte");
    }
    // IPV6_TCLASS: -1 is the default (0), 0..=255 as is.
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_TCLASS, &int(0xb8)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Ipv6Tclass(0xb8));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_TCLASS, &int(-1)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Ipv6Tclass(0));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_TCLASS, &int(256)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_TCLASS, &int(-2)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_TCLASS, &[1, 0]), Err(Errno::EINVAL));
    // Multicast: interface index, hop limit (-1: 1), loopback (0 or 1).
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_IF, &int(2)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(Ipv6MulticastIf, 2));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_IF, &int(-5)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_HOPS, &int(-1)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(Ipv6MulticastHops, 1));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_HOPS, &int(32)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(Ipv6MulticastHops, 32));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_HOPS, &int(256)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_LOOP, &int(0)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(Ipv6MulticastLoop, 0));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MULTICAST_LOOP, &int(2)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MTU_DISCOVER, &int(2)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(Ipv6MtuDiscover, 2));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_MTU_DISCOVER, &int(9)), Err(Errno::EINVAL));
    // IPV6_PKTINFO as an option: exactly a struct in6_pktinfo.
    let mut pi = [0u8; 20];
    pi[..16].copy_from_slice(&Ipv6Addr::LOCALHOST.octets());
    pi[16..].copy_from_slice(&3u32.to_le_bytes());
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_PKTINFO, &pi), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Ipv6PktInfo { addr: Ipv6Addr::LOCALHOST, ifindex: 3 });
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_PKTINFO, &pi[..19]), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_PKTINFO, &int(1)), Err(Errno::EINVAL));
}

#[test]
fn tcp_options_are_decoded() {
    use crate::net::StoredOpt::*;
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Stream) as u64;
    let int = |v: i32| v.to_le_bytes();
    let t = abi::SOL_TCP;
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_FASTOPEN, &int(16)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpFastopen, 16));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_FASTOPEN, &int(-1)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_FASTOPEN_CONNECT, &int(1)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpFastopenConnect, 1));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_FASTOPEN_CONNECT, &int(2)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_QUICKACK, &int(0)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpQuickack, 0));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_CORK, &int(1)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpCork, 1));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_DEFER_ACCEPT, &int(5)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpDeferAccept, 5));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_DEFER_ACCEPT, &int(-1)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpDeferAccept, 0));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_USER_TIMEOUT, &int(10_000)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpUserTimeout, 10_000));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_USER_TIMEOUT, &int(-1)), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_NOTSENT_LOWAT, &int(16384)), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::Stored(TcpNotsentLowat, 16384));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_NOTSENT_LOWAT, &[1, 2]), Err(Errno::EINVAL), "short");
    // TCP_CONGESTION: a name; an unknown one is ENOENT.
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_CONGESTION, b"reno"), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::TcpCongestion(crate::net::Congestion::Reno));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_CONGESTION, b"cubic\0"), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::TcpCongestion(crate::net::Congestion::Cubic));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_CONGESTION, b"nonesuch"), Err(Errno::ENOENT));
    assert_eq!(set_opt(&mut k, fd, t, abi::TCP_CONGESTION, b""), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, t, 99, &int(1)), Err(Errno::ENOPROTOOPT));
}

#[test]
fn multicast_options_take_the_structures_of_the_man_pages() {
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    let group = [224, 0, 0, 251];
    let ip = |b: [u8; 4]| Ipv4Addr::from(b);
    let m = |g: [u8; 4], ifindex: u32, iface: [u8; 4]| Membership { group: ip(g).into(), ifindex, iface: ip(iface) };
    // ip_mreq: 8 bytes, group and interface address.
    let mut mreq = group.to_vec();
    mreq.extend_from_slice(&[10, 0, 2, 15]);
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_ADD_MEMBERSHIP, &mreq), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::AddMembership(m(group, 0, [10, 0, 2, 15])));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_DROP_MEMBERSHIP, &mreq), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::DropMembership(m(group, 0, [10, 0, 2, 15])));
    // ip_mreqn: 12 bytes, with the interface index.
    let mut mreqn = group.to_vec();
    mreqn.extend_from_slice(&[0; 4]);
    mreqn.extend_from_slice(&2i32.to_le_bytes());
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_ADD_MEMBERSHIP, &mreqn), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::AddMembership(m(group, 2, [0; 4])));
    // Short arguments, groups that are not multicast addresses.
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_ADD_MEMBERSHIP, &mreq[..7]), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_ADD_MEMBERSHIP, &[]), Err(Errno::EINVAL));
    let mut bad = vec![10, 1, 2, 3];
    bad.extend_from_slice(&[0; 4]);
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_ADD_MEMBERSHIP, &bad), Err(Errno::EINVAL));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_DROP_MEMBERSHIP, &bad), Err(Errno::EINVAL));
    // IP_MULTICAST_IF: an in_addr, or an ip_mreqn (its imr_address and imr_ifindex).
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_IF, &[10, 0, 2, 15]), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::IpMulticastIf { addr: ip([10, 0, 2, 15]), ifindex: 0 });
    let mut mreqn = vec![0; 4];
    mreqn.extend_from_slice(&[192, 168, 1, 7]);
    mreqn.extend_from_slice(&3i32.to_le_bytes());
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_IF, &mreqn), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::IpMulticastIf { addr: ip([192, 168, 1, 7]), ifindex: 3 });
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_IF, &[1, 2, 3]), Err(Errno::EINVAL));
    // IPv6: struct ipv6_mreq, 20 bytes.
    let mut mreq6 = "ff02::fb".parse::<Ipv6Addr>().unwrap().octets().to_vec();
    mreq6.extend_from_slice(&2u32.to_le_bytes());
    let g6 =
        Membership { group: "ff02::fb".parse::<Ipv6Addr>().unwrap().into(), ifindex: 2, iface: Ipv4Addr::UNSPECIFIED };
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_ADD_MEMBERSHIP, &mreq6), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::AddMembership(g6));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_DROP_MEMBERSHIP, &mreq6), Ok(0));
    assert_eq!(last_opt(&mut k, fd), SockOpt::DropMembership(g6));
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_ADD_MEMBERSHIP, &mreq6[..19]), Err(Errno::EINVAL));
    let mut uni = Ipv6Addr::LOCALHOST.octets().to_vec();
    uni.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(set_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_ADD_MEMBERSHIP, &uni), Err(Errno::EINVAL));
    // Memberships cannot be read back.
    assert_eq!(get_opt(&mut k, fd, abi::SOL_IP, abi::IP_ADD_MEMBERSHIP, 12), Err(Errno::ENOPROTOOPT));
}

#[test]
fn ip_options_are_encoded_by_getsockopt() {
    use crate::net::StoredOpt::*;
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    let ints = [
        (abi::SOL_IP, abi::IP_PKTINFO, IpPktInfo),
        (abi::SOL_IP, abi::IP_MULTICAST_TTL, IpMulticastTtl),
        (abi::SOL_IP, abi::IP_MULTICAST_LOOP, IpMulticastLoop),
        (abi::SOL_IP, abi::IP_MTU_DISCOVER, IpMtuDiscover),
        (abi::SOL_IPV6, abi::IPV6_RECVTCLASS, Ipv6RecvTclass),
        (abi::SOL_TCP, abi::TCP_FASTOPEN, TcpFastopen),
        (abi::SOL_TCP, abi::TCP_USER_TIMEOUT, TcpUserTimeout),
    ];
    for (level, name, o) in ints {
        // The fake answers with the default; an int, 4 bytes.
        assert_eq!(get_opt(&mut k, fd, level, name, 4), Ok(o.default_value().to_le_bytes().to_vec()), "{o:?}");
    }
    // IP_MULTICAST_IF: an in_addr; TCP_CONGESTION: the name and its NUL
    // (cut to the room the caller gave); IPV6_PKTINFO: 20 bytes.
    assert_eq!(get_opt(&mut k, fd, abi::SOL_IP, abi::IP_MULTICAST_IF, 4), Ok(vec![10, 0, 2, 15]));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_TCP, abi::TCP_CONGESTION, 16), Ok(b"cubic\0".to_vec()));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_TCP, abi::TCP_CONGESTION, 3), Ok(b"cub".to_vec()));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_PKTINFO, 20).map(|b| b.len()), Ok(20));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_IPV6, abi::IPV6_TCLASS, 4), Ok(vec![0; 4]));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_IP, abi::IP_MTU, 4), Ok(1500i32.to_le_bytes().to_vec()));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_IP, 200, 4), Err(Errno::ENOPROTOOPT));
    assert_eq!(get_opt(&mut k, fd, abi::SOL_TCP, 99, 4), Err(Errno::ENOPROTOOPT));
    assert_eq!(socket::encode_value(OptValue::bytes(&[1, 2, 3])), [1, 2, 3]);
}

#[test]
fn recvmsg_returns_the_ip_control_messages_the_socket_asked_for() {
    use crate::net::ipopt::{PktInfo, RecvAnc};
    let mut k = kernel();
    let fd = net(&mut k).add(Family::Inet, SockType::Datagram) as u64;
    let from = SockAddr::Inet(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 4000));
    let dst = core::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
    let anc = RecvAnc { v6: false, pktinfo: Some(PktInfo { ifindex: 1, dst }), tos: Some(0xb8), ttl: Some(64) };
    let answer = RecvMsg { from: Some(from.clone()), anc, ..RecvMsg::default() };
    net(&mut k).recvs.push_back(Ok((b"ping".to_vec(), answer.clone())));
    let iovs = [Iovec { base: S + 0x800, len: 16 }];
    let room = abi::cmsg_space(12) + abi::cmsg_space(4) + abi::cmsg_space(1);
    msghdr(&mut k, S, &iovs, (S + 0xa00, 110), (S + 0xb00, room as u64));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, 0]), Ok(4));
    let m = MsgHdr::from_bytes(&k.peek(S, MsgHdr::SIZE).try_into().unwrap());
    assert_eq!(m.flags, 0, "everything fit");
    let msgs = abi::parse_cmsgs(&k.peek(S + 0xb00, m.controllen as usize)).unwrap();
    let got: Vec<(i32, i32, Vec<u8>)> = msgs.into_iter().map(|c| (c.level, c.ty, c.data)).collect();
    assert_eq!(
        got,
        [
            (abi::IPPROTO_IP, abi::IP_PKTINFO, vec![1, 0, 0, 0, 127, 0, 0, 1, 127, 0, 0, 1]),
            (abi::IPPROTO_IP, abi::IP_TTL, vec![64, 0, 0, 0]),
            (abi::IPPROTO_IP, abi::IP_TOS, vec![0xb8]),
        ]
    );
    assert_eq!(m.controllen as usize, room);
    // A control buffer that holds the first message only: MSG_CTRUNC.
    net(&mut k).recvs.push_back(Ok((b"ping".to_vec(), answer)));
    msghdr(&mut k, S, &iovs, (S + 0xa00, 110), (S + 0xb00, abi::cmsg_space(12) as u64));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, 0]), Ok(4));
    let m = MsgHdr::from_bytes(&k.peek(S, MsgHdr::SIZE).try_into().unwrap());
    assert_eq!((m.flags, m.controllen as usize), (abi::MSG_CTRUNC, abi::cmsg_space(12)));
    // IPv6: in6_pktinfo, hop limit and traffic class as ints, at level IPPROTO_IPV6.
    let fd6 = net(&mut k).add(Family::Inet6, SockType::Datagram) as u64;
    let six = Ipv6Addr::LOCALHOST;
    let anc =
        RecvAnc { v6: true, pktinfo: Some(PktInfo { ifindex: 2, dst: six.into() }), tos: Some(0x2e), ttl: Some(255) };
    let from6 = SockAddr::Inet6(SocketAddrV6::new(six, 4001, 0, 0));
    net(&mut k).recvs.push_back(Ok((b"p6".to_vec(), RecvMsg { from: Some(from6), anc, ..RecvMsg::default() })));
    msghdr(&mut k, S, &iovs, (S + 0xa00, 110), (S + 0xb00, 128));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd6, S, 0]), Ok(2));
    let m = MsgHdr::from_bytes(&k.peek(S, MsgHdr::SIZE).try_into().unwrap());
    let msgs = abi::parse_cmsgs(&k.peek(S + 0xb00, m.controllen as usize)).unwrap();
    let got: Vec<(i32, i32, usize)> = msgs.iter().map(|c| (c.level, c.ty, c.data.len())).collect();
    assert_eq!(got, [(41, abi::IPV6_PKTINFO, 20), (41, abi::IPV6_HOPLIMIT, 4), (41, abi::IPV6_TCLASS, 4)]);
    assert_eq!(msgs[1].data, 255i32.to_le_bytes());
    assert_eq!(msgs[2].data, 0x2ei32.to_le_bytes());
    let mut want = six.octets().to_vec();
    want.extend_from_slice(&2u32.to_le_bytes());
    assert_eq!(msgs[0].data, want);
    // No options, no messages.
    net(&mut k).recvs.push_back(Ok((b"x".to_vec(), RecvMsg::default())));
    msghdr(&mut k, S, &iovs, (S + 0xa00, 110), (S + 0xb00, 128));
    assert_eq!(sys(&mut k, nr::RECVMSG, &[fd, S, 0]), Ok(1));
    let m = MsgHdr::from_bytes(&k.peek(S, MsgHdr::SIZE).try_into().unwrap());
    assert_eq!((m.controllen, m.flags), (0, 0));
}
