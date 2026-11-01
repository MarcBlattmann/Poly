//! The kernel's side of `\Device\Afd` (ADR 0095): the NT personality's
//! [`NtSocket`] over the kernel's Internet sockets (`net::file::InetSocket`,
//! the same objects the Linux personality's socket descriptors are,
//! ADR 0048), waits for several sockets at once (`select`, `WSAPoll`), and
//! the worker that finishes overlapped requests.
//!
//! - A call that may wait blocks the calling thread on the socket's wait
//!   queue with a [`ThreadBlocker`], as the Linux socket calls do: the
//!   network thread notifies the queue when the socket changes, and the end
//!   of the thread's process interrupts the wait (`EINTR`). Releasing the
//!   socket (its last handle closed) ends waits on it too.
//! - [`pend`] queues an overlapped send or receive the personality could
//!   not finish at once; the `nt-afd` worker retries every queued request
//!   whenever the stack's readiness generation moves (and every
//!   [`RECHECK_MS`], to notice cancelled requests), completes the request
//!   packet under the NT lock and delivers the completion (`io::drain`).

use super::{io, with};
use crate::fs::poll::ThreadBlocker;
use crate::net::file::{InetSocket, errno as net_errno};
use crate::sync::Mutex;
use crate::thread::{self, WaitQueue};
use crate::{net, time, user};
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::SocketAddr;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use poly_ntobj::NtStatus;
use poly_personality_nt::afd::{
    Domain, Errno, NtSocket, OptQuery, PendIo, Protocol, Readiness, RecvInfo, Shutdown, SockOption, SockType, SockWait,
};
use poly_vfs::poll::{self, Timeout};
use poly_vfs::{PollEvents, Waker, Woke};

/// `EINTR` (Linux `errno.h`), which `poly_net::Errno` has no name for.
const EINTR: Errno = Errno(4);

/// How often the worker looks at its queue even without socket changes.
const RECHECK_MS: u64 = 50;

/// A kernel socket as the NT personality sees it.
#[derive(Debug)]
pub struct KernelSocket {
    inner: Arc<InetSocket>,
    released: AtomicBool,
}

fn vfs_to_net(e: poly_vfs::Errno) -> Errno {
    Errno(e.code() as u16)
}

fn timeout_of(wait: SockWait) -> Timeout {
    match wait {
        SockWait::Now => Timeout::Now,
        SockWait::Forever => Timeout::Never,
        SockWait::Millis(ms) => Timeout::At(time::now_ns().saturating_add(ms.saturating_mul(time::NANOS_PER_MILLI))),
    }
}

impl KernelSocket {
    fn wrap(inner: Arc<InetSocket>) -> Arc<KernelSocket> {
        Arc::new(KernelSocket { inner, released: AtomicBool::new(false) })
    }

    /// Run `attempt` until it stops failing with `EAGAIN`, waiting on the
    /// socket's queue as `wait` allows.
    fn wait_for<T>(
        &self,
        interest: PollEvents,
        wait: SockWait,
        mut attempt: impl FnMut() -> Result<T, poly_vfs::Errno>,
    ) -> Result<T, Errno> {
        let released = || self.released.load(Ordering::SeqCst);
        let mut checked = || if released() { Err(poly_vfs::Errno::EINTR) } else { attempt() };
        let timeout = timeout_of(wait);
        if timeout == Timeout::Now {
            return checked().map_err(vfs_to_net);
        }
        let (Some(process), Some(flags)) = (user::current_process(), user::current_flags()) else {
            return checked().map_err(vfs_to_net);
        };
        let mut blocker = ThreadBlocker::new(&process, flags);
        poll::wait_until_done(self.inner.queue(), interest, &mut blocker, timeout, checked).map_err(vfs_to_net)
    }

    fn net<R>(&self, f: impl FnOnce(&mut poly_net::Stack, poly_net::SocketHandle) -> R) -> R {
        self.inner.with(f)
    }
}

impl NtSocket for KernelSocket {
    fn domain(&self) -> Domain {
        self.inner.domain
    }

    fn kind(&self) -> SockType {
        self.inner.ty
    }

    fn protocol(&self) -> Protocol {
        self.inner.protocol
    }

    fn bind(&self, addr: SocketAddr) -> Result<(), Errno> {
        self.net(|s, h| s.bind(h, addr))
    }

    fn listen(&self, backlog: usize) -> Result<(), Errno> {
        self.net(|s, h| s.listen(h, backlog))?;
        self.inner.update(|st| st.listening = true);
        Ok(())
    }

    fn accept(&self, wait: SockWait) -> Result<(Arc<dyn NtSocket>, SocketAddr), Errno> {
        let (h, peer) =
            self.wait_for(PollEvents::READABLE, wait, || self.net(|s, h| s.accept(h)).map_err(net_errno))?;
        Ok((KernelSocket::wrap(self.inner.accepted(h)), peer))
    }

    fn connect(&self, addr: SocketAddr, wait: SockWait) -> Result<(), Errno> {
        let pending = self.inner.state().connecting;
        match self.net(|s, h| s.connect(h, addr)) {
            Ok(()) => return Ok(()),
            Err(Errno::EINPROGRESS) => {}
            Err(Errno::EALREADY) if pending && wait != SockWait::Now => {}
            Err(Errno::EISCONN) if pending => {
                self.inner.update(|st| st.connecting = false);
                return Ok(());
            }
            Err(e) => return Err(e),
        }
        if wait == SockWait::Now {
            self.inner.update(|st| st.connecting = true);
            return Err(Errno::EINPROGRESS);
        }
        let done = Readiness::WRITABLE.0 | Readiness::ERROR.0 | Readiness::HANGUP.0;
        let r = self.wait_for(PollEvents::WRITABLE, wait, || {
            let ready = self.net(|s, h| s.readiness(h)).map_err(net_errno)?;
            if ready.0 & done == 0 {
                return Err(poly_vfs::Errno::EAGAIN);
            }
            if let Some(e) = self.net(|s, h| s.take_error(h)).map_err(net_errno)? {
                return Err(net_errno(e));
            }
            match self.net(|s, h| s.tcp_state(h)).map_err(net_errno)? {
                poly_net::TcpState::Established | poly_net::TcpState::CloseWait => Ok(()),
                _ => Err(poly_vfs::Errno::ECONNREFUSED),
            }
        });
        self.inner.update(|st| st.connecting = r == Err(EINTR) || r == Err(Errno::EAGAIN));
        match r {
            Err(Errno::EAGAIN) => Err(Errno::ETIMEDOUT),
            r => r,
        }
    }

    fn send(&self, data: &[u8], to: Option<SocketAddr>, wait: SockWait) -> Result<usize, Errno> {
        self.wait_for(PollEvents::WRITABLE, wait, || self.inner.send(data, to))
    }

    fn recv(&self, buf: &mut [u8], peek: bool, wait: SockWait) -> Result<RecvInfo, Errno> {
        // A zero-length receive returns at once (nothing to wait for).
        if buf.is_empty() {
            let from = self.peer_addr().or_else(|_| self.local_addr())?;
            return Ok(RecvInfo { len: 0, full_len: 0, from });
        }
        self.wait_for(PollEvents::READABLE, wait, || self.inner.recv(buf, peek))
    }

    fn shutdown(&self, how: Shutdown) -> Result<(), Errno> {
        let h = self.inner.handle();
        net::call_and_wake(|s| s.shutdown(h, how))
    }

    fn local_addr(&self) -> Result<SocketAddr, Errno> {
        self.net(|s, h| s.local_addr(h))
    }

    fn peer_addr(&self) -> Result<SocketAddr, Errno> {
        self.net(|s, h| s.peer_addr(h))
    }

    fn readiness(&self) -> Readiness {
        self.net(|s, h| s.readiness(h)).unwrap_or_default()
    }

    fn take_error(&self) -> Option<Errno> {
        self.net(|s, h| s.take_error(h)).ok().flatten()
    }

    fn bytes_readable(&self) -> usize {
        self.net(|s, h| s.bytes_readable(h)).unwrap_or(0)
    }

    fn set_option(&self, opt: SockOption) -> Result<(), Errno> {
        let secs = |v: u32| Duration::from_secs(u64::from(v));
        match opt {
            SockOption::ReuseAddr(v) => self.net(|s, h| s.set_reuse_addr(h, v)),
            SockOption::KeepAlive(v) => {
                self.inner.update(|st| st.options.keepalive = v);
                self.net(|s, h| s.set_keepalive(h, v))
            }
            SockOption::Broadcast(v) => self.net(|s, h| s.set_broadcast(h, v)),
            SockOption::NoDelay(v) => self.net(|s, h| s.set_nodelay(h, v)),
            SockOption::SndBuf(v) => {
                let _ = self.net(|s, h| s.set_buffer_sizes(h, Some(v), None));
                Ok(())
            }
            SockOption::RcvBuf(v) => {
                let _ = self.net(|s, h| s.set_buffer_sizes(h, None, Some(v)));
                Ok(())
            }
            SockOption::Linger(l) => {
                self.inner.update(|st| st.options.linger = l);
                Ok(())
            }
            SockOption::Ttl(t) => {
                let (_, tos) = self.net(|s, h| s.ip_options(h))?;
                self.net(|s, h| s.set_ip_options(h, Some(t), tos))
            }
            SockOption::KeepIdle(v) => self.net(|s, h| s.set_keepalive_params(h, Some(secs(v)), None, None)),
            SockOption::KeepIntvl(v) => self.net(|s, h| s.set_keepalive_params(h, None, Some(secs(v)), None)),
            SockOption::KeepCnt(v) => self.net(|s, h| s.set_keepalive_params(h, None, None, Some(v))),
        }
    }

    fn get_option(&self, opt: OptQuery) -> Result<u32, Errno> {
        let flags = self.net(|s, h| s.flags(h))?;
        Ok(match opt {
            OptQuery::ReuseAddr => u32::from(flags.reuse_addr),
            OptQuery::Broadcast => u32::from(flags.broadcast),
            OptQuery::NoDelay => u32::from(flags.nodelay),
            OptQuery::SndBuf | OptQuery::RcvBuf => {
                let (snd, rcv) = self.net(|s, h| s.buffer_sizes(h))?;
                let v = if opt == OptQuery::SndBuf { snd } else { rcv };
                u32::try_from(v).unwrap_or(u32::MAX)
            }
            OptQuery::Ttl => {
                let (ttl, _) = self.net(|s, h| s.ip_options(h))?;
                u32::from(ttl.unwrap_or_else(|| self.net(|s, _| s.default_ttl())))
            }
        })
    }

    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.inner.queue().notify(PollEvents::ALL);
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

/// `socket` for the NT personality.
pub(super) fn create(domain: Domain, ty: SockType, protocol: Protocol) -> Result<Arc<dyn NtSocket>, NtStatus> {
    let inner = InetSocket::new(domain, ty, protocol).map_err(poly_personality_nt::afd::status_of)?;
    Ok(KernelSocket::wrap(inner))
}

fn events_of(r: Readiness) -> PollEvents {
    let mut e = PollEvents::NONE;
    if r.0 & Readiness::READABLE.0 != 0 {
        e = e | PollEvents::READABLE;
    }
    if r.0 & Readiness::WRITABLE.0 != 0 {
        e = e | PollEvents::WRITABLE;
    }
    e | PollEvents::ALWAYS
}

/// Wait until one of `socks` has any of its interest, an error or a
/// hang-up.
pub(super) fn poll_sockets(socks: &[(Arc<dyn NtSocket>, Readiness)], wait: SockWait) -> Vec<Readiness> {
    let any = |r: Readiness, want: Readiness| r.0 & (want.0 | Readiness::ERROR.0 | Readiness::HANGUP.0) != 0;
    let scan = || -> (Vec<Readiness>, bool) {
        let v: Vec<Readiness> = socks.iter().map(|(s, _)| s.readiness()).collect();
        let ready = v.iter().zip(socks).any(|(&r, (_, want))| any(r, *want));
        (v, ready)
    };
    let timeout = timeout_of(wait);
    let (v, ready) = scan();
    if ready || timeout == Timeout::Now {
        return v;
    }
    let (Some(process), Some(flags)) = (user::current_process(), user::current_flags()) else { return v };
    let mut blocker = ThreadBlocker::new(&process, flags);
    let waker: Arc<dyn Waker> = poly_vfs::Blocker::waker(&blocker);
    let _registered: Vec<_> = socks
        .iter()
        .filter_map(|(s, want)| {
            s.as_any().downcast_ref::<KernelSocket>().map(|k| k.inner.queue().register(events_of(*want), waker.clone()))
        })
        .collect();
    let until = match timeout {
        Timeout::At(t) => Some(t),
        _ => None,
    };
    loop {
        let (v, ready) = scan();
        if ready {
            return v;
        }
        match poly_vfs::Blocker::block(&mut blocker, until) {
            Woke::Woken => {}
            Woke::TimedOut | Woke::Interrupted => return scan().0,
        }
    }
}

/// The DNS servers and host name for `ws2_32`.
pub(super) fn net_info() -> (Vec<[u8; 4]>, String) {
    let dns = net::dns_server().map(|a| alloc::vec![a.octets()]).unwrap_or_default();
    let host = String::from_utf8(crate::linux::admin::hostname()).unwrap_or_else(|_| String::from("poly"));
    (dns, if host.is_empty() { String::from("poly") } else { host })
}

// ----- overlapped requests

static PENDING: Mutex<Vec<PendIo>> = Mutex::new(Vec::new());
static WORKER: WaitQueue = WaitQueue::new();
static KICKED: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Queue an overlapped request for the worker (started on first use).
pub(super) fn pend(io: PendIo) -> Result<(), PendIo> {
    if !STARTED.swap(true, Ordering::SeqCst) {
        match thread::spawn_named("nt-afd", worker_main, 0) {
            Ok(tid) => {
                let _ = thread::detach(tid);
            }
            Err(_) => {
                STARTED.store(false, Ordering::SeqCst);
                return Err(io);
            }
        }
    }
    PENDING.lock().push(io);
    KICKED.store(true, Ordering::SeqCst);
    WORKER.wake_all();
    net::kick();
    Ok(())
}

/// Requests the worker holds (tests).
#[allow(dead_code, reason = "read by the kernel tests")]
pub fn pending() -> usize {
    PENDING.lock().len()
}

fn worker_main(_: usize) -> usize {
    loop {
        KICKED.store(false, Ordering::SeqCst);
        let ((), seen) = net::call_and_generation(|_| ());
        let list = core::mem::take(&mut *PENDING.lock());
        let mut keep = Vec::new();
        let mut completed = false;
        for io in list {
            if !with(|s| s.sys.io.irp(io.irp).is_some()) {
                // Cancelled (closed, CancelIo, the thread ended).
                continue;
            }
            match io.attempt() {
                None => keep.push(io),
                Some((status, info, data)) => {
                    with(|s| s.sys.io.complete(io.irp, status, info, data));
                    completed = true;
                }
            }
        }
        if completed {
            io::drain();
        }
        let idle = {
            let mut p = PENDING.lock();
            let newer = core::mem::take(&mut *p);
            *p = keep;
            p.extend(newer);
            p.is_empty()
        };
        if idle {
            WORKER.wait_until(|| KICKED.load(Ordering::SeqCst));
        } else if !KICKED.load(Ordering::SeqCst) {
            let deadline = time::now_ns().saturating_add(RECHECK_MS * time::NANOS_PER_MILLI);
            net::wait_for_change(seen, Some(deadline));
        }
    }
}
