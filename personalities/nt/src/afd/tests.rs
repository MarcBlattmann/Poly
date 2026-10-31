//! Host tests of `\Device\Afd` through the services, against the fake
//! kernel of `crate::tests` with sockets of a `poly-net` stack of their
//! own (loopback only): endpoints opened by name, the requests of
//! [`poly_winsock::afd`], TCP and UDP over 127.0.0.1 and `::1`, the
//! errors the personality decides, options, polling, overlapped requests
//! and `NtReadFile` / `NtWriteFile` on a socket.

extern crate std;

use super::*;
use crate::io::consts::*;
use crate::io::default_volumes;
use crate::services::Service;
use crate::tests::Fake;
use alloc::boxed::Box;
use alloc::string::ToString;
use core::result::Result;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use poly_net::{Config, Instant, MacAddr, NetDevice, RecvFlags, SocketHandle, Stack, TxError};
use poly_ntobj::OBJ_CASE_INSENSITIVE;
use poly_ntobj::status::*;
use poly_vfs::{Clock, Timespec, Tmpfs, Vfs};
use poly_winsock::consts::*;
use poly_winsock::{addr, afd, error as werr};
use std::sync::Mutex;

#[derive(Debug)]
struct FixedClock;

impl Clock for FixedClock {
    fn now(&self) -> Timespec {
        Timespec::new(1_000_000_000, 0)
    }
}

/// A network device that drops everything: the stack still loops back
/// its own addresses.
struct NoDev;

impl NetDevice for NoDev {
    fn mac(&self) -> MacAddr {
        MacAddr([2, 0, 0, 0, 0, 1])
    }
    fn mtu(&self) -> usize {
        1500
    }
    fn transmit(&mut self, _: &[u8]) -> Result<(), TxError> {
        Ok(())
    }
    fn receive(&mut self, _: &mut [u8]) -> Option<usize> {
        None
    }
}

/// The test's stack and clock.
#[derive(Debug)]
struct Net {
    stack: Mutex<Stack>,
    millis: AtomicU64,
}

impl Net {
    fn new() -> Arc<Net> {
        let stack = Stack::new(MacAddr([2, 0, 0, 0, 0, 1]), 1500, Config::default());
        Arc::new(Net { stack: Mutex::new(stack), millis: AtomicU64::new(1000) })
    }

    /// Run a stack call, then poll until loopback traffic settles.
    fn call<R>(&self, f: impl FnOnce(&mut Stack) -> R) -> R {
        let mut s = self.stack.lock().unwrap();
        let r = f(&mut s);
        for _ in 0..16 {
            let now = Instant::from_millis(self.millis.load(Ordering::SeqCst));
            s.poll(&mut NoDev, now);
            if !s.loopback_pending() {
                break;
            }
        }
        r
    }

    /// A blocking call: retry with the clock moving until it stops being
    /// `EAGAIN` (bounded: a test that would block for ever fails with it).
    fn block<R>(&self, wait: SockWait, mut f: impl FnMut(&mut Stack) -> Result<R, Errno>) -> Result<R, Errno> {
        let tries = if wait == SockWait::Now { 1 } else { 50 };
        for _ in 0..tries {
            match self.call(&mut f) {
                Err(Errno::EAGAIN) => {
                    self.millis.fetch_add(10, Ordering::SeqCst);
                }
                r => return r,
            }
        }
        Err(Errno::EAGAIN)
    }
}

#[derive(Debug)]
struct TestSock {
    net: Arc<Net>,
    h: SocketHandle,
    domain: Domain,
    ty: SockType,
    protocol: Protocol,
    released: Arc<AtomicBool>,
}

impl TestSock {
    fn new(net: &Arc<Net>, h: SocketHandle, domain: Domain, ty: SockType, protocol: Protocol) -> TestSock {
        TestSock { net: Arc::clone(net), h, domain, ty, protocol, released: Arc::new(AtomicBool::new(false)) }
    }
}

impl Drop for TestSock {
    fn drop(&mut self) {
        let h = self.h;
        self.net.call(|s| s.close(h)).ok();
    }
}

impl NtSocket for TestSock {
    fn domain(&self) -> Domain {
        self.domain
    }
    fn kind(&self) -> SockType {
        self.ty
    }
    fn protocol(&self) -> Protocol {
        self.protocol
    }
    fn bind(&self, a: SocketAddr) -> Result<(), Errno> {
        self.net.call(|s| s.bind(self.h, a))
    }
    fn listen(&self, backlog: usize) -> Result<(), Errno> {
        self.net.call(|s| s.listen(self.h, backlog))
    }
    fn accept(&self, wait: SockWait) -> Result<(Arc<dyn NtSocket>, SocketAddr), Errno> {
        let (h, peer) = self.net.block(wait, |s| s.accept(self.h))?;
        Ok((Arc::new(TestSock::new(&self.net, h, self.domain, self.ty, self.protocol)), peer))
    }
    fn connect(&self, a: SocketAddr, wait: SockWait) -> Result<(), Errno> {
        match self.net.call(|s| s.connect(self.h, a)) {
            Err(Errno::EINPROGRESS) if wait != SockWait::Now => {}
            r => return r,
        }
        self.net.block(wait, |s| {
            if let Some(e) = s.take_error(self.h)? {
                return Err(e);
            }
            match s.tcp_state(self.h)? {
                poly_net::TcpState::Established | poly_net::TcpState::CloseWait => Ok(()),
                poly_net::TcpState::Closed => Err(Errno::ECONNREFUSED),
                _ => Err(Errno::EAGAIN),
            }
        })
    }
    fn send(&self, data: &[u8], to: Option<SocketAddr>, wait: SockWait) -> Result<usize, Errno> {
        self.net.block(wait, |s| s.send_to(self.h, data, to))
    }
    fn recv(&self, buf: &mut [u8], peek: bool, wait: SockWait) -> Result<RecvInfo, Errno> {
        self.net.block(wait, |s| s.recv_from(self.h, buf, RecvFlags { peek }))
    }
    fn shutdown(&self, how: Shutdown) -> Result<(), Errno> {
        self.net.call(|s| s.shutdown(self.h, how))
    }
    fn local_addr(&self) -> Result<SocketAddr, Errno> {
        self.net.call(|s| s.local_addr(self.h))
    }
    fn peer_addr(&self) -> Result<SocketAddr, Errno> {
        self.net.call(|s| s.peer_addr(self.h))
    }
    fn readiness(&self) -> Readiness {
        self.net.call(|s| s.readiness(self.h)).unwrap_or_default()
    }
    fn take_error(&self) -> Option<Errno> {
        self.net.call(|s| s.take_error(self.h)).ok().flatten()
    }
    fn bytes_readable(&self) -> usize {
        self.net.call(|s| s.bytes_readable(self.h)).unwrap_or(0)
    }
    fn set_option(&self, opt: SockOption) -> Result<(), Errno> {
        self.net.call(|s| match opt {
            SockOption::ReuseAddr(v) => s.set_reuse_addr(self.h, v),
            SockOption::NoDelay(v) => s.set_nodelay(self.h, v),
            SockOption::KeepAlive(v) => s.set_keepalive(self.h, v),
            SockOption::Broadcast(v) => s.set_broadcast(self.h, v),
            _ => Ok(()),
        })
    }
    fn get_option(&self, opt: OptQuery) -> Result<u32, Errno> {
        let f = self.net.call(|s| s.flags(self.h))?;
        Ok(match opt {
            OptQuery::ReuseAddr => u32::from(f.reuse_addr),
            OptQuery::NoDelay => u32::from(f.nodelay),
            OptQuery::Broadcast => u32::from(f.broadcast),
            OptQuery::Ttl => 64,
            OptQuery::SndBuf | OptQuery::RcvBuf => 65536,
        })
    }
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

struct T {
    f: Fake,
    released: Arc<Mutex<alloc::vec::Vec<Arc<AtomicBool>>>>,
}

fn setup() -> T {
    let vfs: &'static Vfs = Box::leak(Box::new(Vfs::new(Arc::new(Tmpfs::new(Arc::new(FixedClock))))));
    let mut f = Fake::new();
    let ctx = vfs.root_context().unwrap();
    let vols = default_volumes(vfs, &ctx);
    f.sys.io.attach(&mut f.sys.objects, vfs, &vols).unwrap();
    let net = Net::new();
    let released: Arc<Mutex<alloc::vec::Vec<Arc<AtomicBool>>>> = Arc::default();
    let r = Arc::clone(&released);
    f.sockets = Some(Box::new(move |d, t, p| {
        let h = net.call(|s| s.socket(d, t, Some(p))).map_err(status_of)?;
        let sock = TestSock::new(&net, h, d, t, p);
        r.lock().unwrap().push(Arc::clone(&sock.released));
        Ok(Arc::new(sock) as Arc<dyn NtSocket>)
    }));
    T { f, released }
}

const RW: u32 = FILE_GENERIC_READ | FILE_GENERIC_WRITE;

fn sa(s: &str) -> alloc::vec::Vec<u8> {
    let (b, n) = addr::encode(&s.parse().unwrap());
    b[..n].to_vec()
}

impl T {
    fn open(&mut self, name: &str) -> (NtStatus, u64) {
        let out = self.f.alloc(8);
        let iosb = self.f.alloc(16);
        let oa = self.f.attrs(name, OBJ_CASE_INSENSITIVE);
        let s = self.f.call(Service::OpenFile, &[out, u64::from(RW | SYNCHRONIZE), oa, iosb, 3, 0]);
        (s, self.f.u64_at(out))
    }

    fn ioctl_ev(
        &mut self,
        h: u64,
        event: u64,
        code: u32,
        input: &[u8],
        outlen: usize,
    ) -> (NtStatus, u64, Vec<u8>, u64) {
        let inp = if input.is_empty() { 0 } else { self.f.put(input) };
        let out = self.f.alloc(outlen.max(1));
        let iosb = self.f.alloc(16);
        let s = self.f.call(
            Service::DeviceIoControlFile,
            &[h, event, 0, 0, iosb, u64::from(code), inp, input.len() as u64, out, outlen as u64],
        );
        self.drain();
        let info = self.f.u64_at(iosb + 8);
        (s, info, self.f.bytes_at(out, outlen), iosb)
    }

    fn ioctl(&mut self, h: u64, code: u32, input: &[u8], outlen: usize) -> (NtStatus, u64, Vec<u8>) {
        let (s, info, out, _) = self.ioctl_ev(h, 0, code, input, outlen);
        (s, info, out)
    }

    fn drain(&mut self) {
        while let Some(c) = self.f.sys.io.pop_completion() {
            if let Some((addr, data)) = &c.data {
                self.f.mem.write_bytes(*addr, data).unwrap();
            }
            if c.iosb != 0 {
                self.f.mem.write_bytes(c.iosb, &c.iosb_bytes()).unwrap();
            }
            let _ = self.f.sys.io.finish(&mut self.f.sys.objects, &c);
        }
    }

    /// A new socket endpoint with a socket of the kind asked for.
    fn socket(&mut self, family: i32, ty: i32) -> u64 {
        let (s, h) = self.open(afd::ENDPOINT);
        assert_eq!(s, STATUS_SUCCESS);
        let req = afd::SocketReq { family, ty, protocol: 0 }.encode();
        assert_eq!(self.ioctl(h, afd::SOCKET, &req, 0).0, STATUS_SUCCESS);
        h
    }

    fn name(&mut self, h: u64, peer: bool) -> Result<SocketAddr, NtStatus> {
        let (s, info, out) = self.ioctl(h, afd::NAME, &u32::from(peer).to_le_bytes(), addr::MAX_SIZE);
        if s != STATUS_SUCCESS {
            return Err(s);
        }
        Ok(addr::decode(&out[..info as usize]).unwrap())
    }

    fn send(&mut self, h: u64, data: &[u8], to: Option<&str>, flags: u32) -> (NtStatus, u64) {
        let buf = self.f.put(data);
        let mut req = afd::SendReq { flags, addr_len: 0, buf, len: data.len() as u64, addr: [0; addr::MAX_SIZE] };
        if let Some(t) = to {
            let b = sa(t);
            req.addr[..b.len()].copy_from_slice(&b);
            req.addr_len = b.len() as u32;
        }
        let (s, info, _) = self.ioctl(h, afd::SEND, &req.encode(), 0);
        (s, info)
    }

    /// Receive into a fresh buffer of `len`: status, data, reply.
    fn recv(&mut self, h: u64, len: usize, flags: u32) -> (NtStatus, Vec<u8>, afd::RecvReply) {
        let buf = self.f.alloc(len.max(1));
        let req = afd::RecvReq { flags, buf, len: len as u64 };
        let (s, info, out) = self.ioctl(h, afd::RECV, &req.encode(), afd::RecvReply::SIZE);
        let data = self.f.bytes_at(buf, (info as usize).min(len));
        (s, data, afd::RecvReply::decode(&out).unwrap())
    }

    fn close(&mut self, h: u64) {
        assert_eq!(self.f.call(Service::Close, &[h]), STATUS_SUCCESS);
    }

    /// A connected pair over `host` (a listener, a client, the accepted end).
    fn tcp_pair(&mut self, family: i32, host: &str) -> (u64, u64, u64) {
        let l = self.socket(family, SOCK_STREAM);
        assert_eq!(self.ioctl(l, afd::BIND, &sa(&alloc::format!("{host}:0")), 0).0, STATUS_SUCCESS);
        assert_eq!(self.ioctl(l, afd::LISTEN, &5u32.to_le_bytes(), 0).0, STATUS_SUCCESS);
        let port = self.name(l, false).unwrap().port();
        let c = self.socket(family, SOCK_STREAM);
        assert_eq!(self.ioctl(c, afd::CONNECT, &sa(&alloc::format!("{host}:{port}")), 0).0, STATUS_SUCCESS);
        let (s, a) = self.open(afd::ENDPOINT);
        assert_eq!(s, STATUS_SUCCESS);
        let (s, info, out) = self.ioctl(l, afd::ACCEPT, &a.to_le_bytes(), addr::MAX_SIZE);
        assert_eq!(s, STATUS_SUCCESS);
        let peer = addr::decode(&out[..info as usize]).unwrap();
        assert_eq!(peer, self.name(c, false).unwrap());
        (l, c, a)
    }
}

fn st(v: u32) -> NtStatus {
    NtStatus(v)
}

#[test]
fn endpoints_open_by_name_only() {
    let mut t = setup();
    assert_eq!(t.open("\\Device\\Afd\\Endpoint").0, STATUS_SUCCESS);
    assert_eq!(t.open("\\Device\\Afd").0, STATUS_SUCCESS);
    assert_eq!(t.open("\\Device\\Afd\\Other").0, STATUS_OBJECT_NAME_NOT_FOUND);
    // A request before SOCKET: not a socket yet.
    let (_, h) = t.open(afd::ENDPOINT);
    assert_eq!(t.ioctl(h, afd::LISTEN, &1u32.to_le_bytes(), 0).0, STATUS_INVALID_HANDLE);
    // A second SOCKET on the same endpoint is refused.
    let h = t.socket(AF_INET, SOCK_STREAM);
    let req = afd::SocketReq { family: AF_INET, ty: SOCK_STREAM, protocol: 0 }.encode();
    assert_eq!(t.ioctl(h, afd::SOCKET, &req, 0).0, STATUS_INVALID_PARAMETER);
    // Unknown codes.
    assert_eq!(t.ioctl(h, afd::code(0x8ff), &[], 0).0, STATUS_INVALID_DEVICE_REQUEST);
}

#[test]
fn tcp_over_loopback() {
    let mut t = setup();
    let (l, c, a) = t.tcp_pair(AF_INET, "127.0.0.1");
    assert_eq!(t.send(c, b"hello", None, 0), (STATUS_SUCCESS, 5));
    let (s, info, out) = t.ioctl(a, afd::READABLE, &[], 4);
    assert_eq!((s, info, u32::from_le_bytes(out[..4].try_into().unwrap())), (STATUS_SUCCESS, 4, 5));
    // A peek leaves the data.
    let (s, data, reply) = t.recv(a, 16, afd::FLAG_PEEK);
    assert_eq!((s, &data[..], reply.from_len), (STATUS_SUCCESS, &b"hello"[..], 0));
    let (s, data, _) = t.recv(a, 16, 0);
    assert_eq!((s, &data[..]), (STATUS_SUCCESS, &b"hello"[..]));
    // The peer's name.
    assert_eq!(t.name(a, true).unwrap(), t.name(c, false).unwrap());
    // Non-blocking and empty: would block.
    assert_eq!(t.ioctl(a, afd::SET_NONBLOCKING, &1u32.to_le_bytes(), 0).0, STATUS_SUCCESS);
    assert_eq!(t.recv(a, 16, 0).0, st(werr::STATUS_CANT_WAIT));
    // The info request reports the mode.
    let (_, _, out) = t.ioctl(a, afd::INFO, &[], afd::Info::SIZE);
    let info = afd::Info::decode(&out).unwrap();
    assert_eq!((info.family, info.ty, info.protocol, info.nonblocking), (AF_INET, SOCK_STREAM, IPPROTO_TCP, 1));
    // Shut down sending: the other end reads the end of the stream.
    assert_eq!(t.ioctl(c, afd::SHUTDOWN, &(SD_SEND as u32).to_le_bytes(), 0).0, STATUS_SUCCESS);
    let (s, data, _) = t.recv(a, 16, 0);
    assert_eq!((s, data.len()), (STATUS_SUCCESS, 0));
    // A send after SD_SEND: WSAESHUTDOWN.
    let (s, _) = t.send(c, b"x", None, 0);
    assert_eq!(werr::wsa_of_status(s.0), WSAESHUTDOWN);
    for h in [l, c, a] {
        t.close(h);
    }
    // Every socket heard that its handle went.
    assert!(t.released.lock().unwrap().iter().all(|r| r.load(Ordering::SeqCst)));
}

#[test]
fn tcp_over_ipv6_loopback() {
    let mut t = setup();
    let (_, c, a) = t.tcp_pair(AF_INET6, "[::1]");
    assert_eq!(t.send(a, b"six", None, 0), (STATUS_SUCCESS, 3));
    let (s, data, _) = t.recv(c, 3, afd::FLAG_WAITALL);
    assert_eq!((s, &data[..]), (STATUS_SUCCESS, &b"six"[..]));
}

#[test]
fn errors_the_personality_decides() {
    let mut t = setup();
    let h = t.socket(AF_INET, SOCK_STREAM);
    // getsockname and listen before bind: WSAEINVAL.
    assert_eq!(t.name(h, false), Err(STATUS_INVALID_PARAMETER));
    assert_eq!(t.ioctl(h, afd::LISTEN, &1u32.to_le_bytes(), 0).0, STATUS_INVALID_PARAMETER);
    // getpeername unconnected: WSAENOTCONN.
    assert_eq!(werr::wsa_of_status(t.name(h, true).unwrap_err().0), WSAENOTCONN);
    // bind with an IPv6 address on an IPv4 socket, or a short one: WSAEFAULT.
    let s = t.ioctl(h, afd::BIND, &sa("[::1]:0"), 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEFAULT);
    let s = t.ioctl(h, afd::BIND, &sa("127.0.0.1:0")[..8], 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEFAULT);
    // connect to the unspecified address or port 0: WSAEADDRNOTAVAIL.
    let s = t.ioctl(h, afd::CONNECT, &sa("0.0.0.0:80"), 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEADDRNOTAVAIL);
    let s = t.ioctl(h, afd::CONNECT, &sa("127.0.0.1:0"), 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEADDRNOTAVAIL);
    // connect with the other family: WSAEAFNOSUPPORT.
    let s = t.ioctl(h, afd::CONNECT, &sa("[::1]:80"), 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEAFNOSUPPORT);
    // Nobody listens: refused.
    let s = t.ioctl(h, afd::CONNECT, &sa("127.0.0.1:9"), 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAECONNREFUSED);
    // accept on a socket that does not listen: WSAEINVAL.
    let (_, e) = t.open(afd::ENDPOINT);
    let h2 = t.socket(AF_INET, SOCK_STREAM);
    assert_eq!(t.ioctl(h2, afd::ACCEPT, &e.to_le_bytes(), addr::MAX_SIZE).0, STATUS_INVALID_PARAMETER);
    // listen on a datagram socket: WSAEOPNOTSUPP.
    let u = t.socket(AF_INET, SOCK_DGRAM);
    assert_eq!(t.ioctl(u, afd::BIND, &sa("127.0.0.1:0"), 0).0, STATUS_SUCCESS);
    let s = t.ioctl(u, afd::LISTEN, &1u32.to_le_bytes(), 0).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEOPNOTSUPP);
    // send on an unconnected datagram socket without a destination.
    let (s, _) = t.send(u, b"x", None, 0);
    assert_eq!(werr::wsa_of_status(s.0), WSAEDESTADDRREQ);
    // recv on an unconnected stream socket.
    let (s, _, _) = t.recv(h2, 4, 0);
    assert_eq!(werr::wsa_of_status(s.0), WSAENOTCONN);
    // Non-blocking accept with nothing queued.
    let (l, _, _) = t.tcp_pair(AF_INET, "127.0.0.1");
    assert_eq!(t.ioctl(l, afd::SET_NONBLOCKING, &1u32.to_le_bytes(), 0).0, STATUS_SUCCESS);
    let (_, e2) = t.open(afd::ENDPOINT);
    let s = t.ioctl(l, afd::ACCEPT, &e2.to_le_bytes(), addr::MAX_SIZE).0;
    assert_eq!(werr::wsa_of_status(s.0), WSAEWOULDBLOCK);
}

#[test]
fn udp_over_loopback() {
    let mut t = setup();
    let a = t.socket(AF_INET, SOCK_DGRAM);
    let b = t.socket(AF_INET, SOCK_DGRAM);
    assert_eq!(t.ioctl(a, afd::BIND, &sa("127.0.0.1:0"), 0).0, STATUS_SUCCESS);
    assert_eq!(t.ioctl(b, afd::BIND, &sa("127.0.0.1:0"), 0).0, STATUS_SUCCESS);
    let pa = t.name(a, false).unwrap();
    let pb = t.name(b, false).unwrap();
    assert_eq!(t.send(a, b"datagram", Some(&pb.to_string()), 0), (STATUS_SUCCESS, 8));
    let (s, data, reply) = t.recv(b, 64, 0);
    assert_eq!((s, &data[..]), (STATUS_SUCCESS, &b"datagram"[..]));
    assert_eq!(addr::decode(&reply.from[..reply.from_len as usize]).unwrap(), pa);
    // A datagram longer than the buffer: the start, WSAEMSGSIZE.
    t.send(a, b"0123456789", Some(&pb.to_string()), 0);
    let (s, data, reply) = t.recv(b, 4, 0);
    assert_eq!((s, &data[..]), (st(werr::STATUS_BUFFER_OVERFLOW), &b"0123"[..]));
    assert_eq!(reply.msg_flags & afd::MSG_TRUNCATED, afd::MSG_TRUNCATED);
    assert_eq!(werr::wsa_of_status(s.0), WSAEMSGSIZE);
    // A connected datagram socket sends without a destination.
    assert_eq!(t.ioctl(a, afd::CONNECT, &sa(&pb.to_string()), 0).0, STATUS_SUCCESS);
    assert_eq!(t.send(a, b"c", None, 0), (STATUS_SUCCESS, 1));
    assert_eq!(t.recv(b, 4, 0).1, b"c");
}

#[test]
fn poll_reports_readiness() {
    let mut t = setup();
    let (_, c, a) = t.tcp_pair(AF_INET, "127.0.0.1");
    let rd = POLLRDNORM as u16 as u32;
    let wr = POLLWRNORM as u16 as u32;
    let (_, raw) = t.open(afd::ENDPOINT);
    let req = afd::PollReq { timeout_ms: 0, count: 3 }.encode(&[(a, rd), (c, wr), (raw, rd)]);
    let (s, info, out) = t.ioctl(a, afd::POLL, &req, 12);
    assert_eq!(s, STATUS_SUCCESS);
    let rev = |i: usize| u32::from_le_bytes(out[i * 4..i * 4 + 4].try_into().unwrap());
    // Nothing to read yet; the client can send; the bare endpoint is no socket.
    assert_eq!((rev(0), rev(1), rev(2)), (0, wr, poly_winsock::poll::nval()));
    assert_eq!(info, 2);
    t.send(c, b"x", None, 0);
    let req = afd::PollReq { timeout_ms: 0, count: 1 }.encode(&[(a, rd)]);
    let (_, info, out) = t.ioctl(a, afd::POLL, &req, 4);
    assert_eq!((info, u32::from_le_bytes(out[..4].try_into().unwrap())), (1, rd));
}

#[test]
fn options() {
    let mut t = setup();
    let h = t.socket(AF_INET, SOCK_STREAM);
    let set = |t: &mut T, level: i32, name: i32, v: &[u8]| {
        let mut b = afd::option_header(level, name).to_vec();
        b.extend_from_slice(v);
        t.ioctl(h, afd::SET_OPTION, &b, 0).0
    };
    let get = |t: &mut T, level: i32, name: i32| {
        let (s, info, out) = t.ioctl(h, afd::GET_OPTION, &afd::option_header(level, name), 8);
        (s, info, u32::from_le_bytes(out[..4].try_into().unwrap()))
    };
    assert_eq!(get(&mut t, SOL_SOCKET, SO_TYPE), (STATUS_SUCCESS, 4, SOCK_STREAM as u32));
    assert_eq!(get(&mut t, SOL_SOCKET, SO_ACCEPTCONN).2, 0);
    assert_eq!(set(&mut t, SOL_SOCKET, SO_RCVTIMEO, &250u32.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(get(&mut t, SOL_SOCKET, SO_RCVTIMEO).2, 250);
    assert_eq!(set(&mut t, IPPROTO_TCP, TCP_NODELAY, &1u32.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(get(&mut t, IPPROTO_TCP, TCP_NODELAY).2, 1);
    // A BOOL as one byte.
    assert_eq!(set(&mut t, SOL_SOCKET, SO_REUSEADDR, &[1]), STATUS_SUCCESS);
    assert_eq!(get(&mut t, SOL_SOCKET, SO_REUSEADDR).2, 1);
    assert_eq!(set(&mut t, SOL_SOCKET, SO_KEEPALIVE, &1u32.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(get(&mut t, SOL_SOCKET, SO_KEEPALIVE).2, 1);
    let linger = [1u8, 0, 5, 0];
    assert_eq!(set(&mut t, SOL_SOCKET, SO_LINGER, &linger), STATUS_SUCCESS);
    assert_eq!(get(&mut t, SOL_SOCKET, SO_LINGER).2, u32::from_le_bytes(linger));
    assert_eq!(get(&mut t, SOL_SOCKET, SO_DONTLINGER).2, 0);
    assert_eq!(set(&mut t, SOL_SOCKET, SO_SNDBUF, &8192u32.to_le_bytes()), STATUS_SUCCESS);
    assert_eq!(get(&mut t, SOL_SOCKET, SO_SNDBUF).2, 8192);
    assert_eq!(get(&mut t, SOL_SOCKET, SO_ERROR).2, 0);
    // Too short a value: WSAEFAULT; an unknown option or one of another
    // level's socket: WSAENOPROTOOPT.
    assert_eq!(werr::wsa_of_status(set(&mut t, SOL_SOCKET, SO_RCVTIMEO, &[1, 2]).0), WSAEFAULT);
    assert_eq!(werr::wsa_of_status(set(&mut t, SOL_SOCKET, 0x7777, &[0; 4]).0), WSAENOPROTOOPT);
    assert_eq!(werr::wsa_of_status(set(&mut t, IPPROTO_IPV6, IPV6_V6ONLY, &[0; 4]).0), WSAENOPROTOOPT);
    assert_eq!(werr::wsa_of_status(set(&mut t, SOL_SOCKET, SO_TYPE, &[0; 4]).0), WSAENOPROTOOPT);
    assert_eq!(werr::wsa_of_status(get(&mut t, IPPROTO_TCP, 0x7777).0.0), WSAENOPROTOOPT);
    let u = t.socket(AF_INET6, SOCK_DGRAM);
    let (s, _, out) = t.ioctl(u, afd::GET_OPTION, &afd::option_header(IPPROTO_IPV6, IPV6_V6ONLY), 4);
    assert_eq!((s, out), (STATUS_SUCCESS, 1u32.to_le_bytes().to_vec()));
    let (s, _, _) = t.ioctl(u, afd::GET_OPTION, &afd::option_header(IPPROTO_TCP, TCP_NODELAY), 4);
    assert_eq!(werr::wsa_of_status(s.0), WSAENOPROTOOPT);
}

#[test]
fn a_receive_that_times_out() {
    let mut t = setup();
    let (_, _, a) = t.tcp_pair(AF_INET, "127.0.0.1");
    let mut b = afd::option_header(SOL_SOCKET, SO_RCVTIMEO).to_vec();
    b.extend_from_slice(&100u32.to_le_bytes());
    assert_eq!(t.ioctl(a, afd::SET_OPTION, &b, 0).0, STATUS_SUCCESS);
    let (s, _, _) = t.recv(a, 4, 0);
    assert_eq!(werr::wsa_of_status(s.0), WSAETIMEDOUT);
}

#[test]
fn overlapped_requests() {
    let mut t = setup();
    let (_, c, a) = t.tcp_pair(AF_INET, "127.0.0.1");
    let ev_out = t.f.alloc(8);
    // An event for the requests (NotificationEvent, not signalled).
    let s = t.f.call(Service::CreateEvent, &[ev_out, 0x1F0003, 0, 0, 0]);
    assert_eq!(s, STATUS_SUCCESS);
    let ev = t.f.u64_at(ev_out);
    // Data is there: the overlapped receive finishes in the call and sets
    // the event.
    t.send(c, b"now", None, 0);
    let buf = t.f.alloc(8);
    let req = afd::RecvReq { flags: afd::FLAG_OVERLAPPED, buf, len: 8 };
    let (s, info, _, _) = t.ioctl_ev(a, ev, afd::RECV, &req.encode(), afd::RecvReply::SIZE);
    assert_eq!((s, info, t.f.bytes_at(buf, 3)), (STATUS_SUCCESS, 3, b"now".to_vec()));
    let query = t.f.alloc(8);
    assert_eq!(t.f.call(Service::QueryEvent, &[ev, 0, query, 8, 0]), STATUS_SUCCESS);
    assert_eq!(t.f.u32_at(query + 4), 1, "the event is set");
    // Nothing there: pending. The fake has no worker, so the request ran
    // in the call (and gave up: the fake's blocking is bounded); its
    // status block shows how it ended once completed.
    let req = afd::RecvReq { flags: afd::FLAG_OVERLAPPED, buf, len: 8 };
    let (s, _, _, iosb) = t.ioctl_ev(a, ev, afd::RECV, &req.encode(), afd::RecvReply::SIZE);
    assert_eq!(s, STATUS_PENDING);
    assert_eq!(t.f.u32_at(iosb), werr::STATUS_CANT_WAIT);
    // An overlapped send finishes at once.
    let data = t.f.put(b"abc");
    let send = afd::SendReq { flags: afd::FLAG_OVERLAPPED, addr_len: 0, buf: data, len: 3, addr: [0; addr::MAX_SIZE] };
    let (s, info, _, _) = t.ioctl_ev(c, ev, afd::SEND, &send.encode(), 0);
    assert_eq!((s, info), (STATUS_SUCCESS, 3));
}

#[test]
fn read_and_write_file_on_a_socket() {
    let mut t = setup();
    let (_, c, a) = t.tcp_pair(AF_INET, "127.0.0.1");
    let data = t.f.put(b"file api");
    let iosb = t.f.alloc(16);
    let s = t.f.call(Service::WriteFile, &[c, 0, 0, 0, iosb, data, 8, 0, 0]);
    assert_eq!((s, t.f.u64_at(iosb + 8)), (STATUS_SUCCESS, 8));
    let buf = t.f.alloc(16);
    let s = t.f.call(Service::ReadFile, &[a, 0, 0, 0, iosb, buf, 16, 0, 0]);
    assert_eq!((s, t.f.u64_at(iosb + 8)), (STATUS_SUCCESS, 8));
    assert_eq!(t.f.bytes_at(buf, 8), b"file api");
}

#[test]
fn net_info() {
    let mut t = setup();
    let (_, h) = t.open(afd::ENDPOINT);
    let (s, info, out) = t.ioctl(h, afd::NET_INFO, &[], afd::NetInfo::SIZE);
    assert_eq!((s, info as usize), (STATUS_SUCCESS, afd::NetInfo::SIZE));
    let n = afd::NetInfo::decode(&out).unwrap();
    assert_eq!((n.servers(), n.host_name()), (&[[10, 0, 2, 3]][..], "polytest"));
    // An output buffer too small: WSAEFAULT.
    let (s, _, _) = t.ioctl(h, afd::NET_INFO, &[], 8);
    assert_eq!(werr::wsa_of_status(s.0), WSAEFAULT);
}
