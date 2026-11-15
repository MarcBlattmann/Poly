//! Our `mswsock.dll` (ADR 0195): the Microsoft extensions to Winsock that programs reach by name
//! (`AcceptEx`, `GetAcceptExSockaddrs`, `TransmitFile`) or through
//! `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` (`ConnectEx`, `DisconnectEx`; `ws2_32` hands out
//! the pointers of this DLL's exports). The overlapped ones are requests of `\Device\Afd`
//! ([`poly_winsock::afd::ACCEPT_EX`], [`poly_winsock::afd::CONNECT_EX`]): they always pend, the
//! kernel's worker finishes them and delivers the completion (event, status block, completion port)
//! like any asynchronous request (ADR 0067).
//!
//! Sources (clean room): Learn's pages of `AcceptEx`, `GetAcceptExSockaddrs`, `ConnectEx`,
//! `DisconnectEx`, `TransmitFile`, "OVERLAPPED"; `mswsock.h`, `winsock2.h`, `winerror.h`; the spec
//! sheet of `AcceptEx` and its visible tests.

#![no_std]
#![allow(clippy::missing_safety_doc)]

use poly_win32_netrt as _;
use poly_win32_rt as _;
use poly_winsock::afd as proto;
use poly_winsock::consts::*;
use poly_winsock::error::{STATUS_PENDING, wsa_of_status};

type Handle = isize;

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtDeviceIoControlFile(
        handle: Handle,
        event: Handle,
        apc: *const u8,
        apc_context: *const u8,
        iosb: *mut Overlapped,
        code: u32,
        input: *const u8,
        input_len: u32,
        output: *mut u8,
        output_len: u32,
    ) -> u32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn SetLastError(e: u32);
}

/// `OVERLAPPED` (`minwinbase.h`); `Internal` and `InternalHigh` are the status block of the request.
#[repr(C)]
pub struct Overlapped {
    internal: usize,
    internal_high: usize,
    offset: u64,
    event: Handle,
}

/// `STATUS_*` severity bits: an error status completes a call with an error.
fn is_error(status: u32) -> bool {
    status >= 0xC000_0000
}

fn fail(e: u32) -> i32 {
    // SAFETY: no preconditions.
    unsafe { SetLastError(e) };
    0
}

/// Make an overlapped request on `s`: `FALSE` with `WSA_IO_PENDING` while it waits, `TRUE` if it
/// finished at once, `FALSE` with the error otherwise.
unsafe fn request(s: usize, code: u32, input: &[u8], ov: *mut Overlapped, count: *mut u32) -> i32 {
    // SAFETY: the caller's OVERLAPPED, which outlives the request (Learn: "OVERLAPPED").
    unsafe {
        (*ov).internal = STATUS_PENDING as usize;
        (*ov).internal_high = 0;
        let status = NtDeviceIoControlFile(
            s as Handle,
            (*ov).event,
            core::ptr::null(),
            ov.cast_const().cast::<u8>(),
            ov,
            code,
            input.as_ptr(),
            input.len() as u32,
            core::ptr::null_mut(),
            0,
        );
        if status == STATUS_PENDING {
            return fail(WSA_IO_PENDING);
        }
        if is_error(status) {
            return fail(wsa_of_status(status));
        }
        if !count.is_null() {
            *count = (*ov).internal_high as u32;
        }
        1
    }
}

/// `AcceptEx(sListenSocket, sAcceptSocket, lpOutputBuffer, dwReceiveDataLength,
/// dwLocalAddressLength, dwRemoteAddressLength, lpdwBytesReceived, lpOverlapped)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn AcceptEx(
    listen: usize,
    accept: usize,
    buf: *mut u8,
    recv_len: u32,
    local_len: u32,
    remote_len: u32,
    received: *mut u32,
    ov: *mut Overlapped,
) -> i32 {
    if ov.is_null() {
        return fail(WSA_INVALID_PARAMETER);
    }
    if buf.is_null() {
        return fail(WSAEFAULT);
    }
    let req = proto::AcceptExReq { accept: accept as u64, buf: buf as u64, recv_len, local_len, remote_len };
    // SAFETY: the caller's OVERLAPPED and count.
    unsafe { request(listen, proto::ACCEPT_EX, &req.encode(), ov, received) }
}

/// `GetAcceptExSockaddrs(lpOutputBuffer, dwReceiveDataLength, dwLocalAddressLength,
/// dwRemoteAddressLength, LocalSockaddr, LocalSockaddrLength, RemoteSockaddr, RemoteSockaddrLength)`:
/// where `AcceptEx` put the two addresses (after the data, each in the length given) and their sizes.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn GetAcceptExSockaddrs(
    buf: *const u8,
    recv_len: u32,
    local_len: u32,
    _remote_len: u32,
    local: *mut *const u8,
    local_size: *mut i32,
    remote: *mut *const u8,
    remote_size: *mut i32,
) {
    if buf.is_null() {
        return;
    }
    // SAFETY: the buffer `AcceptEx` filled and the caller's out pointers.
    unsafe {
        let l = buf.add(recv_len as usize);
        let r = l.add(local_len as usize);
        let size = |p: *const u8| {
            let family = core::ptr::read_unaligned(p.cast::<u16>());
            if i32::from(family) == AF_INET6 { 28 } else { 16 }
        };
        if !local.is_null() {
            *local = l;
        }
        if !local_size.is_null() {
            *local_size = size(l);
        }
        if !remote.is_null() {
            *remote = r;
        }
        if !remote_size.is_null() {
            *remote_size = size(r);
        }
    }
}

/// `ConnectEx(s, name, namelen, lpSendBuffer, dwSendDataLength, lpdwBytesSent, lpOverlapped)`.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn ConnectEx(
    s: usize,
    name: *const u8,
    name_len: i32,
    send: *const u8,
    send_len: u32,
    sent: *mut u32,
    ov: *mut Overlapped,
) -> i32 {
    if ov.is_null() {
        return fail(WSA_INVALID_PARAMETER);
    }
    if name.is_null() || name_len < 0 || (send.is_null() && send_len > 0) {
        return fail(WSAEFAULT);
    }
    let mut addr = [0u8; poly_winsock::addr::MAX_SIZE];
    let n = (name_len as usize).min(addr.len());
    // SAFETY: the caller's `name_len` bytes (at most the 28 an address has).
    unsafe { core::ptr::copy_nonoverlapping(name, addr.as_mut_ptr(), n) };
    let req = proto::ConnectExReq { buf: send as u64, len: send_len, addr_len: n as u32, addr };
    // SAFETY: the caller's OVERLAPPED and count.
    unsafe { request(s, proto::CONNECT_EX, &req.encode(), ov, sent) }
}

#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn shutdown(s: usize, how: i32) -> i32;
}

/// `DisconnectEx(s, lpOverlapped, dwFlags, dwReserved)`: the connection is shut down (both
/// directions); Poly's sockets are not reused, so `TF_REUSE_SOCKET` only ends the connection.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DisconnectEx(s: usize, ov: *mut Overlapped, _flags: u32, _reserved: u32) -> i32 {
    // SAFETY: a plain call; the OVERLAPPED, if any, is the caller's.
    unsafe {
        if shutdown(s, SD_BOTH) != 0 {
            return 0;
        }
        if !ov.is_null() {
            (*ov).internal = 0;
            (*ov).internal_high = 0;
        }
    }
    1
}

/// `TransmitFile`: not supported (`WSAEOPNOTSUPP`).
#[unsafe(no_mangle)]
pub unsafe extern "system" fn TransmitFile(
    _s: usize,
    _file: Handle,
    _bytes: u32,
    _per_send: u32,
    _ov: *mut Overlapped,
    _buffers: *const u8,
    _flags: u32,
) -> i32 {
    fail(WSAEOPNOTSUPP)
}

/// The entry point (`DllMain`): nothing to set up.
#[unsafe(no_mangle)]
pub extern "system" fn mswsock_main(_module: *mut u8, _reason: u32, _reserved: *mut u8) -> i32 {
    1
}
