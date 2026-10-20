//! ICMPv6 (RFC 4443) and the Neighbor Discovery messages this stack uses
//! (RFC 4861 sections 4.3, 4.4 and 4.6.1).

use super::checksum::{self, Sum};
use super::ethernet::MacAddr;
use super::{WireError, be32, proto, put16, put32};
use core::net::Ipv6Addr;

pub const HEADER_LEN: usize = 8;

pub const DEST_UNREACHABLE: u8 = 1;
pub const PACKET_TOO_BIG: u8 = 2;
pub const TIME_EXCEEDED: u8 = 3;
pub const PARAMETER_PROBLEM: u8 = 4;
pub const ECHO_REQUEST: u8 = 128;
pub const ECHO_REPLY: u8 = 129;
pub const ROUTER_SOLICIT: u8 = 133;
pub const ROUTER_ADVERT: u8 = 134;
pub const NEIGHBOR_SOLICIT: u8 = 135;
pub const NEIGHBOR_ADVERT: u8 = 136;

/// Destination unreachable codes (RFC 4443 3.1).
pub mod unreachable {
    pub const NO_ROUTE: u8 = 0;
    pub const ADMIN: u8 = 1;
    pub const ADDRESS: u8 = 3;
    pub const PORT: u8 = 4;
}

/// NDP option: source link-layer address.
pub const OPT_SOURCE_LLADDR: u8 = 1;
/// NDP option: target link-layer address.
pub const OPT_TARGET_LLADDR: u8 = 2;

/// Neighbor Advertisement flags (RFC 4861 4.4).
pub const NA_ROUTER: u32 = 0x8000_0000;
pub const NA_SOLICITED: u32 = 0x4000_0000;
pub const NA_OVERRIDE: u32 = 0x2000_0000;

/// A parsed message.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub ty: u8,
    pub code: u8,
    pub rest: u32,
    pub data: &'a [u8],
}

/// The pseudo-header sum for an ICMPv6 message of `len` bytes.
pub fn pseudo(src: Ipv6Addr, dst: Ipv6Addr, len: usize) -> Sum {
    checksum::pseudo_v6(src, dst, proto::ICMPV6, len)
}

/// Parse and verify the checksum, which covers the pseudo-header.
pub fn parse(buf: &[u8], src: Ipv6Addr, dst: Ipv6Addr) -> Result<Message<'_>, WireError> {
    if buf.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    if pseudo(src, dst, buf.len()).add_bytes(buf).fold() != 0xffff {
        return Err(WireError::Checksum);
    }
    Ok(Message { ty: buf[0], code: buf[1], rest: be32(buf, 4), data: &buf[HEADER_LEN..] })
}

/// Write the header of a message whose body is already in
/// `buf[HEADER_LEN..len]`, and checksum it.
pub fn emit(buf: &mut [u8], len: usize, ty: u8, code: u8, rest: u32, src: Ipv6Addr, dst: Ipv6Addr) {
    buf[0] = ty;
    buf[1] = code;
    put16(buf, 2, 0);
    put32(buf, 4, rest);
    let c = pseudo(src, dst, len).add_bytes(&buf[..len]).finish();
    put16(buf, 2, c);
}

/// Recompute the checksum of a complete message in place.
pub fn fix_checksum(buf: &mut [u8], src: Ipv6Addr, dst: Ipv6Addr) {
    put16(buf, 2, 0);
    let c = pseudo(src, dst, buf.len()).add_bytes(buf).finish();
    put16(buf, 2, c);
}

/// True for error messages (types below 128, RFC 4443 2.1).
pub fn is_error(ty: u8) -> bool {
    ty < 128
}

/// A Neighbor Solicitation or Advertisement body (after the 8-byte header:
/// a 16-byte target address, then options).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Neighbor {
    pub target: Ipv6Addr,
    /// The source (NS) or target (NA) link-layer address option.
    pub lladdr: Option<MacAddr>,
}

/// Parse an NS or NA body. `want` is the link-layer option type to pick out.
/// Options with length zero make the whole message invalid (RFC 4861 4.6).
pub fn parse_neighbor(data: &[u8], want: u8) -> Result<Neighbor, WireError> {
    if data.len() < 16 {
        return Err(WireError::Truncated);
    }
    let mut t = [0u8; 16];
    t.copy_from_slice(&data[..16]);
    let target = Ipv6Addr::from(t);
    let mut lladdr = None;
    let mut opts = &data[16..];
    while !opts.is_empty() {
        if opts.len() < 2 {
            return Err(WireError::Truncated);
        }
        let len = usize::from(opts[1]) * 8;
        if len == 0 {
            return Err(WireError::Malformed);
        }
        if len > opts.len() {
            return Err(WireError::Truncated);
        }
        if opts[0] == want && len >= 8 {
            let mut m = [0u8; 6];
            m.copy_from_slice(&opts[2..8]);
            lladdr = Some(MacAddr(m));
        }
        opts = &opts[len..];
    }
    Ok(Neighbor { target, lladdr })
}

/// Write an NS/NA body (target and one link-layer option) at `buf[HEADER_LEN..]`;
/// returns the message length.
pub fn write_neighbor(buf: &mut [u8], target: Ipv6Addr, opt: u8, mac: MacAddr) -> usize {
    buf[HEADER_LEN..HEADER_LEN + 16].copy_from_slice(&target.octets());
    let o = HEADER_LEN + 16;
    buf[o] = opt;
    buf[o + 1] = 1;
    buf[o + 2..o + 8].copy_from_slice(&mac.0);
    o + 8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_covers_pseudo_header() {
        let src: Ipv6Addr = "fe80::1".parse().unwrap();
        let dst: Ipv6Addr = "fe80::2".parse().unwrap();
        let mut buf = [0u8; 12];
        buf[8..].copy_from_slice(b"abcd");
        emit(&mut buf, 12, ECHO_REQUEST, 0, 7, src, dst);
        let m = parse(&buf, src, dst).unwrap();
        assert_eq!((m.ty, m.rest, m.data), (ECHO_REQUEST, 7, &b"abcd"[..]));
        let other: Ipv6Addr = "fe80::3".parse().unwrap();
        assert_eq!(parse(&buf, src, other).unwrap_err(), WireError::Checksum);
    }

    #[test]
    fn neighbor_options() {
        let mut buf = [0u8; 32];
        let t: Ipv6Addr = "fe80::2".parse().unwrap();
        let mac = MacAddr([2, 0, 0, 0, 0, 9]);
        let len = write_neighbor(&mut buf, t, OPT_SOURCE_LLADDR, mac);
        assert_eq!(len, 32);
        let n = parse_neighbor(&buf[8..len], OPT_SOURCE_LLADDR).unwrap();
        assert_eq!(n, Neighbor { target: t, lladdr: Some(mac) });
        assert_eq!(parse_neighbor(&buf[8..len], OPT_TARGET_LLADDR).unwrap().lladdr, None);
        buf[25] = 0;
        assert_eq!(parse_neighbor(&buf[8..len], OPT_SOURCE_LLADDR).unwrap_err(), WireError::Malformed);
        buf[25] = 2;
        assert_eq!(parse_neighbor(&buf[8..len], OPT_SOURCE_LLADDR).unwrap_err(), WireError::Truncated);
    }
}
