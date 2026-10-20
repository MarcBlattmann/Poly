//! Blocking sockets over the stack, for the system call layers.
//!
//! The stack's calls never block (ADR 0028); these wrap them in the loop
//! every blocking call needs: read the readiness generation, try the call,
//! and if it would block (`EAGAIN`, or a connect still in progress) sleep
//! until the generation moves or the deadline passes, then try again. The
//! network thread and every socket call move the generation whenever
//! readiness may have changed ([`super::wait_for_change`]).
//!
//! Deadlines are monotonic-clock nanoseconds (`time::now_ns`); `None`
//! waits for ever. A timed-out call fails with `EAGAIN` (what Linux
//! returns when `SO_RCVTIMEO` / `SO_SNDTIMEO` run out), except `connect`,
//! which fails with `ETIMEDOUT`. Nonblocking callers use [`super::call`]
//! directly and get the stack's own answers.
//!
//! These are the kernel's own sockets (the shell, the kernel tests): their
//! waits cannot be interrupted. User space's sockets are socket files
//! ([`super::file::InetSocket`], ADR 0048), whose waits a signal ends and
//! whose `SIGPIPE` the personality raises.

use super::{Domain, Errno, Protocol, Readiness, RecvFlags, RecvInfo, Shutdown, SockType, SocketHandle};
use super::{call, call_and_generation, call_and_wake, wait_for_change};
use core::net::SocketAddr;
use poly_net::{Stack, TcpState};

/// An open socket. Closed when dropped (a TCP connection then finishes in
/// the background, as `close(2)` lets it).
#[derive(Debug)]
pub struct Socket {
    handle: SocketHandle,
}

/// Try `op` until it stops failing with `EAGAIN`, waiting for readiness
/// changes in between; `EAGAIN` once `deadline` passes.
fn block_on<T>(deadline: Option<u64>, mut op: impl FnMut(&mut Stack) -> Result<T, Errno>) -> Result<T, Errno> {
    loop {
        let (r, seen) = call_and_generation(&mut op);
        match r {
            Err(Errno::EAGAIN) => {}
            r => return r,
        }
        if !wait_for_change(seen, deadline) {
            return Err(Errno::EAGAIN);
        }
    }
}

impl Socket {
    /// `socket(2)`: `protocol` `None` picks TCP for streams, UDP for
    /// datagrams; `Some(Protocol::Icmp)` with datagrams is a ping socket.
    pub fn new(domain: Domain, ty: SockType, protocol: Option<Protocol>) -> Result<Socket, Errno> {
        call(|s| s.socket(domain, ty, protocol)).map(|handle| Socket { handle })
    }

    pub const fn handle(&self) -> SocketHandle {
        self.handle
    }

    pub fn bind(&self, addr: SocketAddr) -> Result<(), Errno> {
        call(|s| s.bind(self.handle, addr))
    }

    pub fn listen(&self, backlog: usize) -> Result<(), Errno> {
        call(|s| s.listen(self.handle, backlog))
    }

    /// `connect(2)`, waiting until a TCP connection is established or has
    /// failed (with the stack's error: `ECONNREFUSED`, `ETIMEDOUT`,
    /// `ENETUNREACH`, ...). Datagram sockets just set their peer.
    pub fn connect(&self, addr: SocketAddr, deadline: Option<u64>) -> Result<(), Errno> {
        let h = self.handle;
        match call(|s| s.connect(h, addr)) {
            Err(Errno::EINPROGRESS) => {}
            r => return r,
        }
        let done = Readiness::WRITABLE | Readiness::ERROR | Readiness::HANGUP;
        let r = block_on(deadline, |s| {
            if s.readiness(h)?.0 & done.0 == 0 {
                return Err(Errno::EAGAIN);
            }
            if let Some(e) = s.take_error(h)? {
                return Err(e);
            }
            match s.tcp_state(h)? {
                TcpState::Established | TcpState::CloseWait => Ok(()),
                _ => Err(Errno::ECONNREFUSED),
            }
        });
        r.map_err(|e| if e == Errno::EAGAIN { Errno::ETIMEDOUT } else { e })
    }

    /// `accept(2)`: the next established connection and its peer.
    pub fn accept(&self, deadline: Option<u64>) -> Result<(Socket, SocketAddr), Errno> {
        block_on(deadline, |s| s.accept(self.handle)).map(|(handle, peer)| (Socket { handle }, peer))
    }

    /// `send(2)`: waits until the stack takes at least one byte; returns how
    /// many it took.
    pub fn send(&self, data: &[u8], deadline: Option<u64>) -> Result<usize, Errno> {
        self.send_to(data, None, deadline)
    }

    /// `sendto(2)`; `to` is ignored by connected TCP sockets.
    pub fn send_to(&self, data: &[u8], to: Option<SocketAddr>, deadline: Option<u64>) -> Result<usize, Errno> {
        block_on(deadline, |s| s.send_to(self.handle, data, to))
    }

    /// Send all of `data`, waiting for room as often as needed.
    pub fn send_all(&self, mut data: &[u8], deadline: Option<u64>) -> Result<(), Errno> {
        while !data.is_empty() {
            let n = self.send(data, deadline)?;
            data = &data[n..];
        }
        Ok(())
    }

    /// `recv(2)`: waits for data; `Ok(0)` at the end of a TCP stream.
    pub fn recv(&self, buf: &mut [u8], deadline: Option<u64>) -> Result<usize, Errno> {
        self.recv_from(buf, RecvFlags::default(), deadline).map(|i| i.len)
    }

    /// `recvfrom(2)`, with the sender's address.
    pub fn recv_from(&self, buf: &mut [u8], flags: RecvFlags, deadline: Option<u64>) -> Result<RecvInfo, Errno> {
        block_on(deadline, |s| s.recv_from(self.handle, buf, flags))
    }

    pub fn shutdown(&self, how: Shutdown) -> Result<(), Errno> {
        call_and_wake(|s| s.shutdown(self.handle, how))
    }

    pub fn readiness(&self) -> Result<Readiness, Errno> {
        call(|s| s.readiness(self.handle))
    }

    /// Wait until any of `interest` (or an error or hangup) is set, as
    /// `poll(2)` on one socket does; returns the readiness then.
    /// `EAGAIN` at the deadline.
    pub fn wait(&self, interest: Readiness, deadline: Option<u64>) -> Result<Readiness, Errno> {
        let any = interest | Readiness::ERROR | Readiness::HANGUP;
        block_on(deadline, |s| {
            let r = s.readiness(self.handle)?;
            if r.0 & any.0 != 0 { Ok(r) } else { Err(Errno::EAGAIN) }
        })
    }

    /// `getsockopt(SO_ERROR)`.
    pub fn take_error(&self) -> Result<Option<Errno>, Errno> {
        call(|s| s.take_error(self.handle))
    }

    pub fn local_addr(&self) -> Result<SocketAddr, Errno> {
        call(|s| s.local_addr(self.handle))
    }

    pub fn peer_addr(&self) -> Result<SocketAddr, Errno> {
        call(|s| s.peer_addr(self.handle))
    }

    /// Run any other stack call on this socket (options, `tcp_info`, ...).
    pub fn with<R>(&self, f: impl FnOnce(&mut Stack, SocketHandle) -> R) -> R {
        call(|s| f(s, self.handle))
    }

    /// `close(2)`, reporting the stack's answer (dropping closes too, but
    /// silently).
    pub fn close(self) -> Result<(), Errno> {
        let h = self.handle;
        core::mem::forget(self);
        call_and_wake(|s| s.close(h))
    }
}

impl Drop for Socket {
    fn drop(&mut self) {
        let _ = call_and_wake(|s| s.close(self.handle));
    }
}

/// ICMP echo over a ping socket (`SOCK_DGRAM` / `IPPROTO_ICMP`): the
/// stack fills in the identifier (the socket's port) and the checksum.
#[derive(Debug)]
pub struct Pinger {
    sock: Socket,
    payload: usize,
}

/// ICMPv4 echo request and reply types (RFC 792).
const ECHO_REQUEST: u8 = 8;
const ECHO_REPLY: u8 = 0;

impl Pinger {
    /// A ping socket sending `payload` data bytes per request (56, as
    /// ping(8) does by default, makes 64-byte ICMP messages).
    pub fn new(payload: usize) -> Result<Pinger, Errno> {
        Ok(Pinger { sock: Socket::new(Domain::Inet, SockType::Datagram, Some(Protocol::Icmp))?, payload })
    }

    /// Send echo request `seq` to `dst` and wait for its reply until
    /// `deadline`; returns the round-trip time in nanoseconds and the
    /// reply's length (ICMP header and data). Replies to other requests
    /// are skipped. `EAGAIN` at the deadline.
    pub fn echo(&self, dst: core::net::Ipv4Addr, seq: u16, deadline: u64) -> Result<(u64, usize), Errno> {
        let mut req = alloc::vec![0u8; 8 + self.payload];
        req[0] = ECHO_REQUEST;
        req[6..8].copy_from_slice(&seq.to_be_bytes());
        for (i, b) in req[8..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let start = crate::time::now_ns();
        self.sock.send_to(&req, Some(SocketAddr::new(dst.into(), 0)), Some(deadline))?;
        let mut reply = alloc::vec![0u8; req.len() + 64];
        loop {
            let info = self.sock.recv_from(&mut reply, RecvFlags::default(), Some(deadline))?;
            let r = &reply[..info.len];
            if info.from.ip() == dst && r.len() >= 8 && r[0] == ECHO_REPLY && r[6..8] == seq.to_be_bytes() {
                return Ok((crate::time::now_ns() - start, r.len()));
            }
        }
    }
}
