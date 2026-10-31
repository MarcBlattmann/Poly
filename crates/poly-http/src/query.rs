//! `WinHttpQueryHeaders` and `HttpQueryInfoW` answered from a response
//! head (Learn: "Query Info Flags"; `winhttp.h` / `wininet.h` give the
//! levels the same numbers). A level names a header field (the table
//! below), the status line's parts, or every header at once; `CUSTOM`
//! names a field by the caller's string. `FLAG_NUMBER` asks for the value
//! as a 32-bit number. The caller's index picks among repeated fields and
//! is advanced past the one returned.

use crate::consts::*;
use crate::response::Head;
use alloc::string::{String, ToString};

/// The header field of each numbered level (`winhttp.h`'s
/// `WINHTTP_QUERY_*` from 0 to 45, and 63).
const FIELDS: &[(u32, &str)] = &[
    (0, "MIME-Version"),
    (1, "Content-Type"),
    (2, "Content-Transfer-Encoding"),
    (3, "Content-ID"),
    (4, "Content-Description"),
    (5, "Content-Length"),
    (6, "Content-Language"),
    (7, "Allow"),
    (8, "Public"),
    (9, "Date"),
    (10, "Expires"),
    (11, "Last-Modified"),
    (12, "Message-ID"),
    (13, "URI"),
    (14, "Derived-From"),
    (15, "Cost"),
    (16, "Link"),
    (17, "Pragma"),
    (23, "Connection"),
    (24, "Accept"),
    (25, "Accept-Charset"),
    (26, "Accept-Encoding"),
    (27, "Accept-Language"),
    (28, "Authorization"),
    (29, "Content-Encoding"),
    (30, "Forwarded"),
    (31, "From"),
    (32, "If-Modified-Since"),
    (33, "Location"),
    (34, "Orig-URI"),
    (35, "Referer"),
    (36, "Retry-After"),
    (37, "Server"),
    (38, "Title"),
    (39, "User-Agent"),
    (40, "WWW-Authenticate"),
    (41, "Proxy-Authenticate"),
    (42, "Accept-Ranges"),
    (43, "Set-Cookie"),
    (44, "Cookie"),
    (63, "Transfer-Encoding"),
];

/// What a query gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Text(String),
    Number(u32),
}

/// Answer `level` (with its modifier flags) for `head`; `name` is the
/// field of a `CUSTOM` query, `index` the occurrence. Returns the answer
/// and the next index. Errors: `ERROR_*_HEADER_NOT_FOUND`, and
/// `ERROR_*_INVALID_QUERY_REQUEST` for a level Poly does not know or a
/// value that is not a number under `FLAG_NUMBER`.
pub fn query(head: &Head, level: u32, name: Option<&str>, index: u32) -> Result<(Answer, u32), u32> {
    let flags = level & QUERY_MODIFIER_FLAGS_MASK;
    let level = level & !QUERY_MODIFIER_FLAGS_MASK;
    if flags & (QUERY_FLAG_SYSTEMTIME | QUERY_FLAG_REQUEST_HEADERS) != 0 {
        return Err(ERROR_INVALID_QUERY_REQUEST);
    }
    let (text, next) = match level {
        QUERY_STATUS_CODE => (head.status.to_string(), index),
        QUERY_STATUS_TEXT => (head.reason.clone(), index),
        QUERY_VERSION => (head.version(), index),
        QUERY_RAW_HEADERS_CRLF => {
            let mut s = head.status_line();
            s.push_str("\r\n");
            for (n, v) in &head.fields {
                s.push_str(n);
                s.push_str(": ");
                s.push_str(v);
                s.push_str("\r\n");
            }
            s.push_str("\r\n");
            (s, index)
        }
        QUERY_RAW_HEADERS => {
            // Each line ends with a NUL; the list with another.
            let mut s = head.status_line();
            s.push('\0');
            for (n, v) in &head.fields {
                s.push_str(n);
                s.push_str(": ");
                s.push_str(v);
                s.push('\0');
            }
            (s, index)
        }
        _ => {
            let field = if level == QUERY_CUSTOM {
                name.filter(|n| !n.is_empty()).ok_or(ERROR_INVALID_PARAMETER)?
            } else {
                FIELDS.iter().find(|(l, _)| *l == level).map(|(_, f)| *f).ok_or(ERROR_INVALID_QUERY_REQUEST)?
            };
            let v = head.field(field, index as usize).ok_or(ERROR_HEADER_NOT_FOUND)?;
            (v.to_string(), index + 1)
        }
    };
    if flags & QUERY_FLAG_NUMBER != 0 {
        let n = text.trim().parse::<u32>().map_err(|_| ERROR_INVALID_QUERY_REQUEST)?;
        return Ok((Answer::Number(n), next));
    }
    Ok((Answer::Text(text), next))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::response::Parser;

    fn head() -> Head {
        let mut p = Parser::new(false);
        p.feed(b"HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\nContent-Length: 12\r\nSet-Cookie: a\r\nSet-Cookie: b\r\nX-Custom: yes\r\n\r\n").unwrap();
        p.head().unwrap().clone()
    }

    #[test]
    fn levels() {
        let h = head();
        assert_eq!(query(&h, QUERY_STATUS_CODE, None, 0), Ok((Answer::Text("404".into()), 0)));
        assert_eq!(query(&h, QUERY_STATUS_CODE | QUERY_FLAG_NUMBER, None, 0), Ok((Answer::Number(404), 0)));
        assert_eq!(query(&h, QUERY_STATUS_TEXT, None, 0).unwrap().0, Answer::Text("Not Found".into()));
        assert_eq!(query(&h, QUERY_VERSION, None, 0).unwrap().0, Answer::Text("HTTP/1.1".into()));
        assert_eq!(query(&h, QUERY_CONTENT_TYPE, None, 0).unwrap().0, Answer::Text("text/html".into()));
        assert_eq!(query(&h, QUERY_CONTENT_LENGTH | QUERY_FLAG_NUMBER, None, 0).unwrap().0, Answer::Number(12));
        assert_eq!(query(&h, QUERY_CUSTOM, Some("x-custom"), 0), Ok((Answer::Text("yes".into()), 1)));
        // Repeated fields by index.
        assert_eq!(query(&h, 43, None, 1), Ok((Answer::Text("b".into()), 2)));
        assert_eq!(query(&h, 43, None, 2), Err(ERROR_HEADER_NOT_FOUND));
        assert_eq!(query(&h, QUERY_LOCATION, None, 0), Err(ERROR_HEADER_NOT_FOUND));
        assert_eq!(query(&h, QUERY_CONTENT_TYPE | QUERY_FLAG_NUMBER, None, 0), Err(ERROR_INVALID_QUERY_REQUEST));
        assert_eq!(query(&h, 9999, None, 0), Err(ERROR_INVALID_QUERY_REQUEST));
        assert_eq!(query(&h, QUERY_CUSTOM, None, 0), Err(ERROR_INVALID_PARAMETER));
    }

    #[test]
    fn raw_headers() {
        let h = head();
        let Answer::Text(crlf) = query(&h, QUERY_RAW_HEADERS_CRLF, None, 0).unwrap().0 else { panic!() };
        assert!(crlf.starts_with("HTTP/1.1 404 Not Found\r\nContent-Type: text/html\r\n"));
        assert!(crlf.ends_with("X-Custom: yes\r\n\r\n"));
        let Answer::Text(raw) = query(&h, QUERY_RAW_HEADERS, None, 0).unwrap().0 else { panic!() };
        assert_eq!(raw.matches('\0').count(), 6);
    }
}
