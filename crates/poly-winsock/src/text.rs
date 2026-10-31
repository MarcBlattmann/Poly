//! Address text: `inet_pton` / `InetPtonW`, `inet_ntop` / `InetNtopW`,
//! `inet_addr` and `inet_ntoa` (Learn's pages of each).
//!
//! - `inet_pton` takes the standard forms only: dotted decimal with four
//!   parts for IPv4, RFC 4291's text forms for IPv6 (with `::` and an
//!   embedded IPv4 tail), no scope id. Poly uses `core::net`'s parsers,
//!   which follow the same RFCs and refuse leading zeros in IPv4 parts
//!   (to be confirmed by the spec agent: whether Windows takes `01.2.3.4`).
//! - `inet_ntop` writes IPv6 in RFC 5952's recommended form (lower case,
//!   the longest run of zero groups as `::`, IPv4-mapped addresses with a
//!   dotted tail).
//! - `inet_addr` takes the older forms Learn lists: `a.b.c.d`, `a.b.c`
//!   (the last part 16 bits), `a.b` (24 bits) and `a` (32 bits), each part
//!   decimal, octal (a leading `0`) or hexadecimal (`0x`). An empty or bad
//!   string is `INADDR_NONE`. Poly also accepts trailing white space
//!   (to be confirmed by the spec agent).

use crate::consts::*;
use alloc::string::{String, ToString};
use core::net::{Ipv4Addr, Ipv6Addr};

/// `inet_pton` for `AF_INET`: the four address bytes.
pub fn pton4(s: &str) -> Option<[u8; 4]> {
    s.parse::<Ipv4Addr>().ok().map(|a| a.octets())
}

/// `inet_pton` for `AF_INET6`: the sixteen address bytes.
pub fn pton6(s: &str) -> Option<[u8; 16]> {
    s.parse::<Ipv6Addr>().ok().map(|a| a.octets())
}

/// `inet_ntop` of either family's bytes.
pub fn ntop(af: i32, bytes: &[u8]) -> Result<String, u32> {
    match af {
        AF_INET => {
            let b: [u8; 4] = bytes.get(..4).and_then(|b| b.try_into().ok()).ok_or(WSAEFAULT)?;
            Ok(Ipv4Addr::from(b).to_string())
        }
        AF_INET6 => {
            let b: [u8; 16] = bytes.get(..16).and_then(|b| b.try_into().ok()).ok_or(WSAEFAULT)?;
            Ok(Ipv6Addr::from(b).to_string())
        }
        _ => Err(WSAEAFNOSUPPORT),
    }
}

/// One part of an `inet_addr` string: decimal, octal or hexadecimal.
fn part(s: &str) -> Option<u32> {
    if s.is_empty() {
        return None;
    }
    let (digits, radix) = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        (h, 16)
    } else if s.len() > 1 && s.starts_with('0') {
        (&s[1..], 8)
    } else {
        (s, 10)
    };
    if digits.is_empty() {
        // "0x" alone is 0, as a bare "0" is.
        return if radix == 16 { Some(0) } else { None };
    }
    u32::from_str_radix(digits, radix).ok()
}

/// `inet_addr`: the address bytes in network order, or `None` for
/// `INADDR_NONE`'s failure.
pub fn inet_addr(s: &str) -> Option<[u8; 4]> {
    let s = s.trim_end_matches([' ', '\t', '\n', '\r']);
    if s.is_empty() {
        return None;
    }
    let mut parts = [0u32; 4];
    let mut n = 0;
    for p in s.split('.') {
        if n == 4 {
            return None;
        }
        parts[n] = part(p)?;
        n += 1;
    }
    let value = match n {
        1 => parts[0],
        2 if parts[0] <= 0xff && parts[1] <= 0xff_ffff => (parts[0] << 24) | parts[1],
        3 if parts[0] <= 0xff && parts[1] <= 0xff && parts[2] <= 0xffff => {
            (parts[0] << 24) | (parts[1] << 16) | parts[2]
        }
        4 if parts.iter().all(|&p| p <= 0xff) => (parts[0] << 24) | (parts[1] << 16) | (parts[2] << 8) | parts[3],
        _ => return None,
    };
    Some(value.to_be_bytes())
}

/// `inet_ntoa`: dotted decimal.
pub fn ntoa(bytes: [u8; 4]) -> String {
    Ipv4Addr::from(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pton() {
        assert_eq!(pton4("192.168.1.20"), Some([192, 168, 1, 20]));
        assert_eq!(pton4("1.2.3"), None);
        assert_eq!(pton4("256.1.1.1"), None);
        assert_eq!(pton4(" 1.2.3.4"), None);
        assert_eq!(pton4("1.2.3.04"), None);
        let lo = pton6("::1").unwrap();
        assert_eq!(lo[15], 1);
        assert!(lo[..15].iter().all(|&b| b == 0));
        assert_eq!(pton6("::ffff:1.2.3.4").unwrap()[10..], [0xff, 0xff, 1, 2, 3, 4]);
        assert_eq!(pton6("1:2:3:4:5:6:7:8:9"), None);
        assert_eq!(pton6("fe80::1%3"), None);
        assert_eq!(pton6("1.2.3.4"), None);
    }

    #[test]
    fn ntop_forms() {
        assert_eq!(ntop(AF_INET, &[10, 0, 2, 15]).unwrap(), "10.0.2.15");
        assert_eq!(ntop(AF_INET6, &pton6("2001:db8:0:0:1:0:0:1").unwrap()).unwrap(), "2001:db8::1:0:0:1");
        assert_eq!(ntop(AF_INET6, &pton6("0:0:0:0:0:0:0:0").unwrap()).unwrap(), "::");
        assert_eq!(ntop(AF_INET6, &pton6("::FFFF:10.1.2.3").unwrap()).unwrap(), "::ffff:10.1.2.3");
        assert_eq!(ntop(AF_UNSPEC, &[0; 4]), Err(WSAEAFNOSUPPORT));
        assert_eq!(ntop(AF_INET6, &[0; 4]), Err(WSAEFAULT));
    }

    #[test]
    fn inet_addr_forms() {
        assert_eq!(inet_addr("127.0.0.1"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr("0x7f.1"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr("127.1"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr("10.1.258"), Some([10, 1, 1, 2]));
        assert_eq!(inet_addr("2130706433"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr("0177.0.0.01"), Some([127, 0, 0, 1]));
        assert_eq!(inet_addr("255.255.255.255"), Some([255; 4]));
        assert_eq!(inet_addr("1.2.3.4 "), Some([1, 2, 3, 4]));
        assert_eq!(inet_addr(""), None);
        assert_eq!(inet_addr("1.2.3.256"), None);
        assert_eq!(inet_addr("1.2.3.4.5"), None);
        assert_eq!(inet_addr("1..2"), None);
        assert_eq!(inet_addr("08.1.1.1"), None);
        assert_eq!(inet_addr("host"), None);
        assert_eq!(ntoa([192, 0, 2, 7]), "192.0.2.7");
    }
}
