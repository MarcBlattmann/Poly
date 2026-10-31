//! The stub resolver's messages (RFC 1035 section 4, RFC 3596 for
//! `AAAA`): one question per query (recursion desired) sent over UDP to
//! the configured server, and the addresses read back from the answer
//! section, following `CNAME`s within it. No TCP retry when the answer is
//! truncated (what fits in 512 bytes is used), no search domains, no
//! cache.

use alloc::string::String;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const TYPE_A: u16 = 1;
pub const TYPE_CNAME: u16 = 5;
pub const TYPE_PTR: u16 = 12;
pub const TYPE_AAAA: u16 = 28;
pub const CLASS_IN: u16 = 1;
/// The DNS port.
pub const PORT: u16 = 53;
/// The largest UDP answer read (RFC 1035 limits plain UDP to 512 bytes;
/// more room costs nothing).
pub const MAX_UDP: usize = 1232;

/// Why a lookup gave no addresses, as `getaddrinfo` reports it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DnsError {
    /// The name does not exist (`NXDOMAIN`), or exists without records of
    /// the type (`WSAHOST_NOT_FOUND` / `WSANO_DATA`).
    NotFound,
    NoData,
    /// A temporary failure (`SERVFAIL`, no answer): `WSATRY_AGAIN`.
    TryAgain,
    /// Any other failure: `WSANO_RECOVERY`.
    Fail,
    /// Not a valid host name.
    BadName,
}

/// What an answer gave.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    pub addrs: Vec<IpAddr>,
    /// The canonical name, if the answer had a `CNAME` for the question.
    pub canonical: Option<String>,
}

/// Write `name` as labels (a trailing dot is allowed). Each label is
/// 1..=63 bytes, the whole name at most 253 characters (RFC 1035 2.3.4).
pub fn encode_name(name: &str, out: &mut Vec<u8>) -> Result<(), DnsError> {
    let name = name.strip_suffix('.').unwrap_or(name);
    if name.is_empty() || name.len() > 253 {
        return Err(DnsError::BadName);
    }
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 || label.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return Err(DnsError::BadName);
        }
        out.push(label.len() as u8);
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    Ok(())
}

/// A query for `name`'s records of type `qtype`, with id `id`.
pub fn query(id: u16, name: &str, qtype: u16) -> Result<Vec<u8>, DnsError> {
    let mut q = Vec::with_capacity(12 + name.len() + 6);
    q.extend_from_slice(&id.to_be_bytes());
    // QR 0, opcode 0, RD 1.
    q.extend_from_slice(&[0x01, 0x00]);
    q.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0]);
    encode_name(name, &mut q)?;
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&CLASS_IN.to_be_bytes());
    Ok(q)
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, DnsError> {
    b.get(at..at + 2).map(|x| u16::from_be_bytes([x[0], x[1]])).ok_or(DnsError::Fail)
}

/// Read the name at `at` (following compression pointers); returns it
/// (lower case, without the trailing dot) and the offset after it in the
/// record.
fn read_name(msg: &[u8], mut at: usize) -> Result<(String, usize), DnsError> {
    let mut name = String::new();
    let mut end = None;
    let mut jumps = 0;
    loop {
        let len = *msg.get(at).ok_or(DnsError::Fail)? as usize;
        if len & 0xc0 == 0xc0 {
            let ptr = (u16_at(msg, at)? & 0x3fff) as usize;
            end.get_or_insert(at + 2);
            jumps += 1;
            if jumps > 32 {
                return Err(DnsError::Fail);
            }
            at = ptr;
            continue;
        }
        if len & 0xc0 != 0 {
            return Err(DnsError::Fail);
        }
        if len == 0 {
            return Ok((name, end.unwrap_or(at + 1)));
        }
        let label = msg.get(at + 1..at + 1 + len).ok_or(DnsError::Fail)?;
        if !name.is_empty() {
            name.push('.');
        }
        for &b in label {
            name.push(char::from(b.to_ascii_lowercase()));
        }
        if name.len() > 255 {
            return Err(DnsError::Fail);
        }
        at += 1 + len;
    }
}

fn same_name(a: &str, b: &str) -> bool {
    a.strip_suffix('.').unwrap_or(a).eq_ignore_ascii_case(b.strip_suffix('.').unwrap_or(b))
}

/// Read the answer to query `id` for `name`'s `qtype` records.
pub fn parse(msg: &[u8], id: u16, name: &str, qtype: u16) -> Result<Answer, DnsError> {
    if msg.len() < 12 || u16_at(msg, 0)? != id {
        return Err(DnsError::Fail);
    }
    let flags = u16_at(msg, 2)?;
    if flags & 0x8000 == 0 {
        return Err(DnsError::Fail);
    }
    match flags & 0xf {
        0 => {}
        2 => return Err(DnsError::TryAgain),
        3 => return Err(DnsError::NotFound),
        _ => return Err(DnsError::Fail),
    }
    let qd = u16_at(msg, 4)?;
    let an = u16_at(msg, 6)?;
    let mut at = 12;
    for _ in 0..qd {
        let (_, next) = read_name(msg, at)?;
        at = next + 4;
    }
    let mut want = String::from(name.strip_suffix('.').unwrap_or(name));
    let mut answer = Answer::default();
    for _ in 0..an {
        let (owner, next) = read_name(msg, at)?;
        let ty = u16_at(msg, next)?;
        let class = u16_at(msg, next + 2)?;
        let rdlen = u16_at(msg, next + 8)? as usize;
        let rdata = next + 10;
        let data = msg.get(rdata..rdata + rdlen).ok_or(DnsError::Fail)?;
        at = rdata + rdlen;
        if class != CLASS_IN || !same_name(&owner, &want) {
            continue;
        }
        match ty {
            TYPE_CNAME => {
                let (target, _) = read_name(msg, rdata)?;
                answer.canonical = Some(target.clone());
                want = target;
            }
            TYPE_A if qtype == TYPE_A && rdlen == 4 => {
                answer.addrs.push(IpAddr::V4(Ipv4Addr::new(data[0], data[1], data[2], data[3])));
            }
            TYPE_AAAA if qtype == TYPE_AAAA && rdlen == 16 => {
                let mut a = [0u8; 16];
                a.copy_from_slice(data);
                answer.addrs.push(IpAddr::V6(Ipv6Addr::from(a)));
            }
            _ => {}
        }
    }
    if answer.addrs.is_empty() {
        return Err(DnsError::NoData);
    }
    Ok(answer)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer as a server would write it, with a compressed CNAME.
    fn reply(id: u16, rcode: u8) -> Vec<u8> {
        let mut m = Vec::new();
        m.extend_from_slice(&id.to_be_bytes());
        m.extend_from_slice(&[0x81, 0x80 | rcode]);
        m.extend_from_slice(&[0, 1, 0, 3, 0, 0, 0, 0]);
        // Question: www.example.com A IN at offset 12.
        encode_name("www.example.com", &mut m).unwrap();
        m.extend_from_slice(&[0, 1, 0, 1]);
        // www.example.com CNAME web.example.com (pointer to "example.com" at 16).
        m.extend_from_slice(&[0xc0, 12, 0, 5, 0, 1, 0, 0, 0, 60, 0, 6, 3, b'w', b'e', b'b', 0xc0, 16]);
        let web = m.len() - 6;
        // web.example.com A 93.184.216.34 (pointer to "web..." in the CNAME's data).
        m.extend_from_slice(&[0xc0, web as u8, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 93, 184, 216, 34]);
        // An unrelated record is skipped.
        m.extend_from_slice(&[0xc0, 16, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 1, 1, 1]);
        m
    }

    #[test]
    fn query_bytes() {
        let q = query(0xbeef, "a.bc", TYPE_AAAA).unwrap();
        assert_eq!(q, [0xbe, 0xef, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0, 1, b'a', 2, b'b', b'c', 0, 0, 28, 0, 1]);
        assert_eq!(query(1, "x.", TYPE_A).unwrap().len(), 12 + 3 + 4);
        assert_eq!(query(1, "", TYPE_A), Err(DnsError::BadName));
        assert_eq!(query(1, "a..b", TYPE_A), Err(DnsError::BadName));
        let long = "a".repeat(64);
        assert_eq!(query(1, &long, TYPE_A), Err(DnsError::BadName));
        assert_eq!(query(1, "has space", TYPE_A), Err(DnsError::BadName));
    }

    #[test]
    fn answers() {
        let a = parse(&reply(7, 0), 7, "WWW.example.com.", TYPE_A).unwrap();
        assert_eq!(a.addrs, [IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))]);
        assert_eq!(a.canonical.as_deref(), Some("web.example.com"));
        assert_eq!(parse(&reply(7, 0), 8, "www.example.com", TYPE_A), Err(DnsError::Fail));
        assert_eq!(parse(&reply(7, 3), 7, "www.example.com", TYPE_A), Err(DnsError::NotFound));
        assert_eq!(parse(&reply(7, 2), 7, "www.example.com", TYPE_A), Err(DnsError::TryAgain));
        assert_eq!(parse(&reply(7, 0), 7, "www.example.com", TYPE_AAAA), Err(DnsError::NoData));
        let r = reply(7, 0);
        assert_eq!(parse(&r[..r.len() - 3], 7, "www.example.com", TYPE_A), Err(DnsError::Fail));
        // A query is not an answer.
        let q = query(7, "www.example.com", TYPE_A).unwrap();
        assert_eq!(parse(&q, 7, "www.example.com", TYPE_A), Err(DnsError::Fail));
    }

    #[test]
    fn pointer_loops_end() {
        let mut m = reply(1, 0);
        // Make the first answer's owner point at itself.
        let first = 12 + 17 + 4;
        m[first] = 0xc0;
        m[first + 1] = first as u8;
        assert_eq!(parse(&m, 1, "www.example.com", TYPE_A), Err(DnsError::Fail));
    }
}
