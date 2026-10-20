//! Packet formats: parsing that checks every length before touching a byte,
//! and emitting into caller-provided buffers. Nothing here keeps state.
//!
//! Parsers return borrowed views; they never panic, whatever the input.

pub mod arp;
pub mod checksum;
pub mod ethernet;
pub mod icmpv4;
pub mod icmpv6;
pub mod ipv4;
pub mod ipv6;
pub mod tcp;
pub mod udp;

/// Why a packet was not accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WireError {
    /// Shorter than its headers or its own length fields say.
    Truncated,
    /// A field has a value the format does not allow.
    Malformed,
    /// The checksum does not match.
    Checksum,
    /// Valid, but something this stack does not handle.
    Unsupported,
}

/// IP protocol numbers (IANA "Assigned Internet Protocol Numbers"; RFC 790 lists the early ones).
pub mod proto {
    /// ICMP for IPv4 (RFC 792).
    pub const ICMP: u8 = 1;
    /// TCP (RFC 9293).
    pub const TCP: u8 = 6;
    /// UDP (RFC 768).
    pub const UDP: u8 = 17;
    /// IPv6 Hop-by-Hop Options (RFC 8200).
    pub const HOPOPTS: u8 = 0;
    /// IPv6 Routing header (RFC 8200).
    pub const ROUTING: u8 = 43;
    /// IPv6 Fragment header (RFC 8200).
    pub const FRAGMENT: u8 = 44;
    /// ICMPv6 (RFC 4443).
    pub const ICMPV6: u8 = 58;
    /// IPv6 "no next header" (RFC 8200).
    pub const NONE: u8 = 59;
    /// IPv6 Destination Options (RFC 8200).
    pub const DSTOPTS: u8 = 60;
}

pub(crate) fn be16(b: &[u8], at: usize) -> u16 {
    match b.get(at..at + 2) {
        Some(s) => u16::from_be_bytes([s[0], s[1]]),
        None => 0,
    }
}

pub(crate) fn be32(b: &[u8], at: usize) -> u32 {
    match b.get(at..at + 4) {
        Some(s) => u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

pub(crate) fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_be_bytes());
}

pub(crate) fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_be_bytes());
}
