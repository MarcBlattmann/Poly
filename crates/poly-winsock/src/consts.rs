//! Winsock constants and structure sizes, from the public SDK headers
//! named on each group.

// ----- address families, socket types, protocols (ws2def.h, winsock2.h)

pub const AF_UNSPEC: i32 = 0;
pub const AF_INET: i32 = 2;
pub const AF_INET6: i32 = 23;

pub const SOCK_STREAM: i32 = 1;
pub const SOCK_DGRAM: i32 = 2;
pub const SOCK_RAW: i32 = 3;

pub const IPPROTO_IP: i32 = 0;
pub const IPPROTO_ICMP: i32 = 1;
pub const IPPROTO_TCP: i32 = 6;
pub const IPPROTO_UDP: i32 = 17;
pub const IPPROTO_IPV6: i32 = 41;
pub const IPPROTO_ICMPV6: i32 = 58;

/// `INVALID_SOCKET` (`winsock2.h`: `(SOCKET)(~0)`).
pub const INVALID_SOCKET: usize = !0;
/// `SOCKET_ERROR`.
pub const SOCKET_ERROR: i32 = -1;
/// `INADDR_NONE` (`ws2def.h`), `inet_addr`'s failure.
pub const INADDR_NONE: u32 = 0xffff_ffff;

// ----- WSASocket flags (winsock2.h)

pub const WSA_FLAG_OVERLAPPED: u32 = 0x01;
pub const WSA_FLAG_NO_HANDLE_INHERIT: u32 = 0x80;

// ----- send / recv flags (winsock2.h, ws2def.h)

pub const MSG_OOB: i32 = 0x1;
pub const MSG_PEEK: i32 = 0x2;
pub const MSG_DONTROUTE: i32 = 0x4;
pub const MSG_WAITALL: i32 = 0x8;
pub const MSG_PUSH_IMMEDIATE: i32 = 0x20;
pub const MSG_PARTIAL: i32 = 0x8000;
pub const MSG_TRUNC: i32 = 0x0100;

// ----- shutdown (winsock2.h)

pub const SD_RECEIVE: i32 = 0;
pub const SD_SEND: i32 = 1;
pub const SD_BOTH: i32 = 2;

// ----- socket options (winsock2.h, ws2def.h, ws2ipdef.h)

pub const SOL_SOCKET: i32 = 0xffff;
pub const SO_DEBUG: i32 = 0x0001;
pub const SO_ACCEPTCONN: i32 = 0x0002;
pub const SO_REUSEADDR: i32 = 0x0004;
pub const SO_KEEPALIVE: i32 = 0x0008;
pub const SO_DONTROUTE: i32 = 0x0010;
pub const SO_BROADCAST: i32 = 0x0020;
pub const SO_USELOOPBACK: i32 = 0x0040;
pub const SO_LINGER: i32 = 0x0080;
pub const SO_OOBINLINE: i32 = 0x0100;
/// `(int)(~SO_LINGER)`.
pub const SO_DONTLINGER: i32 = !SO_LINGER;
/// `(int)(~SO_REUSEADDR)`.
pub const SO_EXCLUSIVEADDRUSE: i32 = !SO_REUSEADDR;
pub const SO_SNDBUF: i32 = 0x1001;
pub const SO_RCVBUF: i32 = 0x1002;
pub const SO_SNDLOWAT: i32 = 0x1003;
pub const SO_RCVLOWAT: i32 = 0x1004;
pub const SO_SNDTIMEO: i32 = 0x1005;
pub const SO_RCVTIMEO: i32 = 0x1006;
pub const SO_ERROR: i32 = 0x1007;
pub const SO_TYPE: i32 = 0x1008;
/// `SO_CONDITIONAL_ACCEPT` (`ws2def.h`).
pub const SO_CONDITIONAL_ACCEPT: i32 = 0x3002;

pub const TCP_NODELAY: i32 = 0x0001;
/// `TCP_KEEPALIVE` / `TCP_KEEPIDLE` (`ws2ipdef.h`): seconds.
pub const TCP_KEEPALIVE: i32 = 3;
pub const TCP_KEEPCNT: i32 = 16;
pub const TCP_KEEPINTVL: i32 = 17;

pub const IP_TOS: i32 = 3;
pub const IP_TTL: i32 = 4;
pub const IPV6_UNICAST_HOPS: i32 = 4;
pub const IPV6_V6ONLY: i32 = 27;

/// `struct linger` (`winsock2.h`): two `u_short`s.
pub const LINGER_SIZE: usize = 4;

// ----- ioctlsocket / WSAIoctl (winsock2.h: _IOW('f', 126, u_long) ...)

pub const FIONBIO: u32 = 0x8004_667E;
pub const FIONREAD: u32 = 0x4004_667F;
pub const SIOCATMARK: u32 = 0x4004_7307;
/// `SIO_GET_EXTENSION_FUNCTION_POINTER` (`ws2def.h`:
/// `_WSAIORW(IOC_WS2, 6)`).
pub const SIO_GET_EXTENSION_FUNCTION_POINTER: u32 = 0xC800_0006;

// ----- WSAPoll (winsock2.h)

pub const POLLRDNORM: i16 = 0x0100;
pub const POLLRDBAND: i16 = 0x0200;
pub const POLLIN: i16 = POLLRDNORM | POLLRDBAND;
pub const POLLPRI: i16 = 0x0400;
pub const POLLWRNORM: i16 = 0x0010;
pub const POLLOUT: i16 = POLLWRNORM;
pub const POLLWRBAND: i16 = 0x0020;
pub const POLLERR: i16 = 0x0001;
pub const POLLHUP: i16 = 0x0002;
pub const POLLNVAL: i16 = 0x0004;
/// `WSAPOLLFD` on x64: `SOCKET fd; SHORT events; SHORT revents;` padded.
pub const WSAPOLLFD_SIZE: usize = 16;

/// `FD_SETSIZE` (`winsock2.h`, unless a program defines its own: the
/// count comes first, so `select` reads as many as it says).
pub const FD_SETSIZE: usize = 64;

// ----- getaddrinfo / getnameinfo (ws2def.h, ws2tcpip.h)

pub const AI_PASSIVE: i32 = 0x01;
pub const AI_CANONNAME: i32 = 0x02;
pub const AI_NUMERICHOST: i32 = 0x04;
pub const AI_NUMERICSERV: i32 = 0x08;
pub const AI_ALL: i32 = 0x0100;
pub const AI_ADDRCONFIG: i32 = 0x0400;
pub const AI_V4MAPPED: i32 = 0x0800;
/// The flags Poly accepts; any other bit is `WSAEINVAL` (`EAI_BADFLAGS`).
pub const AI_KNOWN: i32 =
    AI_PASSIVE | AI_CANONNAME | AI_NUMERICHOST | AI_NUMERICSERV | AI_ALL | AI_ADDRCONFIG | AI_V4MAPPED;

/// `ADDRINFOA` / `ADDRINFOW` on x64: four `int`s, `size_t ai_addrlen`,
/// then the name, address and next pointers.
pub const ADDRINFO_SIZE: usize = 48;
pub const ADDRINFO_FLAGS: usize = 0;
pub const ADDRINFO_FAMILY: usize = 4;
pub const ADDRINFO_SOCKTYPE: usize = 8;
pub const ADDRINFO_PROTOCOL: usize = 12;
pub const ADDRINFO_ADDRLEN: usize = 16;
pub const ADDRINFO_CANONNAME: usize = 24;
pub const ADDRINFO_ADDR: usize = 32;
pub const ADDRINFO_NEXT: usize = 40;

pub const NI_NOFQDN: i32 = 0x01;
pub const NI_NUMERICHOST: i32 = 0x02;
pub const NI_NAMEREQD: i32 = 0x04;
pub const NI_NUMERICSERV: i32 = 0x08;
pub const NI_DGRAM: i32 = 0x10;
pub const NI_MAXHOST: usize = 1025;
pub const NI_MAXSERV: usize = 32;

/// `INET_ADDRSTRLEN` / `INET6_ADDRSTRLEN` (`ws2ipdef.h`).
pub const INET_ADDRSTRLEN: usize = 22;
pub const INET6_ADDRSTRLEN: usize = 65;

// ----- WSADATA (winsock2.h, the _WIN64 layout)

pub const WSADESCRIPTION_LEN: usize = 256;
pub const WSASYS_STATUS_LEN: usize = 128;
/// `wVersion`, `wHighVersion`, `iMaxSockets`, `iMaxUdpDg` (two bytes
/// each), `lpVendorInfo` (8), `szDescription[257]`, `szSystemStatus[129]`,
/// padded to 8.
pub const WSADATA_SIZE: usize = 408;
pub const WSADATA_DESCRIPTION: usize = 16;
pub const WSADATA_STATUS: usize = WSADATA_DESCRIPTION + WSADESCRIPTION_LEN + 1;

// ----- hostent (winsock2.h, x64)

/// `h_name`, `h_aliases`, `h_addrtype` (short), `h_length` (short),
/// `h_addr_list`.
pub const HOSTENT_SIZE: usize = 32;

// ----- WSA error codes (winerror.h)

pub const WSA_INVALID_HANDLE: u32 = 6;
pub const WSA_NOT_ENOUGH_MEMORY: u32 = 8;
pub const WSA_INVALID_PARAMETER: u32 = 87;
pub const WSA_OPERATION_ABORTED: u32 = 995;
pub const WSA_IO_INCOMPLETE: u32 = 996;
pub const WSA_IO_PENDING: u32 = 997;
pub const WSAEINTR: u32 = 10004;
pub const WSAEBADF: u32 = 10009;
pub const WSAEACCES: u32 = 10013;
pub const WSAEFAULT: u32 = 10014;
pub const WSAEINVAL: u32 = 10022;
pub const WSAEMFILE: u32 = 10024;
pub const WSAEWOULDBLOCK: u32 = 10035;
pub const WSAEINPROGRESS: u32 = 10036;
pub const WSAEALREADY: u32 = 10037;
pub const WSAENOTSOCK: u32 = 10038;
pub const WSAEDESTADDRREQ: u32 = 10039;
pub const WSAEMSGSIZE: u32 = 10040;
pub const WSAEPROTOTYPE: u32 = 10041;
pub const WSAENOPROTOOPT: u32 = 10042;
pub const WSAEPROTONOSUPPORT: u32 = 10043;
pub const WSAESOCKTNOSUPPORT: u32 = 10044;
pub const WSAEOPNOTSUPP: u32 = 10045;
pub const WSAEPFNOSUPPORT: u32 = 10046;
pub const WSAEAFNOSUPPORT: u32 = 10047;
pub const WSAEADDRINUSE: u32 = 10048;
pub const WSAEADDRNOTAVAIL: u32 = 10049;
pub const WSAENETDOWN: u32 = 10050;
pub const WSAENETUNREACH: u32 = 10051;
pub const WSAENETRESET: u32 = 10052;
pub const WSAECONNABORTED: u32 = 10053;
pub const WSAECONNRESET: u32 = 10054;
pub const WSAENOBUFS: u32 = 10055;
pub const WSAEISCONN: u32 = 10056;
pub const WSAENOTCONN: u32 = 10057;
pub const WSAESHUTDOWN: u32 = 10058;
pub const WSAETIMEDOUT: u32 = 10060;
pub const WSAECONNREFUSED: u32 = 10061;
pub const WSAENAMETOOLONG: u32 = 10063;
pub const WSAEHOSTDOWN: u32 = 10064;
pub const WSAEHOSTUNREACH: u32 = 10065;
pub const WSASYSNOTREADY: u32 = 10091;
pub const WSAVERNOTSUPPORTED: u32 = 10092;
pub const WSANOTINITIALISED: u32 = 10093;
pub const WSAEDISCON: u32 = 10101;
pub const WSATYPE_NOT_FOUND: u32 = 10109;
pub const WSAHOST_NOT_FOUND: u32 = 11001;
pub const WSATRY_AGAIN: u32 = 11002;
pub const WSANO_RECOVERY: u32 = 11003;
pub const WSANO_DATA: u32 = 11004;

/// `EAI_*` (`ws2tcpip.h`): the WSA codes `getaddrinfo` returns.
pub const EAI_AGAIN: u32 = WSATRY_AGAIN;
pub const EAI_BADFLAGS: u32 = WSAEINVAL;
pub const EAI_FAIL: u32 = WSANO_RECOVERY;
pub const EAI_FAMILY: u32 = WSAEAFNOSUPPORT;
pub const EAI_MEMORY: u32 = WSA_NOT_ENOUGH_MEMORY;
pub const EAI_NONAME: u32 = WSAHOST_NOT_FOUND;
pub const EAI_SERVICE: u32 = WSATYPE_NOT_FOUND;
pub const EAI_SOCKTYPE: u32 = WSAESOCKTNOSUPPORT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_values() {
        assert_eq!(SO_DONTLINGER as u32, 0xFFFF_FF7F);
        assert_eq!(SO_EXCLUSIVEADDRUSE as u32, 0xFFFF_FFFB);
        assert_eq!(POLLIN, 0x0300);
        assert_eq!(WSADATA_STATUS + WSASYS_STATUS_LEN + 1, 402);
        const { assert!(WSADATA_SIZE >= 402 && WSADATA_SIZE.is_multiple_of(8)) };
        // _IOW('f', 126, u_long): IOC_IN | (4 << 16) | ('f' << 8) | 126.
        assert_eq!(FIONBIO, 0x8000_0000 | (4 << 16) | ((b'f' as u32) << 8) | 126);
        assert_eq!(FIONREAD, 0x4000_0000 | (4 << 16) | ((b'f' as u32) << 8) | 127);
    }
}
