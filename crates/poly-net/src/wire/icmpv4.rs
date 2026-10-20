//! ICMP for IPv4 (RFC 792): type, code, checksum over the whole message,
//! four type-specific bytes, then data.

use super::checksum;
use super::{WireError, be16, be32, put16, put32};

/// Fixed part: type, code, checksum, the "rest of header" word.
pub const HEADER_LEN: usize = 8;

pub const ECHO_REPLY: u8 = 0;
pub const DEST_UNREACHABLE: u8 = 3;
pub const ECHO_REQUEST: u8 = 8;
pub const TIME_EXCEEDED: u8 = 11;
pub const PARAMETER_PROBLEM: u8 = 12;

/// Destination unreachable codes (RFC 792, RFC 1122 3.2.2.1).
pub mod unreachable {
    pub const NET: u8 = 0;
    pub const HOST: u8 = 1;
    pub const PROTOCOL: u8 = 2;
    pub const PORT: u8 = 3;
    /// Fragmentation needed and DF set; RFC 1191 puts the next-hop MTU in
    /// the low 16 bits of the rest-of-header word.
    pub const FRAG_NEEDED: u8 = 4;
}

/// Time exceeded codes (RFC 792).
pub mod time_exceeded {
    pub const TTL: u8 = 0;
    pub const REASSEMBLY: u8 = 1;
}

/// A parsed message.
#[derive(Clone, Copy, Debug)]
pub struct Message<'a> {
    pub ty: u8,
    pub code: u8,
    /// Bytes 4..8: identifier and sequence for echo, the next-hop MTU for
    /// "fragmentation needed", unused otherwise.
    pub rest: u32,
    pub data: &'a [u8],
}

impl Message<'_> {
    /// Echo identifier (high half of `rest`).
    pub fn ident(&self) -> u16 {
        (self.rest >> 16) as u16
    }
}

/// Parse and verify the checksum.
pub fn parse(buf: &[u8]) -> Result<Message<'_>, WireError> {
    if buf.len() < HEADER_LEN {
        return Err(WireError::Truncated);
    }
    if !checksum::verify(buf) {
        return Err(WireError::Checksum);
    }
    Ok(Message { ty: buf[0], code: buf[1], rest: be32(buf, 4), data: &buf[HEADER_LEN..] })
}

/// Write a header for a message whose data is already in
/// `buf[HEADER_LEN..HEADER_LEN + data_len]`, and checksum it all.
pub fn emit(buf: &mut [u8], ty: u8, code: u8, rest: u32, data_len: usize) {
    buf[0] = ty;
    buf[1] = code;
    put16(buf, 2, 0);
    put32(buf, 4, rest);
    let c = checksum::checksum(&buf[..HEADER_LEN + data_len]);
    put16(buf, 2, c);
}

/// Recompute the checksum of a complete message in place.
pub fn fix_checksum(buf: &mut [u8]) {
    put16(buf, 2, 0);
    let c = checksum::checksum(buf);
    put16(buf, 2, c);
}

/// True for the error types (as opposed to queries), which must never be
/// answered with another error (RFC 1122 3.2.2).
pub fn is_error(ty: u8) -> bool {
    matches!(ty, DEST_UNREACHABLE | TIME_EXCEEDED | PARAMETER_PROBLEM | 4 | 5)
}

/// The checksum field of a message, for tests.
pub fn checksum_field(buf: &[u8]) -> u16 {
    be16(buf, 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let mut buf = [0u8; 12];
        buf[8..].copy_from_slice(b"ping");
        emit(&mut buf, ECHO_REQUEST, 0, 0x0001_0002, 4);
        let m = parse(&buf).unwrap();
        assert_eq!((m.ty, m.code, m.ident(), m.data), (ECHO_REQUEST, 0, 1, &b"ping"[..]));
        buf[9] ^= 0x40;
        assert_eq!(parse(&buf).unwrap_err(), WireError::Checksum);
        assert_eq!(parse(&buf[..7]).unwrap_err(), WireError::Truncated);
        assert!(is_error(DEST_UNREACHABLE));
        assert!(!is_error(ECHO_REQUEST));
    }
}
