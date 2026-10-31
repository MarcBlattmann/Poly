//! `getaddrinfo` / `GetAddrInfoW` and `getnameinfo` / `GetNameInfoW`
//! without the network (Learn's pages of each, "addrinfo"): the hints
//! checked, the service turned into a port, the host classified (numeric,
//! `localhost`, the empty host of a passive or loopback lookup, or a name
//! for the stub resolver), and the result list built.
//!
//! Choices to be confirmed by the spec agent against real Windows:
//!
//! - **Order**: IPv6 before IPv4 when both families are allowed (the
//!   default prefix policy of RFC 6724 prefers `::1` to `127.0.0.1`).
//! - **Socket types**: one entry per address, with the hint's socket type;
//!   a zero protocol is filled from the type (`IPPROTO_TCP` for streams,
//!   `IPPROTO_UDP` for datagrams), a zero type stays zero.
//! - **`localhost`** (and the machine's own name) are the loopback
//!   addresses without asking a server; there is no `hosts` file yet.
//! - **`getnameinfo`** gives `localhost` for loopback addresses and the
//!   numeric form for any other address (no reverse lookups yet), unless
//!   `NI_NAMEREQD` asks for a name (`WSAHOST_NOT_FOUND`).

use crate::consts::*;
use crate::text;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV6};

/// The `ADDRINFO` hints that matter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hints {
    pub flags: i32,
    pub family: i32,
    pub socktype: i32,
    pub protocol: i32,
}

/// One result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub family: i32,
    pub socktype: i32,
    pub protocol: i32,
    pub addr: SocketAddr,
}

/// What the host part asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Host {
    /// Addresses known without a lookup.
    Addrs(Vec<IpAddr>),
    /// A name for the resolver (already checked to be a plausible name).
    Lookup(String),
}

/// Learn: "Values other than those listed for the ai_flags member" are
/// `WSAEINVAL` (`EAI_BADFLAGS`); a family other than `AF_UNSPEC`,
/// `AF_INET` and `AF_INET6` is `WSAEAFNOSUPPORT`; a socket type other
/// than 0, `SOCK_STREAM`, `SOCK_DGRAM` and `SOCK_RAW` is
/// `WSAESOCKTNOSUPPORT`; `AI_CANONNAME` without a host name is
/// `WSAEINVAL`.
pub fn check_hints(h: &Hints, node: Option<&str>) -> Result<(), u32> {
    if h.flags & !AI_KNOWN != 0 {
        return Err(EAI_BADFLAGS);
    }
    if !matches!(h.family, AF_UNSPEC | AF_INET | AF_INET6) {
        return Err(EAI_FAMILY);
    }
    if !matches!(h.socktype, 0 | SOCK_STREAM | SOCK_DGRAM | SOCK_RAW) {
        return Err(EAI_SOCKTYPE);
    }
    if h.flags & AI_CANONNAME != 0 && node.is_none() {
        return Err(EAI_BADFLAGS);
    }
    Ok(())
}

/// Well-known services by name (IANA's service name and port registry,
/// RFC 6335): the same port for TCP and UDP here.
pub const SERVICES: &[(&str, u16)] = &[
    ("echo", 7),
    ("discard", 9),
    ("daytime", 13),
    ("ftp-data", 20),
    ("ftp", 21),
    ("ssh", 22),
    ("telnet", 23),
    ("smtp", 25),
    ("time", 37),
    ("domain", 53),
    ("tftp", 69),
    ("gopher", 70),
    ("finger", 79),
    ("http", 80),
    ("www", 80),
    ("kerberos", 88),
    ("pop3", 110),
    ("sunrpc", 111),
    ("nntp", 119),
    ("ntp", 123),
    ("imap", 143),
    ("snmp", 161),
    ("ldap", 389),
    ("https", 443),
    ("submission", 587),
    ("ldaps", 636),
    ("imaps", 993),
    ("pop3s", 995),
];

/// The port of a service name (case-insensitive), if Poly knows it.
pub fn service_by_name(name: &str) -> Option<u16> {
    SERVICES.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|&(_, p)| p)
}

/// The name of a port's service, if Poly knows it (the first name).
pub fn service_by_port(port: u16) -> Option<&'static str> {
    SERVICES.iter().find(|&&(_, p)| p == port).map(|&(n, _)| n)
}

/// The service part: absent is port 0, a decimal number is that port, a
/// name is looked up unless `AI_NUMERICSERV` (`WSATYPE_NOT_FOUND`
/// otherwise).
pub fn service_port(service: Option<&str>, flags: i32) -> Result<u16, u32> {
    let Some(s) = service else { return Ok(0) };
    if s.is_empty() {
        return Ok(0);
    }
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<u16>().map_err(|_| EAI_SERVICE);
    }
    if flags & AI_NUMERICSERV != 0 {
        return Err(EAI_NONAME);
    }
    service_by_name(s).ok_or(EAI_SERVICE)
}

/// A numeric host: IPv4 dotted decimal or IPv6 text, the latter with an
/// optional numeric scope (`fe80::1%3`).
pub fn numeric_host(s: &str) -> Option<(IpAddr, u32)> {
    if let Some(b) = text::pton4(s) {
        return Some((IpAddr::V4(Ipv4Addr::from(b)), 0));
    }
    let (addr, scope) = match s.split_once('%') {
        Some((a, sc)) => (a, sc.parse::<u32>().ok()?),
        None => (s, 0),
    };
    text::pton6(addr).map(|b| (IpAddr::V6(Ipv6Addr::from(b)), scope))
}

/// Is `name` this machine (`localhost`, or the host name `own`)?
pub fn is_local_name(name: &str, own: &str) -> bool {
    let n = name.strip_suffix('.').unwrap_or(name);
    n.eq_ignore_ascii_case("localhost") || (!own.is_empty() && n.eq_ignore_ascii_case(own))
}

/// The families a lookup returns, in order.
fn families(h: &Hints) -> &'static [i32] {
    match h.family {
        AF_INET => &[AF_INET],
        AF_INET6 => &[AF_INET6],
        _ => &[AF_INET6, AF_INET],
    }
}

fn loopback(af: i32) -> IpAddr {
    if af == AF_INET { IpAddr::V4(Ipv4Addr::LOCALHOST) } else { IpAddr::V6(Ipv6Addr::LOCALHOST) }
}

fn unspecified(af: i32) -> IpAddr {
    if af == AF_INET { IpAddr::V4(Ipv4Addr::UNSPECIFIED) } else { IpAddr::V6(Ipv6Addr::UNSPECIFIED) }
}

/// Classify the host part (`own` is the machine's host name). A numeric
/// host of the wrong family is `WSAHOST_NOT_FOUND`; a name with
/// `AI_NUMERICHOST` too.
pub fn classify(node: Option<&str>, h: &Hints, own: &str) -> Result<(Host, u32), u32> {
    let fams = families(h);
    let Some(node) = node.filter(|n| !n.is_empty()) else {
        let pick = if h.flags & AI_PASSIVE != 0 { unspecified } else { loopback };
        return Ok((Host::Addrs(fams.iter().map(|&f| pick(f)).collect()), 0));
    };
    if let Some((ip, scope)) = numeric_host(node) {
        let af = if ip.is_ipv4() { AF_INET } else { AF_INET6 };
        if !fams.contains(&af) {
            return Err(EAI_NONAME);
        }
        return Ok((Host::Addrs(alloc::vec![ip]), scope));
    }
    if h.flags & AI_NUMERICHOST != 0 {
        return Err(EAI_NONAME);
    }
    if is_local_name(node, own) {
        return Ok((Host::Addrs(fams.iter().map(|&f| loopback(f)).collect()), 0));
    }
    if crate::dns::encode_name(node, &mut Vec::new()).is_err() {
        return Err(EAI_NONAME);
    }
    Ok((Host::Lookup(node.to_string()), 0))
}

/// The protocol an entry gets.
fn protocol_for(h: &Hints) -> i32 {
    match (h.socktype, h.protocol) {
        (_, p) if p != 0 => p,
        (SOCK_STREAM, _) => IPPROTO_TCP,
        (SOCK_DGRAM, _) => IPPROTO_UDP,
        _ => 0,
    }
}

/// The result list for `addrs` (filtered by the hints' family, ordered
/// IPv6 first unless the family is fixed) with `port`.
pub fn entries(addrs: &[IpAddr], port: u16, scope: u32, h: &Hints) -> Vec<Entry> {
    let mut out = Vec::new();
    for &af in families(h) {
        for ip in addrs {
            let a = match ip {
                IpAddr::V4(v) if af == AF_INET => SocketAddr::new(IpAddr::V4(*v), port),
                IpAddr::V6(v) if af == AF_INET6 => SocketAddr::V6(SocketAddrV6::new(*v, port, 0, scope)),
                _ => continue,
            };
            if out.iter().any(|e: &Entry| e.addr == a) {
                continue;
            }
            out.push(Entry { family: af, socktype: h.socktype, protocol: protocol_for(h), addr: a });
        }
    }
    out
}

/// `getnameinfo`'s host part.
pub fn name_of_host(addr: &SocketAddr, flags: i32) -> Result<String, u32> {
    let ip = addr.ip();
    if flags & NI_NUMERICHOST == 0 && ip.is_loopback() {
        return Ok("localhost".into());
    }
    if flags & NI_NAMEREQD != 0 {
        return Err(EAI_NONAME);
    }
    Ok(match addr {
        SocketAddr::V6(v) if v.scope_id() != 0 => alloc::format!("{}%{}", v.ip(), v.scope_id()),
        _ => ip.to_string(),
    })
}

/// `getnameinfo`'s service part.
pub fn name_of_service(port: u16, flags: i32) -> String {
    if flags & NI_NUMERICSERV == 0
        && let Some(n) = service_by_port(port)
    {
        return n.into();
    }
    port.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(flags: i32, family: i32, socktype: i32) -> Hints {
        Hints { flags, family, socktype, protocol: 0 }
    }

    #[test]
    fn hints() {
        assert_eq!(check_hints(&h(0, AF_UNSPEC, SOCK_STREAM), Some("x")), Ok(()));
        assert_eq!(check_hints(&h(0x1000_0000, 0, 0), Some("x")), Err(EAI_BADFLAGS));
        assert_eq!(check_hints(&h(0, 7, 0), Some("x")), Err(EAI_FAMILY));
        assert_eq!(check_hints(&h(0, 0, 9), Some("x")), Err(EAI_SOCKTYPE));
        assert_eq!(check_hints(&h(AI_CANONNAME, 0, 0), None), Err(EAI_BADFLAGS));
    }

    #[test]
    fn services() {
        assert_eq!(service_port(None, 0), Ok(0));
        assert_eq!(service_port(Some("8080"), 0), Ok(8080));
        assert_eq!(service_port(Some("70000"), 0), Err(EAI_SERVICE));
        assert_eq!(service_port(Some("HTTP"), 0), Ok(80));
        assert_eq!(service_port(Some("http"), AI_NUMERICSERV), Err(EAI_NONAME));
        assert_eq!(service_port(Some("nosuch"), 0), Err(EAI_SERVICE));
        assert_eq!(name_of_service(443, 0), "https");
        assert_eq!(name_of_service(443, NI_NUMERICSERV), "443");
        assert_eq!(name_of_service(12345, 0), "12345");
    }

    #[test]
    fn hosts() {
        let any = h(0, AF_UNSPEC, SOCK_STREAM);
        let (host, _) = classify(Some("localhost"), &any, "poly").unwrap();
        let Host::Addrs(a) = host else { panic!() };
        let e = entries(&a, 80, 0, &any);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].addr.to_string(), "[::1]:80");
        assert_eq!(e[1].addr.to_string(), "127.0.0.1:80");
        assert_eq!(e[1].protocol, IPPROTO_TCP);
        assert_eq!(e[1].socktype, SOCK_STREAM);

        let v4 = h(0, AF_INET, SOCK_DGRAM);
        let (host, _) = classify(Some("POLY."), &v4, "poly").unwrap();
        let Host::Addrs(a) = host else { panic!() };
        let e = entries(&a, 53, 0, &v4);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].addr.to_string(), "127.0.0.1:53");
        assert_eq!(e[0].protocol, IPPROTO_UDP);

        let passive = h(AI_PASSIVE, AF_INET, SOCK_STREAM);
        let (host, _) = classify(None, &passive, "poly").unwrap();
        assert_eq!(host, Host::Addrs(alloc::vec![IpAddr::V4(Ipv4Addr::UNSPECIFIED)]));

        assert_eq!(classify(Some("::1"), &v4, "poly"), Err(EAI_NONAME));
        assert_eq!(classify(Some("example.com"), &h(AI_NUMERICHOST, 0, 0), "poly"), Err(EAI_NONAME));
        assert_eq!(classify(Some("example.com"), &any, "poly").unwrap().0, Host::Lookup("example.com".into()));
        assert_eq!(classify(Some("bad name"), &any, "poly"), Err(EAI_NONAME));
        let (host, scope) = classify(Some("fe80::1%4"), &any, "poly").unwrap();
        let Host::Addrs(a) = host else { panic!() };
        let e = entries(&a, 1, scope, &h(0, 0, 0));
        assert_eq!(e[0].addr.to_string(), "[fe80::1%4]:1");
        assert_eq!(e[0].socktype, 0);
        assert_eq!(e[0].protocol, 0);
    }

    #[test]
    fn names() {
        let lo: SocketAddr = "127.0.0.1:80".parse().unwrap();
        assert_eq!(name_of_host(&lo, 0).unwrap(), "localhost");
        assert_eq!(name_of_host(&lo, NI_NUMERICHOST).unwrap(), "127.0.0.1");
        let other: SocketAddr = "10.0.2.2:80".parse().unwrap();
        assert_eq!(name_of_host(&other, 0).unwrap(), "10.0.2.2");
        assert_eq!(name_of_host(&other, NI_NAMEREQD), Err(EAI_NONAME));
    }
}
