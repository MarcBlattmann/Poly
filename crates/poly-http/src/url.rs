//! URLs of the forms WinHTTP and WinINet take: `scheme://[user[:pass]@]
//! host[:port][/path][?query][#fragment]` (RFC 3986's generic syntax,
//! restricted to `http` and `https`). The fragment is dropped (it never
//! goes to the server); the path is `/` when absent.

use alloc::string::{String, ToString};

/// `http` or `https`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

/// A URL taken apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Url {
    pub scheme: Scheme,
    /// The host as written (an IPv6 literal without its brackets).
    pub host: String,
    pub port: u16,
    /// The path and query, starting with `/`.
    pub path: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

/// Why a URL was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlError {
    /// Not `http` or `https` (`ERROR_*_UNRECOGNIZED_SCHEME`).
    Scheme,
    /// Anything else (`ERROR_*_INVALID_URL`).
    Invalid,
}

pub fn parse(s: &str) -> Result<Url, UrlError> {
    let (scheme, rest) = s.split_once("://").ok_or(UrlError::Invalid)?;
    let scheme = if scheme.eq_ignore_ascii_case("http") {
        Scheme::Http
    } else if scheme.eq_ignore_ascii_case("https") {
        Scheme::Https
    } else if !scheme.is_empty() && scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) {
        return Err(UrlError::Scheme);
    } else {
        return Err(UrlError::Invalid);
    };
    let rest = rest.split('#').next().unwrap_or("");
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, path) = rest.split_at(end);
    let (userinfo, hostport) = match authority.rsplit_once('@') {
        Some((u, h)) => (Some(u), h),
        None => (None, authority),
    };
    let (user, password) = match userinfo {
        None => (None, None),
        Some(u) => match u.split_once(':') {
            Some((a, b)) => (Some(a.to_string()), Some(b.to_string())),
            None => (Some(u.to_string()), None),
        },
    };
    let (host, port) = if let Some(v6) = hostport.strip_prefix('[') {
        let (h, after) = v6.split_once(']').ok_or(UrlError::Invalid)?;
        let port = match after.strip_prefix(':') {
            Some(p) => Some(p),
            None if after.is_empty() => None,
            None => return Err(UrlError::Invalid),
        };
        (h, port)
    } else {
        match hostport.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (hostport, None),
        }
    };
    if host.is_empty() || host.bytes().any(|b| b <= b' ' || b"/\\?#@".contains(&b)) {
        return Err(UrlError::Invalid);
    }
    let port = match port {
        None | Some("") => scheme.default_port(),
        Some(p) => p.parse::<u16>().ok().filter(|&p| p != 0).ok_or(UrlError::Invalid)?,
    };
    let path = if path.is_empty() {
        "/".to_string()
    } else if path.starts_with('?') {
        alloc::format!("/{path}")
    } else {
        path.to_string()
    };
    if path.bytes().any(|b| b <= b' ' || b == 0x7f) {
        return Err(UrlError::Invalid);
    }
    Ok(Url { scheme, host: host.to_string(), port, path, user, password })
}

// ----- WinHTTP's `URL_COMPONENTS` (`winhttp.h`): crack and create

/// Where a component sits in the URL, in UTF-16 units. A caller that asks
/// for a component without a buffer is handed a pointer into its own
/// string (Learn: "WinHttpCrackUrl", Remarks), so the pieces are spans
/// rather than copies.
pub type Span = core::ops::Range<usize>;

/// A URL split the way `URL_COMPONENTS` wants it: path and extra
/// information (`?query#fragment`) apart, the port converted. The
/// fragment stays in the extra information; WinHTTP never sends it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cracked {
    pub scheme: Span,
    /// `INTERNET_SCHEME_HTTP` (1) or `INTERNET_SCHEME_HTTPS` (2).
    pub scheme_id: i32,
    pub user: Span,
    pub password: Span,
    pub host: Span,
    pub port: u16,
    /// The path; empty when the URL has none (WinHTTP writes `/`).
    pub path: Span,
    /// From `?` or `#` to the end; empty when there is neither.
    pub extra: Span,
}

/// Split a URL for `WinHttpCrackUrl`.
pub fn crack(url: &[u16]) -> Result<Cracked, UrlError> {
    let sep = (0..url.len().saturating_sub(2))
        .find(|&i| url[i] == u16::from(b':') && url[i + 1] == u16::from(b'/') && url[i + 2] == u16::from(b'/'))
        .ok_or(UrlError::Invalid)?;
    let scheme = &url[..sep];
    let scheme = if ascii_eq(scheme, "http") {
        Scheme::Http
    } else if ascii_eq(scheme, "https") {
        Scheme::Https
    } else if !scheme.is_empty()
        && scheme
            .iter()
            .all(|&u| u < 0x80 && (u8::try_from(u).unwrap().is_ascii_alphanumeric() || b"+-.".contains(&(u as u8))))
    {
        return Err(UrlError::Scheme);
    } else {
        return Err(UrlError::Invalid);
    };

    let rest = sep + 3;
    let end = find_any(url, rest, b"/?#").unwrap_or(url.len());
    let at = url[rest..end].iter().rposition(|&u| u == u16::from(b'@'));
    let (userinfo, host_start) = match at {
        Some(i) => (Some((rest, i)), rest + i + 1),
        None => (None, rest),
    };
    let hostport = &url[host_start..end];
    let (user, password) = match userinfo {
        None => (0..0, 0..0),
        Some((start, len)) => {
            let info = &url[start..start + len];
            match info.iter().position(|&u| u == u16::from(b':')) {
                Some(i) => (start..start + i, start + i + 1..start + len),
                None => (start..start + len, 0..0),
            }
        }
    };
    let (host, port_units) = if hostport.first() == Some(&u16::from(b'[')) {
        let close = hostport.iter().position(|&u| u == u16::from(b']')).ok_or(UrlError::Invalid)?;
        let after = &hostport[close + 1..];
        let port = match after.first() {
            None => &[][..],
            Some(&u) if u == u16::from(b':') => &after[1..],
            Some(_) => return Err(UrlError::Invalid),
        };
        (host_start + 1..host_start + close, port)
    } else {
        if hostport.iter().filter(|&&u| u == u16::from(b':')).count() > 1 {
            return Err(UrlError::Invalid);
        }
        match hostport.iter().rposition(|&u| u == u16::from(b':')) {
            Some(i) => (host_start..host_start + i, &hostport[i + 1..]),
            None => (host_start..end, &[][..]),
        }
    };
    if host.is_empty() || url[host.clone()].iter().any(|&u| u <= 0x20 || u < 0x80 && b"/\\?#@".contains(&(u as u8))) {
        return Err(UrlError::Invalid);
    }
    let port = if port_units.is_empty() {
        scheme.default_port()
    } else {
        let mut n: u32 = 0;
        for &u in port_units {
            if !(u16::from(b'0')..=u16::from(b'9')).contains(&u) {
                return Err(UrlError::Invalid);
            }
            n = n * 10 + u32::from(u - u16::from(b'0'));
            if n > u32::from(u16::MAX) {
                return Err(UrlError::Invalid);
            }
        }
        if n == 0 {
            return Err(UrlError::Invalid);
        }
        n as u16
    };
    let path_end = find_any(url, end, b"?#").unwrap_or(url.len());
    Ok(Cracked {
        scheme: 0..sep,
        scheme_id: match scheme {
            Scheme::Http => 1,
            Scheme::Https => 2,
        },
        user,
        password,
        host,
        port,
        path: end..path_end,
        extra: path_end..url.len(),
    })
}

/// The parts `WinHttpCreateUrl` builds a URL from (`URL_COMPONENTS`).
#[derive(Clone, Copy, Debug, Default)]
pub struct Parts<'a> {
    /// The scheme as written; empty falls back to `scheme_id`.
    pub scheme: &'a str,
    /// `INTERNET_SCHEME_*`; 0 when the scheme text decides.
    pub scheme_id: i32,
    pub host: &'a str,
    /// 0 (or the scheme's default) is written without a `:port`.
    pub port: u16,
    pub user: &'a str,
    pub password: &'a str,
    pub path: &'a str,
    /// `?query` and/or `#fragment`, as given.
    pub extra: &'a str,
}

/// Build a URL for `WinHttpCreateUrl`; `escaped` is `ICU_ESCAPE`, which
/// escapes the path and the extra information only (Learn).
pub fn create(p: &Parts, escaped: bool) -> Result<String, UrlError> {
    let scheme = if p.scheme.is_empty() {
        match p.scheme_id {
            1 => "http",
            2 => "https",
            _ => return Err(UrlError::Scheme),
        }
    } else {
        p.scheme
    };
    let scheme = if scheme.eq_ignore_ascii_case("http") {
        Scheme::Http
    } else if scheme.eq_ignore_ascii_case("https") {
        Scheme::Https
    } else if scheme.bytes().all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) {
        return Err(UrlError::Scheme);
    } else {
        return Err(UrlError::Invalid);
    };
    if p.host.is_empty() || p.host.bytes().any(|b| b <= b' ' || b"/\\?#@".contains(&b)) {
        return Err(UrlError::Invalid);
    }
    let mut out = String::new();
    out.push_str(p.scheme);
    if out.is_empty() {
        out.push_str(match scheme {
            Scheme::Http => "http",
            Scheme::Https => "https",
        });
    }
    out.push_str("://");
    if !p.user.is_empty() {
        out.push_str(p.user);
        if !p.password.is_empty() {
            out.push(':');
            out.push_str(p.password);
        }
        out.push('@');
    }
    out.push_str(p.host);
    if p.port != 0 && p.port != scheme.default_port() {
        out.push(':');
        out.push_str(&p.port.to_string());
    }
    let path = if p.path.is_empty() { "/" } else { p.path };
    if !path.starts_with('/') {
        out.push('/');
    }
    out.push_str(&if escaped { escape(path) } else { path.to_string() });
    out.push_str(&if escaped { escape(p.extra) } else { p.extra.to_string() });
    Ok(out)
}

/// `ICU_ESCAPE`: percent-encode every byte of `s` outside the characters
/// RFC 3986 lets a path or query carry, UTF-8 bytes included.
pub fn escape(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/%?".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push(char::from(HEX[usize::from(b >> 4)]));
            out.push(char::from(HEX[usize::from(b & 0xf)]));
        }
    }
    out
}

/// `ICU_DECODE`: `%XX` back to the byte it stands for, the bytes read as
/// UTF-8 (what WinHTTP's wide-character form does for ASCII escapes).
pub fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = alloc::vec::Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escape = if bytes[i] == b'%' && i + 2 < bytes.len() {
            match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(hi), Some(lo)) => Some(hi << 4 | lo),
                _ => None,
            }
        } else {
            None
        };
        match escape {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn ascii_eq(units: &[u16], s: &str) -> bool {
    units.len() == s.len() && units.iter().zip(s.bytes()).all(|(&u, b)| u < 0x80 && (u as u8).eq_ignore_ascii_case(&b))
}

fn find_any(url: &[u16], from: usize, pats: &[u8]) -> Option<usize> {
    url[from..].iter().position(|&u| u < 0x80 && pats.contains(&(u as u8))).map(|i| i + from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forms() {
        let u = parse("http://example.com").unwrap();
        assert_eq!((u.scheme, u.host.as_str(), u.port, u.path.as_str()), (Scheme::Http, "example.com", 80, "/"));
        let u = parse("HTTP://10.0.2.2:8080/a/b?c=d#frag").unwrap();
        assert_eq!((u.host.as_str(), u.port, u.path.as_str()), ("10.0.2.2", 8080, "/a/b?c=d"));
        let u = parse("https://[::1]:8443/x").unwrap();
        assert_eq!((u.scheme, u.host.as_str(), u.port), (Scheme::Https, "::1", 8443));
        let u = parse("https://h").unwrap();
        assert_eq!(u.port, 443);
        let u = parse("http://user:pw@h:81?q").unwrap();
        assert_eq!(
            (u.user.as_deref(), u.password.as_deref(), u.port, u.path.as_str()),
            (Some("user"), Some("pw"), 81, "/?q")
        );
    }

    #[test]
    fn errors() {
        assert_eq!(parse("ftp://h/"), Err(UrlError::Scheme));
        assert_eq!(parse("h/path"), Err(UrlError::Invalid));
        assert_eq!(parse("http:///path"), Err(UrlError::Invalid));
        assert_eq!(parse("http://h:99999/"), Err(UrlError::Invalid));
        assert_eq!(parse("http://h:0/"), Err(UrlError::Invalid));
        assert_eq!(parse("http://[::1/"), Err(UrlError::Invalid));
        assert_eq!(parse("http://h/a b"), Err(UrlError::Invalid));
    }

    fn wide(s: &str) -> alloc::vec::Vec<u16> {
        s.encode_utf16().collect()
    }

    fn text(url: &[u16], span: &Span) -> String {
        char::decode_utf16(url[span.clone()].iter().copied()).map(|c| c.unwrap_or('\u{fffd}')).collect()
    }

    #[test]
    fn cracks_a_url_into_components() {
        let url = wide("HTTP://user:pw@10.0.2.2:8080/a/b?c=d#frag");
        let c = crack(&url).unwrap();
        assert_eq!(
            (text(&url, &c.scheme), c.scheme_id, text(&url, &c.user), text(&url, &c.password)),
            ("HTTP".to_string(), 1, "user".to_string(), "pw".to_string())
        );
        assert_eq!((text(&url, &c.host), c.port), ("10.0.2.2".to_string(), 8080));
        assert_eq!(text(&url, &c.path), "/a/b");
        assert_eq!(text(&url, &c.extra), "?c=d#frag");
    }

    #[test]
    fn cracks_defaults_and_ipv6() {
        let url = wide("http://example.com");
        let c = crack(&url).unwrap();
        assert_eq!(
            (text(&url, &c.host), c.port, c.path.is_empty(), c.extra.is_empty()),
            ("example.com".to_string(), 80, true, true)
        );
        let url = wide("https://[::1]:8443/x");
        let c = crack(&url).unwrap();
        assert_eq!((text(&url, &c.host), c.port, c.scheme_id), ("::1".to_string(), 8443, 2));
        let url = wide("http://h?q");
        let c = crack(&url).unwrap();
        assert_eq!((c.path.is_empty(), text(&url, &c.extra)), (true, "?q".to_string()));
    }

    #[test]
    fn refuses_urls_winhttp_cannot_crack() {
        for (s, e) in [
            ("ftp://h/", UrlError::Scheme),
            ("h/path", UrlError::Invalid),
            ("http:///path", UrlError::Invalid),
            ("http://h:99999/", UrlError::Invalid),
            ("http://h:0/", UrlError::Invalid),
            ("http://[::1/", UrlError::Invalid),
            ("http://h:1:2/", UrlError::Invalid),
            ("http://h:8x/", UrlError::Invalid),
        ] {
            assert_eq!(crack(&wide(s)), Err(e), "{s}");
        }
    }

    #[test]
    fn cracks_without_checking_the_path() {
        // Learn: "WinHttpCrackUrl does not check this URL for validity or
        // correct format before attempting to crack it."
        let url = wide("http://h/a b");
        assert_eq!(text(&url, &crack(&url).unwrap().path), "/a b");
    }

    #[test]
    fn creates_urls_from_parts() {
        let base = Parts { scheme: "http", host: "example.com", ..Parts::default() };
        assert_eq!(create(&base, false).as_deref(), Ok("http://example.com/"));
        assert_eq!(create(&Parts { scheme: "https", port: 443, ..base }, false).as_deref(), Ok("https://example.com/"));
        assert_eq!(
            create(&Parts { scheme: "http", port: 8080, ..base }, false).as_deref(),
            Ok("http://example.com:8080/")
        );
        assert_eq!(
            create(&Parts { scheme: "http", user: "u", password: "p", path: "a/b", extra: "?q=1", ..base }, false)
                .as_deref(),
            Ok("http://u:p@example.com/a/b?q=1")
        );
        assert_eq!(create(&Parts { scheme_id: 2, host: "h", ..Parts::default() }, false).as_deref(), Ok("https://h/"));
        assert_eq!(create(&Parts { scheme: "ftp", host: "h", ..Parts::default() }, false), Err(UrlError::Scheme));
        assert_eq!(create(&Parts { scheme: "http", host: "", ..Parts::default() }, false), Err(UrlError::Invalid));
    }

    #[test]
    fn escapes_and_decodes() {
        assert_eq!(escape("a b/c?d=é"), "a%20b/c?d=%C3%A9");
        assert_eq!(decode("a%20b%2Fc%3Fd%3D%C3%A9"), "a b/c?d=é");
        assert_eq!(decode("%zz%4"), "%zz%4");
        let path = "a b/é";
        assert_eq!(decode(&escape(path)), path);
        assert_eq!(
            create(&Parts { scheme: "http", host: "h", path: "/a b", extra: "?q=é", ..Parts::default() }, true)
                .as_deref(),
            Ok("http://h/a%20b?q=%C3%A9")
        );
    }

    #[test]
    fn cracks_what_create_built() {
        let parts = Parts {
            scheme: "http",
            host: "10.0.2.2",
            port: 8080,
            user: "u",
            password: "p",
            path: "/a/b",
            extra: "?c=d",
            ..Parts::default()
        };
        let url = create(&parts, false).unwrap();
        let units = wide(&url);
        let c = crack(&units).unwrap();
        assert_eq!(text(&units, &c.host), "10.0.2.2");
        assert_eq!(
            (c.port, text(&units, &c.path), text(&units, &c.extra)),
            (8080, "/a/b".to_string(), "?c=d".to_string())
        );
    }
}
