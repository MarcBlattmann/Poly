//! The extension functions `WSAIoctl(SIO_GET_EXTENSION_FUNCTION_POINTER)` hands out (Learn:
//! "WSAIoctl", "SIO_GET_EXTENSION_FUNCTION_POINTER"; `mswsock.h`, `ws2def.h` for the GUIDs). Chrome
//! asks every UDP socket for `WSARecvMsg` and calls it through the pointer; `WSASendMsg` is the
//! same kind. `AcceptEx`, `GetAcceptExSockaddrs`, `ConnectEx`, `DisconnectEx` and `TransmitFile`
//! are `mswsock.dll`'s: the pointer is its export of that name (the same pointer for every socket,
//! as on Windows).

use crate::sock::{Overlapped, Socket, WsaBuf};
use crate::{Handle, fail};
use poly_winsock::consts::*;

#[cfg_attr(target_arch = "x86", link(name = "kernel32", kind = "raw-dylib", import_name_type = "undecorated"))]
#[cfg_attr(not(target_arch = "x86"), link(name = "kernel32", kind = "raw-dylib"))]
unsafe extern "system" {
    fn LoadLibraryW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> usize;
}

/// `WSAID_*` (`mswsock.h`, `ws2def.h`): `Data1`, `Data2`, `Data3`, `Data4` as in memory.
const fn guid(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> [u8; 16] {
    let a = d1.to_le_bytes();
    let b = d2.to_le_bytes();
    let c = d3.to_le_bytes();
    [a[0], a[1], a[2], a[3], b[0], b[1], c[0], c[1], d4[0], d4[1], d4[2], d4[3], d4[4], d4[5], d4[6], d4[7]]
}

const ACCEPTEX: [u8; 16] = guid(0xB536_7DF1, 0xCBAC, 0x11CF, [0x95, 0xCA, 0x00, 0x80, 0x5F, 0x48, 0xA1, 0x92]);
const GETACCEPTEXSOCKADDRS: [u8; 16] =
    guid(0xB536_7DF2, 0xCBAC, 0x11CF, [0x95, 0xCA, 0x00, 0x80, 0x5F, 0x48, 0xA1, 0x92]);
const TRANSMITFILE: [u8; 16] = guid(0xB536_7DF0, 0xCBAC, 0x11CF, [0x95, 0xCA, 0x00, 0x80, 0x5F, 0x48, 0xA1, 0x92]);
const CONNECTEX: [u8; 16] = guid(0x25A2_07B9, 0xDDF3, 0x4660, [0x8E, 0xE9, 0x76, 0xE5, 0x8C, 0x74, 0x06, 0x3E]);
const DISCONNECTEX: [u8; 16] = guid(0x7FDA_2E11, 0x8630, 0x436F, [0xA0, 0x31, 0xF5, 0x36, 0xA6, 0xEE, 0xC1, 0x57]);
const WSARECVMSG: [u8; 16] = guid(0xF689_D7C8, 0x6F1F, 0x436B, [0x8A, 0x53, 0xE5, 0x4F, 0xE3, 0x51, 0xC3, 0x22]);
const WSASENDMSG: [u8; 16] = guid(0xA441_E712, 0x754F, 0x43CA, [0x84, 0xA7, 0x0D, 0xEE, 0x44, 0xCF, 0x60, 0x6D]);

/// `mswsock.dll`'s export `name` (loaded on first use).
fn mswsock(name: &core::ffi::CStr) -> Option<usize> {
    let dll: [u16; 12] = [109, 115, 119, 115, 111, 99, 107, 46, 100, 108, 108, 0];
    // SAFETY: kernel32 calls with NUL-terminated names.
    unsafe {
        let m = LoadLibraryW(dll.as_ptr());
        if m == 0 {
            return None;
        }
        let f = GetProcAddress(m, name.as_ptr().cast());
        (f != 0).then_some(f)
    }
}

/// The function a GUID names, or `None` for one Poly does not know.
pub fn pointer(guid: &[u8]) -> Option<usize> {
    let g: &[u8; 16] = guid.get(..16)?.try_into().ok()?;
    if *g == WSARECVMSG {
        Some(WSARecvMsg as *const () as usize)
    } else if *g == WSASENDMSG {
        Some(WSASendMsg as *const () as usize)
    } else if *g == ACCEPTEX {
        mswsock(c"AcceptEx")
    } else if *g == GETACCEPTEXSOCKADDRS {
        mswsock(c"GetAcceptExSockaddrs")
    } else if *g == CONNECTEX {
        mswsock(c"ConnectEx")
    } else if *g == DISCONNECTEX {
        mswsock(c"DisconnectEx")
    } else if *g == TRANSMITFILE {
        mswsock(c"TransmitFile")
    } else {
        None
    }
}

/// `WSAMSG` (`ws2def.h`, x64): `name` 0, `namelen` 8, `lpBuffers` 16, `dwBufferCount` 24, `Control`
/// (`WSABUF`) 32, `dwFlags` 48.
#[repr(C)]
pub struct WsaMsg {
    pub name: *mut u8,
    pub name_len: i32,
    pub buffers: *mut WsaBuf,
    pub buffer_count: u32,
    pub control: WsaBuf,
    pub flags: u32,
}

type Routine = Option<unsafe extern "system" fn(u32, u32, *mut Overlapped, u32)>;

/// `WSARecvMsg(s, lpMsg, lpdwNumberOfBytesRecvd, lpOverlapped, lpCompletionRoutine)`: a receive
/// with the sender's address (`lpMsg->name`); no control data is ever returned.
#[allow(non_snake_case)]
pub unsafe extern "system" fn WSARecvMsg(
    s: Socket,
    msg: *mut WsaMsg,
    received: *mut u32,
    ov: *mut Overlapped,
    routine: Routine,
) -> i32 {
    if msg.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's WSAMSG.
    unsafe {
        let m = &mut *msg;
        let mut flags = m.flags;
        let r = crate::sock::WSARecvFrom(
            s,
            m.buffers,
            m.buffer_count,
            received,
            &mut flags,
            m.name,
            if m.name.is_null() { core::ptr::null_mut() } else { &mut m.name_len },
            ov,
            routine,
        );
        m.control.len = 0;
        m.flags = if r == 0 { flags } else { m.flags };
        r
    }
}

/// `WSASendMsg(s, lpMsg, dwFlags, lpNumberOfBytesSent, lpOverlapped, lpCompletionRoutine)`.
#[allow(non_snake_case)]
pub unsafe extern "system" fn WSASendMsg(
    s: Socket,
    msg: *const WsaMsg,
    flags: u32,
    sent: *mut u32,
    ov: *mut Overlapped,
    routine: Routine,
) -> i32 {
    if msg.is_null() {
        return fail(WSAEFAULT);
    }
    // SAFETY: the caller's WSAMSG.
    unsafe {
        let m = &*msg;
        crate::sock::WSASendTo(s, m.buffers, m.buffer_count, sent, flags, m.name, m.name_len, ov, routine)
    }
}
