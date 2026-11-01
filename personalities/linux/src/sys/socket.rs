//! The socket calls (ADR 0048): socket(2), socketpair(2), bind(2),
//! listen(2), accept(2) / accept4(2), connect(2), getsockname(2),
//! getpeername(2), sendto(2), recvfrom(2), sendmsg(2), recvmsg(2),
//! sendmmsg(2), recvmmsg(2), shutdown(2), setsockopt(2), getsockopt(2).
//!
//! The handlers check flags and lengths, decode `sockaddr`s, `msghdr`s,
//! control messages and option values ([`crate::net::abi`]), move the data
//! between user memory and kernel buffers, and leave the sockets to the
//! kernel ([`LinuxNet`]). Two rules of signal(7) and socket(7) live here:
//!
//! - A blocking call a signal interrupted is restarted under `SA_RESTART`
//!   (`RESTART_IF_SA_RESTART`), unless the socket has a timeout for that
//!   direction (`SO_RCVTIMEO` for accept and the receives, `SO_SNDTIMEO`
//!   for connect and the sends): then it fails with `EINTR`.
//! - `EPIPE` from a send raises `SIGPIPE` for the calling thread unless
//!   `MSG_NOSIGNAL` was given (socket(7), "Signals").
//!
//! Orders of checks the pages leave open are Poly's, to be confirmed by
//! the diff runner: flags before descriptors, descriptors before
//! addresses; `EFAULT` for an address buffer is reported after the call
//! did its work (accept: the connection is then lost, as in other
//! systems).

use crate::abi::signal::{SI_KERNEL, SIGPIPE};
use crate::abi::{IOV_MAX, Iovec, MAX_RW_COUNT, SigInfo};
use crate::args::Args;
use crate::errno::{Errno, SysResult};
use crate::fileio::NewFd;
use crate::kernel::{Clock, LinuxKernel, Target, UserPtr};
use crate::net::abi::{self, MsgHdr, SockAddr, Ucred};
use crate::net::{Family, Kind, LinuxNet, OptName, OptValue, RecvMsg, RecvReq, SendMsg, Shutdown, SockOpt, SockType};
use crate::signal::deliver::send as send_signal;
use crate::sys::io::bounce;
use crate::table::Handler;
use crate::table::nr;
use alloc::vec;
use alloc::vec::Vec;

/// The socket calls, registered in [`crate::table::GROUPS`].
pub const CALLS: &[(u64, Handler)] = &[
    (nr::SOCKET, socket),
    (nr::CONNECT, connect),
    (nr::ACCEPT, accept),
    (nr::SENDTO, sendto),
    (nr::RECVFROM, recvfrom),
    (nr::SENDMSG, sendmsg),
    (nr::RECVMSG, recvmsg),
    (nr::SHUTDOWN, shutdown),
    (nr::BIND, bind),
    (nr::LISTEN, listen),
    (nr::GETSOCKNAME, getsockname),
    (nr::GETPEERNAME, getpeername),
    (nr::SOCKETPAIR, socketpair),
    (nr::SETSOCKOPT, setsockopt),
    (nr::GETSOCKOPT, getsockopt),
    (nr::ACCEPT4, accept4),
    (nr::RECVMMSG, recvmmsg),
    (nr::SENDMMSG, sendmmsg),
];

/// Streams are sent in chunks of this many bytes (each a kernel buffer).
const STREAM_CHUNK: usize = 64 * 1024;
/// The largest message a datagram send or receive moves at once.
const MAX_MESSAGE: usize = 1 << 20;
/// The largest control buffer sendmsg(2) takes (`ENOBUFS` above; Poly's
/// limit, standing in for Linux's `optmem_max`).
const MAX_CONTROL: usize = 64 * 1024;
/// `UIO_MAXIOV`: the most messages one sendmmsg / recvmmsg moves.
const MAX_MMSG: u32 = 1024;
/// `SOMAXCONN` (listen(2)).
const SOMAXCONN: u32 = 4096;
/// The largest value `SO_SNDBUF` / `SO_RCVBUF` take before doubling
/// (standing in for `wmem_max` / `rmem_max`, socket(7)).
const MAX_BUFFER: usize = 212_992;

/// The kernel's sockets, `ENOSYS` without them.
fn net(k: &mut dyn LinuxKernel) -> Result<&mut dyn LinuxNet, Errno> {
    k.net().ok_or(Errno::ENOSYS)
}

/// A descriptor argument: negative ones are `EBADF`.
fn fd_arg(a: &Args, i: usize) -> Result<i32, Errno> {
    let fd = a.fd(i);
    if fd < 0 { Err(Errno::EBADF) } else { Ok(fd) }
}

/// A call that a signal interrupted: restarted under `SA_RESTART`, unless
/// the socket has a timeout in that direction (signal(7)).
fn interrupted(k: &mut dyn LinuxKernel, fd: i32, send: bool, e: Errno) -> Errno {
    if e != Errno::EINTR {
        return e;
    }
    if k.net().is_some_and(|n| n.timed(fd, send)) { Errno::EINTR } else { Errno::RESTART_IF_SA_RESTART }
}

/// `SIGPIPE` for the calling thread after `EPIPE` (socket(7)).
fn sigpipe(k: &mut dyn LinuxKernel) {
    let tid = k.tid();
    let _ = send_signal(k, Target::Thread(tid), Some(tid), None, SigInfo::new(SIGPIPE, SI_KERNEL));
}

/// socket(2)'s checks on `(domain, type, protocol)`; returns the kind
/// and the descriptor flags.
pub fn check_kind(domain: i32, ty: u32, protocol: i32) -> Result<(Kind, NewFd), Errno> {
    let flags = ty & !abi::SOCK_TYPE_MASK;
    if flags & !(abi::SOCK_NONBLOCK | abi::SOCK_CLOEXEC) != 0 {
        return Err(Errno::EINVAL);
    }
    let family = u16::try_from(domain).ok().and_then(Family::from_linux).ok_or(Errno::EAFNOSUPPORT)?;
    let st = match SockType::from_linux(ty & abi::SOCK_TYPE_MASK) {
        Some(st) => st,
        // A type number below SOCK_MAX (`linux/net.h`: 10) that Unix
        // sockets do not have (socket's spec sheet: 9).
        None if family == Family::Unix && ty & abi::SOCK_TYPE_MASK < 10 => return Err(Errno::ESOCKTNOSUPPORT),
        None => return Err(Errno::EINVAL),
    };
    let protocol = match (family, st) {
        (Family::Unix, SockType::Raw) => return Err(Errno::ESOCKTNOSUPPORT),
        // unix(7): "EPROTONOSUPPORT Passed protocol is not AF_UNIX".
        (Family::Unix, _) if protocol == 0 || protocol == i32::from(abi::AF_UNIX) => 0,
        (Family::Unix, _) => return Err(Errno::EPROTONOSUPPORT),
        (Family::Inet | Family::Inet6, SockType::Stream) if protocol == 0 || protocol == abi::IPPROTO_TCP => {
            abi::IPPROTO_TCP
        }
        (Family::Inet | Family::Inet6, SockType::Datagram) if protocol == 0 || protocol == abi::IPPROTO_UDP => {
            abi::IPPROTO_UDP
        }
        // Ping sockets (icmp(7)).
        (Family::Inet, SockType::Datagram) if protocol == abi::IPPROTO_ICMP => protocol,
        (Family::Inet6, SockType::Datagram) if protocol == abi::IPPROTO_ICMPV6 => protocol,
        (Family::Inet | Family::Inet6, SockType::SeqPacket | SockType::Raw) => {
            return Err(Errno::ESOCKTNOSUPPORT);
        }
        (Family::Inet | Family::Inet6, _) => return Err(Errno::EPROTONOSUPPORT),
        (Family::Netlink, SockType::Raw | SockType::Datagram) if protocol == abi::NETLINK_ROUTE => protocol,
        (Family::Netlink, SockType::Raw | SockType::Datagram) => return Err(Errno::EPROTONOSUPPORT),
        (Family::Netlink, _) => return Err(Errno::ESOCKTNOSUPPORT),
    };
    Ok((Kind { family, ty: st, protocol }, NewFd::from_flags(flags)))
}

/// socket(domain, type, protocol)
pub fn socket(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (kind, new) = check_kind(a.int(0), a.uint(1), a.int(2))?;
    net(k)?.socket(kind, new).map(|fd| fd as u64)
}

/// socketpair(domain, type, protocol, sv): `AF_UNIX` only
/// (socketpair(2): `EOPNOTSUPP` "The specified protocol does not support
/// creation of socket pairs").
pub fn socketpair(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let (kind, new) = check_kind(a.int(0), a.uint(1), a.int(2))?;
    if kind.family != Family::Unix {
        return Err(Errno::EOPNOTSUPP);
    }
    let [x, y] = net(k)?.socketpair(kind, new)?;
    let mut out = [0u8; 8];
    out[..4].copy_from_slice(&x.to_le_bytes());
    out[4..].copy_from_slice(&y.to_le_bytes());
    if let Err(e) = k.copy_to_user(a.ptr(3), &out) {
        let _ = k.close(x);
        let _ = k.close(y);
        return Err(e);
    }
    Ok(0)
}

/// Read a `sockaddr` of `len` bytes: `EINVAL` for a negative length or
/// one above `sizeof(struct sockaddr_storage)`, `EFAULT`.
fn read_addr(k: &mut dyn LinuxKernel, ptr: UserPtr, len: i32) -> Result<SockAddr, Errno> {
    if len < 0 || len as usize > abi::SOCKADDR_MAX {
        return Err(Errno::EINVAL);
    }
    let mut b = vec![0u8; len as usize];
    k.copy_from_user(ptr, &mut b)?;
    SockAddr::decode(&b)
}

/// Store `addr` at `ptr` (at most the `*len_ptr` bytes the caller has
/// room for) and its real length at `len_ptr`; nothing if `ptr` is null.
fn write_addr(k: &mut dyn LinuxKernel, ptr: UserPtr, len_ptr: UserPtr, addr: &SockAddr) -> Result<(), Errno> {
    if ptr.0 == 0 {
        return Ok(());
    }
    let mut lb = [0u8; 4];
    k.copy_from_user(len_ptr, &mut lb)?;
    let room = i32::from_le_bytes(lb);
    if room < 0 {
        return Err(Errno::EINVAL);
    }
    let bytes = addr.encode();
    let n = bytes.len().min(room as usize);
    k.copy_to_user(ptr, &bytes[..n])?;
    k.copy_to_user(len_ptr, &(bytes.len() as u32).to_le_bytes())
}

/// bind(fd, addr, addrlen)
pub fn bind(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let mut addr = read_addr(k, a.ptr(1), a.int(2))?;
    if addr == SockAddr::Unspec {
        addr = unspec_bind(k, fd, a.ptr(1), a.int(2))?;
    }
    crate::sys::landlock::check_socket(k, fd, &addr, false)?;
    net(k)?.bind(fd, &addr).map(|()| 0)
}

/// bind's spec sheet: `AF_UNSPEC` with `INADDR_ANY` is accepted on an
/// `AF_INET` socket (as that address with the given port); any other
/// address is `EAFNOSUPPORT`. Other sockets get the address as it is.
fn unspec_bind(k: &mut dyn LinuxKernel, fd: i32, ptr: UserPtr, len: i32) -> Result<SockAddr, Errno> {
    if net(k)?.getsockopt(fd, OptName::Domain)? != OptValue::Int(i32::from(abi::AF_INET)) {
        return Ok(SockAddr::Unspec);
    }
    if (len as usize) < abi::SOCKADDR_IN_SIZE {
        return Err(Errno::EINVAL);
    }
    let mut b = [0u8; abi::SOCKADDR_IN_SIZE];
    k.copy_from_user(ptr, &mut b)?;
    if b[4..8] != [0; 4] {
        return Err(Errno::EAFNOSUPPORT);
    }
    let port = u16::from_be_bytes([b[2], b[3]]);
    Ok(SockAddr::Inet(core::net::SocketAddrV4::new(core::net::Ipv4Addr::UNSPECIFIED, port)))
}

/// listen(fd, backlog): a backlog above `SOMAXCONN` is capped (listen(2));
/// a negative one is taken as the largest.
pub fn listen(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let backlog = a.int(1);
    let backlog = if backlog < 0 { SOMAXCONN } else { (backlog as u32).min(SOMAXCONN) };
    net(k)?.listen(fd, backlog).map(|()| 0)
}

/// accept(fd, addr, addrlen)
pub fn accept(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    do_accept(k, a, 0)
}

/// accept4(fd, addr, addrlen, flags): `SOCK_NONBLOCK`, `SOCK_CLOEXEC`
/// (`EINVAL` otherwise).
pub fn accept4(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(3);
    if flags & !(abi::SOCK_NONBLOCK | abi::SOCK_CLOEXEC) != 0 {
        return Err(Errno::EINVAL);
    }
    do_accept(k, a, flags)
}

fn do_accept(k: &mut dyn LinuxKernel, a: &Args, flags: u32) -> SysResult {
    let fd = fd_arg(a, 0)?;
    // accept(2): "EINVAL ... addrlen is invalid (e.g., is negative)",
    // checked before waiting.
    if a.ptr(1).0 != 0 {
        let mut lb = [0u8; 4];
        k.copy_from_user(a.ptr(2), &mut lb)?;
        if i32::from_le_bytes(lb) < 0 {
            // A non-blocking listener with nothing waiting says so first
            // (accept4's spec sheet).
            let nonblock = crate::sys::path::fs(k)
                .and_then(|fs| fs.status_flags(fd))
                .is_ok_and(|f| f & crate::fileio::flags::O_NONBLOCK != 0);
            let mut p = [crate::fileio::PollFd { fd, events: crate::fileio::events::POLLIN, revents: 0 }];
            let idle = k.file_io().is_some_and(|io| io.poll(&mut p, crate::fileio::WaitTime::Poll, false) == Ok(0));
            return Err(if nonblock && idle { Errno::EAGAIN } else { Errno::EINVAL });
        }
    }
    let r = net(k)?.accept(fd, NewFd::from_flags(flags));
    let (new, peer) = r.map_err(|e| interrupted(k, fd, false, e))?;
    if let Err(e) = write_addr(k, a.ptr(1), a.ptr(2), &peer) {
        let _ = k.close(new);
        return Err(e);
    }
    Ok(new as u64)
}

/// connect(fd, addr, addrlen)
pub fn connect(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let addr = read_addr(k, a.ptr(1), a.int(2))?;
    crate::sys::landlock::check_socket(k, fd, &addr, true)?;
    let r = net(k)?.connect(fd, &addr);
    r.map(|()| 0).map_err(|e| interrupted(k, fd, true, e))
}

/// getsockname(fd, addr, addrlen)
pub fn getsockname(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    name(k, a, false)
}

/// getpeername(fd, addr, addrlen)
pub fn getpeername(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    name(k, a, true)
}

fn name(k: &mut dyn LinuxKernel, a: &Args, peer: bool) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let addr = net(k)?.name(fd, peer)?;
    // getsockname(2): the buffer pointer must be valid even for a
    // zero-length result.
    if a.ptr(1).0 == 0 {
        return Err(Errno::EFAULT);
    }
    write_addr(k, a.ptr(1), a.ptr(2), &addr)?;
    Ok(0)
}

/// shutdown(fd, how)
pub fn shutdown(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let how = match a.int(1) {
        abi::SHUT_RD => Shutdown::Read,
        abi::SHUT_WR => Shutdown::Write,
        abi::SHUT_RDWR => Shutdown::Both,
        _ => return Err(Errno::EINVAL),
    };
    net(k)?.shutdown(fd, how).map(|()| 0)
}

// ---- data ------------------------------------------------------------------

/// Read and check an iovec array (readv(2)'s rules); returns the vectors
/// and their byte total.
fn load_iovecs(k: &mut dyn LinuxKernel, iov: UserPtr, count: u64) -> Result<(Vec<Iovec>, u64), Errno> {
    if count > u64::from(IOV_MAX) {
        return Err(Errno::EMSGSIZE);
    }
    let mut raw = bounce(count as usize * Iovec::SIZE)?;
    if count > 0 {
        k.copy_from_user(iov, &mut raw)?;
    }
    let mut total: i64 = 0;
    let mut out = Vec::with_capacity(count as usize);
    for e in raw.chunks_exact(Iovec::SIZE) {
        let v = Iovec::from_bytes(e.try_into().expect("whole entries"));
        let len = i64::try_from(v.len).map_err(|_| Errno::EINVAL)?;
        total = total.checked_add(len).ok_or(Errno::EINVAL)?;
        out.push(v);
    }
    Ok((out, (total as u64).min(MAX_RW_COUNT)))
}

/// Copy the first `n` bytes the iovecs describe, from `skip` on, out of
/// user memory.
fn gather(k: &mut dyn LinuxKernel, iovs: &[Iovec], skip: usize, n: usize) -> Result<Vec<u8>, Errno> {
    let mut out = bounce(n)?;
    let (mut done, mut pos) = (0usize, 0usize);
    for v in iovs {
        if done == n {
            break;
        }
        let len = v.len as usize;
        if pos + len <= skip {
            pos += len;
            continue;
        }
        let from = skip.saturating_sub(pos);
        let take = (len - from).min(n - done);
        k.copy_from_user(UserPtr(v.base + from as u64), &mut out[done..done + take])?;
        done += take;
        pos += len;
    }
    Ok(out)
}

/// Copy `data` into the user buffers the iovecs describe, in order.
fn scatter(k: &mut dyn LinuxKernel, iovs: &[Iovec], data: &[u8]) -> Result<(), Errno> {
    let mut done = 0;
    for v in iovs {
        if done == data.len() {
            break;
        }
        let take = (v.len as usize).min(data.len() - done);
        k.copy_to_user(UserPtr(v.base), &data[done..done + take])?;
        done += take;
    }
    Ok(())
}

/// Control data of a sendmsg: descriptors and credentials.
#[derive(Default)]
struct Control {
    rights: Vec<i32>,
    creds: Option<Ucred>,
}

/// Parse sendmsg(2)'s control buffer: `SCM_RIGHTS` (at most
/// `SCM_MAX_FD`, one message of it) and `SCM_CREDENTIALS` (checked as
/// unix(7) says: the caller's own process id and real or effective ids,
/// unless it is root: `EPERM`); anything else at `SOL_SOCKET` is `EINVAL`,
/// other levels are ignored.
fn parse_control(k: &mut dyn LinuxKernel, ptr: UserPtr, len: u64) -> Result<Control, Errno> {
    let mut c = Control::default();
    if ptr.0 == 0 || len == 0 {
        return Ok(c);
    }
    if len as usize > MAX_CONTROL {
        return Err(Errno::ENOBUFS);
    }
    let mut buf = bounce(len as usize)?;
    k.copy_from_user(ptr, &mut buf)?;
    let mut seen_rights = false;
    for m in abi::parse_cmsgs(&buf)? {
        if m.level != abi::SOL_SOCKET {
            continue;
        }
        match m.ty {
            abi::SCM_RIGHTS => {
                if seen_rights || m.data.len() % 4 != 0 {
                    return Err(Errno::EINVAL);
                }
                seen_rights = true;
                c.rights = m.data.chunks_exact(4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
                if c.rights.len() > 253 {
                    return Err(Errno::EINVAL);
                }
            }
            abi::SCM_CREDENTIALS => {
                if m.data.len() != Ucred::SIZE || c.creds.is_some() {
                    return Err(Errno::EINVAL);
                }
                let u = Ucred::from_bytes(&m.data);
                // unix(7): the caller's own process id (any with
                // CAP_SYS_ADMIN), its real, effective or saved user id (any
                // with CAP_SETUID) and group id (any with CAP_SETGID).
                // The ids are the caller's namespaces' (ADR 0054).
                let me = crate::cred::current(k);
                let own = crate::ns::pid::view(k, k.pid());
                let pid_ok = u.pid == own || me.capable(crate::cred::cap::SYS_ADMIN);
                let (uid, gid) = (me.kuid(u.uid), me.kgid(u.gid));
                let uid_ok = uid.is_ok_and(|id| [me.uid, me.euid, me.suid].contains(&id))
                    || me.capable(crate::cred::cap::SETUID);
                let gid_ok = gid.is_ok_and(|id| [me.gid, me.egid, me.sgid].contains(&id))
                    || me.capable(crate::cred::cap::SETGID);
                if !(pid_ok && uid_ok && gid_ok) {
                    return Err(Errno::EPERM);
                }
                // Another process's id must name one (sendmsg's spec
                // sheet: ESRCH).
                if u.pid != own {
                    let pid = crate::ns::pid::find(k, u.pid)?;
                    crate::signal::deliver::with_target(k, Target::Process(pid), |_, _| ())?;
                }
                c.creds = Some(u);
            }
            _ => return Err(Errno::EINVAL),
        }
    }
    Ok(c)
}

/// One send of the `total` bytes the iovecs describe. Streams go in
/// chunks (the first carries the control data); a datagram goes whole.
#[allow(clippy::too_many_arguments)] // the parts of a msghdr
fn send_data(
    k: &mut dyn LinuxKernel,
    fd: i32,
    iovs: &[Iovec],
    total: u64,
    to: Option<SockAddr>,
    flags: u32,
    control: Control,
) -> Result<usize, Errno> {
    if flags & abi::MSG_OOB != 0 {
        return Err(Errno::EOPNOTSUPP);
    }
    let dontwait = flags & abi::MSG_DONTWAIT != 0;
    let stream = matches!(net(k)?.getsockopt(fd, OptName::Type)?, OptValue::Int(t) if t == abi::SOCK_STREAM as i32);
    let result = if stream {
        let total = total as usize;
        let mut done = 0usize;
        let mut control = Some(control);
        let mut first = true;
        loop {
            let n = (total - done).min(STREAM_CHUNK);
            let data = match gather(k, iovs, done, n) {
                Ok(d) => d,
                Err(e) if done == 0 => break Err(e),
                Err(_) => break Ok(done),
            };
            let c = control.take().unwrap_or_default();
            let msg = SendMsg { data: &data, to: to.clone(), dontwait, rights: c.rights, creds: c.creds };
            match net(k)?.send(fd, &msg) {
                Ok(sent) => {
                    done += sent;
                    if sent < n || done >= total {
                        break Ok(done);
                    }
                }
                Err(e) if done == 0 && first => break Err(e),
                Err(_) => break Ok(done),
            }
            first = false;
        }
    } else {
        if total as usize > MAX_MESSAGE {
            return Err(Errno::EMSGSIZE);
        }
        let data = gather(k, iovs, 0, total as usize)?;
        let msg = SendMsg { data: &data, to, dontwait, rights: control.rights, creds: control.creds };
        net(k)?.send(fd, &msg)
    };
    match result {
        Err(Errno::EPIPE) if flags & abi::MSG_NOSIGNAL == 0 => {
            sigpipe(k);
            Err(Errno::EPIPE)
        }
        Err(e) => Err(interrupted(k, fd, true, e)),
        ok => ok,
    }
}

/// sendto(fd, buf, len, flags, dest_addr, addrlen)
pub fn sendto(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let len = a.ulong(2).min(MAX_RW_COUNT);
    let flags = a.uint(3);
    let to = if a.ptr(4).0 == 0 { None } else { Some(read_addr(k, a.ptr(4), a.int(5))?) };
    let iov = [Iovec { base: a.ptr(1).0, len }];
    send_data(k, fd, &iov, len, to, flags, Control::default()).map(|n| n as u64)
}

/// Read a `struct msghdr`.
fn read_msghdr(k: &mut dyn LinuxKernel, ptr: UserPtr) -> Result<MsgHdr, Errno> {
    let mut b = [0u8; MsgHdr::SIZE];
    k.copy_from_user(ptr, &mut b)?;
    Ok(MsgHdr::from_bytes(&b))
}

fn sendmsg_one(k: &mut dyn LinuxKernel, fd: i32, msg_ptr: UserPtr, flags: u32) -> Result<usize, Errno> {
    let m = read_msghdr(k, msg_ptr)?;
    let to = if m.name == 0 { None } else { Some(read_addr(k, UserPtr(m.name), m.namelen as i32)?) };
    let (iovs, total) = load_iovecs(k, UserPtr(m.iov), m.iovlen)?;
    let control = parse_control(k, UserPtr(m.control), m.controllen)?;
    send_data(k, fd, &iovs, total, to, flags, control)
}

/// sendmsg(fd, msg, flags)
pub fn sendmsg(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    sendmsg_one(k, fd, a.ptr(1), a.uint(2)).map(|n| n as u64)
}

/// sendmmsg(fd, msgvec, vlen, flags): the messages one by one, each
/// `msg_len` set; stops at the first error after a message went (that
/// count is returned) (sendmmsg(2)).
///
/// sendmmsg's spec sheet: `MSG_CMSG_COMPAT` is `EINVAL` before anything
/// else, then the descriptor (`EBADF`, `ENOTSOCK`), then a `vlen` of 0
/// returns 0. A sent message whose `msg_len` cannot be stored does not
/// count (`EFAULT` if it was the first).
pub fn sendmmsg(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let flags = a.uint(3);
    if flags & abi::MSG_CMSG_COMPAT != 0 {
        return Err(Errno::EINVAL);
    }
    let fd = fd_arg(a, 0)?;
    net(k)?.getsockopt(fd, OptName::Type)?;
    let vlen = a.uint(2).min(MAX_MMSG);
    let mut sent = 0u32;
    while sent < vlen {
        let at = a.ptr(1).0 + u64::from(sent) * abi::MMSGHDR_SIZE;
        let stored = sendmsg_one(k, fd, UserPtr(at), flags)
            .and_then(|n| k.copy_to_user(UserPtr(at + MsgHdr::SIZE as u64), &(n as u32).to_le_bytes()));
        match stored {
            Ok(()) => sent += 1,
            Err(e) if sent == 0 => return Err(e),
            Err(_) => break,
        }
    }
    Ok(u64::from(sent))
}

/// The receive flags shared by recvfrom and recvmsg; `max_rights` from
/// the control buffer.
fn recv_req(flags: u32, max_rights: usize) -> Result<RecvReq, Errno> {
    if flags & abi::MSG_OOB != 0 {
        // No out-of-band data on any of Poly's sockets.
        return Err(Errno::EINVAL);
    }
    if flags & abi::MSG_ERRQUEUE != 0 {
        // No error queues: nothing is ever queued there.
        return Err(Errno::EAGAIN);
    }
    Ok(RecvReq {
        peek: flags & abi::MSG_PEEK != 0,
        dontwait: flags & abi::MSG_DONTWAIT != 0,
        waitall: flags & abi::MSG_WAITALL != 0,
        max_rights,
        cloexec: flags & abi::MSG_CMSG_CLOEXEC != 0,
    })
}

/// Receive into the user buffers of `iovs`; returns what the kernel said
/// and the value the call returns (the full length of a datagram with
/// `MSG_TRUNC`).
fn recv_data(
    k: &mut dyn LinuxKernel,
    fd: i32,
    iovs: &[Iovec],
    total: u64,
    flags: u32,
    max_rights: usize,
) -> Result<(RecvMsg, usize), Errno> {
    let req = recv_req(flags, max_rights)?;
    let mut buf = bounce((total as usize).min(MAX_MESSAGE))?;
    let r = net(k)?.recv(fd, &mut buf, &req);
    let got = r.map_err(|e| interrupted(k, fd, false, e))?;
    let stream = got.from.is_none() && got.full_len == got.len;
    // MSG_TRUNC on a stream discards the data (tcp(7)); on a datagram it
    // returns the real length.
    if !(stream && flags & abi::MSG_TRUNC != 0) {
        scatter(k, iovs, &buf[..got.len])?;
    }
    let ret = if flags & abi::MSG_TRUNC != 0 { got.full_len } else { got.len };
    Ok((got, ret))
}

/// recvfrom(fd, buf, len, flags, src_addr, addrlen)
pub fn recvfrom(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let len = a.ulong(2).min(MAX_RW_COUNT);
    let iov = [Iovec { base: a.ptr(1).0, len }];
    let (got, ret) = recv_data(k, fd, &iov, len, a.uint(3), 0)?;
    if a.ptr(4).0 != 0 {
        match &got.from {
            Some(SockAddr::Unix(abi::UnixAddr::Unnamed)) => k.copy_to_user(a.ptr(5), &0u32.to_le_bytes())?,
            Some(from) => write_addr(k, a.ptr(4), a.ptr(5), from)?,
            // No address from this protocol: length 0.
            None => k.copy_to_user(a.ptr(5), &0u32.to_le_bytes())?,
        }
    }
    Ok(ret as u64)
}

/// The control data recvmsg returns, at most `room` bytes: `SCM_RIGHTS`
/// first, then `SCM_CREDENTIALS`; whether something did not fit.
fn build_control(got: &RecvMsg, room: usize) -> (Vec<u8>, bool) {
    let mut out = Vec::new();
    let mut truncated = got.rights_truncated;
    if !got.rights.is_empty() {
        let data: Vec<u8> = got.rights.iter().flat_map(|fd| fd.to_le_bytes()).collect();
        abi::push_cmsg(&mut out, abi::SOL_SOCKET, abi::SCM_RIGHTS, &data);
    }
    if let Some(c) = got.creds {
        if out.len() + abi::cmsg_len(Ucred::SIZE) <= room {
            abi::push_cmsg(&mut out, abi::SOL_SOCKET, abi::SCM_CREDENTIALS, &c.to_bytes());
        } else {
            truncated = true;
        }
    }
    out.truncate(room);
    (out, truncated)
}

/// How many descriptors a control buffer of `len` bytes holds.
fn rights_room(len: u64) -> usize {
    (len as usize).saturating_sub(abi::CMSG_HDR) / 4
}

fn recvmsg_one(k: &mut dyn LinuxKernel, fd: i32, msg_ptr: UserPtr, flags: u32) -> Result<usize, Errno> {
    let m = read_msghdr(k, msg_ptr)?;
    if (m.namelen as i32) < 0 {
        return Err(Errno::EINVAL);
    }
    let (iovs, total) = load_iovecs(k, UserPtr(m.iov), m.iovlen)?;
    let controllen = if m.control == 0 { 0 } else { m.controllen };
    let (got, ret) = recv_data(k, fd, &iovs, total, flags, rights_room(controllen))?;
    let (control, ctrunc) = build_control(&got, controllen as usize);
    let mut out_flags = 0;
    if got.full_len > got.len {
        out_flags |= abi::MSG_TRUNC;
    }
    if ctrunc {
        out_flags |= abi::MSG_CTRUNC;
    }
    if !control.is_empty() {
        k.copy_to_user(UserPtr(m.control), &control)?;
    }
    k.copy_to_user(UserPtr(msg_ptr.0 + MsgHdr::CONTROLLEN_OFFSET), &(control.len() as u64).to_le_bytes())?;
    k.copy_to_user(UserPtr(msg_ptr.0 + MsgHdr::FLAGS_OFFSET), &out_flags.to_le_bytes())?;
    let namelen = match (&got.from, m.name) {
        // An unnamed sender (a socketpair peer) gives namelen 0 (recvmsg's
        // spec sheet).
        (Some(SockAddr::Unix(abi::UnixAddr::Unnamed)), _) => 0,
        (Some(from), name) if name != 0 => {
            let bytes = from.encode();
            let n = bytes.len().min(m.namelen as usize);
            k.copy_to_user(UserPtr(name), &bytes[..n])?;
            bytes.len() as u32
        }
        _ => 0,
    };
    k.copy_to_user(UserPtr(msg_ptr.0 + MsgHdr::NAMELEN_OFFSET), &namelen.to_le_bytes())?;
    Ok(ret)
}

/// recvmsg(fd, msg, flags)
pub fn recvmsg(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    recvmsg_one(k, fd, a.ptr(1), a.uint(2)).map(|n| n as u64)
}

/// recvmmsg(fd, msgvec, vlen, flags, timeout): up to `vlen` messages (no
/// cap); `MSG_WAITFORONE` makes every receive after the first
/// non-blocking; the timeout (recvmmsg(2) BUGS: only checked after each
/// datagram) ends the call once it has passed, and the time left is
/// written back (`EFAULT` if it cannot be, the messages taken all the
/// same).
///
/// recvmmsg's spec sheet: the timeout is read (`EFAULT`) and checked
/// (`EINVAL`) first, then `MSG_CMSG_COMPAT` (`EINVAL`), the descriptor
/// (`EBADF`, `ENOTSOCK`) and a `vlen` of 0. An error after a message was
/// received ends the call with the count and, unless it is `EAGAIN`, is
/// kept on the socket for its next receive ([`LinuxNet::keep_error`]); an
/// interruption by a signal is kept as error 512
/// ([`Errno::RECV_INTERRUPTED`]).
pub fn recvmmsg(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let ts_ptr = a.ptr(4);
    let timeout = if ts_ptr.is_null() {
        None
    } else {
        let mut t = [0u8; 16];
        k.copy_from_user(ts_ptr, &mut t)?;
        let ts = crate::abi::Timespec::from_bytes(&t);
        if !ts.is_valid() {
            return Err(Errno::EINVAL);
        }
        Some(ts)
    };
    let flags = a.uint(3);
    if flags & abi::MSG_CMSG_COMPAT != 0 {
        return Err(Errno::EINVAL);
    }
    let fd = fd_arg(a, 0)?;
    net(k)?.getsockopt(fd, OptName::Type)?;
    let vlen = a.uint(2);
    if vlen == 0 {
        return Ok(0);
    }
    let deadline = match timeout {
        Some(ts) => Some(k.clock(Clock::Monotonic)?.saturating_add(ts)),
        None => None,
    };
    let mut got = 0u32;
    while got < vlen {
        let mut f = flags;
        if got > 0 && flags & abi::MSG_WAITFORONE != 0 {
            f |= abi::MSG_DONTWAIT;
        }
        let at = a.ptr(1).0 + u64::from(got) * abi::MMSGHDR_SIZE;
        match recvmsg_one(k, fd, UserPtr(at), f) {
            Ok(n) => {
                k.copy_to_user(UserPtr(at + MsgHdr::SIZE as u64), &(n as u32).to_le_bytes())?;
                got += 1;
            }
            Err(e) if got == 0 => return Err(e),
            Err(e) => {
                if e.is_restart() || e == Errno::EINTR {
                    net(k)?.keep_error(fd, Errno::RECV_INTERRUPTED);
                } else if e != Errno::EAGAIN {
                    net(k)?.keep_error(fd, e);
                }
                break;
            }
        }
        if let Some(d) = deadline {
            let now = k.clock(Clock::Monotonic)?;
            if (now.sec, now.nsec) >= (d.sec, d.nsec) {
                break;
            }
        }
    }
    if let Some(d) = deadline {
        let now = k.clock(Clock::Monotonic)?;
        let to_ns = |t: crate::abi::Timespec| (t.sec.max(0) as u128) * 1_000_000_000 + t.nsec.max(0) as u128;
        let left = to_ns(d).saturating_sub(to_ns(now)).min(u128::from(u64::MAX)) as u64;
        k.copy_to_user(ts_ptr, &crate::abi::Timespec::from_nanos(left).to_bytes())?;
    }
    Ok(u64::from(got))
}

// ---- options ------------------------------------------------------------------

fn read_int(k: &mut dyn LinuxKernel, ptr: UserPtr, len: u32) -> Result<i32, Errno> {
    if len < 4 {
        return Err(Errno::EINVAL);
    }
    let mut b = [0u8; 4];
    k.copy_from_user(ptr, &mut b)?;
    Ok(i32::from_le_bytes(b))
}

/// A `struct timeval` timeout in nanoseconds (0: none); `EINVAL` for
/// microseconds outside `0..1000000`. A negative time is no timeout
/// (Poly's choice, to be confirmed).
fn read_timeval(k: &mut dyn LinuxKernel, ptr: UserPtr, len: u32) -> Result<u64, Errno> {
    if (len as usize) < abi::TIMEVAL_SIZE {
        return Err(Errno::EINVAL);
    }
    let mut b = [0u8; abi::TIMEVAL_SIZE];
    k.copy_from_user(ptr, &mut b)?;
    let sec = i64::from_le_bytes(b[0..8].try_into().expect("8 bytes"));
    let usec = i64::from_le_bytes(b[8..16].try_into().expect("8 bytes"));
    // setsockopt's spec sheet: EDOM for a tv_usec of a million or more.
    if !(0..1_000_000).contains(&usec) {
        return Err(Errno::EDOM);
    }
    if sec < 0 {
        return Ok(0);
    }
    Ok((sec as u64).saturating_mul(1_000_000_000).saturating_add(usec as u64 * 1000))
}

/// `SO_SNDBUF` / `SO_RCVBUF`: the value doubled (socket(7)), within
/// `[min, 2 * MAX_BUFFER]` (the `*FORCE` variants skip the maximum).
fn buffer_size(v: i32, min: usize, force: bool) -> usize {
    let v = v.max(0) as usize;
    let v = if force { v } else { v.min(MAX_BUFFER) };
    v.saturating_mul(2).max(min)
}

/// Decode setsockopt(2)'s option: `ENOPROTOOPT` for unknown or read-only
/// ones, `EINVAL` for short or out-of-range values.
fn decode_opt(k: &mut dyn LinuxKernel, level: i32, name: i32, val: UserPtr, len: u32) -> Result<SockOpt, Errno> {
    let int = |k: &mut dyn LinuxKernel| read_int(k, val, len);
    let flag = |k: &mut dyn LinuxKernel| read_int(k, val, len).map(|v| v != 0);
    let hops = |v: i32| match v {
        -1 => Ok(None),
        1..=255 => Ok(Some(v as u8)),
        _ => Err(Errno::EINVAL),
    };
    Ok(match (level, name) {
        (abi::SOL_SOCKET, abi::SO_REUSEADDR) => SockOpt::ReuseAddr(flag(k)?),
        (abi::SOL_SOCKET, abi::SO_REUSEPORT) => SockOpt::ReusePort(flag(k)?),
        (abi::SOL_SOCKET, abi::SO_KEEPALIVE) => SockOpt::KeepAlive(flag(k)?),
        (abi::SOL_SOCKET, abi::SO_BROADCAST) => SockOpt::Broadcast(flag(k)?),
        (abi::SOL_SOCKET, abi::SO_PASSCRED) => SockOpt::PassCred(flag(k)?),
        (abi::SOL_SOCKET, abi::SO_OOBINLINE) => SockOpt::OobInline(flag(k)?),
        (abi::SOL_SOCKET, abi::SO_PRIORITY) => SockOpt::Priority(int(k)?),
        (abi::SOL_SOCKET, abi::SO_RCVLOWAT) => SockOpt::RcvLowat(int(k)?.max(1)),
        (abi::SOL_SOCKET, n @ (abi::SO_SNDBUF | abi::SO_SNDBUFFORCE)) => {
            SockOpt::SndBuf(buffer_size(int(k)?, 2048, n == abi::SO_SNDBUFFORCE))
        }
        (abi::SOL_SOCKET, n @ (abi::SO_RCVBUF | abi::SO_RCVBUFFORCE)) => {
            SockOpt::RcvBuf(buffer_size(int(k)?, 256, n == abi::SO_RCVBUFFORCE))
        }
        (abi::SOL_SOCKET, abi::SO_RCVTIMEO) => SockOpt::RcvTimeout(read_timeval(k, val, len)?),
        (abi::SOL_SOCKET, abi::SO_SNDTIMEO) => SockOpt::SndTimeout(read_timeval(k, val, len)?),
        (abi::SOL_SOCKET, abi::SO_LINGER) => {
            if (len as usize) < abi::LINGER_SIZE {
                return Err(Errno::EINVAL);
            }
            let mut b = [0u8; abi::LINGER_SIZE];
            k.copy_from_user(val, &mut b)?;
            let onoff = i32::from_le_bytes(b[0..4].try_into().expect("4 bytes"));
            let secs = i32::from_le_bytes(b[4..8].try_into().expect("4 bytes"));
            SockOpt::Linger((onoff != 0).then_some(secs.max(0) as u32))
        }
        (abi::SOL_TCP, abi::TCP_NODELAY) => SockOpt::TcpNoDelay(flag(k)?),
        (abi::SOL_TCP, abi::TCP_KEEPIDLE) => match int(k)? {
            v @ 1..=32767 => SockOpt::TcpKeepIdle(v as u32),
            _ => return Err(Errno::EINVAL),
        },
        (abi::SOL_TCP, abi::TCP_KEEPINTVL) => match int(k)? {
            v @ 1..=32767 => SockOpt::TcpKeepIntvl(v as u32),
            _ => return Err(Errno::EINVAL),
        },
        (abi::SOL_TCP, abi::TCP_KEEPCNT) => match int(k)? {
            v @ 1..=127 => SockOpt::TcpKeepCnt(v as u32),
            _ => return Err(Errno::EINVAL),
        },
        (abi::SOL_TCP, abi::TCP_MAXSEG) => match int(k)? {
            v @ 0..=65535 => SockOpt::TcpMaxSeg(v as u32),
            _ => return Err(Errno::EINVAL),
        },
        (abi::SOL_IP, abi::IP_TTL) => SockOpt::IpTtl(hops(int(k)?)?),
        (abi::SOL_IP, abi::IP_TOS) => SockOpt::IpTos((int(k)? & 0xff) as u8),
        (abi::SOL_IPV6, abi::IPV6_V6ONLY) => SockOpt::Ipv6V6Only(flag(k)?),
        (abi::SOL_IPV6, abi::IPV6_UNICAST_HOPS) => SockOpt::Ipv6UnicastHops(hops(int(k)?)?),
        (abi::SOL_NETLINK, abi::NETLINK_ADD_MEMBERSHIP) => SockOpt::NetlinkAddMembership(int(k)? as u32),
        (abi::SOL_NETLINK, abi::NETLINK_DROP_MEMBERSHIP) => SockOpt::NetlinkDropMembership(int(k)? as u32),
        _ => return Err(Errno::ENOPROTOOPT),
    })
}

/// setsockopt(fd, level, optname, optval, optlen)
pub fn setsockopt(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let len = a.int(4);
    if len < 0 {
        return Err(Errno::EINVAL);
    }
    // ENOTSOCK / EBADF before the value is looked at.
    net(k)?.getsockopt(fd, OptName::Type)?;
    check_level(k, fd, a.int(1))?;
    let opt = decode_opt(k, a.int(1), a.int(2), a.ptr(3), len as u32)?;
    net(k)?.setsockopt(fd, opt).map(|()| 0)
}

/// An Internet protocol's level on a Unix socket is `EOPNOTSUPP`
/// (getsockopt's spec sheet: a TCP option on an `AF_UNIX` socket).
fn check_level(k: &mut dyn LinuxKernel, fd: i32, level: i32) -> Result<(), Errno> {
    if !matches!(level, abi::SOL_TCP | abi::SOL_IP | abi::SOL_IPV6) {
        return Ok(());
    }
    match net(k)?.getsockopt(fd, OptName::Domain)? {
        OptValue::Int(d) if d == i32::from(abi::AF_UNIX) => Err(Errno::EOPNOTSUPP),
        _ => Ok(()),
    }
}

fn opt_name(level: i32, name: i32) -> Result<OptName, Errno> {
    Ok(match (level, name) {
        (abi::SOL_SOCKET, abi::SO_TYPE) => OptName::Type,
        (abi::SOL_SOCKET, abi::SO_DOMAIN) => OptName::Domain,
        (abi::SOL_SOCKET, abi::SO_PROTOCOL) => OptName::Protocol,
        (abi::SOL_SOCKET, abi::SO_ERROR) => OptName::Error,
        (abi::SOL_SOCKET, abi::SO_ACCEPTCONN) => OptName::AcceptConn,
        (abi::SOL_SOCKET, abi::SO_PEERCRED) => OptName::PeerCred,
        (abi::SOL_SOCKET, abi::SO_REUSEADDR) => OptName::ReuseAddr,
        (abi::SOL_SOCKET, abi::SO_REUSEPORT) => OptName::ReusePort,
        (abi::SOL_SOCKET, abi::SO_KEEPALIVE) => OptName::KeepAlive,
        (abi::SOL_SOCKET, abi::SO_BROADCAST) => OptName::Broadcast,
        (abi::SOL_SOCKET, abi::SO_SNDBUF) => OptName::SndBuf,
        (abi::SOL_SOCKET, abi::SO_RCVBUF) => OptName::RcvBuf,
        (abi::SOL_SOCKET, abi::SO_RCVTIMEO) => OptName::RcvTimeout,
        (abi::SOL_SOCKET, abi::SO_SNDTIMEO) => OptName::SndTimeout,
        (abi::SOL_SOCKET, abi::SO_LINGER) => OptName::Linger,
        (abi::SOL_SOCKET, abi::SO_PASSCRED) => OptName::PassCred,
        (abi::SOL_SOCKET, abi::SO_OOBINLINE) => OptName::OobInline,
        (abi::SOL_SOCKET, abi::SO_PRIORITY) => OptName::Priority,
        (abi::SOL_SOCKET, abi::SO_RCVLOWAT) => OptName::RcvLowat,
        (abi::SOL_SOCKET, abi::SO_SNDLOWAT) => OptName::SndLowat,
        (abi::SOL_TCP, abi::TCP_NODELAY) => OptName::TcpNoDelay,
        (abi::SOL_TCP, abi::TCP_KEEPIDLE) => OptName::TcpKeepIdle,
        (abi::SOL_TCP, abi::TCP_KEEPINTVL) => OptName::TcpKeepIntvl,
        (abi::SOL_TCP, abi::TCP_KEEPCNT) => OptName::TcpKeepCnt,
        (abi::SOL_TCP, abi::TCP_MAXSEG) => OptName::TcpMaxSeg,
        (abi::SOL_IP, abi::IP_TTL) => OptName::IpTtl,
        (abi::SOL_IP, abi::IP_TOS) => OptName::IpTos,
        (abi::SOL_IPV6, abi::IPV6_V6ONLY) => OptName::Ipv6V6Only,
        (abi::SOL_IPV6, abi::IPV6_UNICAST_HOPS) => OptName::Ipv6UnicastHops,
        _ => return Err(Errno::ENOPROTOOPT),
    })
}

/// An option value as getsockopt(2) stores it.
pub fn encode_value(v: OptValue) -> Vec<u8> {
    match v {
        OptValue::Int(i) => i.to_le_bytes().to_vec(),
        OptValue::Linger { onoff, seconds } => {
            let mut b = Vec::with_capacity(abi::LINGER_SIZE);
            b.extend_from_slice(&i32::from(onoff).to_le_bytes());
            b.extend_from_slice(&(seconds as i32).to_le_bytes());
            b
        }
        OptValue::Timeval(ns) => {
            let mut b = Vec::with_capacity(abi::TIMEVAL_SIZE);
            b.extend_from_slice(&((ns / 1_000_000_000) as i64).to_le_bytes());
            b.extend_from_slice(&(((ns % 1_000_000_000) / 1000) as i64).to_le_bytes());
            b
        }
        OptValue::Ucred(u) => u.to_bytes().to_vec(),
    }
}

/// getsockopt(fd, level, optname, optval, optlen): at most `*optlen`
/// bytes of the value are stored, and `*optlen` set to how many.
pub fn getsockopt(k: &mut dyn LinuxKernel, a: &Args) -> SysResult {
    let fd = fd_arg(a, 0)?;
    let mut lb = [0u8; 4];
    k.copy_from_user(a.ptr(4), &mut lb)?;
    let room = i32::from_le_bytes(lb);
    if room < 0 {
        return Err(Errno::EINVAL);
    }
    net(k)?.getsockopt(fd, OptName::Type)?;
    check_level(k, fd, a.int(1))?;
    let name = opt_name(a.int(1), a.int(2))?;
    let value = net(k)?.getsockopt(fd, name)?;
    let bytes = encode_value(value);
    let n = bytes.len().min(room as usize);
    k.copy_to_user(a.ptr(3), &bytes[..n])?;
    k.copy_to_user(a.ptr(4), &(n as u32).to_le_bytes())?;
    Ok(0)
}

/// `ioctl(SIOCOUTQ)` on a socket: the unsent bytes, as an int. `None`
/// for descriptors that are not sockets (the caller goes on as for any
/// other ioctl).
pub fn ioctl_outq(k: &mut dyn LinuxKernel, fd: i32, arg: UserPtr) -> Option<SysResult> {
    let n = match k.net()?.send_queued(fd) {
        Err(Errno::ENOTSOCK) => return None,
        Err(e) => return Some(Err(e)),
        Ok(n) => n,
    };
    let n = n.min(i32::MAX as usize) as i32;
    Some(k.copy_to_user(arg, &n.to_le_bytes()).map(|()| 0))
}
