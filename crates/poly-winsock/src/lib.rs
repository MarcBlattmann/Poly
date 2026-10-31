//! Winsock without the machine (ADR 0095).
//!
//! Poly's `ws2_32.dll` (built in the Win32 workspace) keeps sockets as
//! file objects on `\Device\Afd`, a device of the NT personality backed by
//! the kernel's own TCP/IP stack (`poly-net`, ADR 0028). Everything that
//! needs neither the kernel nor a process is here, tested on the host:
//!
//! - [`consts`]: the constants of `winsock2.h`, `ws2def.h`, `ws2ipdef.h`
//!   and `ws2tcpip.h`, and the WSA error codes of `winerror.h`.
//! - [`addr`]: `SOCKADDR_IN` / `SOCKADDR_IN6` read and written.
//! - [`text`]: `inet_pton`, `inet_ntop`, `inet_addr` and `inet_ntoa`.
//! - [`error`]: the stack's errors as `NTSTATUS` (the personality) and
//!   `NTSTATUS` as WSA error codes (`ws2_32`).
//! - [`afd`]: Poly's own protocol between `ws2_32` and `\Device\Afd`
//!   (`NtDeviceIoControlFile` codes and their buffers).
//! - [`dns`]: the stub resolver's queries and the answers it reads
//!   (RFC 1035, RFC 3596).
//! - [`resolve`]: `getaddrinfo`'s and `getnameinfo`'s decisions: hints,
//!   numeric hosts, `localhost`, service names, the results' order.
//! - [`exports`]: the names `ws2_32.dll` exports.
//! - [`poll`]: `select` and `WSAPoll` events over the device's readiness.
//!
//! Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's Winsock
//! reference (each function's page, "Winsock Error Codes", "SOL_SOCKET
//! Socket Options", "IPPROTO_TCP Socket Options", "IPPROTO_IPV6 Socket
//! Options", "WSAPOLLFD", "addrinfo", "sockaddr", "Handling Winsock
//! Errors"); the public SDK headers' constants and layouts (`winsock2.h`,
//! `ws2def.h`, `ws2ipdef.h`, `ws2tcpip.h`, `winerror.h`, `ntstatus.h`,
//! `winioctl.h`); RFC 1035, RFC 3596, RFC 4291 and RFC 5952 for DNS and
//! IPv6 text. No Windows binary, Wine or ReactOS source was looked at.

#![no_std]

extern crate alloc;

pub mod addr;
pub mod afd;
pub mod consts;
pub mod dns;
pub mod error;
pub mod exports;
pub mod poll;
pub mod resolve;
pub mod text;

/// `MAKEWORD(2, 2)`: the highest Winsock version Poly offers.
pub const HIGHEST_VERSION: u16 = 0x0202;

/// `WSAStartup`'s version negotiation (Learn: "WSAStartup"): `requested`
/// is `MAKEWORD(major, minor)`. A request below 1.0 is
/// `WSAVERNOTSUPPORTED`; otherwise the version used is the lower of the
/// request and 2.2. Returns `(wVersion, wHighVersion)`.
pub fn negotiate(requested: u16) -> Result<(u16, u16), u32> {
    let major = requested & 0xff;
    let minor = requested >> 8;
    if major == 0 {
        return Err(consts::WSAVERNOTSUPPORTED);
    }
    let used = if (major, minor) > (2, 2) { HIGHEST_VERSION } else { requested };
    Ok((used, HIGHEST_VERSION))
}

/// Byte-order swaps (`htons`, `htonl`, `ntohs`, `ntohl`): Windows on x64
/// is little-endian, network order big-endian.
pub const fn swap16(v: u16) -> u16 {
    v.swap_bytes()
}

pub const fn swap32(v: u32) -> u32 {
    v.swap_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(negotiate(0x0202), Ok((0x0202, 0x0202)));
        assert_eq!(negotiate(0x0101), Ok((0x0101, 0x0202)));
        assert_eq!(negotiate(0x0002), Ok((0x0002, 0x0202)));
        // 2.3 and 3.0 are above what Poly has: 2.2.
        assert_eq!(negotiate(0x0302), Ok((0x0202, 0x0202)));
        assert_eq!(negotiate(0x0003), Ok((0x0202, 0x0202)));
        assert_eq!(negotiate(0x0100), Err(consts::WSAVERNOTSUPPORTED));
        assert_eq!(negotiate(0), Err(consts::WSAVERNOTSUPPORTED));
    }

    #[test]
    fn swaps() {
        assert_eq!(swap16(0x1234), 0x3412);
        assert_eq!(swap32(0x1234_5678), 0x7856_3412);
    }
}
