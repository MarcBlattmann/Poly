//! A request's bytes (RFC 9112 section 3, RFC 9110): the request line,
//! `Host` (the port only when it is not the scheme's default; IPv6
//! literals in brackets), `User-Agent` (the session's agent, if any), the
//! caller's header lines (which replace a field of the same name), and
//! `Content-Length` when a body is announced, then the empty line.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Header lines of a caller (`WinHttpAddRequestHeaders`,
/// `HttpSendRequestW`'s `lpszHeaders`): `Name: value` separated by CRLF
/// (or LF); empty lines skipped. `None` for a line without a colon or
/// with an empty name.
pub fn parse_lines(text: &str) -> Option<Vec<(String, String)>> {
    let mut v = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            continue;
        }
        let (name, value) = line.split_once(':')?;
        let name = name.trim();
        if name.is_empty() || name.bytes().any(|b| b <= b' ' || b == 0x7f) {
            return None;
        }
        v.push((name.to_string(), value.trim().to_string()));
    }
    Some(v)
}

/// Add `fields` to `headers`: with `replace`, a field of the same name
/// (case-insensitive) is replaced, else appended.
pub fn add(headers: &mut Vec<(String, String)>, fields: Vec<(String, String)>, replace: bool) {
    for (n, v) in fields {
        match headers.iter_mut().find(|(h, _)| h.eq_ignore_ascii_case(&n)) {
            Some(slot) if replace => slot.1 = v,
            _ => headers.push((n, v)),
        }
    }
}

/// The `Host` field for `host` and `port` (`default_port`: the scheme's).
pub fn host_field(host: &str, port: u16, default_port: u16) -> String {
    let h = if host.contains(':') { alloc::format!("[{host}]") } else { host.to_string() };
    if port == default_port { h } else { alloc::format!("{h}:{port}") }
}

/// The request head.
pub fn head(
    verb: &str,
    path: &str,
    host: &str,
    agent: Option<&str>,
    headers: &[(String, String)],
    body_len: Option<u64>,
) -> Vec<u8> {
    let mut fields: Vec<(String, String)> = Vec::new();
    fields.push(("Host".into(), host.into()));
    if let Some(a) = agent.filter(|a| !a.is_empty()) {
        fields.push(("User-Agent".into(), a.into()));
    }
    add(&mut fields, headers.to_vec(), true);
    if let Some(n) = body_len
        && !fields.iter().any(|(h, _)| h.eq_ignore_ascii_case("Content-Length"))
        && (n > 0 || !matches!(verb, "GET" | "HEAD"))
    {
        fields.push(("Content-Length".into(), n.to_string()));
    }
    let mut out = alloc::format!("{verb} {path} HTTP/1.1\r\n");
    for (n, v) in &fields {
        out.push_str(n);
        out.push_str(": ");
        out.push_str(v);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heads() {
        let extra = parse_lines("Accept: */*\r\nX-Test:  1 \r\n\r\n").unwrap();
        let h = head("GET", "/a?b", &host_field("example.com", 80, 80), Some("PolyTest"), &extra, Some(0));
        assert_eq!(
            core::str::from_utf8(&h).unwrap(),
            "GET /a?b HTTP/1.1\r\nHost: example.com\r\nUser-Agent: PolyTest\r\nAccept: */*\r\nX-Test: 1\r\n\r\n"
        );
        let h = head("POST", "/", &host_field("::1", 8080, 80), None, &[], Some(3));
        assert_eq!(
            core::str::from_utf8(&h).unwrap(),
            "POST / HTTP/1.1\r\nHost: [::1]:8080\r\nContent-Length: 3\r\n\r\n"
        );
        // A caller's Host replaces ours.
        let mine = parse_lines("host: other").unwrap();
        let h = head("GET", "/", "h", None, &mine, None);
        assert_eq!(core::str::from_utf8(&h).unwrap(), "GET / HTTP/1.1\r\nHost: other\r\n\r\n");
    }

    #[test]
    fn lines() {
        assert_eq!(parse_lines("no colon"), None);
        assert_eq!(parse_lines(": empty name"), None);
        assert_eq!(parse_lines("").unwrap().len(), 0);
        let mut v = parse_lines("A: 1\nB: 2").unwrap();
        add(&mut v, parse_lines("a: 3").unwrap(), true);
        assert_eq!(v[0].1, "3");
        add(&mut v, parse_lines("a: 4").unwrap(), false);
        assert_eq!(v.len(), 3);
    }
}
