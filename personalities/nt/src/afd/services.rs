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
use super::{Endpoint, NtSocket, OptQuery, PendIo, PendOp, SockOption, SockWait, status_of};
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
    family: i32,
    ty: i32,
    protocol: i32,
    recv_wait: SockWait,
    send_wait: SockWait,
    listening: bool,
    nonblocking: bool,
}

impl Snap {
    fn sock(&self) -> Result<&Arc<dyn NtSocket>> {
        self.sock.as_ref().ok_or(STATUS_INVALID_HANDLE)
    }
}

fn snap<K: NtKernel>(k: &mut K, cookie: u64) -> Result<Snap> {
    k.with_state(|sys, _, _| {
        let e = endpoint(sys, cookie)?;
        Ok(Snap {
            sock: e.sock.clone(),
            family: e.family,
            ty: e.ty,
            protocol: e.protocol,
            recv_wait: e.recv_wait(),
            send_wait: e.send_wait(),
            listening: e.listening,
            nonblocking: e.nonblocking,
        })
    })
}

fn af_of(d: Domain) -> i32 {
    match d {
        Domain::Inet => AF_INET,
        Domain::Inet6 => AF_INET6,
    }
}

/// A socket address from a request; `mismatch` is the status for one of
/// the other family.
fn socket_addr(bytes: &[u8], domain: Domain, mismatch: NtStatus) -> Result<SocketAddr> {
    let a = addr::decode(bytes).map_err(|e| if e == WSAEFAULT { BUFFER_TOO_SMALL } else { INVALID_ADDRESS })?;
    if addr::af_of(&a) != af_of(domain) {
        return Err(mismatch);
    }
    Ok(a)
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
        Ok(())
    })?;
    ok()
}

fn bind<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    // Learn ("bind"): an address of another family is WSAEFAULT.
    let a = socket_addr(input, sock.domain(), BUFFER_TOO_SMALL)?;
    sock.bind(a).map_err(status_of)?;
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
    sock.listen((backlog as usize).clamp(1, 128)).map_err(status_of)?;
    k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.listening = true))?;
    ok()
}

fn accept<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let h = afd::read_u64(input, 0).ok_or(BUFFER_TOO_SMALL)?;
    let s = snap(k, cookie)?;
    let sock = s.sock()?.clone();
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
    let (conn, peer) = sock.accept(s.recv_wait).map_err(status_of)?;
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
        // Learn ("accept"): the new socket has the listener's properties.
        e.nonblocking = s.nonblocking;
        e.rcv_timeout = rcv;
        e.snd_timeout = snd;
        e.stored = stored;
        Ok(())
    });
    placed?;
    let (b, len) = addr::encode(&peer);
    done(STATUS_SUCCESS, len as u64, b[..len].to_vec())
}

fn connect<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    if s.listening {
        return Err(STATUS_INVALID_PARAMETER);
    }
    let a = socket_addr(input, sock.domain(), INVALID_ADDRESS)?;
    if sock.kind() == SockType::Stream && (a.ip().is_unspecified() || a.port() == 0) {
        return Err(ADDRESS_COMPONENT);
    }
    // A blocking connect waits for the stack's own timeout, not
    // SO_SNDTIMEO (Learn: "connect" names no timeout option).
    let wait = if s.nonblocking { SockWait::Now } else { SockWait::Forever };
    sock.connect(a, wait).map_err(status_of)?;
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
    if sock.kind() == SockType::Stream && sock.peer_addr().is_err() {
        return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
    }
    sock.shutdown(how).map_err(status_of)?;
    ok()
}

fn name<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let peer = u32_in(input, 0)? != 0;
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    let a = if peer {
        sock.peer_addr().map_err(|_| NtStatus(werr::STATUS_INVALID_CONNECTION))?
    } else {
        // Learn ("getsockname"): WSAEINVAL before bind (or connect).
        match sock.local_addr() {
            Ok(a) if a.port() != 0 => a,
            _ => return Err(STATUS_INVALID_PARAMETER),
        }
    };
    let (b, len) = addr::encode(&a);
    done(STATUS_SUCCESS, len as u64, b[..len].to_vec())
}

// ----- send and receive

fn read_data<K: NtKernel>(k: &mut K, buf: u64, len: usize) -> Result<Vec<u8>> {
    let mut data = alloc::vec![0u8; len];
    if len > 0 {
        abi::read(k, buf, &mut data)?;
    }
    Ok(data)
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

fn send<K: NtKernel>(k: &mut K, file: (ObjectId, u64), n: Notify, input: &[u8]) -> Result<NtStatus> {
    let req = afd::SendReq::decode(input).ok_or(BUFFER_TOO_SMALL)?;
    let s = snap(k, file.1)?;
    let sock = s.sock()?.clone();
    let len = usize::try_from(req.len).map_err(|_| STATUS_INVALID_PARAMETER)?;
    let to = match req.to() {
        Some(b) if sock.kind() == SockType::Datagram => Some(socket_addr(b, sock.domain(), INVALID_ADDRESS)?),
        _ => None,
    };
    if sock.kind() == SockType::Datagram && to.is_none() && sock.peer_addr().is_err() {
        return Err(NtStatus(werr::STATUS_ADDRESS_NOT_ASSOCIATED));
    }
    let overlapped = req.flags & afd::FLAG_OVERLAPPED != 0;
    let wait = if overlapped { SockWait::Now } else { s.send_wait };
    let r = send_now(k, &sock, req.buf, len, to, wait);
    if overlapped && r == Err(status_of(Errno::EAGAIN)) && len <= MAX_CAPTURED_WRITE {
        let data = read_data(k, req.buf, len)?;
        let op = PendOp::Send { data, to };
        let irp_op = IrpOp::Write { offset: None, data: Vec::new(), key: 0 };
        return pend(k, file, n, sock, op, irp_op);
    }
    let count = r?;
    finish(k, file, &n, Done { status: STATUS_SUCCESS, info: count as u64, output: Vec::new() }, 0, 0)
}

/// Make an overlapped request pending, or run it in the calling thread
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
    let irp = k.with_state(|sys, _, _| -> Result<u64> {
        n.check_event(sys, &caller)?;
        let irp = n.irp(sys, &caller, file, false, irp_op)?;
        Ok(sys.io.park(irp))
    })?;
    match k.socket_pend(PendIo { irp, sock, op }) {
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
                Err(e) => (status_of(e), 0, None),
            }
        }
        PendOp::Send { data, to } => match io.sock.send(data, *to, SockWait::Forever) {
            Ok(n) => (STATUS_SUCCESS, n as u64, None),
            Err(e) => (status_of(e), 0, None),
        },
    }
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
    if stream && sock.peer_addr().is_err() && sock.readiness().0 & Readiness::READABLE.0 == 0 {
        return Err(NtStatus(werr::STATUS_INVALID_CONNECTION));
    }
    let wait = if overlapped { SockWait::Now } else { s.recv_wait };
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
        r => r.map_err(status_of)?,
    };
    let mut total = info.len;
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
        let (b, l) = addr::encode(&info.from);
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
pub(crate) fn read_file<K: NtKernel>(k: &mut K, h: u64, n: Notify, buf: u64, len: usize) -> Option<Result<NtStatus>> {
    let file = socket_file(k, h)?;
    let input = afd::RecvReq { flags: 0, buf, len: len as u64 }.encode();
    Some(recv(k, file, n, &input, 0, 0))
}

/// `NtWriteFile` on a socket handle: a send. `None` if `h` is not a socket.
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
    // Each entry's socket, or None for a handle that is not one.
    let socks: Vec<Option<(Arc<dyn NtSocket>, bool)>> = k.with_state(|sys, _, _| {
        entries
            .iter()
            .map(|&(h, _)| {
                let f = file_handle(sys, &caller, Handle(h), 0).ok()?;
                let e = endpoint(sys, f.1).ok()?;
                e.sock.clone().map(|s| (s, e.listening))
            })
            .collect()
    });
    let wanted: Vec<(Arc<dyn NtSocket>, Readiness)> = socks
        .iter()
        .zip(&entries)
        .filter_map(|(s, &(_, ev))| s.as_ref().map(|(s, _)| (s.clone(), interest_of(ev))))
        .collect();
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
            Some((sock, _)) => {
                let r = it.next().unwrap_or_default();
                device_bits(sock.as_ref(), r) & (ev | 0x7)
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

/// The device's poll bits for a socket's readiness. A stream that never
/// connected (and does not listen) is not "hung up" for Winsock, though
/// the stack reports it so.
fn device_bits(sock: &dyn NtSocket, r: Readiness) -> u32 {
    let connected = sock.kind() == SockType::Datagram || sock.peer_addr().is_ok();
    let error = r.contains(Readiness::ERROR);
    let hangup = (r.contains(Readiness::HANGUP) && (connected || error)) || r.contains(Readiness::READ_HANGUP);
    let readable = r.contains(Readiness::READABLE) && (connected || !error);
    let writable = r.contains(Readiness::WRITABLE) && connected;
    poly_winsock::poll::from_readiness(readable, writable, error, hangup)
}

// ----- options

fn int_value(input: &[u8]) -> Result<u32> {
    u32_in(input, 8)
}

fn set_option<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let level = u32_in(input, 0)? as i32;
    let name = u32_in(input, 4)? as i32;
    let value = input.get(8..).unwrap_or(&[]).to_vec();
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    let tcp = sock.protocol() == Protocol::Tcp;
    let v4 = sock.domain() == Domain::Inet;
    let flag = |input: &[u8]| -> Result<bool> {
        // Learn: a BOOL option may be passed as a single byte, too.
        match input.get(8..) {
            Some([b]) => Ok(*b != 0),
            _ => int_value(input).map(|v| v != 0),
        }
    };
    let mut store = true;
    match (level, name) {
        (SOL_SOCKET, SO_REUSEADDR) => sock.set_option(SockOption::ReuseAddr(flag(input)?)).map_err(status_of)?,
        (SOL_SOCKET, SO_KEEPALIVE) => {
            let on = flag(input)?;
            if tcp {
                sock.set_option(SockOption::KeepAlive(on)).map_err(status_of)?;
            }
        }
        (SOL_SOCKET, SO_BROADCAST) => {
            let on = flag(input)?;
            if !tcp {
                sock.set_option(SockOption::Broadcast(on)).map_err(status_of)?;
            }
        }
        (SOL_SOCKET, SO_SNDBUF) => {
            let _ = sock.set_option(SockOption::SndBuf(int_value(input)? as usize));
        }
        (SOL_SOCKET, SO_RCVBUF) => {
            let _ = sock.set_option(SockOption::RcvBuf(int_value(input)? as usize));
        }
        (SOL_SOCKET, SO_RCVTIMEO | SO_SNDTIMEO) => {
            let ms = int_value(input)?;
            k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| if name == SO_RCVTIMEO { e.rcv_timeout = ms } else { e.snd_timeout = ms })
            })?;
            store = false;
        }
        (SOL_SOCKET, SO_LINGER) => {
            let b = input.get(8..12).ok_or(BUFFER_TOO_SMALL)?;
            let on = u16::from_le_bytes([b[0], b[1]]) != 0;
            let secs = u16::from_le_bytes([b[2], b[3]]);
            sock.set_option(SockOption::Linger(on.then_some(u32::from(secs)))).map_err(status_of)?;
        }
        (SOL_SOCKET, SO_DONTLINGER) => {
            let off = flag(input)?;
            let mut l = [0u8; 4];
            l[0] = u8::from(!off);
            sock.set_option(SockOption::Linger((!off).then_some(0))).map_err(status_of)?;
            k.with_state(|sys, _, _| {
                endpoint(sys, cookie).map(|e| e.stored.insert((SOL_SOCKET, SO_LINGER), l.to_vec()))
            })?;
            store = false;
        }
        (SOL_SOCKET, SO_DEBUG | SO_DONTROUTE | SO_OOBINLINE | SO_EXCLUSIVEADDRUSE | SO_CONDITIONAL_ACCEPT) => {
            let _ = flag(input)?;
        }
        (IPPROTO_TCP, TCP_NODELAY) if tcp => sock.set_option(SockOption::NoDelay(flag(input)?)).map_err(status_of)?,
        (IPPROTO_TCP, TCP_KEEPALIVE) if tcp => {
            sock.set_option(SockOption::KeepIdle(int_value(input)?)).map_err(status_of)?
        }
        (IPPROTO_TCP, TCP_KEEPINTVL) if tcp => {
            sock.set_option(SockOption::KeepIntvl(int_value(input)?)).map_err(status_of)?
        }
        (IPPROTO_TCP, TCP_KEEPCNT) if tcp => {
            sock.set_option(SockOption::KeepCnt(int_value(input)?)).map_err(status_of)?
        }
        (IPPROTO_IP, IP_TTL) if v4 => sock.set_option(SockOption::Ttl(ttl(int_value(input)?)?)).map_err(status_of)?,
        (IPPROTO_IP, IP_TOS) if v4 => {
            let _ = int_value(input)?;
        }
        (IPPROTO_IPV6, IPV6_UNICAST_HOPS) if !v4 => {
            sock.set_option(SockOption::Ttl(ttl(int_value(input)?)?)).map_err(status_of)?
        }
        // The stack's IPv6 sockets carry only IPv6, whatever this says
        // (to be done: dual-stack sockets).
        (IPPROTO_IPV6, IPV6_V6ONLY) if !v4 => {
            let _ = flag(input)?;
        }
        _ => return Err(NO_PROTOCOL_OPTION),
    }
    if store {
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.stored.insert((level, name), value)))?;
    }
    ok()
}

fn ttl(v: u32) -> Result<u8> {
    u8::try_from(v).ok().filter(|&t| t > 0).ok_or(STATUS_INVALID_PARAMETER)
}

fn get_option<K: NtKernel>(k: &mut K, cookie: u64, input: &[u8]) -> Result<Option<Done>> {
    let level = u32_in(input, 0)? as i32;
    let name = u32_in(input, 4)? as i32;
    let s = snap(k, cookie)?;
    let sock = s.sock()?;
    let tcp = sock.protocol() == Protocol::Tcp;
    let v4 = sock.domain() == Domain::Inet;
    let stored = k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| e.stored.get(&(level, name)).cloned()))?;
    let (timeouts, ty) =
        k.with_state(|sys, _, _| endpoint(sys, cookie).map(|e| ((e.rcv_timeout, e.snd_timeout), e.ty)))?;
    let int = |v: u32| done(STATUS_SUCCESS, 4, v.to_le_bytes().to_vec());
    let flag_of =
        |stored: &Option<Vec<u8>>| stored.as_ref().and_then(|v| v.first().copied()).map_or(0, |b| u32::from(b != 0));
    let from_stack = |q: OptQuery| sock.get_option(q).map_err(status_of);
    match (level, name) {
        (SOL_SOCKET, SO_TYPE) => int(ty as u32),
        (SOL_SOCKET, SO_ACCEPTCONN) => int(u32::from(s.listening)),
        (SOL_SOCKET, SO_ERROR) => {
            let e = sock.take_error().map_or(0, |e| werr::wsa_of_status(status_of(e).0));
            int(e)
        }
        (SOL_SOCKET, SO_RCVTIMEO) => int(timeouts.0),
        (SOL_SOCKET, SO_SNDTIMEO) => int(timeouts.1),
        (SOL_SOCKET, SO_REUSEADDR) => int(from_stack(OptQuery::ReuseAddr)?),
        (SOL_SOCKET, SO_BROADCAST) => int(if tcp { 0 } else { from_stack(OptQuery::Broadcast)? }),
        (SOL_SOCKET, SO_SNDBUF) => match stored {
            Some(v) if v.len() >= 4 => done(STATUS_SUCCESS, 4, v[..4].to_vec()),
            _ => int(from_stack(OptQuery::SndBuf)?),
        },
        (SOL_SOCKET, SO_RCVBUF) => match stored {
            Some(v) if v.len() >= 4 => done(STATUS_SUCCESS, 4, v[..4].to_vec()),
            _ => int(from_stack(OptQuery::RcvBuf)?),
        },
        (SOL_SOCKET, SO_LINGER) => {
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
        (
            SOL_SOCKET,
            SO_KEEPALIVE | SO_DEBUG | SO_DONTROUTE | SO_OOBINLINE | SO_EXCLUSIVEADDRUSE | SO_CONDITIONAL_ACCEPT,
        ) => int(flag_of(&stored)),
        (IPPROTO_TCP, TCP_NODELAY) if tcp => int(from_stack(OptQuery::NoDelay)?),
        (IPPROTO_TCP, TCP_KEEPALIVE | TCP_KEEPINTVL | TCP_KEEPCNT) if tcp => {
            int(stored.and_then(|v| afd::read_u32(&v, 0)).unwrap_or(0))
        }
        (IPPROTO_IP, IP_TTL) if v4 => int(from_stack(OptQuery::Ttl)?),
        (IPPROTO_IP, IP_TOS) if v4 => int(stored.and_then(|v| afd::read_u32(&v, 0)).unwrap_or(0)),
        (IPPROTO_IPV6, IPV6_UNICAST_HOPS) if !v4 => int(from_stack(OptQuery::Ttl)?),
        // Poly's IPv6 sockets are IPv6-only.
        (IPPROTO_IPV6, IPV6_V6ONLY) if !v4 => int(1),
        _ => Err(NO_PROTOCOL_OPTION),
    }
}

/// The last handle to socket endpoint `e` went (`IoState::file_closed`):
/// calls waiting on it end, and the socket closes once they let go.
pub(crate) fn endpoint_closed(e: Endpoint) {
    if let Some(s) = e.sock {
        s.release();
    }
}
