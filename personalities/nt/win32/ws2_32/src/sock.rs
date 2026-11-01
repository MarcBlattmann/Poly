//! The socket calls (Learn's page of each). A `SOCKET` is a handle to an
//! endpoint on `\Device\Afd`; each call is one or two of the device's
//! requests, its `NTSTATUS` turned into a WSA error by
//! `poly_winsock::error`.
//!
//! Overlapped I/O (`WSASend`, `WSARecv`, `WSASendTo`, `WSARecvFrom` with
//! an `OVERLAPPED`): the request is made with the `OVERLAPPED` as its
//! status block (`Internal` holds the status, `InternalHigh` the count, as
//! Learn's "OVERLAPPED" describes) and its `hEvent` as the event; a
//! request that cannot finish at once returns `WSA_IO_PENDING` and the
//! device completes it later. A completion routine runs as the request's
//! APC, at the thread's next alertable wait. Requests with more than one
//! buffer, and `WSARecvFrom`'s address, are done synchronously and then
//! reported as completed (the event set, the routine called at once).

use crate::afd::{self, IoStatusBlock};
use crate::{Handle, check_started, fail, set_error};
use alloc::vec::Vec;
use poly_winsock::afd as proto;
use poly_winsock::consts::*;
use poly_winsock::error::{STATUS_BUFFER_OVERFLOW, STATUS_PENDING, STATUS_SUCCESS, wsa_of_status};
use poly_winsock::{addr, poll};

pub type Socket = usize;

fn handle(s: Socket) -> Handle {
    s as Handle
}

fn is_error(status: u32) -> bool {
    status >= 0xC000_0000
}

/// One request on socket `s`: `Information`, or the WSA error.
fn call(s: Socket, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize, u32> {
    check_started()?;
    let (status, info) = afd::ioctl(handle(s), code, input, output);
    if is_error(status) {
        return Err(wsa_of_status(status));
    }
    Ok(info)
}

/// The bytes of a socket address argument.
unsafe fn sockaddr_in<'a>(name: *const u8, len: i32) -> Result<&'a [u8], u32> {
    if name.is_null() || len < 0 {
        return Err(WSAEFAULT);
    }
    // SAFETY: the caller's `len` bytes.
    Ok(unsafe { core::slice::from_raw_parts(name, len as usize) })
}

/// Copy an address result to `(addr, len)`: `WSAEFAULT` if `*len` is too
/// small; nothing if `addr` is null.
unsafe fn sockaddr_out(bytes: &[u8], out: *mut u8, len: *mut i32) -> Result<(), u32> {
    if out.is_null() {
        return Ok(());
    }
    if len.is_null() {
        return Err(WSAEFAULT);
    }
    // SAFETY: the caller's length and buffer.
    unsafe {
        if *len < 0 || (*len as usize) < bytes.len() {
            return Err(WSAEFAULT);
        }
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out, bytes.len());
        *len = bytes.len() as i32;
    }
    Ok(())
}

fn ret(r: Result<i32, u32>) -> i32 {
    r.unwrap_or_else(fail)
}

fn ret_socket(r: Result<Socket, u32>) -> Socket {
    r.unwrap_or_else(|e| {
        set_error(e);
        INVALID_SOCKET
    })
}

// ----- socket, bind, listen, accept, connect, shutdown, closesocket

fn new_socket(af: i32, ty: i32, protocol: i32) -> Result<Socket, u32> {
    check_started()?;
    let req = proto::check_socket(af, ty, protocol)?;
    let h = afd::open_endpoint().map_err(|s| if s == 0xC000_0002 { WSAENETDOWN } else { WSAEMFILE })?;
    let (status, _) = afd::ioctl(h, proto::SOCKET, &req.encode(), &mut []);
    if is_error(status) {
        afd::close(h);
        return Err(match wsa_of_status(status) {
            WSAEOPNOTSUPP => WSAENETDOWN,
            e => e,
        });
    }
    Ok(h as Socket)
}

/// `socket`.
#[unsafe(no_mangle)]
pub extern "system" fn socket(af: i32, ty: i32, protocol: i32) -> Socket {
    ret_socket(new_socket(af, ty, protocol))
}

/// `WSASocketW`: `lpProtocolInfo` and the group are not supported (a
/// protocol-info request is `WSAEINVAL`); `WSA_FLAG_OVERLAPPED` is how
/// every Poly socket works anyway.
#[unsafe(no_mangle)]
pub extern "system" fn WSASocketW(af: i32, ty: i32, protocol: i32, info: *const u8, _g: u32, _flags: u32) -> Socket {
    if !info.is_null() {
        set_error(WSAEINVAL);
        return INVALID_SOCKET;
    }
    socket(af, ty, protocol)
}

/// `WSASocketA`.
#[unsafe(no_mangle)]
pub extern "system" fn WSASocketA(af: i32, ty: i32, protocol: i32, info: *const u8, g: u32, flags: u32) -> Socket {
    WSASocketW(af, ty, protocol, info, g, flags)
}

/// `bind`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn bind(s: Socket, name: *const u8, len: i32) -> i32 {
    // SAFETY: the caller's address.
    ret(unsafe { sockaddr_in(name, len) }.and_then(|a| call(s, proto::BIND, a, &mut []).map(|_| 0)))
}

/// `listen`.
#[unsafe(no_mangle)]
pub extern "system" fn listen(s: Socket, backlog: i32) -> i32 {
    ret(call(s, proto::LISTEN, &(backlog.max(0) as u32).to_le_bytes(), &mut []).map(|_| 0))
}

fn accept_into(s: Socket, peer: &mut [u8; addr::MAX_SIZE]) -> Result<(Socket, usize), u32> {
    check_started()?;
    let new = afd::open_endpoint().map_err(|_| WSAEMFILE)?;
    match call(s, proto::ACCEPT, &(new as u64).to_le_bytes(), peer) {
        Ok(n) => Ok((new as Socket, n)),
        Err(e) => {
            afd::close(new);
            Err(e)
        }
    }
}

/// `accept`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn accept(s: Socket, out: *mut u8, len: *mut i32) -> Socket {
    let mut peer = [0u8; addr::MAX_SIZE];
    ret_socket(accept_into(s, &mut peer).and_then(|(new, n)| {
        // SAFETY: the caller's buffer and length.
        match unsafe { sockaddr_out(&peer[..n], out, len) } {
            Ok(()) => Ok(new),
            Err(e) => {
                afd::close(handle(new));
                Err(e)
            }
        }
    }))
}

/// `connect`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn connect(s: Socket, name: *const u8, len: i32) -> i32 {
    // SAFETY: the caller's address.
    ret(unsafe { sockaddr_in(name, len) }.and_then(|a| call(s, proto::CONNECT, a, &mut []).map(|_| 0)))
}

/// `WSAConnect`: `connect`; connect data and quality of service are not
/// supported (ignored).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAConnect(
    s: Socket,
    name: *const u8,
    len: i32,
    _caller: *const u8,
    _callee: *mut u8,
    _sqos: *const u8,
    _gqos: *const u8,
) -> i32 {
    // SAFETY: as `connect`.
    unsafe { connect(s, name, len) }
}

/// `shutdown`.
#[unsafe(no_mangle)]
pub extern "system" fn shutdown(s: Socket, how: i32) -> i32 {
    ret(call(s, proto::SHUTDOWN, &(how as u32).to_le_bytes(), &mut []).map(|_| 0))
}

/// `closesocket`: `WSAENOTSOCK` for a handle that is not a socket.
#[unsafe(no_mangle)]
pub extern "system" fn closesocket(s: Socket) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if afd::info(handle(s)).is_none() {
        // An endpoint that never got a socket is still ours to close.
        let mut out = [0u8; proto::Info::SIZE];
        let (status, _) = afd::ioctl(handle(s), proto::INFO, &[], &mut out);
        if is_error(status) {
            return fail(WSAENOTSOCK);
        }
    }
    afd::close(handle(s));
    0
}

// ----- names and options

fn name_of(s: Socket, peer: bool, out: *mut u8, len: *mut i32) -> i32 {
    let mut b = [0u8; addr::MAX_SIZE];
    ret(call(s, proto::NAME, &u32::from(peer).to_le_bytes(), &mut b).and_then(|n| {
        if out.is_null() {
            return Err(WSAEFAULT);
        }
        // SAFETY: the caller's buffer and length.
        unsafe { sockaddr_out(&b[..n], out, len) }.map(|()| 0)
    }))
}

/// `getsockname`.
#[unsafe(no_mangle)]
pub extern "system" fn getsockname(s: Socket, out: *mut u8, len: *mut i32) -> i32 {
    name_of(s, false, out, len)
}

/// `getpeername`.
#[unsafe(no_mangle)]
pub extern "system" fn getpeername(s: Socket, out: *mut u8, len: *mut i32) -> i32 {
    name_of(s, true, out, len)
}

/// `setsockopt`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn setsockopt(s: Socket, level: i32, name: i32, value: *const u8, len: i32) -> i32 {
    if len < 0 || (value.is_null() && len > 0) {
        return fail(WSAEFAULT);
    }
    let mut b = proto::option_header(level, name).to_vec();
    if len > 0 {
        // SAFETY: the caller's `len` bytes.
        b.extend_from_slice(unsafe { core::slice::from_raw_parts(value, len as usize) });
    }
    ret(call(s, proto::SET_OPTION, &b, &mut []).map(|_| 0))
}

/// `getsockopt`: the value and its length; `WSAEFAULT` when `*len` is
/// smaller than the value (a BOOL may be read into one byte).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn getsockopt(s: Socket, level: i32, name: i32, value: *mut u8, len: *mut i32) -> i32 {
    if value.is_null() || len.is_null() {
        return fail(WSAEFAULT);
    }
    let mut out = [0u8; 16];
    ret(call(s, proto::GET_OPTION, &proto::option_header(level, name), &mut out).and_then(|n| {
        // SAFETY: the caller's buffer and length.
        unsafe {
            let have = *len;
            if have <= 0 {
                return Err(WSAEFAULT);
            }
            let put = if (have as usize) < n { if have == 1 && n == 4 { 1 } else { return Err(WSAEFAULT) } } else { n };
            core::ptr::copy_nonoverlapping(out.as_ptr(), value, put);
            *len = put as i32;
        }
        Ok(0)
    }))
}

/// `ioctlsocket`: `FIONBIO`, `FIONREAD`, `SIOCATMARK` (always true:
/// Poly's stack has no out-of-band data); anything else `WSAEINVAL`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ioctlsocket(s: Socket, cmd: i32, arg: *mut u32) -> i32 {
    if arg.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's argument.
    let r = unsafe {
        match cmd as u32 {
            FIONBIO => call(s, proto::SET_NONBLOCKING, &u32::from(*arg != 0).to_le_bytes(), &mut []).map(|_| 0),
            FIONREAD => {
                let mut b = [0u8; 4];
                call(s, proto::READABLE, &[], &mut b).map(|_| {
                    *arg = u32::from_le_bytes(b);
                    0
                })
            }
            SIOCATMARK => call(s, proto::INFO, &[], &mut [0u8; proto::Info::SIZE]).map(|_| {
                *arg = 1;
                0
            }),
            _ => Err(WSAEINVAL),
        }
    };
    ret(r)
}

/// `WSAIoctl`: the `ioctlsocket` codes; extension functions
/// (`SIO_GET_EXTENSION_FUNCTION_POINTER`: `AcceptEx`, `ConnectEx`, ...)
/// are not there yet (`WSAEINVAL`), nor overlapped control codes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAIoctl(
    s: Socket,
    code: u32,
    input: *const u8,
    input_len: u32,
    output: *mut u8,
    output_len: u32,
    returned: *mut u32,
    _overlapped: *mut u8,
    _routine: *const u8,
) -> i32 {
    match code {
        FIONBIO => {
            if input.is_null() || input_len < 4 {
                return fail(WSAEFAULT);
            }
            let mut v = 0u32;
            // SAFETY: four bytes of the caller's input.
            unsafe { core::ptr::copy_nonoverlapping(input, (&raw mut v).cast::<u8>(), 4) };
            // SAFETY: a local.
            unsafe { ioctlsocket(s, FIONBIO as i32, &mut v) }
        }
        FIONREAD | SIOCATMARK => {
            if output.is_null() || output_len < 4 {
                return fail(WSAEFAULT);
            }
            let mut v = 0u32;
            // SAFETY: a local.
            let r = unsafe { ioctlsocket(s, code as i32, &mut v) };
            if r == 0 {
                // SAFETY: four bytes of the caller's output.
                unsafe {
                    core::ptr::copy_nonoverlapping((&raw const v).cast::<u8>(), output, 4);
                    if !returned.is_null() {
                        *returned = 4;
                    }
                }
            }
            r
        }
        _ => fail(WSAEINVAL),
    }
}

// ----- send and receive

fn send_req(flags: u32, buf: *const u8, len: usize, to: Option<&[u8]>) -> Result<proto::SendReq, u32> {
    let mut req = proto::SendReq { flags, addr_len: 0, buf: buf as u64, len: len as u64, addr: [0; addr::MAX_SIZE] };
    if let Some(t) = to {
        let n = t.len().min(addr::MAX_SIZE);
        req.addr[..n].copy_from_slice(&t[..n]);
        req.addr_len = n as u32;
    }
    Ok(req)
}

fn check_send_flags(flags: i32) -> Result<(), u32> {
    if flags & MSG_OOB != 0 {
        return Err(WSAEOPNOTSUPP);
    }
    if flags & !(MSG_DONTROUTE | MSG_PARTIAL | MSG_PUSH_IMMEDIATE) != 0 {
        return Err(WSAEOPNOTSUPP);
    }
    Ok(())
}

fn recv_flags(flags: i32) -> Result<u32, u32> {
    if flags & MSG_OOB != 0 || flags & !(MSG_PEEK | MSG_WAITALL | MSG_PUSH_IMMEDIATE) != 0 {
        return Err(WSAEOPNOTSUPP);
    }
    let mut f = 0;
    if flags & MSG_PEEK != 0 {
        f |= proto::FLAG_PEEK;
    }
    if flags & MSG_WAITALL != 0 {
        f |= proto::FLAG_WAITALL;
    }
    Ok(f)
}

fn do_send(s: Socket, buf: *const u8, len: usize, flags: i32, to: Option<&[u8]>) -> Result<usize, u32> {
    check_send_flags(flags)?;
    if buf.is_null() && len > 0 {
        return Err(WSAEFAULT);
    }
    let req = send_req(0, buf, len, to)?;
    call(s, proto::SEND, &req.encode(), &mut [])
}

/// A receive: the count and the sender's address bytes (datagrams).
/// A datagram longer than the buffer is `WSAEMSGSIZE` (Learn: "recv").
fn do_recv(s: Socket, buf: *mut u8, len: usize, flags: u32) -> Result<(usize, proto::RecvReply), u32> {
    check_started()?;
    if buf.is_null() && len > 0 {
        return Err(WSAEFAULT);
    }
    let req = proto::RecvReq { flags, buf: buf as u64, len: len as u64 };
    let mut out = [0u8; proto::RecvReply::SIZE];
    let (status, info) = afd::ioctl(handle(s), proto::RECV, &req.encode(), &mut out);
    if is_error(status) {
        return Err(wsa_of_status(status));
    }
    let reply = proto::RecvReply::decode(&out).ok_or(WSAEFAULT)?;
    if status == STATUS_BUFFER_OVERFLOW {
        return Err(WSAEMSGSIZE);
    }
    Ok((info, reply))
}

fn count(n: usize) -> i32 {
    n.min(i32::MAX as usize) as i32
}

/// `send`.
#[unsafe(no_mangle)]
pub extern "system" fn send(s: Socket, buf: *const u8, len: i32, flags: i32) -> i32 {
    if len < 0 {
        return fail(WSAEFAULT);
    }
    ret(do_send(s, buf, len as usize, flags, None).map(count))
}

/// `sendto`: the destination is ignored by a connected stream.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn sendto(
    s: Socket,
    buf: *const u8,
    len: i32,
    flags: i32,
    to: *const u8,
    tolen: i32,
) -> i32 {
    if len < 0 {
        return fail(WSAEFAULT);
    }
    let dest = if to.is_null() {
        None
    } else {
        // SAFETY: the caller's address.
        match unsafe { sockaddr_in(to, tolen) } {
            Ok(d) => Some(d),
            Err(e) => return fail(e),
        }
    };
    ret(do_send(s, buf, len as usize, flags, dest).map(count))
}

/// `recv`.
#[unsafe(no_mangle)]
pub extern "system" fn recv(s: Socket, buf: *mut u8, len: i32, flags: i32) -> i32 {
    if len < 0 {
        return fail(WSAEFAULT);
    }
    ret(recv_flags(flags).and_then(|f| do_recv(s, buf, len as usize, f)).map(|(n, _)| count(n)))
}

/// `recvfrom`: the sender's address for a datagram socket.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn recvfrom(
    s: Socket,
    buf: *mut u8,
    len: i32,
    flags: i32,
    from: *mut u8,
    fromlen: *mut i32,
) -> i32 {
    if len < 0 || (!from.is_null() && fromlen.is_null()) {
        return fail(WSAEFAULT);
    }
    let r = recv_flags(flags).and_then(|f| do_recv(s, buf, len as usize, f)).and_then(|(n, reply)| {
        if reply.from_len > 0 {
            // SAFETY: the caller's buffer and length.
            unsafe { sockaddr_out(&reply.from[..reply.from_len as usize], from, fromlen) }?;
        }
        Ok(count(n))
    });
    ret(r)
}

/// `WSABUF` (`ws2def.h`): `ULONG len; CHAR *buf;` (16 bytes on x64).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct WsaBuf {
    pub len: u32,
    pub buf: *mut u8,
}

/// `WSAOVERLAPPED` (`OVERLAPPED`, `minwinbase.h`).
#[repr(C)]
#[derive(Debug)]
pub struct Overlapped {
    pub internal: usize,
    pub internal_high: usize,
    pub offset: u64,
    pub event: Handle,
}

type CompletionRoutine = unsafe extern "system" fn(error: u32, transferred: u32, ov: *mut Overlapped, flags: u32);

/// The APC that turns a completed request into a call of the program's
/// completion routine (`PIO_APC_ROUTINE`: context, status block,
/// reserved).
unsafe extern "system" fn completion_apc(context: *const u8, iosb: *mut IoStatusBlock, _reserved: u32) {
    // SAFETY: the context is the program's routine and the status block
    // its OVERLAPPED, as `overlapped` set them.
    unsafe {
        let routine: CompletionRoutine = core::mem::transmute(context);
        let status = (*iosb).status as u32;
        let error = if is_error(status) { wsa_of_status(status) } else { 0 };
        routine(error, (*iosb).information as u32, iosb.cast::<Overlapped>(), 0);
    }
}

fn buffers<'a>(bufs: *const WsaBuf, n: u32) -> Result<&'a [WsaBuf], u32> {
    if bufs.is_null() || n == 0 {
        return Err(WSAEFAULT);
    }
    // SAFETY: the caller's array of `n` buffers.
    Ok(unsafe { core::slice::from_raw_parts(bufs, n as usize) })
}

/// Report a request done synchronously through its `OVERLAPPED`, as a
/// completion would.
unsafe fn complete_now(ov: *mut Overlapped, routine: Option<CompletionRoutine>, n: usize) {
    // SAFETY: the caller's OVERLAPPED.
    unsafe {
        (*ov).internal = STATUS_SUCCESS as usize;
        (*ov).internal_high = n;
        if (*ov).event != 0 {
            crate::SetEvent((*ov).event);
        }
        if let Some(r) = routine {
            r(0, n as u32, ov, 0);
        }
    }
}

/// Make an overlapped request; `Ok(Some(n))` when it finished at once.
unsafe fn overlapped(
    s: Socket,
    code: u32,
    input: &[u8],
    output: &mut [u8],
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> Result<Option<usize>, u32> {
    check_started()?;
    // SAFETY: the caller's OVERLAPPED, which outlives the request.
    unsafe {
        (*ov).internal = STATUS_PENDING as usize;
        (*ov).internal_high = 0;
        let (apc, ctx) = match routine {
            Some(r) => (completion_apc as *const u8, r as *const u8),
            None => (core::ptr::null(), core::ptr::null()),
        };
        let status = afd::ioctl_overlapped(handle(s), (*ov).event, apc, ctx, ov.cast(), code, input, output);
        if status == STATUS_PENDING {
            return Ok(None);
        }
        if is_error(status) {
            return Err(wsa_of_status(status));
        }
        if status == STATUS_BUFFER_OVERFLOW {
            return Err(WSAEMSGSIZE);
        }
        Ok(Some((*ov).internal_high))
    }
}

unsafe fn finish_wsa(r: Result<Option<usize>, u32>, sent: *mut u32) -> i32 {
    match r {
        Ok(Some(n)) => {
            if !sent.is_null() {
                // SAFETY: the caller's count.
                unsafe { *sent = n as u32 };
            }
            0
        }
        Ok(None) => fail(WSA_IO_PENDING),
        Err(e) => fail(e),
    }
}

/// `WSASend` / `WSASendTo`'s work.
#[allow(clippy::too_many_arguments)]
unsafe fn wsa_send(
    s: Socket,
    bufs: *const WsaBuf,
    n: u32,
    sent: *mut u32,
    flags: u32,
    to: Option<&[u8]>,
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> i32 {
    let bufs = match buffers(bufs, n).and_then(|b| check_send_flags(flags as i32).map(|()| b)) {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    if !ov.is_null() && bufs.len() == 1 {
        let req = match send_req(proto::FLAG_OVERLAPPED, bufs[0].buf, bufs[0].len as usize, to) {
            Ok(r) => r,
            Err(e) => return fail(e),
        };
        // SAFETY: the caller's OVERLAPPED.
        return unsafe { finish_wsa(overlapped(s, proto::SEND, &req.encode(), &mut [], ov, routine), sent) };
    }
    let mut total = 0usize;
    for b in bufs {
        match do_send(s, b.buf, b.len as usize, 0, to) {
            Ok(k) => {
                total += k;
                if k < b.len as usize {
                    break;
                }
            }
            Err(_) if total > 0 => break,
            Err(e) => return fail(e),
        }
    }
    if !ov.is_null() {
        // SAFETY: the caller's OVERLAPPED.
        unsafe { complete_now(ov, routine, total) };
    }
    // SAFETY: the caller's count.
    unsafe { finish_wsa(Ok(Some(total)), sent) }
}

/// `WSASend`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSASend(
    s: Socket,
    bufs: *const WsaBuf,
    n: u32,
    sent: *mut u32,
    flags: u32,
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> i32 {
    // SAFETY: as documented.
    unsafe { wsa_send(s, bufs, n, sent, flags, None, ov, routine) }
}

/// `WSASendTo`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSASendTo(
    s: Socket,
    bufs: *const WsaBuf,
    n: u32,
    sent: *mut u32,
    flags: u32,
    to: *const u8,
    tolen: i32,
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> i32 {
    let dest = if to.is_null() {
        None
    } else {
        // SAFETY: the caller's address.
        match unsafe { sockaddr_in(to, tolen) } {
            Ok(d) => Some(d),
            Err(e) => return fail(e),
        }
    };
    // SAFETY: as documented.
    unsafe { wsa_send(s, bufs, n, sent, flags, dest, ov, routine) }
}

/// `WSARecv` / `WSARecvFrom`'s work.
#[allow(clippy::too_many_arguments)]
unsafe fn wsa_recv(
    s: Socket,
    bufs: *const WsaBuf,
    n: u32,
    received: *mut u32,
    flags: *mut u32,
    from: *mut u8,
    fromlen: *mut i32,
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> i32 {
    let in_flags = if flags.is_null() {
        return fail(WSAEFAULT);
    } else {
        // SAFETY: the caller's flags.
        unsafe { *flags }
    };
    let (bufs, f) = match buffers(bufs, n).and_then(|b| recv_flags(in_flags as i32).map(|f| (b, f))) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    // SAFETY: the caller's flags: no MSG_PARTIAL on Poly.
    unsafe { *flags = 0 };
    if !ov.is_null() && bufs.len() == 1 && from.is_null() {
        let req =
            proto::RecvReq { flags: f | proto::FLAG_OVERLAPPED, buf: bufs[0].buf as u64, len: u64::from(bufs[0].len) };
        let mut out = [0u8; proto::RecvReply::SIZE];
        // SAFETY: the caller's OVERLAPPED.
        return unsafe { finish_wsa(overlapped(s, proto::RECV, &req.encode(), &mut out, ov, routine), received) };
    }
    // Synchronously: into the one buffer, or a temporary one scattered.
    let total_len: usize = bufs.iter().map(|b| b.len as usize).sum();
    let r = if bufs.len() == 1 {
        do_recv(s, bufs[0].buf, total_len, f)
    } else {
        let mut tmp: Vec<u8> = alloc::vec![0u8; total_len];
        do_recv(s, tmp.as_mut_ptr(), total_len, f).map(|(got, reply)| {
            let mut at = 0;
            for b in bufs {
                let k = (b.len as usize).min(got - at);
                // SAFETY: the caller's buffer of `b.len` bytes.
                unsafe { core::ptr::copy_nonoverlapping(tmp.as_ptr().add(at), b.buf, k) };
                at += k;
            }
            (got, reply)
        })
    };
    let (got, reply) = match r {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    if reply.from_len > 0
        // SAFETY: the caller's buffer and length.
        && let Err(e) = unsafe { sockaddr_out(&reply.from[..reply.from_len as usize], from, fromlen) }
    {
        return fail(e);
    }
    if !ov.is_null() {
        // SAFETY: the caller's OVERLAPPED.
        unsafe { complete_now(ov, routine, got) };
    }
    // SAFETY: the caller's count.
    unsafe { finish_wsa(Ok(Some(got)), received) }
}

/// `WSARecv`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSARecv(
    s: Socket,
    bufs: *const WsaBuf,
    n: u32,
    received: *mut u32,
    flags: *mut u32,
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> i32 {
    // SAFETY: as documented.
    unsafe { wsa_recv(s, bufs, n, received, flags, core::ptr::null_mut(), core::ptr::null_mut(), ov, routine) }
}

/// `WSARecvFrom`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSARecvFrom(
    s: Socket,
    bufs: *const WsaBuf,
    n: u32,
    received: *mut u32,
    flags: *mut u32,
    from: *mut u8,
    fromlen: *mut i32,
    ov: *mut Overlapped,
    routine: Option<CompletionRoutine>,
) -> i32 {
    // SAFETY: as documented.
    unsafe { wsa_recv(s, bufs, n, received, flags, from, fromlen, ov, routine) }
}

/// `WSAGetOverlappedResult` (Learn): `WSA_IO_INCOMPLETE` while pending
/// unless `wait`, which waits on the event (or the socket).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAGetOverlappedResult(
    s: Socket,
    ov: *mut Overlapped,
    transferred: *mut u32,
    wait: i32,
    flags: *mut u32,
) -> i32 {
    if ov.is_null() || transferred.is_null() || flags.is_null() {
        set_error(WSAEFAULT);
        return 0;
    }
    // SAFETY: the caller's OVERLAPPED and outputs.
    unsafe {
        if (*ov).internal as u32 == STATUS_PENDING {
            if wait == 0 {
                set_error(WSA_IO_INCOMPLETE);
                return 0;
            }
            let h = if (*ov).event != 0 { (*ov).event } else { handle(s) };
            while core::ptr::read_volatile(&(*ov).internal) as u32 == STATUS_PENDING {
                crate::WaitForSingleObjectEx(h, u32::MAX, 0);
            }
        }
        let status = (*ov).internal as u32;
        *transferred = (*ov).internal_high as u32;
        *flags = 0;
        if is_error(status) {
            set_error(wsa_of_status(status));
            return 0;
        }
        if status == STATUS_BUFFER_OVERFLOW {
            set_error(WSAEMSGSIZE);
            return 0;
        }
    }
    1
}

// ----- select and WSAPoll

/// Wait for `entries` (handle, device interest): their `revents`, or the
/// WSA error. `timeout_ms` negative waits for ever.
fn poll_handles(entries: &[(u64, u32)], timeout_ms: i64) -> Result<Vec<u32>, u32> {
    check_started()?;
    let h = afd::control().map_err(|_| WSAENETDOWN)?;
    let req = proto::PollReq { timeout_ms, count: entries.len() as u32 }.encode(entries);
    let mut out = alloc::vec![0u8; entries.len() * 4];
    let (status, _) = afd::ioctl(h, proto::POLL, &req, &mut out);
    if is_error(status) {
        return Err(wsa_of_status(status));
    }
    Ok(out.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

/// An `fd_set`'s sockets (`u_int fd_count`, then `SOCKET fd_array[]` at
/// 8 on x64).
unsafe fn fd_list(set: *const u8) -> Vec<Socket> {
    if set.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller's fd_set, as long as its count says.
    unsafe {
        let n = core::ptr::read_unaligned(set.cast::<u32>()) as usize;
        (0..n).map(|i| core::ptr::read_unaligned(set.add(8 + 8 * i).cast::<usize>())).collect()
    }
}

unsafe fn fd_store(set: *mut u8, socks: &[Socket]) {
    if set.is_null() {
        return;
    }
    // SAFETY: the caller's fd_set held at least as many entries.
    unsafe {
        core::ptr::write_unaligned(set.cast::<u32>(), socks.len() as u32);
        for (i, s) in socks.iter().enumerate() {
            core::ptr::write_unaligned(set.add(8 + 8 * i).cast::<usize>(), *s);
        }
    }
}

/// `select` (Learn): `nfds` is ignored; the sets are rewritten to the
/// sockets that are ready; the count of ready entries in all three sets
/// is returned. No socket at all is `WSAEINVAL`; an entry that is not a
/// socket `WSAENOTSOCK`. `timeout` null waits for ever.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn select(
    _nfds: i32,
    readfds: *mut u8,
    writefds: *mut u8,
    exceptfds: *mut u8,
    timeout: *const i32,
) -> i32 {
    // SAFETY: the caller's sets.
    let (r, w, e) = unsafe { (fd_list(readfds), fd_list(writefds), fd_list(exceptfds)) };
    let mut all: Vec<Socket> = Vec::new();
    for s in r.iter().chain(&w).chain(&e) {
        if !all.contains(s) {
            all.push(*s);
        }
    }
    if all.is_empty() {
        return fail(WSAEINVAL);
    }
    let ms = if timeout.is_null() {
        -1
    } else {
        // SAFETY: the caller's `timeval` (two `long`s).
        let (sec, usec) = unsafe { (*timeout, *timeout.add(1)) };
        if sec < 0 || usec < 0 {
            return fail(WSAEINVAL);
        }
        i64::from(sec) * 1000 + (i64::from(usec) + 999) / 1000
    };
    let entries: Vec<(u64, u32)> =
        all.iter().map(|s| (*s as u64, poll::select_interest(r.contains(s), w.contains(s), e.contains(s)))).collect();
    let revents = match poll_handles(&entries, ms) {
        Ok(v) => v,
        Err(err) => return fail(err),
    };
    if revents.iter().any(|&b| b & poll::nval() != 0) {
        return fail(WSAENOTSOCK);
    }
    let ready = |set: &[Socket], which: usize| -> Vec<Socket> {
        set.iter()
            .copied()
            .filter(|s| {
                let i = all.iter().position(|a| a == s).unwrap_or(0);
                let (rd, wr, ex) = poll::select_ready(revents[i]);
                [rd, wr, ex][which]
            })
            .collect()
    };
    let (rr, wr, er) = (ready(&r, 0), ready(&w, 1), ready(&e, 2));
    // SAFETY: the caller's sets, rewritten no longer than they were.
    unsafe {
        fd_store(readfds, &rr);
        fd_store(writefds, &wr);
        fd_store(exceptfds, &er);
    }
    (rr.len() + wr.len() + er.len()) as i32
}

/// `__WSAFDIsSet` (the `FD_ISSET` macro).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn __WSAFDIsSet(s: Socket, set: *const u8) -> i32 {
    // SAFETY: the caller's set.
    i32::from(unsafe { fd_list(set) }.contains(&s))
}

/// `WSAPoll` (Learn): `WSAPOLLFD`s of 16 bytes (`SOCKET fd; SHORT events;
/// SHORT revents;`); a negative descriptor is skipped; returns how many
/// have `revents`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAPoll(fds: *mut u8, n: u32, timeout: i32) -> i32 {
    if let Err(e) = check_started() {
        return fail(e);
    }
    if fds.is_null() && n > 0 {
        return fail(WSAEFAULT);
    }
    let mut entries = Vec::new();
    let mut map = Vec::new();
    for i in 0..n as usize {
        // SAFETY: the caller's array of `n` entries.
        let (fd, events) = unsafe {
            let p = fds.add(i * WSAPOLLFD_SIZE);
            core::ptr::write_unaligned(p.add(10).cast::<i16>(), 0);
            (core::ptr::read_unaligned(p.cast::<usize>()), core::ptr::read_unaligned(p.add(8).cast::<i16>()))
        };
        if (fd as isize) < 0 {
            continue;
        }
        match poll::wsapoll_interest(events) {
            Ok(ev) => {
                entries.push((fd as u64, ev));
                map.push((i, events));
            }
            Err(e) => return fail(e),
        }
    }
    if entries.is_empty() {
        return fail(WSAEINVAL);
    }
    let revents = match poll_handles(&entries, i64::from(timeout)) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };
    let mut count = 0;
    for ((i, events), dev) in map.into_iter().zip(revents) {
        let r = poll::wsapoll_revents(events, dev);
        if r != 0 {
            count += 1;
        }
        // SAFETY: the caller's entry.
        unsafe { core::ptr::write_unaligned(fds.add(i * WSAPOLLFD_SIZE + 10).cast::<i16>(), r) };
    }
    count
}

// ----- events (Learn: "WSACreateEvent" ...: Win32 events)

/// `WSACreateEvent`: a manual-reset event, not signalled.
#[unsafe(no_mangle)]
pub extern "system" fn WSACreateEvent() -> Handle {
    // SAFETY: no preconditions.
    let h = unsafe { crate::CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    if h == 0 {
        set_error(WSA_NOT_ENOUGH_MEMORY);
    }
    h
}

/// `WSACloseEvent`.
#[unsafe(no_mangle)]
pub extern "system" fn WSACloseEvent(h: Handle) -> i32 {
    // SAFETY: no memory preconditions.
    let ok = unsafe { crate::CloseHandle(h) };
    if ok == 0 {
        set_error(WSA_INVALID_HANDLE);
    }
    ok
}

/// `WSASetEvent`.
#[unsafe(no_mangle)]
pub extern "system" fn WSASetEvent(h: Handle) -> i32 {
    // SAFETY: no memory preconditions.
    let ok = unsafe { crate::SetEvent(h) };
    if ok == 0 {
        set_error(WSA_INVALID_HANDLE);
    }
    ok
}

/// `WSAResetEvent`.
#[unsafe(no_mangle)]
pub extern "system" fn WSAResetEvent(h: Handle) -> i32 {
    // SAFETY: no memory preconditions.
    let ok = unsafe { crate::ResetEvent(h) };
    if ok == 0 {
        set_error(WSA_INVALID_HANDLE);
    }
    ok
}

/// `WSAWaitForMultipleEvents`: `WaitForMultipleObjectsEx`, whose results
/// are the `WSA_WAIT_*` values (`winsock2.h` defines them so).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn WSAWaitForMultipleEvents(
    n: u32,
    events: *const Handle,
    all: i32,
    ms: u32,
    alertable: i32,
) -> u32 {
    // SAFETY: the caller's array of `n` handles.
    unsafe { crate::WaitForMultipleObjectsEx(n, events, all, ms, alertable) }
}
