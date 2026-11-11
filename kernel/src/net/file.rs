//! Internet sockets as open files (ADR 0048): [`InetSocket`] is the
//! `poly_vfs::FileObject` behind a socket descriptor of `AF_INET` or
//! `AF_INET6`. Its operations never block (the stack's calls never do);
//! its wait queue is notified by [`super::notify`] whenever the stack
//! reports a change of the socket, so blocking reads and writes, `poll`
//! and `epoll` wait on it like on a pipe (ADR 0042).
//!
//! It also keeps what the stack does not: the socket's type and
//! protocol, the generic options the system calls report back
//! (`SO_LINGER`, `SO_REUSEPORT`, timeouts, ...), whether it listens, and
//! whether a blocking connect was interrupted (a restarted connect goes
//! on waiting instead of failing with `EALREADY`).
//!
//! A socket lives in one stack for its whole life: the interface's, or
//! the stack of the network namespace it was made in ([`InetSocket::new_in`],
//! ADR 0178), which it keeps alive together with whatever the system call
//! layer says belongs to it ([`InetSocket::home`]: the namespace).

use super::{Domain, Errno as NetErrno, NsStack, Protocol, RecvFlags, SockType, SocketHandle};
use super::{call_and_wake_in, call_in, notify, stack_id};
use crate::sync::SpinLock;
use alloc::sync::Arc;
use core::any::Any;
use core::net::SocketAddr;
use poly_unix::SocketOptions;
use poly_vfs::{Errno, FileObject, PollEvents, WaitQueue};

/// What the kernel keeps for an Internet socket besides the stack's state.
#[derive(Clone, Copy, Debug, Default)]
pub struct InetState {
    pub options: SocketOptions,
    /// `SO_RCVTIMEO` / `SO_SNDTIMEO` in nanoseconds; 0: none.
    pub rcv_timeout: u64,
    pub snd_timeout: u64,
    /// `SO_SNDBUF` / `SO_RCVBUF` as set (doubled), if set.
    pub sndbuf: Option<usize>,
    pub rcvbuf: Option<usize>,
    pub listening: bool,
    /// A blocking connect was interrupted before it completed.
    pub connecting: bool,
    /// `IPV6_V6ONLY` as set (the stack's IPv6 sockets carry only IPv6
    /// whatever it says).
    pub v6only: bool,
    /// `IP_RECVERR` / `IPV6_RECVERR` as set (ip(7), ipv6(7)): reported
    /// back. The stack keeps no queue of extended errors, so
    /// `MSG_ERRQUEUE` finds none (`EAGAIN`) whatever it says.
    pub recverr: bool,
    /// `TCP_MAXSEG` as set (reported back; the stack sizes segments from
    /// the path).
    pub maxseg: Option<u32>,
}

/// An `AF_INET` / `AF_INET6` socket.
#[derive(Debug)]
pub struct InetSocket {
    handle: SocketHandle,
    pub domain: Domain,
    pub ty: SockType,
    pub protocol: Protocol,
    queue: Arc<WaitQueue>,
    state: SpinLock<InetState>,
    /// The stack it is in (`None`: the interface's).
    stack: Option<Arc<NsStack>>,
    /// What the socket keeps alive besides its stack.
    home: Option<Arc<dyn Any + Send + Sync>>,
}

/// A stack error as a VFS error (both are Linux numbers).
pub fn errno(e: NetErrno) -> Errno {
    Errno::from_code(i32::from(e.0)).unwrap_or(Errno::EIO)
}

impl InetSocket {
    /// A new socket in the interface's stack.
    pub fn new(domain: Domain, ty: SockType, protocol: Protocol) -> Result<Arc<InetSocket>, NetErrno> {
        InetSocket::new_in(None, None, domain, ty, protocol)
    }

    /// A new socket in `stack` (`None`: the interface's), keeping `home`
    /// alive for as long as it lives.
    pub fn new_in(
        stack: Option<Arc<NsStack>>,
        home: Option<Arc<dyn Any + Send + Sync>>,
        domain: Domain,
        ty: SockType,
        protocol: Protocol,
    ) -> Result<Arc<InetSocket>, NetErrno> {
        let handle = call_in(stack.as_deref(), |s| s.socket(domain, ty, Some(protocol)))?;
        Ok(InetSocket::wrap(handle, domain, ty, protocol, InetState::default(), stack, home))
    }

    /// A connection `accept` took from a listener with `parent`'s state,
    /// in its stack.
    pub fn accepted(&self, handle: SocketHandle) -> Arc<InetSocket> {
        let mut st = self.state();
        st.listening = false;
        st.connecting = false;
        InetSocket::wrap(handle, self.domain, self.ty, self.protocol, st, self.stack.clone(), self.home.clone())
    }

    fn wrap(
        handle: SocketHandle,
        domain: Domain,
        ty: SockType,
        protocol: Protocol,
        st: InetState,
        stack: Option<Arc<NsStack>>,
        home: Option<Arc<dyn Any + Send + Sync>>,
    ) -> Arc<InetSocket> {
        let sock = Arc::new(InetSocket {
            handle,
            domain,
            ty,
            protocol,
            queue: Arc::new(WaitQueue::new()),
            state: SpinLock::new(st),
            stack,
            home,
        });
        notify::watch(stack_id(sock.stack.as_deref()), handle, &sock.queue);
        sock
    }

    /// What the socket was made with ([`new_in`](Self::new_in)).
    pub fn home(&self) -> Option<&Arc<dyn Any + Send + Sync>> {
        self.home.as_ref()
    }

    pub fn handle(&self) -> SocketHandle {
        self.handle
    }

    pub fn queue(&self) -> &Arc<WaitQueue> {
        &self.queue
    }

    /// A copy of the kernel-side state.
    pub fn state(&self) -> InetState {
        *self.state.lock()
    }

    /// Change the kernel-side state (`f` must not block).
    pub fn update<R>(&self, f: impl FnOnce(&mut InetState) -> R) -> R {
        f(&mut self.state.lock())
    }

    /// Run a stack call on this socket.
    pub fn with<R>(&self, f: impl FnOnce(&mut poly_net::Stack, SocketHandle) -> R) -> R {
        let h = self.handle;
        call_in(self.stack.as_deref(), |s| f(s, h))
    }

    /// Run a stack call on this socket that changes what other threads
    /// waiting on it see (`shutdown`).
    pub fn with_wake<R>(&self, f: impl FnOnce(&mut poly_net::Stack, SocketHandle) -> R) -> R {
        let h = self.handle;
        call_and_wake_in(self.stack.as_deref(), |s| f(s, h))
    }

    /// One receive attempt: `EAGAIN` if nothing is there.
    pub fn recv(&self, buf: &mut [u8], peek: bool) -> Result<poly_net::RecvInfo, Errno> {
        self.with(|s, h| s.recv_from(h, buf, RecvFlags { peek })).map_err(errno)
    }

    /// One send attempt: `EAGAIN` without room.
    pub fn send(&self, data: &[u8], to: Option<SocketAddr>) -> Result<usize, Errno> {
        self.with(|s, h| s.send_to(h, data, to)).map_err(errno)
    }
}

impl Drop for InetSocket {
    fn drop(&mut self) {
        notify::unwatch(stack_id(self.stack.as_deref()), self.handle);
        let h = self.handle;
        // SO_LINGER with a zero timeout closes with a reset (socket(7));
        // otherwise the connection finishes in the background (a positive
        // linger time does not make close wait: ADR 0048).
        let abort = self.state.lock().options.linger == Some(0);
        if abort {
            let _ = call_and_wake_in(self.stack.as_deref(), |s| s.abort(h));
        } else {
            let _ = call_and_wake_in(self.stack.as_deref(), |s| s.close(h));
        }
    }
}

fn nonzero(t: u64) -> Option<u64> {
    (t != 0).then_some(t)
}

impl FileObject for InetSocket {
    fn read(&self, buf: &mut [u8]) -> Result<usize, Errno> {
        self.recv(buf, false).map(|i| i.len)
    }

    fn write(&self, buf: &[u8]) -> Result<usize, Errno> {
        self.send(buf, None)
    }

    fn poll(&self) -> PollEvents {
        let r = self.with(|s, h| s.readiness(h)).unwrap_or_default();
        PollEvents::from_bits(u32::from(r.to_linux_poll()))
    }

    fn wait_queue(&self) -> Option<&Arc<WaitQueue>> {
        Some(&self.queue)
    }

    fn ioctl(&self, request: u32, _arg: u64) -> Result<u64, Errno> {
        // FIONREAD on a listening TCP socket (tcp(7): EINVAL); everything
        // else a socket does not know.
        const FIONREAD: u32 = 0x541B;
        match request {
            FIONREAD => self.with(|s, h| s.bytes_readable(h)).map(|_| 0).map_err(errno),
            _ => Err(Errno::ENOTTY),
        }
    }

    fn bytes_readable(&self) -> Option<usize> {
        self.with(|s, h| s.bytes_readable(h)).ok()
    }

    fn timeouts(&self) -> (Option<u64>, Option<u64>) {
        let st = self.state.lock();
        (nonzero(st.rcv_timeout), nonzero(st.snd_timeout))
    }
}
