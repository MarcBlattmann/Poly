//! The Windows `hosts` file (`%SystemRoot%\System32\drivers\etc\hosts`).
//!
//! Each non-comment line maps one IPv4 or IPv6 address to one or more host
//! names (the first name and any aliases that follow it, separated by spaces
//! or tabs; `#` begins a comment to the end of the line). Name matching is
//! ASCII case-insensitive and ignores a single trailing dot.

use crate::resolve::numeric_host;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::net::IpAddr;

/// What a `hosts` lookup found for a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostsEntry {
    /// All IP addresses mapped to the name, in file order (deduplicated).
    pub addrs: Vec<IpAddr>,
    /// The first name on the first matching line (the canonical name).
    pub canonical: String,
}

fn strip_dot(s: &str) -> &str {
    s.strip_suffix('.').unwrap_or(s)
}

/// Look up `name` in the raw bytes of a `hosts` file.
///
/// Returns `Some` when at least one line names `name` (or an alias of it),
/// collecting every IPv4 and IPv6 address mapped to that name across the file.
pub fn lookup(contents: &[u8], name: &str) -> Option<HostsEntry> {
    let want = strip_dot(name);
    if want.is_empty() {
        return None;
    }
    let mut addrs: Vec<IpAddr> = Vec::new();
    let mut canonical: Option<String> = None;
    for raw_line in contents.split(|&b| b == b'\n') {
        let without_cr = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        let before_comment = match without_cr.iter().position(|&b| b == b'#') {
            Some(i) => &without_cr[..i],
            None => without_cr,
        };
        let Ok(line) = core::str::from_utf8(before_comment) else {
            continue;
        };
        let mut tokens = line.split([' ', '\t']).filter(|t| !t.is_empty());
        let Some(addr_tok) = tokens.next() else {
            continue;
        };
        let Some((ip, _scope)) = numeric_host(addr_tok) else {
            continue;
        };
        let Some(primary) = tokens.next() else {
            continue;
        };
        let matched = strip_dot(primary).eq_ignore_ascii_case(want)
            || tokens.any(|alias| strip_dot(alias).eq_ignore_ascii_case(want));
        if matched {
            if canonical.is_none() {
                canonical = Some(strip_dot(primary).to_string());
            }
            if !addrs.contains(&ip) {
                addrs.push(ip);
            }
        }
    }
    if addrs.is_empty() {
        None
    } else {
        Some(HostsEntry { addrs, canonical: canonical.unwrap_or_else(|| want.to_string()) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn parses_v4_v6_both_comments_aliases_and_case() {
        let file = b"\
# Leading comment\r\n\
\r\n\
192.0.2.55   polytest-v4.invalid # inline comment\r\n\
2001:db8::55\tpolytest-v6.invalid\n\
192.0.2.56   polytest-both.invalid\r\n\
2001:db8::56 polytest-both.invalid\r\n\
192.0.2.78   polytest-second.invalid polytest-alias.invalid polytest-alias2.invalid # trailing\r\n\
not-an-ip    ignored.invalid\r\n\
192.0.2.99\r\n\
";
        let v4 = lookup(file, "polytest-v4.invalid").unwrap();
        assert_eq!(v4.addrs, [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 55))]);
        assert_eq!(v4.canonical, "polytest-v4.invalid");

        // Case-insensitive and trailing dot.
        assert_eq!(lookup(file, "POLYTEST-V4.INVALID"), Some(v4.clone()));
        assert_eq!(lookup(file, "polytest-v4.invalid."), Some(v4));

        let v6 = lookup(file, "polytest-v6.invalid").unwrap();
        assert_eq!(v6.addrs, [IpAddr::V6("2001:db8::55".parse::<Ipv6Addr>().unwrap())]);

        let both = lookup(file, "polytest-both.invalid").unwrap();
        assert_eq!(
            both.addrs,
            [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 56)), IpAddr::V6("2001:db8::56".parse::<Ipv6Addr>().unwrap()),]
        );

        let alias = lookup(file, "POLYTEST-ALIAS.INVALID.").unwrap();
        assert_eq!(alias.addrs, [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 78))]);
        assert_eq!(alias.canonical, "polytest-second.invalid");

        let alias2 = lookup(file, "polytest-alias2.invalid").unwrap();
        assert_eq!(alias2.addrs, [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 78))]);

        assert_eq!(lookup(file, "ignored.invalid"), None);
        assert_eq!(lookup(file, "missing.invalid"), None);
        assert_eq!(lookup(file, ""), None);
        assert_eq!(lookup(file, "."), None);
    }

    #[test]
    fn deduplicates_identical_addresses() {
        let file = b"192.0.2.1 dup.invalid\n192.0.2.1 DUP.INVALID\n";
        let entry = lookup(file, "dup.invalid").unwrap();
        assert_eq!(entry.addrs, [IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))]);
    }
}
