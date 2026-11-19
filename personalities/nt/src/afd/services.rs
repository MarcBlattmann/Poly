//! `\Device\Afd`'s requests (ADR 0095): `NtDeviceIoControlFile` with the
//! codes of [`poly_winsock::afd`], and `NtReadFile` / `NtWriteFile` on a
//! socket handle. Arguments are read from the caller's memory, the
//! endpoint looked up under the NT lock, and the socket called with no
//! lock held (it may wait). A request that finished writes its output and
//! status block and signals its event and APC like any completed I/O
//! (ADR 0067); an overlapped one that must wait is pending.
//!
//! Errors the personality decides itself (before the stack sees the
//! request) use the statuses `poly_winsock::error` maps to the WSA codes
//! Learn gives for the situation: a socket address too short or of the
//! wrong family for `bind` (`WSAEFAULT`: `STATUS_BUFFER_TOO_SMALL`), the
//! wrong family for `connect` (`WSAEAFNOSUPPORT`:
//! `STATUS_INVALID_ADDRESS`), `listen` before `bind` and `accept` on a
//! socket that does not listen (`WSAEINVAL`), `listen` on a datagram
//! socket (`WSAEOPNOTSUPP`), connecting to the unspecified address or port
//! 0 (`WSAEADDRNOTAVAIL`), an option a socket does not have
//! (`WSAENOPROTOOPT`).

use super::{Domain, Errno, Protocol, Readiness, Shutdown, SockType};
use super::{Endpoint, NtSocket, OptQuery, PendIo, PendOp, Placement, SockOption, SockWait, status_of};
use crate::abi;
use crate::io::Target;
use crate::io::consts::STATUS_INVALID_DEVICE_REQUEST;
use crate::io::irp::IrpOp;
use crate::io::services::{MAX_CAPTURED_WRITE, Notify, file_handle, write_iosb};
use crate::kernel::NtKernel;
use crate::system::NtSystem;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::SocketAddr;
use poly_ntobj::status::*;
use poly_ntobj::{Handle, NtStatus, ObjectId};
use poly_winsock::consts::*;
use poly_winsock::error as werr;
use poly_winsock::{addr, afd};

type Result<T> = core::result::Result<T, NtStatus>;

const BUFFER_TOO_SMALL: NtStatus = NtStatus(werr::STATUS_BUFFER_TOO_SMALL);
const INVALID_ADDRESS: NtStatus = NtStatus(werr::STATUS_INVALID_ADDRESS);
const ADDRESS_COMPONENT: NtStatus = NtStatus(werr::STATUS_INVALID_ADDRESS_COMPONENT);
const NO_PROTOCOL_OPTION: NtStatus = NtStatus(werr::STATUS_PROTOCOL_UNREACHABLE);
const NOT_SUPPORTED: NtStatus = NtStatus(werr::STATUS_NOT_SUPPORTED);
const OVERFLOW: NtStatus = NtStatus(werr::STATUS_BUFFER_OVERFLOW);
/// The most a blocking receive reads into the kernel at once (a longer
/// receive returns what came, as a stream receive may).
const MAX_RECV: usize = 1 << 20;
/// A blocking send moves through the kernel in pieces this big.
const SEND_CHUNK: usize = 64 * 1024;

/// Is file object `cookie` a socket endpoint?
pub(crate) fn is_endpoint(sys: &NtSystem, cookie: u64) -> bool {
    matches!(sys.io.file(cookie).map(|r| &r.target), Some(Target::Socket(_)))
}

fn endpoint(sys: &mut NtSystem, cookie: u64) -> Result<&mut Endpoint> {
    match sys.io.file_mut(cookie).map(|r| &mut r.target) {
        Some(Target::Socket(e)) => Ok(e),
        _ => Err(STATUS_INVALID_HANDLE),
    }
}

/// What a request needs of its endpoint, copied out under the lock.
#[derive(Clone, Debug)]
struct Snap {
    sock: Option<Arc<dyn NtSocket>>,
    twin: Option<Arc<dyn NtSocket>>,
    family: i32,
    ty: i32,
    protocol: i32,
    recv_wait: SockWait,
    send_wait: SockWait,
    listening: bool,
    nonblocking: bool,
    v6only: bool,
    mapped: bool,
    broken: Option<NtStatus>,
    connected: bool,
    /// The connect failed (its error is kept for `SO_ERROR`).
    failed: bool,
    /// When the last non-blocking connect started (0: none).
    connect_at: u64,
    shut_rd: bool,
    shut_wr: bool,
}

impl Snap {
    fn sock(&self) -> Result<&Arc<dyn NtSocket>> {
        self.sock.as_ref().ok_or(STATUS_INVALID_HANDLE)
    }

    /// An `AF_INET6` socket that takes IPv4 traffic too.
    fn dual(&self) -> bool {
        self.family == AF_INET6 && !self.v6only
    }
}

/// The endpoint's snapshot, with `connected` brought up to date: a stream whose connect finished
/// (a non-blocking one finishes without the program making a call) is connected from then on, also
/// after the stack lets the connection go.
fn snap<K: NtKernel>(k: &mut K, cookie: u64) -> Result<Snap> {
    let mut s = snap_raw(k, cookie)?;
    if !s.connected
        && let Some(sock) = &s.sock
        && sock.kind() == SockType::Stream
        && sock.established()
    {
        s.connected = true;
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.connected = true))?;
    }
    Ok(s)
}

fn snap_raw<K: NtKernel>(k: &mut K, cookie: u64) -> Result<Snap> {
    k.with_state(|sys, _, _| {
        let e = endpoint(sys, cookie)?;
        Ok(Snap {
            sock: e.sock.clone(),
            twin: e.twin.clone(),
            family: e.family,
            ty: e.ty,
            protocol: e.protocol,
            recv_wait: e.recv_wait(),
            send_wait: e.send_wait(),
            listening: e.listening,
            nonblocking: e.nonblocking,
            v6only: e.v6only,
            mapped: e.mapped,
            broken: e.broken,
            connected: e.connected,
            failed: e.conn_error.is_some(),
            connect_at: e.connect_at,
            shut_rd: e.shut_rd,
            shut_wr: e.shut_wr,
        })
    })
}

/// A socket address from a request for an endpoint of family `family`; `mismatch` is the status
/// for one of the other family (an `AF_INET` address for an `AF_INET6` socket is a buffer that is
/// too short for the socket's own addresses: `WSAEFAULT`).
fn socket_addr(bytes: &[u8], family: i32, mismatch: NtStatus) -> Result<SocketAddr> {
    let given = addr::family(bytes);
    if bytes.len() >= 2 && given != family && matches!(given, AF_INET | AF_INET6) {
        return Err(if family == AF_INET6 { BUFFER_TOO_SMALL } else { mismatch });
    }
    let a = addr::decode(bytes).map_err(|e| if e == WSAEFAULT { BUFFER_TOO_SMALL } else { INVALID_ADDRESS })?;
    if addr::af_of(&a) != family {
        return Err(if family == AF_INET6 { BUFFER_TOO_SMALL } else { mismatch });
    }
    Ok(a)
}

/// The address a stack socket reports, as the program sees it: IPv4-mapped when the socket stands
/// in for an `AF_INET6` one.
fn shown(mapped: bool, a: SocketAddr) -> SocketAddr {
    if mapped { addr::map_v4(&a) } else { a }
}

/// The result of a request that finished in the call.
struct Done {
    status: NtStatus,
    info: u64,
    output: Vec<u8>,
}

fn done(status: NtStatus, info: u64, output: Vec<u8>) -> Result<Option<Done>> {
    Ok(Some(Done { status, info, output }))
}

fn ok() -> Result<Option<Done>> {
    done(STATUS_SUCCESS, 0, Vec::new())
}

fn u32_in(input: &[u8], at: usize) -> Result<u32> {
    afd::read_u32(input, at).ok_or(BUFFER_TOO_SMALL)
}

/// `NtDeviceIoControlFile` on a socket handle (`file`), the arguments
/// already read: run the request and finish it.
// Not inlined: its locals would otherwise sit in the frame of every system
// call (`dispatch_with` inlines the services), where the kernel stack is
// tight (`NtCreateUserProcess`).
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn control<K: NtKernel>(
    k: &mut K,
    file: (ObjectId, u64),
    n: Notify,
    code: u32,
    input: &[u8],
    out: u64,
    outlen: usize,
) -> Result<NtStatus> {
    let r = match code {
        afd::SOCKET => create(k, file.1, input),
        afd::BIND => bind(k, file.1, input),
        afd::LISTEN => listen(k, file.1, input),
        afd::ACCEPT => accept(k, file.1, input),
        afd::CONNECT => connect(k, file.1, input),
        afd::SEND => return send(k, file, n, input),
        afd::ACCEPT_EX => return accept_ex(k, file, n, input),
        afd::CONNECT_EX => return connect_ex(k, file, n, input),
        afd::RECV => return recv(k, file, n, input, out, outlen),
        afd::SHUTDOWN => shutdown(k, file.1, input),
        afd::NAME => name(k, file.1, input),
        afd::SET_OPTION => set_option(k, file.1, input),
        afd::GET_OPTION => get_option(k, file.1, input),
        afd::SET_NONBLOCKING => {
            let on = u32_in(input, 0)? != 0;
            k.with_state(|sys, _, _| endpoint(sys, file.1).map(|e| e.nonblocking = on))?;
            ok()
        }
        afd::READABLE => {
            let s = snap(k, file.1)?;
            let n = if s.listening { 0 } else { s.sock()?.bytes_readable() };
            let v = u32::try_from(n).unwrap_or(u32::MAX);
            done(STATUS_SUCCESS, 4, v.to_le_bytes().to_vec())
        }
        afd::POLL => poll(k, input),
        afd::NET_INFO => {
            let (dns, host) = k.net_info();
            let b = afd::NetInfo::new(&dns, &host).encode();
            done(STATUS_SUCCESS, b.len() as u64, b.to_vec())
        }
        afd::IF_LIST => {
            let b = poly_iphlp::wire::encode(&k.net_interfaces());
            done(STATUS_SUCCESS, b.len() as u64, b)
        }
        afd::INFO => {
            let s = snap(k, file.1)?;
            let i = afd::Info {
                family: s.family,
                ty: s.ty,
                protocol: s.protocol,
                listening: u32::from(s.listening),
                nonblocking: u32::from(s.nonblocking),
            };
            done(STATUS_SUCCESS, afd::Info::SIZE as u64, i.encode().to_vec())
        }
        _ => Err(STATUS_INVALID_DEVICE_REQUEST),
    };
    match r? {
        Some(d) => finish(k, file, &n, d, out, outlen),
        None => Ok(STATUS_PENDING),
    }
}

/// Write a finished request's output and status block, signal its event
/// and APC.
fn finish<K: NtKernel>(
    k: &mut K,
    file: (ObjectId, u64),
    n: &Notify,
    d: Done,
    out: u64,
    outlen: usize,
) -> Result<NtStatus> {
    if d.output.len() > outlen {
        return Err(BUFFER_TOO_SMALL);
    }
    if !d.output.is_empty() {
        abi::write(k, out, &d.output)?;
    }
    if n.iosb != 0 {
        write_iosb(k, n.iosb, d.status, d.info)?;
    }
    let caller = k.caller();
    k.with_state(|sys, _, _| n.signal_done(sys, &caller, file, d.status, d.info));
    Ok(d.status)
}

fn create<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let req = afd::SocketReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    let req = afd::check_socket(req.family, req.ty, req.protocol).map_err(|_| STATUS_INVALID_PARAMETER)?;
    if k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.sock.is_some()))? {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let domain = if req.family == AF_INET { Domain::Inet } else { Domain::Inet6 };
    let (ty, proto) =
        if req.ty == SOCK_STREAM { (SockType::Stream, Protocol::Tcp) } else { (SockType::Datagram, Protocol::Udp) };
    let sock = k.socket_create(domain, ty, proto)?;
    k.with_state(|sys, _, _| {
        let e = endpoint(sys, cookie)?;
        e.sock = Some(sock);
        e.family = req.family;
        e.ty = req.ty;
        e.protocol = req.protocol;
        // Winsock's default: an IPv6 socket carries only IPv6 until `IPV6_V6ONLY` is turned off.
        e.v6only = true;
        Ok(())
    })?;
    ok()
}

/// Make the endpoint's socket an IPv4 stack socket that stands in for its `AF_INET6` one (a
/// dual-stack socket that connects or binds to an IPv4-mapped address: the stack's IPv6 sockets
/// carry only IPv6): the old socket's local port and options move over, the addresses it reports
/// are IPv4-mapped from now on. Returns the new socket.
fn switch_to_v4<K: NtKernel>(k: &mut K, cookie: u64, s: &Snap) -> Result<Arc<dyn NtSocket>> {
    let old = s.sock()?.clone();
    let new = k.socket_create(Domain::Inet, old.kind(), old.protocol())?;
    if let Ok(l) = old.local_addr()
        && l.port() != 0
    {
        let _ = new.bind(SocketAddr::new(core::net::Ipv4Addr::UNSPECIFIED.into(), l.port()));
    }
    let stored = k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.stored.clone()))?;
    replay_options(&new, true, &stored);
    let put = new.clone();
    k.with_state(|sys, _, _| {
        endpoint(sys, cookie).map(|e| {
            e.sock = Some(put);
            e.mapped = true;
        })
    })?;
    old.release();
    Ok(new)
}

fn bind<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    // Learn ("bind"): an address of another family is WSAEFAULT.
    let a = socket_addr(input, s.family, BUFFER_TOO_SMALL)?;
    if s.family == AF_INET6
        && let Some(v4) = addr::unmap_v4(&a)
    {
        // An IPv4-mapped address: only a dual-stack socket has it (Learn: "Dual-Stack Sockets").
        if s.v6only {
            return Err(ADDRESS_COMPONENT);
        }
        let on4 = if s.mapped { sock.clone() } else { switch_to_v4(k, cookie, &s)? };
        on4.bind(v4).map_err(status_of)?;
        return ok();
    }
    if s.mapped {
        return Err(INVALID_ADDRESS);
    }
    sock.bind(a).map_err(status_of)?;
    if s.dual() && s.ty == SOCK_STREAM && a.ip().is_unspecified() {
        // A dual-stack listener also takes IPv4 clients: its IPv4 half shares the port.
        let port = sock.local_addr().map_or(0, |l| l.port());
        if let Ok(twin) = k.socket_create(Domain::Inet, SockType::Stream, Protocol::Tcp) {
            let stored = k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.stored.clone()))?;
            replay_options(&twin, true, &stored);
            if twin.bind(SocketAddr::new(core::net::Ipv4Addr::UNSPECIFIED.into(), port)).is_ok() {
                k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.twin = Some(twin)))?;
            }
        }
    }
    ok()
}

/// Is the socket bound (a port of its own)?
fn bound(sock: &dyn NtSocket) -> bool {
    sock.local_addr().is_ok_and(|a| a.port() != 0)
}

fn listen<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let backlog = u32_in(input, 0).unwrap_or(0);
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    if sock.kind() != SockType::Stream {
        return Err(NOT_SUPPORTED);
    }
    if !bound(sock.as_ref()) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    // SOMAXCONN (0x7fffffff) and SOMAXCONN_HINT values: the stack's
    // queue is bounded anyway.
    let n = (backlog as usize).clamp(1, 128);
    sock.listen(n).map_err(status_of)?;
    if let Some(t) = &s.twin {
        let _ = t.listen(n);
    }
    k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.listening = true))?;
    ok()
}

/// The next connection of a listener: from its IPv6 socket or, for a dual-stack one, its IPv4
/// half; `true` when it came from the IPv4 half.
fn accept_any<K: NtKernel>(k: &mut K, s: &Snap) -> core::result::Result<(Arc<dyn NtSocket>, SocketAddr, bool), Errno> {
    let sock = s.sock().map_err(|_| Errno::EBADF)?;
    let Some(twin) = &s.twin else {
        return sock.accept(s.recv_wait).map(|(c, p)| (c, p, false));
    };
    loop {
        match sock.accept(SockWait::Now) {
            Ok((c, p)) => return Ok((c, p, false)),
            Err(Errno::EAGAIN) => {}
            Err(e) => return Err(e),
        }
        match twin.accept(SockWait::Now) {
            Ok((c, p)) => return Ok((c, p, true)),
            Err(Errno::EAGAIN) => {}
            Err(e) => return Err(e),
        }
        if s.recv_wait == SockWait::Now {
            return Err(Errno::EAGAIN);
        }
        let both = [(sock.clone(), Readiness::READABLE), (twin.clone(), Readiness::READABLE)];
        let ready = k.socket_poll(&both, s.recv_wait).map_err(|_| Errno(4))?;
        if ready.iter().all(|r| r.is_empty()) {
            return Err(Errno::EAGAIN);
        }
    }
}

fn accept<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let h = afd::read_u64(input, 0).ok_or(BUFFER_TOO_SMALL)?;
    let s = snap(k, cookie)?;
    s.sock()?;
    if !s.listening {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let caller = k.caller();
    // The endpoint that takes the connection: a fresh one of this process.
    let target = k.with_state(|sys, _, _| -> Result<u64> {
        let f = file_handle(sys, &caller, Handle(h), 0)?;
        let e = endpoint(sys, f.1)?;
        if e.sock.is_some() {
            return Err(STATUS_INVALID_PARAMETER);
        }
        Ok(f.1)
    })?;
    let (conn, peer, from_twin) = accept_any(k, &s).map_err(status_of)?;
    let placed = k.with_state(|sys, _, _| -> Result<()> {
        let (rcv, snd, stored) = {
            let e = endpoint(sys, cookie)?;
            (e.rcv_timeout, e.snd_timeout, e.stored.clone())
        };
        let e = endpoint(sys, target)?;
        e.sock = Some(conn);
        e.family = s.family;
        e.ty = s.ty;
        e.protocol = s.protocol;
        e.v6only = s.v6only;
        e.mapped = from_twin;
        e.connected = true;
        // Learn ("accept"): the new socket has the listener's properties.
        e.nonblocking = s.nonblocking;
        e.rcv_timeout = rcv;
        e.snd_timeout = snd;
        e.stored = stored;
        Ok(())
    });
    placed?;
    let (b, len) = addr::encode(&shown(from_twin, peer));
    done(STATUS_SUCCESS, len as u64, b[..len].to_vec())
}

fn connect<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let s = snap(k, cookie)?;
    let sock = s.sock()?.clone();
    if s.listening {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let datagram = sock.kind() == SockType::Datagram;
    // Learn ("connect"): an address of family AF_UNSPEC dissolves a datagram socket's association.
    if datagram && input.len() >= 2 && addr::family(input) == AF_UNSPEC {
        sock.disconnect().map_err(status_of)?;
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.connected = false))?;
        return ok();
    }
    let a = socket_addr(input, s.family, INVALID_ADDRESS)?;
    if !datagram && (a.ip().is_unspecified() || a.port() == 0) {
        return Err(ADDRESS_COMPONENT);
    }
    // A stream socket that has a connection (also one that was shut down, reset or closed by its
    // peer) is `WSAEISCONN` for another connect (the sheet's `connected`, `after_shutdown`).
    if !datagram && (s.connected || sock.established()) {
        return Err(NtStatus(werr::STATUS_CONNECTION_ACTIVE));
    }
    if datagram && a.ip().is_unspecified() && a.port() == 0 {
        // The unspecified address and port 0: no destination (Winsock keeps the socket unconnected).
        let _ = sock.disconnect();
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.connected = false))?;
        return ok();
    }
    // A dual-stack socket connects to an IPv4-mapped address through an IPv4 stack socket.
    let (target, sock) = if s.family == AF_INET6
        && let Some(v4) = addr::unmap_v4(&a)
    {
        if s.v6only {
            return Err(ADDRESS_COMPONENT);
        }
        let on4 = if s.mapped { sock } else { switch_to_v4(k, cookie, &s)? };
        (v4, on4)
    } else if s.mapped {
        return Err(INVALID_ADDRESS);
    } else {
        (a, sock)
    };
    // A blocking connect waits for the stack's own timeout, not
    // SO_SNDTIMEO (Learn: "connect" names no timeout option).
    let wait = if s.nonblocking { SockWait::Now } else { SockWait::Forever };
    // A second connect soon after a non-blocking one that is not through (Windows' loopback refusal
    // takes a moment): the first is still under way.
    // Once the program has read the failure (SO_ERROR), the socket may connect again.
    let now = k.system_time();
    const UNDER_WAY: u64 = 1_000_000; // 100 ms
    let failure_known =
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.conn_error.is_some() && e.conn_error_read))?;
    if s.nonblocking
        && s.connect_at != 0
        && now.saturating_sub(s.connect_at) < UNDER_WAY
        && !sock.established()
        && !failure_known
    {
        return Err(NtStatus(werr::STATUS_INVALID_DEVICE_STATE));
    }
    let stores_before = k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.conn_error_stores))?;
    let r = sock.connect(target, wait);
    match r {
        Ok(()) => {
            k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| {
                    e.connected = true;
                    e.conn_error = None;
                    e.conn_error_read = false;
                    e.broken = None;
                })
            })?;
        }
        Err(Errno::EINPROGRESS) => {
            k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.connect_under_way(stores_before, now)))?;
        }
        Err(_) => {}
    }
    r.map_err(status_of)?;
    ok()
}

fn shutdown<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let how = match u32_in(input, 0)? as i32 {
        SD_RECEIVE => Shutdown::Read,
        SD_SEND => Shutdown::Write,
        SD_BOTH => Shutdown::Both,
        _ => return Err(STATUS_INVALID_PARAMETER),
    };
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    let stream = sock.kind() == SockType::Stream;
    if stream && (s.listening || !(sock.established() || s.connected)) {
        return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
    }
    if stream && matches!(how, Shutdown::Read | Shutdown::Both) {
        // Winsock: data for a connection nobody reads any more resets it.
        let _ = sock.set_option(SockOption::AbortOnReadShutdown(true));
    }
    let r = sock.shutdown(how);
    // A datagram socket that never connected takes the call as done; a connection the peer
    // already ended too.
    if let Err(e) = r
        && !(s.connected || !stream)
    {
        return Err(status_of(e));
    }
    k.with_state(|sys, _, _| {
        endpoint(sys, cookie).map(|e| {
            if matches!(how, Shutdown::Read | Shutdown::Both) {
                e.shut_rd = true;
            }
            if matches!(how, Shutdown::Write | Shutdown::Both) {
                e.shut_wr = true;
            }
        })
    })?;
    ok()
}

fn name<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let peer = u32_in(input, 0)? != 0;
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    let needs_update = k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.needs_update))?;
    if needs_update {
        // A socket `AcceptEx` / `ConnectEx` made does not tell its addresses until it is updated.
        return Err(if peer { NtStatus(werr::STATUS_INVALID_CONNECTION) } else { STATUS_INVALID_PARAMETER });
    }
    let a = if peer {
        let live = sock.peer_addr().ok().filter(|_| sock.established() || sock.kind() == SockType::Datagram);
        let p = match live {
            Some(p) => {
                k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.peer = Some(p)))?;
                Some(p)
            }
            None => k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.peer))?.filter(|_| s.connected),
        };
        p.ok_or(NtStatus(werr::STATUS_INVALID_CONNECTION))?
    } else {
        // Learn ("getsockname"): WSAEINVAL before bind (or connect).
        match sock.local_addr() {
            Ok(a) if a.port() != 0 => a,
            _ => return Err(STATUS_INVALID_PARAMETER),
        }
    };
    let (b, len) = addr::encode(&shown(s.mapped, a));
    done(STATUS_SUCCESS, len as u64, b[..len].to_vec())
}

// ----- send and receive

fn read_data<K: NtKernel>(k: &mut K, buf: u64, len: usize) -> Result<Vec<u8>> {
    let mut data = crate::io::services::capture_buffer(len)?;
    if len > 0 {
        abi::read(k, buf, &mut data)?;
    }
    Ok(data)
}

/// The status Winsock keeps reporting for a connection the stack ended with `e` (a reset by the
/// peer, a reset after the peer had closed its side, a timeout): the same one for every later
/// receive and send, where the stack reports it once.
use super::sticky_status;

/// Remember `status` as the endpoint's broken-connection status.
fn set_broken<K: NtKernel>(k: &mut K, cookie: u64, status: NtStatus) {
    let _ = k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.broken = Some(status)));
}

/// What a connection that went wrong says to a receive or send: the stored status, or one made of
/// an error the stack has pending now (a reset throws queued data away, as on Windows).
fn broken_now<K: NtKernel>(k: &mut K, cookie: u64, s: &Snap) -> Option<NtStatus> {
    if let Some(b) = s.broken {
        return Some(b);
    }
    let sock = s.sock.as_ref()?;
    if sock.kind() != SockType::Stream || !s.connected && !sock.established() {
        return None;
    }
    if !sock.readiness().contains(Readiness::ERROR) {
        return None;
    }
    let e = sock.take_error()?;
    let st = sticky_status(e, sock).unwrap_or_else(|| status_of(e));
    set_broken(k, cookie, st);
    Some(st)
}

/// Send `len` bytes at `buf`: a datagram whole; a stream all of it when
/// the call may wait (Learn: "send" on a blocking socket), else what the
/// stack takes now. Returns the count.
fn send_now<K: NtKernel>(
    k: &mut K,
    sock: &Arc<dyn NtSocket>,
    buf: u64,
    len: usize,
    to: Option<SocketAddr>,
    wait: SockWait,
) -> Result<usize> {
    if sock.kind() == SockType::Datagram {
        if len > 65_507 {
            return Err(status_of(Errno::EMSGSIZE));
        }
        let data = read_data(k, buf, len)?;
        return sock.send(&data, to, wait).map_err(status_of);
    }
    let mut sent = 0;
    loop {
        let piece = (len - sent).min(SEND_CHUNK);
        let data = read_data(k, buf + sent as u64, piece)?;
        let mut at = 0;
        while at < data.len() {
            match sock.send(&data[at..], None, wait) {
                Ok(n) => at += n,
                Err(_) if sent + at > 0 => return Ok(sent + at),
                // A stream that never connected: WSAENOTCONN.
                Err(Errno::EPIPE) if sock.peer_addr().is_err() => {
                    return Err(status_of(Errno::ENOTCONN));
                }
                Err(e) => return Err(status_of(e)),
            }
            if wait == SockWait::Now {
                return Ok(sent + at);
            }
        }
        sent += data.len();
        if sent >= len {
            return Ok(sent);
        }
    }
}

/// A datagram's destination for the socket `s`: IPv4-mapped addresses of a dual-stack socket go
/// out through an IPv4 stack socket (made now for an unconnected one). `Ok(None)` for a send
/// that is to be reported done without going anywhere.
fn datagram_target<K: NtKernel>(
    k: &mut K,
    cookie: u64,
    s: &Snap,
    sock: &Arc<dyn NtSocket>,
    bytes: &[u8],
) -> Result<Option<(SocketAddr, Arc<dyn NtSocket>)>> {
    let a = socket_addr(bytes, s.family, INVALID_ADDRESS)?;
    if s.family == AF_INET6
        && let Some(v4) = addr::unmap_v4(&a)
    {
        if s.v6only {
            return Err(ADDRESS_COMPONENT);
        }
        if s.mapped {
            return Ok(Some((v4, sock.clone())));
        }
        if sock.established() {
            // Connected to an IPv6 peer: nothing to send an IPv4 datagram through.
            return Ok(None);
        }
        let on4 = switch_to_v4(k, cookie, s)?;
        return Ok(Some((v4, on4)));
    }
    if s.mapped {
        return Err(INVALID_ADDRESS);
    }
    Ok(Some((a, sock.clone())))
}

fn send<K: NtKernel>(k: &mut K, file: (ObjectId, u64), n: Notify, input: &[u8]) -> Result<NtStatus> {
    let req = afd::SendReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    let s = snap(k, file.1)?;
    let mut sock = s.sock()?.clone();
    let len = usize::try_from(req.len).map_err(|_| STATUS_INVALID_PARAMETER)?;
    let datagram = sock.kind() == SockType::Datagram;
    if let Some(b) = broken_now(k, file.1, &s) {
        return Err(b);
    }
    if s.shut_wr {
        // Learn ("shutdown"): a send after SD_SEND is WSAESHUTDOWN.
        return Err(NtStatus(werr::STATUS_PIPE_DISCONNECTED));
    }
    if sock.kind() == SockType::Stream && !s.connected {
        // Never connected, or the connect failed (a failed one keeps its error for `SO_ERROR`).
        return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
    }
    let mut to = None;
    if datagram {
        if let Some(b) = req.to() {
            match datagram_target(k, file.1, &s, &sock, b)? {
                Some((a, on)) => {
                    to = Some(a);
                    sock = on;
                }
                None => {
                    let done = Done { status: STATUS_SUCCESS, info: len as u64, output: Vec::new() };
                    return finish(k, file, &n, done, 0, 0);
                }
            }
        } else if !sock.established() {
            // Learn ("send", "sendto"): no destination on a datagram socket that never connected.
            return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
        }
    }
    let overlapped = req.flags & afd::FLAG_OVERLAPPED != 0;
    let wait = if overlapped { SockWait::Now } else { s.send_wait };
    let r = send_now(k, &sock, req.buf, len, to, wait);
    if let Err(st) = &r
        && let Some(e) = errno_of_status(st.0)
        && let Some(b) = sticky_status(e, &sock)
        && !datagram
    {
        set_broken(k, file.1, b);
        return Err(b);
    }
    if overlapped && len <= MAX_CAPTURED_WRITE {
        // Winsock finishes an overlapped send when all of it went (a datagram whole, a stream
        // as much as it takes): what the stack could not take now is sent by the worker.
        let first = match &r {
            Err(st) if *st == status_of(Errno::EAGAIN) => Some(0),
            Ok(done) if !datagram && *done < len => Some(*done),
            _ => None,
        };
        if let Some(done) = first {
            let data = read_data(k, req.buf, len)?;
            let op = PendOp::Send { data, to, done };
            let irp_op = IrpOp::Write { offset: None, data: Vec::new(), key: 0 };
            return pend(k, file, n, sock, op, irp_op);
        }
    }
    let count = r?;
    finish(k, file, &n, Done { status: STATUS_SUCCESS, info: count as u64, output: Vec::new() }, 0, 0)
}

/// The stack error behind a status `status_of` made (for the ones sticky errors are made of).
fn errno_of_status(status: u32) -> Option<Errno> {
    match status {
        werr::STATUS_CONNECTION_RESET => Some(Errno::ECONNRESET),
        werr::STATUS_CONNECTION_ABORTED => Some(Errno::ECONNABORTED),
        werr::STATUS_PIPE_DISCONNECTED => Some(Errno::EPIPE),
        werr::STATUS_IO_TIMEOUT => Some(Errno::ETIMEDOUT),
        _ => None,
    }
}

// Make an overlapped request pending, or run it in the calling thread
/// if the kernel cannot finish it later.
fn pend<K: NtKernel>(
    k: &mut K,
    file: (ObjectId, u64),
    n: Notify,
    sock: Arc<dyn NtSocket>,
    op: PendOp,
    irp_op: IrpOp,
) -> Result<NtStatus> {
    let caller = k.caller();
    // An `AcceptEx` ends when the socket it is to fill is closed.
    let accept_target = match &op {
        PendOp::Accept { target, .. } => Some(*target),
        _ => None,
    };
    let irp = k.with_state(|sys, _, _| -> Result<u64> {
        n.check_event(sys, &caller)?;
        let irp = n.irp(sys, &caller, file, false, irp_op)?;
        let id = sys.io.park(irp);
        if let Some(t) = accept_target {
            sys.io.accept_targets.insert(id, t);
        }
        Ok(id)
    })?;
    match k.socket_pend(PendIo::new(irp, file.1, sock, op)) {
        Ok(()) => Ok(STATUS_PENDING),
        Err(io) => {
            // No worker: wait here, then complete the request as the
            // worker would.
            let (status, info, data) = run_blocking(&io);
            k.with_state(|sys, _, _| sys.io.complete(irp, status, info, data));
            Ok(STATUS_PENDING)
        }
    }
}

/// An overlapped request done in the calling thread.
fn run_blocking(io: &PendIo) -> super::PendResult {
    match &io.op {
        PendOp::Recv { buf, len, peek } => {
            let mut data = alloc::vec![0u8; (*len).min(MAX_RECV)];
            match io.sock.recv(&mut data, *peek, SockWait::Forever) {
                Ok(info) => {
                    data.truncate(info.len);
                    (STATUS_SUCCESS, info.len as u64, Some((*buf, data)))
                }
                Err(e) if io.sock.kind() == SockType::Datagram => {
                    (NtStatus(poly_winsock::error::datagram_receive_status(e.0)), 0, None)
                }
                Err(e) => (status_of(e), 0, None),
            }
        }
        PendOp::Send { data, to, done } => match io.sock.send(&data[*done..], *to, SockWait::Forever) {
            Ok(n) => (STATUS_SUCCESS, (*done + n) as u64, None),
            Err(e) => (status_of(e), 0, None),
        },
        // `AcceptEx` and `ConnectEx` need the kernel's worker (they place a socket in an endpoint).
        PendOp::Accept { .. } | PendOp::Connect { .. } => (NOT_SUPPORTED, 0, None),
    }
}

/// `AcceptEx` (`mswsock.dll`): the request's endpoint is the listener; the connection goes into the
/// endpoint `accept` names (a socket the program made) when it comes, and, with a receive length,
/// when its first data has come. Always pends: the worker finishes it ([`PendIo::attempt`]).
fn accept_ex<K: NtKernel>(k: &mut K, file: (ObjectId, u64), n: Notify, input: &[u8]) -> Result<NtStatus> {
    let req = afd::AcceptExReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    let s = snap(k, file.1)?;
    let sock = s.sock()?.clone();
    if !s.listening {
        return Err(STATUS_INVALID_PARAMETER);
    }
    // Learn ("AcceptEx"): each address needs room for its structure and 16 bytes more; Windows
    // takes 16 and refuses 0.
    if req.local_len < 16 || req.remote_len < 16 {
        return Err(BUFFER_TOO_SMALL);
    }
    let caller = k.caller();
    let target = k.with_state(|sys, _, _| -> Result<u64> {
        let f = file_handle(sys, &caller, Handle(req.accept), 0)?;
        if f.1 == file.1 {
            return Err(STATUS_INVALID_PARAMETER);
        }
        let e = endpoint(sys, f.1)?;
        if e.sock.is_none() || e.listening {
            return Err(STATUS_INVALID_PARAMETER);
        }
        Ok(f.1)
    })?;
    let op = PendOp::Accept {
        target,
        buf: req.buf,
        recv_len: req.recv_len as usize,
        local_len: req.local_len as usize,
        remote_len: req.remote_len as usize,
        conn: None,
    };
    let len = (req.recv_len + req.local_len + req.remote_len) as usize;
    let irp_op = IrpOp::Read { offset: 0, len, buf: req.buf, key: crate::io::irp::SOCKET_CONNECT_KEY };
    pend(k, file, n, sock, op, irp_op)
}

/// `ConnectEx` (`mswsock.dll`): connect the request's endpoint, then send the data; pends, the
/// worker finishes it. A socket that is not bound is `WSAEINVAL`, a connected one `WSAEISCONN`.
fn connect_ex<K: NtKernel>(k: &mut K, file: (ObjectId, u64), n: Notify, input: &[u8]) -> Result<NtStatus> {
    let req = afd::ConnectExReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    let s = snap(k, file.1)?;
    let sock = s.sock()?.clone();
    if sock.kind() != SockType::Stream || s.listening {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if sock.established() || s.connected {
        return Err(NtStatus(werr::STATUS_CONNECTION_ACTIVE));
    }
    if !bound(sock.as_ref()) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    if req.to().len() < 16 {
        return Err(ADDRESS_COMPONENT);
    }
    let a = socket_addr(req.to(), s.family, INVALID_ADDRESS)?;
    if a.ip().is_unspecified() || a.port() == 0 {
        return Err(ADDRESS_COMPONENT);
    }
    let a = if s.mapped { a } else { addr::unmap_v4(&a).filter(|_| s.dual()).unwrap_or(a) };
    let len = req.len as usize;
    if len > MAX_CAPTURED_WRITE {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let data = read_data(k, req.buf, len)?;
    let op = PendOp::Connect { target: file.1, addr: a, started: false, data };
    let irp_op = IrpOp::Write { offset: None, data: Vec::new(), key: crate::io::irp::SOCKET_CONNECT_KEY };
    pend(k, file, n, sock, op, irp_op)
}

/// What a finished `AcceptEx` / `ConnectEx` changes in its endpoint (the kernel's worker applies it
/// with the NT lock held, just before the request completes): the accepted connection becomes the
/// endpoint's socket and the socket tells its addresses only after `SO_UPDATE_ACCEPT_CONTEXT` /
/// `SO_UPDATE_CONNECT_CONTEXT`.
pub fn place(sys: &mut NtSystem, p: Placement) {
    let Ok(e) = endpoint(sys, p.target) else { return };
    if let Some(b) = p.broken {
        e.broken = Some(b);
        return;
    }
    if let Some(sock) = p.sock {
        if let Some(old) = e.sock.replace(sock) {
            old.release();
        }
        // The new socket is the listener's (family, mode): IPv4 for `AcceptEx` of an IPv4 listener.
        e.mapped = false;
    }
    e.connected = true;
    e.needs_update = true;
}

fn recv<K: NtKernel>(
    k: &mut K,
    file: (ObjectId, u64),
    n: Notify,
    input: &[u8],
    out: u64,
    outlen: usize,
) -> Result<NtStatus> {
    let req = afd::RecvReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    if outlen != 0 && outlen < afd::RecvReply::SIZE {
        return Err(BUFFER_TOO_SMALL);
    }
    let s = snap(k, file.1)?;
    let sock = s.sock()?.clone();
    if s.listening {
        return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
    }
    let len = usize::try_from(req.len).map_err(|_| STATUS_INVALID_PARAMETER)?;
    let peek = req.flags & afd::FLAG_PEEK != 0;
    let overlapped = req.flags & afd::FLAG_OVERLAPPED != 0;
    let stream = sock.kind() == SockType::Stream;
    if stream && !s.connected {
        let r = sock.readiness();
        // Never connected, or the connect failed (its error stays for `SO_ERROR`).
        if s.failed || r.0 & Readiness::READABLE.0 == 0 || r.contains(Readiness::ERROR) {
            return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
        }
    }
    if !stream && !bound(sock.as_ref()) {
        // Learn ("recv"): a datagram socket that was never bound (or connected) has nothing to receive.
        return Err(STATUS_INVALID_PARAMETER);
    }
    if let Some(b) = broken_now(k, file.1, &s) {
        return Err(b);
    }
    let wait = if overlapped { SockWait::Now } else { s.recv_wait };
    if stream && req.flags & afd::FLAG_WAITALL != 0 && s.nonblocking {
        // Learn ("recv"): MSG_WAITALL is not supported on a non-blocking socket.
        return Err(NOT_SUPPORTED);
    }
    let mut buf = alloc::vec![0u8; len.min(MAX_RECV)];
    let first = sock.recv(&mut buf, peek, wait);
    let info = match first {
        Err(Errno::EAGAIN) if overlapped => {
            let op = PendOp::Recv { buf: req.buf, len, peek };
            let irp_op = IrpOp::Read { offset: 0, len, buf: req.buf, key: 0 };
            return pend(k, file, n, sock, op, irp_op);
        }
        // A blocking receive whose SO_RCVTIMEO ran out: WSAETIMEDOUT.
        Err(Errno::EAGAIN) if matches!(wait, SockWait::Millis(_)) => return Err(status_of(Errno::ETIMEDOUT)),
        // The handle was closed under the call: Windows aborts the connection (the peer sees a reset).
        Err(Errno(4)) if stream => {
            sock.abort();
            let b = NtStatus(werr::STATUS_CONNECTION_ABORTED);
            set_broken(k, file.1, b);
            return Err(b);
        }
        Err(e) if stream => {
            if let Some(b) = sticky_status(e, &sock) {
                set_broken(k, file.1, b);
                return Err(b);
            }
            return Err(status_of(e));
        }
        // A datagram socket sees the ICMP "port unreachable" of an earlier send as a reset.
        r => r.map_err(|e| NtStatus(poly_winsock::error::datagram_receive_status(e.0)))?,
    };
    let mut total = info.len;
    if stream && s.shut_rd && total == 0 {
        // Learn ("shutdown"): a receive after SD_RECEIVE is WSAESHUTDOWN.
        return Err(NtStatus(werr::STATUS_PIPE_DISCONNECTED));
    }
    if stream && req.flags & afd::FLAG_WAITALL != 0 && !peek && wait != SockWait::Now {
        while total < buf.len() && info.len > 0 {
            match sock.recv(&mut buf[total..], false, wait) {
                Ok(i) if i.len > 0 => total += i.len,
                _ => break,
            }
        }
    }
    if total > 0 {
        abi::write(k, req.buf, &buf[..total])?;
    }
    let truncated = !stream && info.full_len > info.len;
    let mut reply = afd::RecvReply { from: [0; addr::MAX_SIZE], from_len: 0, msg_flags: 0 };
    if !stream {
        let (b, l) = addr::encode(&shown(s.mapped, info.from));
        reply.from = b;
        reply.from_len = l as u32;
    }
    if truncated {
        reply.msg_flags |= afd::MSG_TRUNCATED;
    }
    let status = if truncated { OVERFLOW } else { STATUS_SUCCESS };
    let output = if outlen == 0 { Vec::new() } else { reply.encode().to_vec() };
    finish(k, file, &n, Done { status, info: total as u64, output }, out, outlen)
}

// ----- NtReadFile / NtWriteFile on a socket

/// `NtReadFile` on a socket handle: a receive as the socket's mode says.
/// `None` if `h` is not a socket.
#[inline(never)]
pub(crate) fn read_file<K: NtKernel>(k: &mut K, h: u64, n: Notify, buf: u64, len: usize) -> Option<Result<NtStatus>> {
    let file = socket_file(k, h)?;
    let input = afd::RecvReq { flags: 0, buf, len: len as u64 }.encode();
    Some(recv(k, file, n, &input, 0, 0))
}

/// `NtWriteFile` on a socket handle: a send. `None` if `h` is not a socket.
#[inline(never)]
pub(crate) fn write_file<K: NtKernel>(k: &mut K, h: u64, n: Notify, buf: u64, len: usize) -> Option<Result<NtStatus>> {
    let file = socket_file(k, h)?;
    let input = afd::SendReq { flags: 0, addr_len: 0, buf, len: len as u64, addr: [0; addr::MAX_SIZE] }.encode();
    Some(send(k, file, n, &input))
}

fn socket_file<K: NtKernel>(k: &mut K, h: u64) -> Option<(ObjectId, u64)> {
    let caller = k.caller();
    k.with_state(|sys, _, _| {
        let f = file_handle(sys, &caller, Handle(h), 0).ok()?;
        is_endpoint(sys, f.1).then_some(f)
    })
}

// ----- select / WSAPoll

fn poll<K: NtKernel>(k: &mut K, input: &[u8]) -> Result<Option<Done>> {
    let (req, entries) = afd::PollReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    let caller = k.caller();
    // Each entry's sockets (the second is the IPv4 half of a dual-stack listener) and whether it
    // ever connected, or None for a handle that is not a socket.
    type Entry = Option<(Arc<dyn NtSocket>, Option<Arc<dyn NtSocket>>, bool, bool)>;
    let socks: Vec<Entry> = k.with_state(|sys, _, _| {
        entries
            .iter()
            .map(|&(h, _)| {
                let f = file_handle(sys, &caller, Handle(h), 0).ok()?;
                let e = endpoint(sys, f.1).ok()?;
                e.sock.clone().map(|s| (s, e.twin.clone(), e.connected, e.conn_error.is_some()))
            })
            .collect()
    });
    let mut wanted: Vec<(Arc<dyn NtSocket>, Readiness)> = Vec::new();
    for (s, &(_, ev)) in socks.iter().zip(&entries) {
        if let Some((sock, twin, _, _)) = s {
            wanted.push((sock.clone(), interest_of(ev)));
            if let Some(t) = twin {
                wanted.push((t.clone(), interest_of(ev)));
            }
        }
    }
    let any_bad = socks.iter().any(Option::is_none);
    let wait = if any_bad || req.timeout_ms == 0 {
        SockWait::Now
    } else if req.timeout_ms < 0 {
        SockWait::Forever
    } else {
        SockWait::Millis(req.timeout_ms as u64)
    };
    let ready = if wanted.is_empty() {
        if wait != SockWait::Now {
            // Nothing to watch: just wait out the timeout.
            let t = match wait {
                SockWait::Millis(ms) => crate::kernel::NtTimeout::Relative(ms * 10_000),
                _ => crate::kernel::NtTimeout::Infinite,
            };
            let _ = k.delay(t, false);
        }
        Vec::new()
    } else {
        k.socket_poll(&wanted, wait)?
    };
    let mut out = Vec::with_capacity(entries.len() * 4);
    let mut count = 0u64;
    let mut it = ready.into_iter();
    for (s, &(_, ev)) in socks.iter().zip(&entries) {
        let bits = match s {
            None => poly_winsock::poll::nval(),
            Some((sock, twin, connected, failed)) => {
                let mut r = it.next().unwrap_or_default();
                if twin.is_some() {
                    r = Readiness(r.0 | it.next().unwrap_or_default().0);
                }
                device_bits(sock.as_ref(), r, *connected, *failed) & (ev | 0x7 | poly_winsock::poll::CONNECT_FAILED)
            }
        };
        if bits != 0 {
            count += 1;
        }
        out.extend_from_slice(&bits.to_le_bytes());
    }
    done(STATUS_SUCCESS, count, out)
}

/// The stack readiness a device interest waits for.
fn interest_of(ev: u32) -> Readiness {
    let mut r = 0;
    if ev & (POLLRDNORM as u16 as u32) != 0 {
        r |= Readiness::READABLE.0;
    }
    if ev & (POLLWRNORM as u16 as u32) != 0 {
        r |= Readiness::WRITABLE.0;
    }
    Readiness(r)
}

/// The device's poll bits for a socket's readiness (`connected`: the endpoint had a connection; `failed`: its connect failed).
/// A stream that never connected (and does not listen) is not "hung up" for Winsock, though the
/// stack reports it so; a connect that failed is writable (and has the error), as `WSAPoll` shows
/// it; the end of a stream is a hang-up and not "readable" while no data is left to read.
fn device_bits(sock: &dyn NtSocket, r: Readiness, connected: bool, failed: bool) -> u32 {
    let stream = sock.kind() == SockType::Stream;
    let connected = !stream || connected || sock.established();
    let error = r.contains(Readiness::ERROR) || failed;
    let hangup = (r.contains(Readiness::HANGUP) && (connected || error)) || r.contains(Readiness::READ_HANGUP);
    let mut readable = r.contains(Readiness::READABLE) && (connected || !error);
    let writable = r.contains(Readiness::WRITABLE) && (connected || error);
    if stream && hangup && readable && sock.bytes_readable() == 0 && !error {
        readable = false;
    }
    let mut bits = poly_winsock::poll::from_readiness(readable, writable, error, hangup);
    if stream && error && !connected {
        bits |= poly_winsock::poll::CONNECT_FAILED;
    }
    bits
}

// ----- options

/// `SO_RANDOMIZE_PORT`, `SO_MAX_MSG_SIZE`, `SO_PROTOCOL_INFOW`, `SO_CONNECT_TIME` (`ws2def.h`,
/// `winsock2.h`, `mswsock.h`) and `IPV6_ECN` / `IP_ECN` (`ws2ipdef.h`).
const SO_RANDOMIZE_PORT: i32 = 0x3005;
const SO_MAX_MSG_SIZE: i32 = 0x2003;
const SO_PROTOCOL_INFOW: i32 = 0x2005;
const SO_CONNECT_TIME: i32 = 0x700C;
const SO_UPDATE_ACCEPT_CONTEXT: i32 = 0x700B;
const SO_UPDATE_CONNECT_CONTEXT: i32 = 0x7010;
const IP_ECN: i32 = 50;
const IPV6_ECN: i32 = 50;
/// The size of a `WSAPROTOCOL_INFOW` (`winsock2.h`).
const PROTOCOL_INFO_SIZE: usize = 628;

/// A BOOL option's value: a DWORD, or fewer bytes (Learn: a BOOL may be passed as one byte); none
/// at all is `WSAEFAULT`.
fn flag_value(value: &[u8]) -> Result<bool> {
    match value {
        [] => Err(BUFFER_TOO_SMALL),
        [a, b, c, d, ..] => Ok(u32::from_le_bytes([*a, *b, *c, *d]) != 0),
        [a, ..] => Ok(*a != 0),
    }
}

/// An int option's value: a whole DWORD (a shorter buffer is `WSAEFAULT`).
fn int_of(value: &[u8]) -> Result<u32> {
    afd::read_u32(value, 0).ok_or(BUFFER_TOO_SMALL)
}

fn ttl(v: u32) -> Result<u8> {
    u8::try_from(v).ok().filter(|&t| t > 0).ok_or(STATUS_INVALID_PARAMETER)
}

/// The options that act on the stack's socket, set on `sock` (an IPv4 stack socket standing in for
/// an `AF_INET6` one gets the options of the family the program sees: `fam_v4` is that family).
/// `Ok(true)` when the option is also kept in the endpoint (`stored`).
fn stack_set(sock: &Arc<dyn NtSocket>, fam_v4: bool, level: i32, name: i32, value: &[u8]) -> Result<bool> {
    let tcp = sock.protocol() == Protocol::Tcp;
    match (level, name) {
        (SOL_SOCKET, SO_REUSEADDR) => sock.set_option(SockOption::ReuseAddr(flag_value(value)?)).map_err(status_of)?,
        (SOL_SOCKET, SO_KEEPALIVE) => {
            let on = flag_value(value)?;
            if tcp {
                sock.set_option(SockOption::KeepAlive(on)).map_err(status_of)?;
            }
        }
        (SOL_SOCKET, SO_BROADCAST) => {
            let on = flag_value(value)?;
            if tcp {
                return Err(NO_PROTOCOL_OPTION);
            }
            sock.set_option(SockOption::Broadcast(on)).map_err(status_of)?;
        }
        (SOL_SOCKET, SO_SNDBUF) => {
            let _ = sock.set_option(SockOption::SndBuf(int_of(value)? as usize));
        }
        (SOL_SOCKET, SO_RCVBUF) => {
            let _ = sock.set_option(SockOption::RcvBuf(int_of(value)? as usize));
        }
        (SOL_SOCKET, SO_LINGER) => {
            if !tcp {
                return Err(NO_PROTOCOL_OPTION);
            }
            let b = value.get(0..4).ok_or(BUFFER_TOO_SMALL)?;
            let on = u16::from_le_bytes([b[0], b[1]]) != 0;
            let secs = u16::from_le_bytes([b[2], b[3]]);
            sock.set_option(SockOption::Linger(on.then_some(u32::from(secs)))).map_err(status_of)?;
        }
        (IPPROTO_TCP, TCP_NODELAY) => {
            let on = flag_value(value)?;
            if !tcp {
                return Err(STATUS_INVALID_PARAMETER);
            }
            sock.set_option(SockOption::NoDelay(on)).map_err(status_of)?;
        }
        (IPPROTO_TCP, TCP_KEEPALIVE) if tcp => {
            sock.set_option(SockOption::KeepIdle(int_of(value)?)).map_err(status_of)?
        }
        (IPPROTO_TCP, TCP_KEEPINTVL) if tcp => {
            sock.set_option(SockOption::KeepIntvl(int_of(value)?)).map_err(status_of)?
        }
        (IPPROTO_TCP, TCP_KEEPCNT) if tcp => sock.set_option(SockOption::KeepCnt(int_of(value)?)).map_err(status_of)?,
        (IPPROTO_IP, IP_TTL) if fam_v4 => sock.set_option(SockOption::Ttl(ttl(int_of(value)?)?)).map_err(status_of)?,
        (IPPROTO_IPV6, IPV6_UNICAST_HOPS) if !fam_v4 => {
            sock.set_option(SockOption::Ttl(ttl(int_of(value)?)?)).map_err(status_of)?
        }
        _ => return Ok(false),
    }
    Ok(true)
}

/// Put the options an endpoint kept on a socket that replaces its own (errors do not matter: they
/// were accepted once).
fn replay_options(sock: &Arc<dyn NtSocket>, fam_v4: bool, stored: &alloc::collections::BTreeMap<(i32, i32), Vec<u8>>) {
    for (&(level, name), value) in stored {
        let _ = stack_set(sock, fam_v4, level, name, value);
    }
}

fn set_option<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let level = u32_in(input, 0)? as i32;
    let name = u32_in(input, 4)? as i32;
    let value = input.get(8..).unwrap_or(&[]).to_vec();
    let s = snap(k, cookie)?;
    let sock = s.sock()?.clone();
    let tcp = sock.protocol() == Protocol::Tcp;
    let v4 = s.family == AF_INET;
    if !matches!(level, SOL_SOCKET | IPPROTO_TCP | IPPROTO_IP | IPPROTO_IPV6) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let stored_flag = |k: &mut K, level: i32, name: i32| -> bool {
        k.with_state(|sys, _, _| {
            endpoint(sys, cookie)
                .map(|e| e.stored.get(&(level, name)).is_some_and(|v| v.first().is_some_and(|b| *b != 0)))
        })
        .unwrap_or(false)
    };
    // What is kept for reading back: BOOL options as one byte, the others as a DWORD.
    let mut keep: Option<Vec<u8>> = None;
    match (level, name) {
        (SOL_SOCKET, SO_REUSEADDR) => {
            if flag_value(&value)? && stored_flag(k, SOL_SOCKET, SO_EXCLUSIVEADDRUSE) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(alloc::vec![u8::from(flag_value(&value)?)]);
        }
        (SOL_SOCKET, SO_EXCLUSIVEADDRUSE) => {
            let on = flag_value(&value)?;
            if on && stored_flag(k, SOL_SOCKET, SO_REUSEADDR) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            keep = Some(alloc::vec![u8::from(on)]);
        }
        (SOL_SOCKET, SO_KEEPALIVE | SO_BROADCAST | SO_DEBUG | SO_DONTROUTE | SO_OOBINLINE | SO_CONDITIONAL_ACCEPT) => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(alloc::vec![u8::from(flag_value(&value)?)]);
        }
        (SOL_SOCKET, SO_SNDBUF | SO_RCVBUF) => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(int_of(&value)?.to_le_bytes().to_vec());
        }
        (SOL_SOCKET, SO_RCVTIMEO | SO_SNDTIMEO) => {
            let ms = int_of(&value)?;
            k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| if name == SO_RCVTIMEO { e.rcv_timeout = ms } else { e.snd_timeout = ms })
            })?;
        }
        (SOL_SOCKET, SO_LINGER) => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(value[..4].to_vec());
        }
        (SOL_SOCKET, SO_DONTLINGER) => {
            let off = flag_value(&value)?;
            let mut l = [0u8; 4];
            l[0] = u8::from(!off);
            sock.set_option(SockOption::Linger((!off).then_some(0))).map_err(status_of)?;
            k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| e.stored.insert((SOL_SOCKET, SO_LINGER), l.to_vec()))
            })?;
        }
        (SOL_SOCKET, SO_ERROR) => {
            // Settable, and read back as set (the value is only kept).
            let v = int_of(&value)?;
            k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.conn_error = (v != 0).then_some(v)))?;
        }
        (SOL_SOCKET, SO_RANDOMIZE_PORT) => {
            let on = flag_value(&value)?;
            if tcp && !v4 && bound(sock.as_ref()) {
                return Err(STATUS_INVALID_PARAMETER);
            }
            keep = Some(alloc::vec![u8::from(on)]);
        }
        (SOL_SOCKET, SO_UPDATE_ACCEPT_CONTEXT | SO_UPDATE_CONNECT_CONTEXT) => {
            // The socket AcceptEx / ConnectEx made is a connected socket, with addresses, from now on.
            k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| {
                    e.connected = true;
                    e.needs_update = false;
                })
            })?;
        }
        (IPPROTO_TCP, TCP_NODELAY) => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(alloc::vec![u8::from(flag_value(&value)?)]);
        }
        (IPPROTO_TCP, TCP_KEEPALIVE | TCP_KEEPINTVL | TCP_KEEPCNT) if tcp => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(int_of(&value)?.to_le_bytes().to_vec());
        }
        (IPPROTO_IP, IP_TTL) if v4 => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(int_of(&value)?.to_le_bytes().to_vec());
        }
        (IPPROTO_IP, IP_TOS) if v4 => keep = Some(int_of(&value)?.to_le_bytes().to_vec()),
        (IPPROTO_IP, IP_ECN) if v4 && !tcp => keep = Some(u32::from(int_of(&value)? != 0).to_le_bytes().to_vec()),
        (IPPROTO_IPV6, IPV6_UNICAST_HOPS) if !v4 => {
            stack_set(&sock, v4, level, name, &value)?;
            keep = Some(int_of(&value)?.to_le_bytes().to_vec());
        }
        (IPPROTO_IPV6, IPV6_V6ONLY) => {
            let on = flag_value(&value)?;
            if tcp && bound(sock.as_ref()) {
                // Learn ("IPV6_V6ONLY"): the option has to be set before the socket is bound.
                return Err(STATUS_INVALID_PARAMETER);
            }
            if !v4 {
                k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.v6only = on))?;
            }
            keep = Some(alloc::vec![u8::from(on)]);
        }
        (IPPROTO_IPV6, IPV6_ECN) if !v4 => {
            if tcp {
                return Err(STATUS_INVALID_PARAMETER);
            }
            keep = Some(u32::from(int_of(&value)? != 0).to_le_bytes().to_vec());
        }
        _ => return Err(NO_PROTOCOL_OPTION),
    }
    if let Some(v) = keep {
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.stored.insert((level, name), v)))?;
    }
    ok()
}

fn get_option<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let level = u32_in(input, 0)? as i32;
    // The event helper's own looks do not count as the program reading `SO_ERROR`.
    let internal = level & afd::INTERNAL_LOOK != 0;
    let level = level & !afd::INTERNAL_LOOK;
    let name = u32_in(input, 4)? as i32;
    let s = snap(k, cookie)?;
    let sock = s.sock()?.clone();
    let tcp = sock.protocol() == Protocol::Tcp;
    let v4 = s.family == AF_INET;
    if !matches!(level, SOL_SOCKET | IPPROTO_TCP | IPPROTO_IP | IPPROTO_IPV6) {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let (stored, timeouts, conn_error, connected) = k.with_state(|sys, _, _| {
        endpoint(sys, cookie)
            .map(|e| (e.stored.get(&(level, name)).cloned(), (e.rcv_timeout, e.snd_timeout), e.conn_error, e.connected))
    })?;
    let int = |v: u32| done(STATUS_SUCCESS, 4, v.to_le_bytes().to_vec());
    let byte = |v: bool| done(STATUS_SUCCESS, 1, alloc::vec![u8::from(v)]);
    let flag_of = |stored: &Option<Vec<u8>>| stored.as_ref().and_then(|v| v.first().copied()).is_some_and(|b| b != 0);
    let int_of_stored =
        |stored: &Option<Vec<u8>>, default: u32| stored.as_ref().and_then(|v| afd::read_u32(v, 0)).unwrap_or(default);
    let from_stack = |q: OptQuery| sock.get_option(q).map_err(status_of);
    match (level, name) {
        (SOL_SOCKET, SO_TYPE) => int(s.ty as u32),
        (SOL_SOCKET, SO_ACCEPTCONN) => {
            if !tcp {
                return Err(NO_PROTOCOL_OPTION);
            }
            int(u32::from(s.listening))
        }
        (SOL_SOCKET, SO_ERROR) => {
            // A failed connect keeps reading its error; a reset connection reads 0 (the error is
            // reported by the calls that fail with it).
            if let Some(e) = conn_error {
                if !internal {
                    k.with_state(|sys, _, _| endpoint(sys, cookie).map(|ep| ep.conn_error_read = true))?;
                }
                return int(e);
            }
            match sock.take_error() {
                Some(e) if connected || sock.established() => {
                    if let Some(b) = sticky_status(e, &sock) {
                        set_broken(k, cookie, b);
                    }
                    int(0)
                }
                Some(e) => {
                    let w = werr::wsa_of_status(status_of(e).0);
                    k.with_state(|sys, _, _| endpoint(sys, cookie).map(|ep| ep.store_conn_error(w, !internal)))?;
                    int(w)
                }
                None => {
                    // The stack had nothing: but another reader (the event helper, or the program
                    // on another thread) may have taken the error and stored it since the first
                    // look above.
                    let late = k.with_state(|sys, _, _| {
                        endpoint(sys, cookie).map(|ep| {
                            let late = ep.conn_error;
                            if late.is_some() && !internal {
                                ep.conn_error_read = true;
                            }
                            late
                        })
                    })?;
                    int(late.unwrap_or(0))
                }
            }
        }
        (SOL_SOCKET, SO_RCVTIMEO) => int(timeouts.0),
        (SOL_SOCKET, SO_SNDTIMEO) => int(timeouts.1),
        (SOL_SOCKET, SO_REUSEADDR) => int(u32::from(flag_of(&stored))),
        (SOL_SOCKET, SO_EXCLUSIVEADDRUSE) => int(u32::from(flag_of(&stored))),
        (SOL_SOCKET, SO_BROADCAST) => {
            if tcp {
                return Err(NO_PROTOCOL_OPTION);
            }
            int(from_stack(OptQuery::Broadcast)?)
        }
        (SOL_SOCKET, SO_SNDBUF) => match stored {
            Some(v) if v.len() >= 4 => done(STATUS_SUCCESS, 4, v[..4].to_vec()),
            _ => int(from_stack(OptQuery::SndBuf)?),
        },
        (SOL_SOCKET, SO_RCVBUF) => match stored {
            Some(v) if v.len() >= 4 => done(STATUS_SUCCESS, 4, v[..4].to_vec()),
            _ => int(from_stack(OptQuery::RcvBuf)?),
        },
        (SOL_SOCKET, SO_LINGER) => {
            if !tcp {
                return Err(NO_PROTOCOL_OPTION);
            }
            let mut v = stored.unwrap_or_default();
            v.resize(4, 0);
            done(STATUS_SUCCESS, 4, v)
        }
        (SOL_SOCKET, SO_DONTLINGER) => {
            let l = k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| e.stored.get(&(SOL_SOCKET, SO_LINGER)).cloned())
            })?;
            int(u32::from(l.and_then(|v| v.first().copied()).unwrap_or(0) == 0))
        }
        (SOL_SOCKET, SO_KEEPALIVE | SO_DONTROUTE) => byte(flag_of(&stored)),
        (SOL_SOCKET, SO_DEBUG | SO_OOBINLINE | SO_CONDITIONAL_ACCEPT) => int(u32::from(flag_of(&stored))),
        (SOL_SOCKET, SO_RANDOMIZE_PORT) => {
            if tcp {
                int(u32::from(flag_of(&stored)))
            } else {
                byte(flag_of(&stored))
            }
        }
        (SOL_SOCKET, SO_MAX_MSG_SIZE) => int(if tcp { u32::MAX } else { 65_507 }),
        (SOL_SOCKET, SO_CONNECT_TIME) => int(if tcp && (connected || sock.established()) { 0 } else { u32::MAX }),
        (SOL_SOCKET, SO_PROTOCOL_INFOW) => {
            let rec = poly_winsock::dup::encode(&poly_winsock::dup::Duplicate {
                handle: 0,
                family: s.family,
                ty: s.ty,
                protocol: s.protocol,
            });
            let mut v = rec.to_vec();
            v.resize(PROTOCOL_INFO_SIZE, 0);
            done(STATUS_SUCCESS, PROTOCOL_INFO_SIZE as u64, v)
        }
        (IPPROTO_TCP, TCP_NODELAY) => {
            if !tcp {
                return Err(STATUS_INVALID_PARAMETER);
            }
            byte(from_stack(OptQuery::NoDelay)? != 0)
        }
        (IPPROTO_TCP, TCP_KEEPALIVE | TCP_KEEPINTVL | TCP_KEEPCNT) if tcp => int(int_of_stored(&stored, 0)),
        (IPPROTO_IP, IP_TTL) if v4 => int(int_of_stored(&stored, 128)),
        (IPPROTO_IP, IP_TOS) if v4 => int(int_of_stored(&stored, 0)),
        (IPPROTO_IP, IP_ECN) if v4 && !tcp => int(int_of_stored(&stored, 0)),
        (IPPROTO_IPV6, IPV6_UNICAST_HOPS) if !v4 => int(int_of_stored(&stored, 128)),
        (IPPROTO_IPV6, IPV6_V6ONLY) => {
            let on = if v4 { stored.as_ref().is_none_or(|v| v.first().is_some_and(|b| *b != 0)) } else { s.v6only };
            if tcp { int(u32::from(on)) } else { byte(on) }
        }
        (IPPROTO_IPV6, IPV6_ECN) if !v4 => {
            if tcp {
                return Err(STATUS_INVALID_PARAMETER);
            }
            int(int_of_stored(&stored, 0))
        }
        _ => Err(NO_PROTOCOL_OPTION),
    }
}

/// The last handle to socket endpoint `e` went (`IoState::file_closed`):
/// calls waiting on it end, and the socket closes once they let go.
pub(crate) fn endpoint_closed(e: Endpoint) {
    if let Some(s) = e.sock {
        s.release();
    }
    if let Some(t) = e.twin {
        t.release();
    }
}
