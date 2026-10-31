//! `SOCKADDR_IN` and `SOCKADDR_IN6` (`ws2def.h`, `ws2ipdef.h`): the
//! family in host order, the port and the address in network order.
//!
//! | Structure | Size | Layout |
//! |---|---|---|
//! | `SOCKADDR_IN` | 16 | family (2), port (2), address (4), zero (8) |
//! | `SOCKADDR_IN6` | 28 | family (2), port (2), flow info (4), address (16), scope id (4) |

use crate::consts::*;
use core::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

pub const SOCKADDR_IN_SIZE: usize = 16;
pub const SOCKADDR_IN6_SIZE: usize = 28;
/// Room for either (`SOCKADDR_STORAGE` is larger; 28 bytes hold both
/// families Poly has).
pub const MAX_SIZE: usize = SOCKADDR_IN6_SIZE;

/// The size of a family's structure (`None` for other families).
pub fn size_of_family(family: i32) -> Option<usize> {
    match family {
        AF_INET => Some(SOCKADDR_IN_SIZE),
        AF_INET6 => Some(SOCKADDR_IN6_SIZE),
        _ => None,
    }
}

/// The family field of a socket address (0 if too short to have one).
pub fn family(bytes: &[u8]) -> i32 {
    match bytes {
        [a, b, ..] => i32::from(u16::from_le_bytes([*a, *b])),
        _ => AF_UNSPEC,
    }
}

/// Read a socket address of `len` bytes. Too short for its family is
/// `WSAEFAULT` (Learn: "bind", "connect": `namelen` too small); a family
/// other than `AF_INET` / `AF_INET6` is `WSAEAFNOSUPPORT`.
pub fn decode(bytes: &[u8]) -> Result<SocketAddr, u32> {
    if bytes.len() < 2 {
        return Err(WSAEFAULT);
    }
    let fam = family(bytes);
    let need = size_of_family(fam).ok_or(WSAEAFNOSUPPORT)?;
    if bytes.len() < need {
        return Err(WSAEFAULT);
    }
    let port = u16::from_be_bytes([bytes[2], bytes[3]]);
    Ok(if fam == AF_INET {
        let ip = Ipv4Addr::new(bytes[4], bytes[5], bytes[6], bytes[7]);
        SocketAddr::V4(SocketAddrV4::new(ip, port))
    } else {
        let flow = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let mut a = [0u8; 16];
        a.copy_from_slice(&bytes[8..24]);
        let scope = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
        SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::from(a), port, flow, scope))
    })
}

/// Write a socket address: its bytes (only the first `len` count) and
/// `len`.
pub fn encode(a: &SocketAddr) -> ([u8; MAX_SIZE], usize) {
    let mut b = [0u8; MAX_SIZE];
    b[2..4].copy_from_slice(&a.port().to_be_bytes());
    match a {
        SocketAddr::V4(v) => {
            b[0..2].copy_from_slice(&(AF_INET as u16).to_le_bytes());
            b[4..8].copy_from_slice(&v.ip().octets());
            (b, SOCKADDR_IN_SIZE)
        }
        SocketAddr::V6(v) => {
            b[0..2].copy_from_slice(&(AF_INET6 as u16).to_le_bytes());
            b[4..8].copy_from_slice(&v.flowinfo().to_be_bytes());
            b[8..24].copy_from_slice(&v.ip().octets());
            b[24..28].copy_from_slice(&v.scope_id().to_le_bytes());
            (b, SOCKADDR_IN6_SIZE)
        }
    }
}

/// The family of an address as `AF_*`.
pub fn af_of(a: &SocketAddr) -> i32 {
    if a.is_ipv4() { AF_INET } else { AF_INET6 }
}

/// The unspecified address of family `af` with port 0.
pub fn any(af: i32) -> Option<SocketAddr> {
    match af {
        AF_INET => Some(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0))),
        AF_INET6 => Some(SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn ipv4_round_trip() {
        let a: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let (b, n) = encode(&a);
        assert_eq!(n, 16);
        assert_eq!(&b[..8], &[2, 0, 0x1f, 0x90, 127, 0, 0, 1]);
        assert_eq!(&b[8..16], &[0; 8]);
        assert_eq!(decode(&b[..n]), Ok(a));
        // Longer buffers are fine (a SOCKADDR_STORAGE).
        assert_eq!(decode(&b), Ok(a));
    }

    #[test]
    fn ipv6_round_trip() {
        let a = SocketAddr::V6(SocketAddrV6::new("fe80::1".parse().unwrap(), 443, 0x12345, 3));
        let (b, n) = encode(&a);
        assert_eq!(n, 28);
        assert_eq!(&b[..4], &[23, 0, 0x01, 0xbb]);
        assert_eq!(&b[4..8], &[0, 1, 0x23, 0x45]);
        assert_eq!(b[8], 0xfe);
        assert_eq!(&b[24..28], &[3, 0, 0, 0]);
        assert_eq!(decode(&b[..n]), Ok(a));
    }

    #[test]
    fn errors() {
        let (b, _) = encode(&"10.0.0.1:1".parse().unwrap());
        assert_eq!(decode(&b[..15]), Err(WSAEFAULT));
        assert_eq!(decode(&b[..1]), Err(WSAEFAULT));
        let mut x = b;
        x[0] = 1;
        assert_eq!(decode(&x[..16]), Err(WSAEAFNOSUPPORT));
        let (b6, _) = encode(&"[::1]:1".parse().unwrap());
        assert_eq!(decode(&b6[..16]), Err(WSAEFAULT));
        assert_eq!(any(AF_INET6).unwrap().to_string(), "[::]:0");
        assert_eq!(size_of_family(AF_UNSPEC), None);
    }
}
