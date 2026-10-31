//! Socket errors on their way to a program: the kernel's stack speaks
//! Linux error numbers (`poly-net`), `\Device\Afd` completes requests with
//! an `NTSTATUS`, and `ws2_32` reports a WSA error code
//! (`WSAGetLastError`). Both tables are Poly's: each `NTSTATUS` is the
//! `ntstatus.h` value whose documented meaning is nearest, and every one
//! the first table produces maps back to the WSA code Learn's "Winsock
//! Error Codes" page describes for the situation. A host test checks the
//! round trip, so the pair stays consistent.

use crate::consts::*;

// `ntstatus.h`.
pub const STATUS_SUCCESS: u32 = 0;
pub const STATUS_PENDING: u32 = 0x0000_0103;
pub const STATUS_BUFFER_OVERFLOW: u32 = 0x8000_0005;
pub const STATUS_UNSUCCESSFUL: u32 = 0xC000_0001;
pub const STATUS_NOT_IMPLEMENTED: u32 = 0xC000_0002;
pub const STATUS_INVALID_HANDLE: u32 = 0xC000_0008;
pub const STATUS_INVALID_PARAMETER: u32 = 0xC000_000D;
pub const STATUS_INVALID_DEVICE_REQUEST: u32 = 0xC000_0010;
pub const STATUS_ACCESS_DENIED: u32 = 0xC000_0022;
pub const STATUS_BUFFER_TOO_SMALL: u32 = 0xC000_0023;
pub const STATUS_OBJECT_TYPE_MISMATCH: u32 = 0xC000_0024;
pub const STATUS_INSUFFICIENT_RESOURCES: u32 = 0xC000_009A;
pub const STATUS_DEVICE_NOT_READY: u32 = 0xC000_00A3;
pub const STATUS_PIPE_DISCONNECTED: u32 = 0xC000_00B0;
pub const STATUS_IO_TIMEOUT: u32 = 0xC000_00B5;
pub const STATUS_NOT_SUPPORTED: u32 = 0xC000_00BB;
pub const STATUS_NETWORK_BUSY: u32 = 0xC000_00BF;
pub const STATUS_CANT_WAIT: u32 = 0xC000_00D8;
pub const STATUS_CANCELLED: u32 = 0xC000_0120;
pub const STATUS_INVALID_CONNECTION: u32 = 0xC000_0140;
pub const STATUS_INVALID_ADDRESS: u32 = 0xC000_0141;
pub const STATUS_INVALID_DEVICE_STATE: u32 = 0xC000_0184;
pub const STATUS_INVALID_ADDRESS_COMPONENT: u32 = 0xC000_0207;
pub const STATUS_ADDRESS_ALREADY_EXISTS: u32 = 0xC000_020A;
pub const STATUS_CONNECTION_DISCONNECTED: u32 = 0xC000_020C;
pub const STATUS_CONNECTION_RESET: u32 = 0xC000_020D;
pub const STATUS_CONNECTION_REFUSED: u32 = 0xC000_0236;
pub const STATUS_ADDRESS_NOT_ASSOCIATED: u32 = 0xC000_0239;
pub const STATUS_CONNECTION_ACTIVE: u32 = 0xC000_023B;
pub const STATUS_NETWORK_UNREACHABLE: u32 = 0xC000_023C;
pub const STATUS_HOST_UNREACHABLE: u32 = 0xC000_023D;
pub const STATUS_PROTOCOL_UNREACHABLE: u32 = 0xC000_023E;
pub const STATUS_CONNECTION_ABORTED: u32 = 0xC000_0241;
pub const STATUS_NO_MEMORY: u32 = 0xC000_0017;

// Linux error numbers (`errno.h`), as `poly-net` reports them.
pub const EINTR: u16 = 4;
pub const EBADF: u16 = 9;
pub const EAGAIN: u16 = 11;
pub const ENOMEM: u16 = 12;
pub const EACCES: u16 = 13;
pub const EFAULT: u16 = 14;
pub const EINVAL: u16 = 22;
pub const EPIPE: u16 = 32;
pub const ENOTSOCK: u16 = 88;
pub const EDESTADDRREQ: u16 = 89;
pub const EMSGSIZE: u16 = 90;
pub const ENOPROTOOPT: u16 = 92;
pub const EPROTONOSUPPORT: u16 = 93;
pub const ESOCKTNOSUPPORT: u16 = 94;
pub const EOPNOTSUPP: u16 = 95;
pub const EAFNOSUPPORT: u16 = 97;
pub const EADDRINUSE: u16 = 98;
pub const EADDRNOTAVAIL: u16 = 99;
pub const ENETDOWN: u16 = 100;
pub const ENETUNREACH: u16 = 101;
pub const ECONNABORTED: u16 = 103;
pub const ECONNRESET: u16 = 104;
pub const ENOBUFS: u16 = 105;
pub const EISCONN: u16 = 106;
pub const ENOTCONN: u16 = 107;
pub const ETIMEDOUT: u16 = 110;
pub const ECONNREFUSED: u16 = 111;
pub const EHOSTUNREACH: u16 = 113;
pub const EALREADY: u16 = 114;
pub const EINPROGRESS: u16 = 115;

/// The `NTSTATUS` `\Device\Afd` completes a request with when the stack
/// refused it with Linux error `errno`.
pub fn status_of_errno(errno: u16) -> u32 {
    match errno {
        EAGAIN | EINPROGRESS => STATUS_CANT_WAIT,
        EALREADY => STATUS_INVALID_DEVICE_STATE,
        EISCONN => STATUS_CONNECTION_ACTIVE,
        ENOTCONN => STATUS_INVALID_CONNECTION,
        EPIPE => STATUS_PIPE_DISCONNECTED,
        ECONNREFUSED => STATUS_CONNECTION_REFUSED,
        ECONNRESET => STATUS_CONNECTION_RESET,
        ECONNABORTED => STATUS_CONNECTION_ABORTED,
        ETIMEDOUT => STATUS_IO_TIMEOUT,
        ENETUNREACH => STATUS_NETWORK_UNREACHABLE,
        EHOSTUNREACH => STATUS_HOST_UNREACHABLE,
        ENETDOWN => STATUS_NETWORK_BUSY,
        EADDRINUSE => STATUS_ADDRESS_ALREADY_EXISTS,
        EADDRNOTAVAIL => STATUS_INVALID_ADDRESS_COMPONENT,
        EAFNOSUPPORT => STATUS_INVALID_ADDRESS,
        EDESTADDRREQ => STATUS_ADDRESS_NOT_ASSOCIATED,
        EMSGSIZE => STATUS_BUFFER_OVERFLOW,
        ENOPROTOOPT | EPROTONOSUPPORT | ESOCKTNOSUPPORT => STATUS_PROTOCOL_UNREACHABLE,
        EOPNOTSUPP => STATUS_NOT_SUPPORTED,
        ENOBUFS | ENOMEM => STATUS_INSUFFICIENT_RESOURCES,
        EACCES => STATUS_ACCESS_DENIED,
        EBADF | ENOTSOCK => STATUS_INVALID_HANDLE,
        EINTR => STATUS_CANCELLED,
        EFAULT => STATUS_ACCESS_DENIED,
        _ => STATUS_INVALID_PARAMETER,
    }
}

/// The WSA error `ws2_32` reports for a request `\Device\Afd` (or another
/// service) failed with `status`.
pub fn wsa_of_status(status: u32) -> u32 {
    match status {
        STATUS_SUCCESS => 0,
        STATUS_PENDING => WSA_IO_PENDING,
        STATUS_CANT_WAIT | STATUS_DEVICE_NOT_READY => WSAEWOULDBLOCK,
        STATUS_INVALID_DEVICE_STATE => WSAEALREADY,
        STATUS_CONNECTION_ACTIVE => WSAEISCONN,
        STATUS_INVALID_CONNECTION => WSAENOTCONN,
        STATUS_PIPE_DISCONNECTED => WSAESHUTDOWN,
        STATUS_CONNECTION_REFUSED => WSAECONNREFUSED,
        STATUS_CONNECTION_RESET | STATUS_CONNECTION_DISCONNECTED => WSAECONNRESET,
        STATUS_CONNECTION_ABORTED => WSAECONNABORTED,
        STATUS_IO_TIMEOUT => WSAETIMEDOUT,
        STATUS_NETWORK_UNREACHABLE => WSAENETUNREACH,
        STATUS_HOST_UNREACHABLE => WSAEHOSTUNREACH,
        STATUS_NETWORK_BUSY => WSAENETDOWN,
        STATUS_ADDRESS_ALREADY_EXISTS => WSAEADDRINUSE,
        STATUS_INVALID_ADDRESS_COMPONENT => WSAEADDRNOTAVAIL,
        STATUS_INVALID_ADDRESS => WSAEAFNOSUPPORT,
        STATUS_ADDRESS_NOT_ASSOCIATED => WSAEDESTADDRREQ,
        STATUS_BUFFER_OVERFLOW => WSAEMSGSIZE,
        STATUS_PROTOCOL_UNREACHABLE => WSAENOPROTOOPT,
        STATUS_NOT_SUPPORTED | STATUS_NOT_IMPLEMENTED | STATUS_INVALID_DEVICE_REQUEST => WSAEOPNOTSUPP,
        STATUS_INSUFFICIENT_RESOURCES | STATUS_NO_MEMORY => WSAENOBUFS,
        STATUS_ACCESS_DENIED => WSAEACCES,
        STATUS_INVALID_HANDLE | STATUS_OBJECT_TYPE_MISMATCH => WSAENOTSOCK,
        STATUS_CANCELLED => WSAEINTR,
        STATUS_BUFFER_TOO_SMALL => WSAEFAULT,
        _ => WSAEINVAL,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let pairs = [
            (EAGAIN, WSAEWOULDBLOCK),
            (EINPROGRESS, WSAEWOULDBLOCK),
            (EALREADY, WSAEALREADY),
            (EISCONN, WSAEISCONN),
            (ENOTCONN, WSAENOTCONN),
            (EPIPE, WSAESHUTDOWN),
            (ECONNREFUSED, WSAECONNREFUSED),
            (ECONNRESET, WSAECONNRESET),
            (ECONNABORTED, WSAECONNABORTED),
            (ETIMEDOUT, WSAETIMEDOUT),
            (ENETUNREACH, WSAENETUNREACH),
            (EHOSTUNREACH, WSAEHOSTUNREACH),
            (ENETDOWN, WSAENETDOWN),
            (EADDRINUSE, WSAEADDRINUSE),
            (EADDRNOTAVAIL, WSAEADDRNOTAVAIL),
            (EAFNOSUPPORT, WSAEAFNOSUPPORT),
            (EDESTADDRREQ, WSAEDESTADDRREQ),
            (EMSGSIZE, WSAEMSGSIZE),
            (ENOPROTOOPT, WSAENOPROTOOPT),
            (EOPNOTSUPP, WSAEOPNOTSUPP),
            (ENOBUFS, WSAENOBUFS),
            (EACCES, WSAEACCES),
            (EBADF, WSAENOTSOCK),
            (EINTR, WSAEINTR),
            (EINVAL, WSAEINVAL),
            (999, WSAEINVAL),
        ];
        for (e, w) in pairs {
            assert_eq!(wsa_of_status(status_of_errno(e)), w, "errno {e}");
        }
        assert_eq!(wsa_of_status(STATUS_PENDING), WSA_IO_PENDING);
        assert_eq!(wsa_of_status(STATUS_SUCCESS), 0);
        // Every errno maps to an error status (never success or pending).
        for e in 0..200 {
            let s = status_of_errno(e);
            assert!(s >= 0x8000_0000, "errno {e} -> {s:#x}");
        }
    }
}
