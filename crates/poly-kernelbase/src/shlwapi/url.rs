//! `UrlCanonicalizeW`: a URL or a file path in canonical form. The rules are the sheet's
//! (`specs/windows/shlwapi.dll/UrlCanonicalizeW.yaml`, whose examples are normative); the flag
//! values are `shlwapi.h`'s.
//!
//! What it does, in order:
//! * spaces and control characters around the text are dropped;
//! * a DOS path (`C:\a`) or a UNC path (`\\server\share`) becomes a `file:` URL, its unsafe
//!   characters escaped;
//! * the known schemes (`http`, `https`, `ftp`, `file`) and their hosts are lower-cased, the
//!   default port is dropped, backslashes in the path become slashes, an empty `http`-style
//!   path becomes `/`, `file:c:/x` becomes `file:///c:/x`;
//! * dot segments of the path collapse (not in the query or fragment). A known scheme whose
//!   path starts with a `.` or `..` segment keeps its path as it is (`http://a/./b/../c` stays);
//! * the escape flags escape or unescape what follows the host;
//! * `URL_FILE_USE_PATHURL` / `URL_WININET_COMPATIBILITY` give a `file:` URL in the path form
//!   (`file://C:\a b`, `file://\\server\share`).

use alloc::vec::Vec;

/// `URL_UNESCAPE` (`shlwapi.h`).
pub const URL_UNESCAPE: u32 = 0x1000_0000;
/// `URL_ESCAPE_UNSAFE` (`shlwapi.h`).
pub const URL_ESCAPE_UNSAFE: u32 = 0x2000_0000;
/// `URL_PLUGGABLE_PROTOCOL` (`shlwapi.h`).
pub const URL_PLUGGABLE_PROTOCOL: u32 = 0x4000_0000;
/// `URL_WININET_COMPATIBILITY` (`shlwapi.h`).
pub const URL_WININET_COMPATIBILITY: u32 = 0x8000_0000;
/// `URL_ESCAPE_SPACES_ONLY` (`shlwapi.h`).
pub const URL_ESCAPE_SPACES_ONLY: u32 = 0x0400_0000;
/// `URL_DONT_SIMPLIFY` (`shlwapi.h`).
pub const URL_DONT_SIMPLIFY: u32 = 0x0800_0000;
/// `URL_FILE_USE_PATHURL` (`shlwapi.h`).
pub const URL_FILE_USE_PATHURL: u32 = 0x0001_0000;
/// `URL_ESCAPE_PERCENT` (`shlwapi.h`).
pub const URL_ESCAPE_PERCENT: u32 = 0x0000_1000;

const SLASH: u16 = b'/' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const COLON: u16 = b':' as u16;
const PERCENT: u16 = b'%' as u16;
const DOT: u16 = b'.' as u16;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn lower(c: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&c) { c + 32 } else { c }
}

fn is_alpha(c: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&lower(c))
}

fn hex_value(c: u16) -> Option<u16> {
    match c {
        0x30..=0x39 => Some(c - 0x30),
        0x41..=0x46 => Some(c - 0x41 + 10),
        0x61..=0x66 => Some(c - 0x61 + 10),
        _ => None,
    }
}

fn push_escaped(out: &mut Vec<u16>, byte: u8) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    out.extend([PERCENT, u16::from(HEX[usize::from(byte >> 4)]), u16::from(HEX[usize::from(byte & 15)])]);
}

/// The characters `URL_ESCAPE_UNSAFE` escapes: white space and control characters, the
/// characters the sheet lists (`" < > | ^ { }`), the grave accent, and everything outside ASCII.
fn is_unsafe(c: u16) -> bool {
    c <= 0x20 || c >= 0x7F || b"\"<>|^{}`".iter().any(|&u| u16::from(u) == c)
}

/// Escape one unit: a unit up to 0xFF as one `%XX` (U+00E9 is `%E9`, as the sheet shows),
/// anything above as its UTF-8 bytes (not recorded).
fn escape_unit(out: &mut Vec<u16>, c: u16) {
    if c <= 0xFF {
        push_escaped(out, c as u8);
    } else {
        let ch = char::from_u32(u32::from(c)).unwrap_or('\u{FFFD}');
        let mut buf = [0u8; 4];
        for &b in ch.encode_utf8(&mut buf).as_bytes() {
            push_escaped(out, b);
        }
    }
}

/// The escape flags applied to `s` (what follows the host).
fn apply_escapes(s: &[u16], flags: u32) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    if flags & URL_ESCAPE_SPACES_ONLY != 0 {
        for &c in s {
            if c == u16::from(b' ') { push_escaped(&mut out, b' ') } else { out.push(c) }
        }
    } else if flags & URL_ESCAPE_UNSAFE != 0 {
        for &c in s {
            if is_unsafe(c) || (c == PERCENT && flags & URL_ESCAPE_PERCENT != 0) {
                escape_unit(&mut out, c);
            } else {
                out.push(c);
            }
        }
    } else if flags & URL_UNESCAPE != 0 {
        out = unescape(s);
    } else {
        out.extend_from_slice(s);
    }
    out
}

/// `%XX` (two hex digits) decoded to the unit of that value; other `%` stay.
fn unescape(s: &[u16]) -> Vec<u16> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == PERCENT
            && let (Some(&h), Some(&l)) = (s.get(i + 1), s.get(i + 2))
            && let (Some(h), Some(l)) = (hex_value(h), hex_value(l))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// Remove the dot segments of a path (`/`-separated). A rooted path drops a `..` that would
/// climb above its root; a relative one keeps a leading `..`. A `.` or `..` as the last
/// segment leaves the trailing slash.
fn remove_dots(path: &[u16]) -> Vec<u16> {
    let rooted = path.first() == Some(&SLASH);
    let body = if rooted { &path[1..] } else { path };
    let segments: Vec<&[u16]> = body.split(|&c| c == SLASH).collect();
    let last = segments.len() - 1;
    let mut stack: Vec<&[u16]> = Vec::new();
    for (i, seg) in segments.iter().enumerate() {
        let is_last = i == last;
        if *seg == [DOT] {
            if is_last {
                stack.push(&[]);
            }
        } else if *seg == [DOT, DOT] {
            if !rooted && (stack.is_empty() || stack.last() == Some(&&[DOT, DOT][..])) {
                stack.push(seg);
                continue;
            }
            stack.pop();
            if is_last {
                stack.push(&[]);
            }
        } else {
            stack.push(seg);
        }
    }
    let mut out = Vec::with_capacity(path.len());
    if rooted {
        out.push(SLASH);
    }
    for (i, seg) in stack.iter().enumerate() {
        if i > 0 {
            out.push(SLASH);
        }
        out.extend_from_slice(seg);
    }
    out
}

/// Where the path ends: at the first `?` or `#`.
fn path_end(s: &[u16]) -> usize {
    s.iter().position(|&c| c == u16::from(b'?') || c == u16::from(b'#')).unwrap_or(s.len())
}

/// The path of `s` (up to the query) with its dot segments removed; `guard` keeps a path whose
/// first segment is `.` or `..` as it is.
fn simplify(s: &[u16], guard: bool) -> Vec<u16> {
    let end = path_end(s);
    let path = &s[..end];
    let first = path.strip_prefix(&[SLASH]).unwrap_or(path);
    let first = &first[..first.iter().position(|&c| c == SLASH).unwrap_or(first.len())];
    let mut out = if guard && (first == [DOT] || first == [DOT, DOT]) { path.to_vec() } else { remove_dots(path) };
    out.extend_from_slice(&s[end..]);
    out
}

/// The length of a scheme at the start of `s` (letters, digits, `+ - .`, starting with a
/// letter, at least two characters so that a drive letter is no scheme), with its `:`.
fn scheme_len(s: &[u16]) -> Option<usize> {
    let colon = s.iter().position(|&c| c == COLON)?;
    let ok = colon >= 2
        && is_alpha(s[0])
        && s[..colon].iter().all(|&c| {
            is_alpha(c) || (0x30..=0x39).contains(&c) || c == u16::from(b'+') || c == u16::from(b'-') || c == DOT
        });
    ok.then_some(colon)
}

/// A DOS path with a drive (`C:\` or `C:/`).
fn is_dos_path(s: &[u16]) -> bool {
    s.len() >= 3 && is_alpha(s[0]) && s[1] == COLON && (s[2] == BACKSLASH || s[2] == SLASH)
}

/// A file path as a `file:` URL: backslashes become slashes and the characters a URL cannot
/// hold are escaped.
fn path_to_file_url(s: &[u16]) -> Vec<u16> {
    let (mut out, rest) = if is_dos_path(s) { (w("file:///"), s) } else { (w("file://"), &s[2..]) };
    for &c in rest {
        if c == BACKSLASH {
            out.push(SLASH);
        } else if (c < 0x7F && is_unsafe(c)) || c == PERCENT || c == u16::from(b'#') {
            escape_unit(&mut out, c);
        } else {
            out.push(c);
        }
    }
    out
}

/// A `file:` URL (`file://host/path`, canonical) in the path form: `file://C:\a b` or
/// `file://\\host\share`, unescaped.
fn path_url(url: &[u16]) -> Vec<u16> {
    let rest = &url[7..]; // behind "file://"
    let host_end = rest.iter().position(|&c| c == SLASH).unwrap_or(rest.len());
    let (host, path) = rest.split_at(host_end);
    let mut out = w("file://");
    let path = if host.is_empty() {
        path.strip_prefix(&[SLASH]).unwrap_or(path)
    } else {
        out.extend([BACKSLASH, BACKSLASH]);
        out.extend_from_slice(host);
        path
    };
    out.extend(unescape(path).iter().map(|&c| if c == SLASH { BACKSLASH } else { c }));
    out
}

/// The default port of a known scheme (`http` 80, `https` 443, `ftp` 21).
fn default_port(scheme: &[u16]) -> Option<&'static str> {
    match scheme {
        s if s == w("http").as_slice() => Some("80"),
        s if s == w("https").as_slice() => Some("443"),
        s if s == w("ftp").as_slice() => Some("21"),
        _ => None,
    }
}

/// `UrlCanonicalizeW(url, ...)` with these flags: the canonical form (without NUL).
pub fn canonicalize(url: &[u16], flags: u32) -> Vec<u16> {
    let start = url.iter().position(|&c| c > 0x20).unwrap_or(url.len());
    let end = url.iter().rposition(|&c| c > 0x20).map_or(start, |e| e + 1);
    let url = &url[start..end];
    if url.is_empty() {
        return Vec::new();
    }
    let simplify_ok = flags & URL_DONT_SIMPLIFY == 0;
    let pathurl = flags & (URL_FILE_USE_PATHURL | URL_WININET_COMPATIBILITY) != 0;

    let converted;
    let url = if is_dos_path(url) || url.starts_with(&[BACKSLASH, BACKSLASH]) {
        converted = path_to_file_url(url);
        &converted[..]
    } else {
        url
    };

    let Some(colon) = scheme_len(url) else {
        // No scheme: a relative URL or a plain name; only its dot segments go.
        let s = if simplify_ok { simplify(url, false) } else { url.to_vec() };
        return apply_escapes(&s, flags);
    };
    let scheme: Vec<u16> = url[..colon].iter().map(|&c| lower(c)).collect();
    let mut rest: Vec<u16> = url[colon + 1..].to_vec();
    let is_file = scheme == w("file");
    let known = is_file || default_port(&scheme).is_some();
    if !known {
        let mut out = url[..=colon].to_vec();
        if rest.starts_with(&[SLASH, SLASH]) && simplify_ok && flags & URL_PLUGGABLE_PROTOCOL == 0 {
            let host_end = 2 + rest[2..]
                .iter()
                .position(|&c| c == SLASH || c == u16::from(b'?') || c == u16::from(b'#'))
                .unwrap_or(rest.len() - 2);
            out.extend_from_slice(&rest[..host_end]);
            out.extend(apply_escapes(&simplify(&rest[host_end..], false), flags));
        } else {
            out.extend(apply_escapes(&rest, flags));
        }
        return out;
    }

    if is_file && !rest.starts_with(&[SLASH, SLASH]) {
        // `file:c:/a` is `file:///c:/a`.
        let skip = rest.iter().take_while(|&&c| c == SLASH || c == BACKSLASH).count();
        let mut r = w("///");
        r.extend_from_slice(&rest[skip..]);
        rest = r;
    }
    let mut out = scheme.clone();
    out.push(COLON);
    if !rest.starts_with(&[SLASH, SLASH]) {
        // An opaque URL of a known scheme (`http:x`): nothing more to do.
        out.extend(apply_escapes(&rest, flags));
        return out;
    }
    out.extend([SLASH, SLASH]);
    let rest = &rest[2..];
    let host_end = rest
        .iter()
        .position(|&c| c == SLASH || c == BACKSLASH || c == u16::from(b'?') || c == u16::from(b'#'))
        .unwrap_or(rest.len());
    let authority = &rest[..host_end];
    // User information is kept as it is; the host is lower-cased and loses a default port.
    let at = authority.iter().rposition(|&c| c == u16::from(b'@')).map_or(0, |a| a + 1);
    out.extend_from_slice(&authority[..at]);
    let mut host: Vec<u16> = authority[at..].iter().map(|&c| lower(c)).collect();
    if let Some(port) = default_port(&scheme)
        && let Some(c) = host.iter().rposition(|&c| c == COLON)
        && host[c + 1..] == *w(port)
    {
        host.truncate(c);
    }
    out.extend_from_slice(&host);
    let mut tail = rest[host_end..].to_vec();
    let pend = path_end(&tail);
    for c in &mut tail[..pend] {
        if *c == BACKSLASH {
            *c = SLASH;
        }
    }
    if pend == 0 && !is_file {
        tail.insert(0, SLASH);
    }
    let tail = if simplify_ok { simplify(&tail, true) } else { tail };
    if is_file && pathurl {
        out.extend_from_slice(&tail);
        return path_url(&out);
    }
    out.extend(apply_escapes(&tail, flags));
    out
}

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use std::string::String;

    fn c(url: &str, flags: u32) -> String {
        String::from_utf16(&canonicalize(&w(url), flags)).unwrap()
    }

    /// Every example of the sheet: (flags, input, result).
    #[test]
    fn the_sheets_examples() {
        let cases: &[(u32, &str, &str)] = &[
            (0, "http://www.example.com/a/b", "http://www.example.com/a/b"),
            (0, "http://www.example.com", "http://www.example.com/"),
            (0, "HTTP://WWW.EXAMPLE.COM/A", "http://www.example.com/A"),
            (0, "http://a/./b/../c", "http://a/./b/../c"),
            (0, "http://a/b/../../c", "http://a/c"),
            (0, "http://a/b/./", "http://a/b/"),
            (0, "http://a/b/..", "http://a/"),
            (0, "http://a/b/c/../../../d", "http://a/d"),
            (0, "http://a//b///c", "http://a//b///c"),
            (0, "http://a/b c/d e", "http://a/b c/d e"),
            (0, "http://a/b%20c", "http://a/b%20c"),
            (0, "http://a/b%2fc", "http://a/b%2fc"),
            (0, "http://a/%7euser", "http://a/%7euser"),
            (0, "http://a/b?x=1 2&y=./z", "http://a/b?x=1 2&y=./z"),
            (0, "http://a/b?q=../c", "http://a/b?q=../c"),
            (0, "http://a/b#frag ment", "http://a/b#frag ment"),
            (0, "http://a/b\\c\\d", "http://a/b/c/d"),
            (0, "http://a\\b", "http://a/b"),
            (0, "http://user:pw@host:80/p", "http://user:pw@host/p"),
            (0, "http://a/b|c^d{e}f", "http://a/b|c^d{e}f"),
            (0, "http://a/\u{e9}", "http://a/\u{e9}"),
            (0, "www.example.com/a/../b", "www.example.com/b"),
            (0, "example", "example"),
            (0, "a b", "a b"),
            (0, "/a/./b/../c", "/a/c"),
            (0, "./a/b", "a/b"),
            (0, "..\\a", "..\\a"),
            (0, "C:\\a\\b c.txt", "file:///C:/a/b%20c.txt"),
            (0, "C:\\a\\.\\b\\..\\c", "file:///C:/a/c"),
            (0, "\\\\server\\share\\a b", "file://server/share/a%20b"),
            (0, "file:///C:/a/b%20c", "file:///C:/a/b%20c"),
            (0, "file:///C:/a b/c", "file:///C:/a b/c"),
            (0, "file:///C:\\a\\b", "file:///C:/a/b"),
            (0, "file://server/share/a b", "file://server/share/a b"),
            (0, "file:///C:/a/./b/../c", "file:///C:/a/c"),
            (0, "file:c:/a/b", "file:///c:/a/b"),
            (0, "mailto:a@b.c", "mailto:a@b.c"),
            (0, "javascript:alert(1)", "javascript:alert(1)"),
            (0, "about:blank", "about:blank"),
            (0, "res://x.dll/1", "res://x.dll/1"),
            (0, "ftp://h/a b", "ftp://h/a b"),
            (0, "https://a/b/./c", "https://a/b/c"),
            (0, "x-custom://a/./b/../c d", "x-custom://a/c d"),
            (0, "", ""),
            (0, "  http://a/b  ", "http://a/b"),
            (0, "http://a/b/", "http://a/b/"),
            (0, "http://a/b/.", "http://a/b/"),
            (0, "http://a/.", "http://a/."),
            (0, "http://a/..", "http://a/.."),
            (0, "http://a/%", "http://a/%"),
            (0, "http://a/%zz", "http://a/%zz"),
            (0, "http://a/a%20%20b", "http://a/a%20%20b"),
            (0x1000_0000, "http://a/b%20c%41", "http://a/b cA"),
            (0x1000_0000, "http://a/b c", "http://a/b c"),
            (0x1000_0000, "file:///C:/a%20b", "file:///C:/a b"),
            (0x1000_0000, "http://a/%2e%2e/c", "http://a/../c"),
            (0x1000_0000, "http://a/b%zz", "http://a/b%zz"),
            (0x2000_0000, "http://a/b c<d>e\"f", "http://a/b%20c%3Cd%3Ee%22f"),
            (0x2000_0000, "http://a/b%20c", "http://a/b%20c"),
            (0x2000_0000, "http://a/\u{e9}", "http://a/%E9"),
            (0x2000_0000, "http://a/b|c^d{e}", "http://a/b%7Cc%5Ed%7Be%7D"),
            (0x0400_0000, "http://a/b c<d>", "http://a/b%20c<d>"),
            (0x0400_0000, "http://a/b%20 c", "http://a/b%20%20c"),
            (0x1000, "http://a/b%20c", "http://a/b%20c"),
            (0x1000, "http://a/50%", "http://a/50%"),
            (0x1000, "http://a/b c", "http://a/b c"),
            (0x2000, "a/b c?d#e", "a/b c?d#e"),
            (0x2000, "http://a/b c", "http://a/b c"),
            (0x0800_0000, "http://a/./b/../c", "http://a/./b/../c"),
            (0x0800_0000, "http://a/b/../../c", "http://a/b/../../c"),
            (0x0800_0000, "http://a//b", "http://a//b"),
            (0x0800_0000, "http://a/b c", "http://a/b c"),
            (0x2800_0000, "http://a/./b c", "http://a/./b%20c"),
            (0x1_0000, "file:///C:/a/b%20c", "file://C:\\a\\b c"),
            (0x1_0000, "file:///C:\\a\\b c", "file://C:\\a\\b c"),
            (0x1_0000, "file://server/share/a b", "file://\\\\server\\share\\a b"),
            (0x1_0000, "C:\\a\\b c", "file://C:\\a\\b c"),
            (0x8000_0000, "file:///C:/a b", "file://C:\\a b"),
            (0x8000_0000, "http://a/./b", "http://a/./b"),
            (0x8000_0000, "file://server/s/a", "file://\\\\server\\s\\a"),
            (0x8001_0000, "file:///C:/a b", "file://C:\\a b"),
            (0x4000_0000, "x-custom://a/./b", "x-custom://a/./b"),
            (0x4000_0000, "http://a/./b", "http://a/./b"),
            (0x4000_0000, "res://x/./y", "res://x/./y"),
            (0x20_0000, "C:\\a\\b c", "file:///C:/a/b%20c"),
            (0x20_0000, "\\\\server\\share\\a", "file://server/share/a"),
            (0x20_0000, "http://a/b", "http://a/b"),
            (0x80_0000, "C:\\a\\b c", "file:///C:/a/b%20c"),
            (0x80_0000, "http://a/b c", "http://a/b c"),
            (0x3000_0000, "http://a/b%20c d", "http://a/b%20c%20d"),
            (0x2, "www.example.com", "www.example.com"),
            (0x2, "ftp.example.com", "ftp.example.com"),
            (0x2, "example.com", "example.com"),
            (0x2, "a b", "a b"),
            (0x2, "file.txt", "file.txt"),
            (0x4, "C:\\a\\b", "file:///C:/a/b"),
            (0x4, "\\\\server\\share", "file://server/share"),
            (0x4, "a b", "a b"),
            (0x1, "example", "example"),
            (0x1, "http://x", "http://x/"),
            (0x1, "a b", "a b"),
            (0xe, "www.example.com", "www.example.com"),
            (0xe, "C:\\a\\b", "file:///C:/a/b"),
            (0x1, "http://a/./b", "http://a/./b"),
            (0x4000, "http://a/b c", "http://a/b c"),
            (0x100, "http://a/./b", "http://a/./b"),
        ];
        for &(flags, input, want) in cases {
            assert_eq!(c(input, flags), want, "flags {flags:#x} <{input}>");
        }
    }

    #[test]
    fn long_urls_are_whole() {
        let mut s = String::from("http://a/");
        for _ in 0..30 {
            s.push_str("0123456789");
        }
        assert_eq!(c(&s, 0), s);
    }
}
