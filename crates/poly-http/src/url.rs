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
}
