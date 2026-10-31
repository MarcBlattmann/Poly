//! Poly's own protocol between `ws2_32` and `\Device\Afd` (ADR 0095).
//!
//! A socket is a file object on `\Device\Afd\Endpoint` (opened with
//! `NtCreateFile`, closed with `NtClose`); everything else is
//! `NtDeviceIoControlFile` with the codes below. Windows' own driver
//! interface behind Winsock is not documented, so these codes, names and
//! buffers are Poly's, chosen like the console's (ADR 0080):
//! `CTL_CODE(FILE_DEVICE_NETWORK, function, METHOD_NEITHER,
//! FILE_ANY_ACCESS)` with functions from 0x800, the range Learn ("Defining
//! I/O Control Codes") leaves to vendors. Buffers are little-endian with
//! fixed offsets; socket addresses inside them are the program's own
//! `SOCKADDR_IN` / `SOCKADDR_IN6` bytes ([`crate::addr`]); data buffers are
//! passed by address and length and read or written in the caller's
//! memory directly (no copy through the ioctl buffers). Results: the
//! `IO_STATUS_BLOCK`'s status, and its `Information` as the request says.
//!
//! | Code | Input | Output | `Information` |
//! |---|---|---|---|
//! | [`SOCKET`] | [`SocketReq`] | | |
//! | [`BIND`], [`CONNECT`] | a socket address | | |
//! | [`LISTEN`] | backlog (u32) | | |
//! | [`ACCEPT`] | the new endpoint's handle (u64) | the peer's address | its length |
//! | [`SEND`] | [`SendReq`] | | bytes sent |
//! | [`RECV`] | [`RecvReq`] | [`RecvReply`] | bytes received |
//! | [`SHUTDOWN`] | `SD_*` (u32) | | |
//! | [`NAME`] | 0 local, 1 peer (u32) | the address | its length |
//! | [`SET_OPTION`] | level, name (i32 each), value | | |
//! | [`GET_OPTION`] | level, name | the value | its length |
//! | [`SET_NONBLOCKING`] | 0 or 1 (u32) | | |
//! | [`READABLE`] | | bytes readable (u32) | 4 |
//! | [`POLL`] | [`PollReq`] and its entries | `revents` per entry (u32) | entries ready |
//! | [`NET_INFO`] | | [`NetInfo`] | its size |
//! | [`INFO`] | | [`Info`] | its size |
//!
//! [`SEND`] and [`RECV`] with [`FLAG_OVERLAPPED`] never wait in the call:
//! what cannot finish at once is pending (`STATUS_PENDING`) and completes
//! later through the event, the status block and the APC, like any
//! asynchronous request (ADR 0067). Without it a call waits as the
//! socket's mode says (non-blocking: `STATUS_CANT_WAIT`, else until done
//! or the socket's `SO_RCVTIMEO` / `SO_SNDTIMEO`).

use crate::addr::MAX_SIZE;

/// The device and the name `ws2_32` opens a new endpoint by.
pub const DEVICE: &str = "\\Device\\Afd";
pub const ENDPOINT: &str = "\\Device\\Afd\\Endpoint";

/// `FILE_DEVICE_NETWORK` (`winioctl.h`).
pub const FILE_DEVICE_NETWORK: u32 = 0x12;
/// `METHOD_NEITHER` (`winioctl.h`).
pub const METHOD_NEITHER: u32 = 3;

pub const fn code(function: u32) -> u32 {
    (FILE_DEVICE_NETWORK << 16) | (function << 2) | METHOD_NEITHER
}

pub const SOCKET: u32 = code(0x800);
pub const BIND: u32 = code(0x801);
pub const LISTEN: u32 = code(0x802);
pub const ACCEPT: u32 = code(0x803);
pub const CONNECT: u32 = code(0x804);
pub const SEND: u32 = code(0x805);
pub const RECV: u32 = code(0x806);
pub const SHUTDOWN: u32 = code(0x807);
pub const NAME: u32 = code(0x808);
pub const SET_OPTION: u32 = code(0x809);
pub const GET_OPTION: u32 = code(0x80A);
pub const SET_NONBLOCKING: u32 = code(0x80B);
pub const READABLE: u32 = code(0x80C);
pub const POLL: u32 = code(0x80D);
pub const NET_INFO: u32 = code(0x80E);
pub const INFO: u32 = code(0x80F);

/// Pend instead of waiting ([`SEND`], [`RECV`]).
pub const FLAG_OVERLAPPED: u32 = 0x1;
/// `MSG_PEEK`.
pub const FLAG_PEEK: u32 = 0x2;
/// `MSG_WAITALL`.
pub const FLAG_WAITALL: u32 = 0x4;

/// [`RecvReply::msg_flags`]: the datagram was longer than the buffer
/// (`MSG_TRUNC`; the request's status is `STATUS_BUFFER_OVERFLOW`).
pub const MSG_TRUNCATED: u32 = 0x0100;

/// [`POLL`]'s events are `WSAPoll`'s bits (`POLLRDNORM`, `POLLWRNORM`,
/// `POLLERR`, `POLLHUP`, `POLLNVAL`, as u32).
pub const POLL_ENTRY_SIZE: usize = 16;
pub const POLL_HEADER_SIZE: usize = 16;
/// The most entries one [`POLL`] takes.
pub const POLL_MAX: usize = 4096;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|x| {
        let mut a = [0u8; 8];
        a.copy_from_slice(x);
        u64::from_le_bytes(a)
    })
}

/// A 4-byte little-endian value in an ioctl buffer.
pub fn read_u32(b: &[u8], at: usize) -> Option<u32> {
    u32_at(b, at)
}

/// An 8-byte little-endian value in an ioctl buffer.
pub fn read_u64(b: &[u8], at: usize) -> Option<u64> {
    u64_at(b, at)
}

/// [`SOCKET`]: the arguments of `socket` / `WSASocket`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketReq {
    pub family: i32,
    pub ty: i32,
    pub protocol: i32,
}

impl SocketReq {
    pub const SIZE: usize = 12;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.family.to_le_bytes());
        b[4..8].copy_from_slice(&self.ty.to_le_bytes());
        b[8..12].copy_from_slice(&self.protocol.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<SocketReq> {
        Some(SocketReq { family: u32_at(b, 0)? as i32, ty: u32_at(b, 4)? as i32, protocol: u32_at(b, 8)? as i32 })
    }
}

/// [`SEND`]: flags, the destination (`addr_len` 0: none), the data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendReq {
    pub flags: u32,
    pub addr_len: u32,
    pub buf: u64,
    pub len: u64,
    pub addr: [u8; MAX_SIZE],
}

impl SendReq {
    pub const SIZE: usize = 24 + MAX_SIZE;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.flags.to_le_bytes());
        b[4..8].copy_from_slice(&self.addr_len.to_le_bytes());
        b[8..16].copy_from_slice(&self.buf.to_le_bytes());
        b[16..24].copy_from_slice(&self.len.to_le_bytes());
        b[24..].copy_from_slice(&self.addr);
        b
    }

    pub fn decode(b: &[u8]) -> Option<SendReq> {
        let mut addr = [0u8; MAX_SIZE];
        addr.copy_from_slice(b.get(24..24 + MAX_SIZE)?);
        Some(SendReq { flags: u32_at(b, 0)?, addr_len: u32_at(b, 4)?, buf: u64_at(b, 8)?, len: u64_at(b, 16)?, addr })
    }

    /// The destination's bytes, if any.
    pub fn to(&self) -> Option<&[u8]> {
        (self.addr_len != 0).then(|| &self.addr[..(self.addr_len as usize).min(MAX_SIZE)])
    }
}

/// [`RECV`]: flags and where the data goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecvReq {
    pub flags: u32,
    pub buf: u64,
    pub len: u64,
}

impl RecvReq {
    pub const SIZE: usize = 24;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.flags.to_le_bytes());
        b[8..16].copy_from_slice(&self.buf.to_le_bytes());
        b[16..24].copy_from_slice(&self.len.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<RecvReq> {
        Some(RecvReq { flags: u32_at(b, 0)?, buf: u64_at(b, 8)?, len: u64_at(b, 16)? })
    }
}

/// [`RECV`]'s output: the sender (datagrams; `from_len` 0 for a stream)
/// and [`MSG_TRUNCATED`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecvReply {
    pub from: [u8; MAX_SIZE],
    pub from_len: u32,
    pub msg_flags: u32,
}

impl RecvReply {
    pub const SIZE: usize = MAX_SIZE + 8;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[..MAX_SIZE].copy_from_slice(&self.from);
        b[MAX_SIZE..MAX_SIZE + 4].copy_from_slice(&self.from_len.to_le_bytes());
        b[MAX_SIZE + 4..].copy_from_slice(&self.msg_flags.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<RecvReply> {
        let mut from = [0u8; MAX_SIZE];
        from.copy_from_slice(b.get(..MAX_SIZE)?);
        Some(RecvReply { from, from_len: u32_at(b, MAX_SIZE)?, msg_flags: u32_at(b, MAX_SIZE + 4)? })
    }
}

/// [`POLL`]'s header: the timeout (milliseconds; negative waits for
/// ever) and the number of entries that follow, each a handle (u64) and
/// its events (u32, then 4 bytes of padding).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PollReq {
    pub timeout_ms: i64,
    pub count: u32,
}

impl PollReq {
    pub fn encode(&self, entries: &[(u64, u32)]) -> alloc::vec::Vec<u8> {
        let mut b = alloc::vec![0u8; POLL_HEADER_SIZE + entries.len() * POLL_ENTRY_SIZE];
        b[0..8].copy_from_slice(&self.timeout_ms.to_le_bytes());
        b[8..12].copy_from_slice(&self.count.to_le_bytes());
        for (i, (h, ev)) in entries.iter().enumerate() {
            let at = POLL_HEADER_SIZE + i * POLL_ENTRY_SIZE;
            b[at..at + 8].copy_from_slice(&h.to_le_bytes());
            b[at + 8..at + 12].copy_from_slice(&ev.to_le_bytes());
        }
        b
    }

    /// The header and its entries; `None` if the buffer is short or there
    /// are more than [`POLL_MAX`].
    pub fn decode(b: &[u8]) -> Option<(PollReq, alloc::vec::Vec<(u64, u32)>)> {
        let req = PollReq { timeout_ms: u64_at(b, 0)? as i64, count: u32_at(b, 8)? };
        if req.count as usize > POLL_MAX || b.len() < POLL_HEADER_SIZE + req.count as usize * POLL_ENTRY_SIZE {
            return None;
        }
        let mut v = alloc::vec::Vec::with_capacity(req.count as usize);
        for i in 0..req.count as usize {
            let at = POLL_HEADER_SIZE + i * POLL_ENTRY_SIZE;
            v.push((u64_at(b, at)?, u32_at(b, at + 8)?));
        }
        Some((req, v))
    }
}

/// [`NET_INFO`]: the DNS servers (IPv4, up to three) and the host name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetInfo {
    pub dns: [[u8; 4]; 3],
    pub dns_count: u32,
    pub host: [u8; 64],
    pub host_len: u32,
}

impl NetInfo {
    pub const SIZE: usize = 4 + 12 + 4 + 64;

    pub fn new(dns: &[[u8; 4]], host: &str) -> NetInfo {
        let mut n = NetInfo { dns: [[0; 4]; 3], dns_count: 0, host: [0; 64], host_len: 0 };
        for (i, d) in dns.iter().take(3).enumerate() {
            n.dns[i] = *d;
            n.dns_count += 1;
        }
        let h = host.as_bytes();
        let len = h.len().min(63);
        n.host[..len].copy_from_slice(&h[..len]);
        n.host_len = len as u32;
        n
    }

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.dns_count.to_le_bytes());
        for (i, d) in self.dns.iter().enumerate() {
            b[4 + i * 4..8 + i * 4].copy_from_slice(d);
        }
        b[16..20].copy_from_slice(&self.host_len.to_le_bytes());
        b[20..].copy_from_slice(&self.host);
        b
    }

    pub fn decode(b: &[u8]) -> Option<NetInfo> {
        let mut n = NetInfo { dns: [[0; 4]; 3], dns_count: u32_at(b, 0)?.min(3), host: [0; 64], host_len: 0 };
        for i in 0..3 {
            n.dns[i].copy_from_slice(b.get(4 + i * 4..8 + i * 4)?);
        }
        n.host_len = u32_at(b, 16)?.min(63);
        n.host.copy_from_slice(b.get(20..84)?);
        Some(n)
    }

    pub fn servers(&self) -> &[[u8; 4]] {
        &self.dns[..self.dns_count as usize]
    }

    pub fn host_name(&self) -> &str {
        core::str::from_utf8(&self.host[..self.host_len as usize]).unwrap_or("poly")
    }
}

/// [`INFO`]: what the endpoint is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Info {
    pub family: i32,
    pub ty: i32,
    pub protocol: i32,
    pub listening: u32,
    pub nonblocking: u32,
}

impl Info {
    pub const SIZE: usize = 20;

    pub fn encode(&self) -> [u8; Self::SIZE] {
        let mut b = [0u8; Self::SIZE];
        b[0..4].copy_from_slice(&self.family.to_le_bytes());
        b[4..8].copy_from_slice(&self.ty.to_le_bytes());
        b[8..12].copy_from_slice(&self.protocol.to_le_bytes());
        b[12..16].copy_from_slice(&self.listening.to_le_bytes());
        b[16..20].copy_from_slice(&self.nonblocking.to_le_bytes());
        b
    }

    pub fn decode(b: &[u8]) -> Option<Info> {
        Some(Info {
            family: u32_at(b, 0)? as i32,
            ty: u32_at(b, 4)? as i32,
            protocol: u32_at(b, 8)? as i32,
            listening: u32_at(b, 12)?,
            nonblocking: u32_at(b, 16)?,
        })
    }
}

/// `socket`'s checks (Learn: "socket"): the family (`WSAEAFNOSUPPORT`),
/// the type (`SOCK_RAW` and others: `WSAESOCKTNOSUPPORT`), the protocol
/// (`WSAEPROTOTYPE` when it belongs to the other type,
/// `WSAEPROTONOSUPPORT` when Poly has no such protocol). A zero protocol
/// becomes the type's (TCP for streams, UDP for datagrams); the request
/// returned is what [`SOCKET`] takes.
pub fn check_socket(family: i32, ty: i32, protocol: i32) -> Result<SocketReq, u32> {
    use crate::consts::*;
    if !matches!(family, AF_INET | AF_INET6) {
        return Err(WSAEAFNOSUPPORT);
    }
    let protocol = match (ty, protocol) {
        (SOCK_STREAM, 0 | IPPROTO_TCP) => IPPROTO_TCP,
        (SOCK_DGRAM, 0 | IPPROTO_UDP) => IPPROTO_UDP,
        (SOCK_STREAM, IPPROTO_UDP) | (SOCK_DGRAM, IPPROTO_TCP) => return Err(WSAEPROTOTYPE),
        (SOCK_STREAM | SOCK_DGRAM, _) => return Err(WSAEPROTONOSUPPORT),
        _ => return Err(WSAESOCKTNOSUPPORT),
    };
    Ok(SocketReq { family, ty, protocol })
}

/// [`SET_OPTION`]'s and [`GET_OPTION`]'s level and name, then the value.
pub fn option_header(level: i32, name: i32) -> [u8; 8] {
    let mut b = [0u8; 8];
    b[0..4].copy_from_slice(&level.to_le_bytes());
    b[4..8].copy_from_slice(&name.to_le_bytes());
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert_eq!(SOCKET, 0x0012_2003);
        assert_eq!(INFO, 0x0012_203F);
        let all = [
            SOCKET,
            BIND,
            LISTEN,
            ACCEPT,
            CONNECT,
            SEND,
            RECV,
            SHUTDOWN,
            NAME,
            SET_OPTION,
            GET_OPTION,
            SET_NONBLOCKING,
            READABLE,
            POLL,
            NET_INFO,
            INFO,
        ];
        for (i, a) in all.iter().enumerate() {
            assert!(all[i + 1..].iter().all(|b| b != a));
        }
    }

    #[test]
    fn round_trips() {
        let s = SocketReq { family: 23, ty: 2, protocol: 17 };
        assert_eq!(SocketReq::decode(&s.encode()), Some(s));
        let (addr, n) = crate::addr::encode(&"10.0.2.2:53".parse().unwrap());
        let send = SendReq { flags: FLAG_OVERLAPPED, addr_len: n as u32, buf: 0x1000, len: 99, addr };
        let d = SendReq::decode(&send.encode()).unwrap();
        assert_eq!(d, send);
        assert_eq!(d.to().map(|t| t.len()), Some(16));
        assert_eq!(SendReq { addr_len: 0, ..send }.to(), None);
        let r = RecvReq { flags: FLAG_PEEK | FLAG_WAITALL, buf: 0x2000, len: 7 };
        assert_eq!(RecvReq::decode(&r.encode()), Some(r));
        let rr = RecvReply { from: addr, from_len: 16, msg_flags: MSG_TRUNCATED };
        assert_eq!(RecvReply::decode(&rr.encode()), Some(rr));
        let entries = [(0x40u64, 0x100u32), (0x44, 0x10)];
        let p = PollReq { timeout_ms: -1, count: 2 }.encode(&entries);
        let (h, e) = PollReq::decode(&p).unwrap();
        assert_eq!(h, PollReq { timeout_ms: -1, count: 2 });
        assert_eq!(e, entries);
        assert_eq!(PollReq::decode(&p[..p.len() - 1]), None);
        let ni = NetInfo::new(&[[10, 0, 2, 3]], "poly");
        let d = NetInfo::decode(&ni.encode()).unwrap();
        assert_eq!(d.servers(), &[[10, 0, 2, 3]]);
        assert_eq!(d.host_name(), "poly");
        let i = Info { family: 2, ty: 1, protocol: 6, listening: 1, nonblocking: 0 };
        assert_eq!(Info::decode(&i.encode()), Some(i));
        assert_eq!(read_u32(&option_header(0xffff, 4), 4), Some(4));
    }

    #[test]
    fn socket_checks() {
        use crate::consts::*;
        assert_eq!(check_socket(AF_INET, SOCK_STREAM, 0), Ok(SocketReq { family: 2, ty: 1, protocol: 6 }));
        assert_eq!(check_socket(AF_INET6, SOCK_DGRAM, IPPROTO_UDP).map(|r| r.protocol), Ok(17));
        assert_eq!(check_socket(AF_UNSPEC, SOCK_STREAM, 0), Err(WSAEAFNOSUPPORT));
        assert_eq!(check_socket(AF_INET, SOCK_RAW, 0), Err(WSAESOCKTNOSUPPORT));
        assert_eq!(check_socket(AF_INET, SOCK_STREAM, IPPROTO_UDP), Err(WSAEPROTOTYPE));
        assert_eq!(check_socket(AF_INET, SOCK_DGRAM, 99), Err(WSAEPROTONOSUPPORT));
    }
}
