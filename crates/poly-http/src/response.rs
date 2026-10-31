//! An incremental HTTP/1.1 response parser (RFC 9112 sections 4-7): bytes
//! from the connection go in ([`Parser::feed`]), the head comes out once
//! complete ([`Parser::head`]) and the body's bytes as they are decoded
//! ([`Parser::read`]).
//!
//! The body's length (RFC 9112 section 6.3): none for a `HEAD` request
//! and for `1xx`, `204` and `304` responses; `chunked` as the final
//! transfer coding decodes chunks (extensions and trailer fields
//! ignored); otherwise `Content-Length` (several equal values are one,
//! different ones an error); otherwise until the connection closes.
//! Interim `1xx` responses (`100 Continue`) are skipped.

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// The most a head may take before the parser gives up.
pub const MAX_HEAD: usize = 64 * 1024;

/// The parsed status line and header fields.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Head {
    pub major: u8,
    pub minor: u8,
    pub status: u16,
    pub reason: String,
    /// Fields in order, names as received.
    pub fields: Vec<(String, String)>,
}

impl Head {
    /// The `index`th value of field `name` (case-insensitive).
    pub fn field(&self, name: &str, index: usize) -> Option<&str> {
        self.fields.iter().filter(|(n, _)| n.eq_ignore_ascii_case(name)).nth(index).map(|(_, v)| v.as_str())
    }

    /// `HTTP/1.1`.
    pub fn version(&self) -> String {
        alloc::format!("HTTP/{}.{}", self.major, self.minor)
    }

    /// The status line, without its CRLF.
    pub fn status_line(&self) -> String {
        alloc::format!("{} {} {}", self.version(), self.status, self.reason)
    }
}

/// The response was not HTTP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BadResponse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chunk {
    Size,
    Data(u64),
    DataEnd,
    Trailer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Body {
    None,
    Length(u64),
    Chunked(Chunk),
    UntilClose,
}

#[derive(Debug)]
pub struct Parser {
    head_request: bool,
    buf: Vec<u8>,
    head: Option<Head>,
    body: Body,
    out: VecDeque<u8>,
    done: bool,
}

fn find_crlf(b: &[u8]) -> Option<usize> {
    b.windows(2).position(|w| w == b"\r\n")
}

fn parse_head(text: &[u8]) -> Result<Head, BadResponse> {
    let text = core::str::from_utf8(text).map_err(|_| BadResponse)?;
    let mut lines = text.split("\r\n");
    let status = lines.next().ok_or(BadResponse)?;
    let rest = status.strip_prefix("HTTP/").ok_or(BadResponse)?;
    let (ver, rest) = rest.split_once(' ').ok_or(BadResponse)?;
    let (major, minor) = ver.split_once('.').ok_or(BadResponse)?;
    let (code, reason) = match rest.split_once(' ') {
        Some((c, r)) => (c, r),
        None => (rest, ""),
    };
    if code.len() != 3 {
        return Err(BadResponse);
    }
    let mut head = Head {
        major: major.parse().map_err(|_| BadResponse)?,
        minor: minor.parse().map_err(|_| BadResponse)?,
        status: code.parse().map_err(|_| BadResponse)?,
        reason: reason.to_string(),
        fields: Vec::new(),
    };
    for line in lines {
        if line.is_empty() {
            continue;
        }
        // An obsolete folded line continues the previous value.
        if line.starts_with([' ', '\t']) {
            let (_, v) = head.fields.last_mut().ok_or(BadResponse)?;
            v.push(' ');
            v.push_str(line.trim());
            continue;
        }
        let (n, v) = line.split_once(':').ok_or(BadResponse)?;
        if n.is_empty() || n.bytes().any(|b| b <= b' ') {
            return Err(BadResponse);
        }
        head.fields.push((n.to_string(), v.trim().to_string()));
    }
    Ok(head)
}

impl Parser {
    /// A parser for the response to a request (`head_request`: the verb
    /// was `HEAD`).
    pub fn new(head_request: bool) -> Parser {
        Parser { head_request, buf: Vec::new(), head: None, body: Body::None, out: VecDeque::new(), done: false }
    }

    pub fn head(&self) -> Option<&Head> {
        self.head.as_ref()
    }

    /// The whole body has arrived.
    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Decoded body bytes waiting to be read.
    pub fn available(&self) -> usize {
        self.out.len()
    }

    /// Take up to `buf.len()` decoded body bytes.
    pub fn read(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.out.len());
        for (i, b) in self.out.drain(..n).enumerate() {
            buf[i] = b;
        }
        n
    }

    /// The `Content-Length` of the head, if it frames the body.
    pub fn content_length(&self) -> Option<u64> {
        match (self.head.as_ref(), self.body) {
            (Some(h), Body::Length(_)) => h.field("Content-Length", 0).and_then(|v| v.parse().ok()),
            _ => None,
        }
    }

    /// Bytes from the connection.
    pub fn feed(&mut self, data: &[u8]) -> Result<(), BadResponse> {
        self.buf.extend_from_slice(data);
        loop {
            if self.head.is_none() {
                let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                    if self.buf.len() > MAX_HEAD {
                        return Err(BadResponse);
                    }
                    return Ok(());
                };
                let head = parse_head(&self.buf[..end])?;
                self.buf.drain(..end + 4);
                if (100..200).contains(&head.status) {
                    // An interim response: the real one follows.
                    continue;
                }
                self.body = self.framing(&head)?;
                self.head = Some(head);
                if self.body == Body::None || self.body == Body::Length(0) {
                    self.done = true;
                }
            }
            if self.done {
                self.buf.clear();
                return Ok(());
            }
            if !self.step()? {
                return Ok(());
            }
        }
    }

    fn framing(&self, head: &Head) -> Result<Body, BadResponse> {
        if self.head_request || head.status == 204 || head.status == 304 {
            return Ok(Body::None);
        }
        if let Some(te) = head.field("Transfer-Encoding", 0) {
            let last = te.rsplit(',').next().unwrap_or("").trim();
            if last.eq_ignore_ascii_case("chunked") {
                return Ok(Body::Chunked(Chunk::Size));
            }
            return Ok(Body::UntilClose);
        }
        let mut length = None;
        for (_, v) in head.fields.iter().filter(|(n, _)| n.eq_ignore_ascii_case("Content-Length")) {
            for part in v.split(',') {
                let n: u64 = part.trim().parse().map_err(|_| BadResponse)?;
                if length.is_some_and(|l| l != n) {
                    return Err(BadResponse);
                }
                length = Some(n);
            }
        }
        Ok(length.map_or(Body::UntilClose, Body::Length))
    }

    /// Decode what the buffer holds of the body; true if it may hold more
    /// to decode.
    fn step(&mut self) -> Result<bool, BadResponse> {
        match self.body {
            Body::None => {
                self.done = true;
                Ok(false)
            }
            Body::UntilClose => {
                self.out.extend(self.buf.drain(..));
                Ok(false)
            }
            Body::Length(left) => {
                let n = (left as usize).min(self.buf.len());
                self.out.extend(self.buf.drain(..n));
                let left = left - n as u64;
                self.body = Body::Length(left);
                if left == 0 {
                    self.done = true;
                }
                Ok(false)
            }
            Body::Chunked(state) => self.chunk(state),
        }
    }

    fn chunk(&mut self, state: Chunk) -> Result<bool, BadResponse> {
        match state {
            Chunk::Size => {
                let Some(end) = find_crlf(&self.buf) else {
                    if self.buf.len() > 1024 {
                        return Err(BadResponse);
                    }
                    return Ok(false);
                };
                let line = core::str::from_utf8(&self.buf[..end]).map_err(|_| BadResponse)?;
                let size = line.split(';').next().unwrap_or("").trim();
                let size = u64::from_str_radix(size, 16).map_err(|_| BadResponse)?;
                self.buf.drain(..end + 2);
                self.body = Body::Chunked(if size == 0 { Chunk::Trailer } else { Chunk::Data(size) });
                Ok(true)
            }
            Chunk::Data(left) => {
                if self.buf.is_empty() {
                    return Ok(false);
                }
                let n = (left as usize).min(self.buf.len());
                self.out.extend(self.buf.drain(..n));
                let left = left - n as u64;
                self.body = Body::Chunked(if left == 0 { Chunk::DataEnd } else { Chunk::Data(left) });
                Ok(left == 0)
            }
            Chunk::DataEnd => {
                if self.buf.len() < 2 {
                    return Ok(false);
                }
                if &self.buf[..2] != b"\r\n" {
                    return Err(BadResponse);
                }
                self.buf.drain(..2);
                self.body = Body::Chunked(Chunk::Size);
                Ok(true)
            }
            Chunk::Trailer => {
                let Some(end) = find_crlf(&self.buf) else { return Ok(false) };
                self.buf.drain(..end + 2);
                if end == 0 {
                    self.done = true;
                    return Ok(false);
                }
                Ok(true)
            }
        }
    }

    /// The connection closed: the end of an until-close body; an error if
    /// the head or a framed body is incomplete.
    pub fn eof(&mut self) -> Result<(), BadResponse> {
        if self.head.is_none() {
            return Err(BadResponse);
        }
        match self.body {
            Body::UntilClose => {
                self.done = true;
                Ok(())
            }
            _ if self.done => Ok(()),
            _ => Err(BadResponse),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(p: &mut Parser) -> Vec<u8> {
        let mut v = alloc::vec![0u8; p.available()];
        let n = p.read(&mut v);
        v.truncate(n);
        v
    }

    #[test]
    fn content_length_byte_by_byte() {
        let r = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhelloEXTRA";
        let mut p = Parser::new(false);
        for b in r.iter() {
            p.feed(core::slice::from_ref(b)).unwrap();
        }
        let h = p.head().unwrap();
        assert_eq!((h.status, h.reason.as_str(), h.major, h.minor), (200, "OK", 1, 1));
        assert_eq!(h.field("content-type", 0), Some("text/plain"));
        assert_eq!(p.content_length(), Some(5));
        assert!(p.is_done());
        assert_eq!(all(&mut p), b"hello");
    }

    #[test]
    fn chunked() {
        let r = b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n4;ext=1\r\nWiki\r\n6\r\npedia \r\nE\r\nin \r\n\r\nchunks.\r\n0\r\nTrailer: x\r\n\r\n";
        let mut p = Parser::new(false);
        let (a, b) = r.split_at(70);
        p.feed(a).unwrap();
        assert!(!p.is_done());
        p.feed(b).unwrap();
        assert!(p.is_done());
        assert_eq!(p.head().unwrap().status, 200);
        assert_eq!(all(&mut p), b"Wikipedia in \r\n\r\nchunks.");
        assert_eq!(p.content_length(), None);
    }

    #[test]
    fn until_close_and_no_body() {
        let mut p = Parser::new(false);
        p.feed(b"HTTP/1.0 200 OK\r\nServer: tiny\r\n\r\nabc").unwrap();
        assert!(!p.is_done());
        p.feed(b"def").unwrap();
        p.eof().unwrap();
        assert!(p.is_done());
        assert_eq!(all(&mut p), b"abcdef");
        let mut p = Parser::new(true);
        p.feed(b"HTTP/1.1 200 OK\r\nContent-Length: 99\r\n\r\n").unwrap();
        assert!(p.is_done());
        let mut p = Parser::new(false);
        p.feed(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap();
        assert!(p.is_done());
    }

    #[test]
    fn errors() {
        assert_eq!(Parser::new(false).feed(b"SSH-2.0-x\r\n\r\n"), Err(BadResponse));
        assert_eq!(Parser::new(false).feed(b"HTTP/1.1 20 OK\r\n\r\n"), Err(BadResponse));
        let two = b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\n";
        assert_eq!(Parser::new(false).feed(two), Err(BadResponse));
        let mut p = Parser::new(false);
        p.feed(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nab").unwrap();
        assert_eq!(p.eof(), Err(BadResponse));
        assert_eq!(Parser::new(false).eof(), Err(BadResponse));
        let mut p = Parser::new(false);
        assert_eq!(p.feed(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n"), Err(BadResponse));
    }

    #[test]
    fn folded_and_repeated_fields() {
        let mut p = Parser::new(false);
        p.feed(
            b"HTTP/1.1 302 Found\r\nSet-Cookie: a=1\r\nX: one\r\n  two\r\nSet-Cookie: b=2\r\nContent-Length: 0\r\n\r\n",
        )
        .unwrap();
        let h = p.head().unwrap();
        assert_eq!(h.field("set-cookie", 1), Some("b=2"));
        assert_eq!(h.field("x", 0), Some("one two"));
        assert_eq!(h.status_line(), "HTTP/1.1 302 Found");
    }
}
