//! URL text for CFURL (RFC 3986 and Apple's public CFURL documentation):
//! file URLs from paths and back, percent escapes, schemes, relative
//! references resolved against a base.

use alloc::string::String;
use alloc::vec::Vec;

/// Whether a byte may stand unescaped in a file URL's path.
fn path_safe(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"-._~!$&'()*+,;=:@/".contains(&b)
}

/// Percent-escape a file system path for a URL's path.
pub fn escape_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path.as_bytes() {
        if path_safe(b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(char::from(b"0123456789ABCDEF"[usize::from(b >> 4)]));
            out.push(char::from(b"0123456789ABCDEF"[usize::from(b & 15)]));
        }
    }
    out
}

/// Undo percent escapes; `None` when the result is not UTF-8 or an escape
/// is malformed.
pub fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = |c: u8| (c as char).to_digit(16);
            let v = h(*b.get(i + 1)?)? * 16 + h(*b.get(i + 2)?)?;
            out.push(v as u8);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The scheme of a URL string (`file` of `file:///x`), if it has one.
pub fn scheme(url: &str) -> Option<&str> {
    let colon = url.find(':')?;
    let s = &url[..colon];
    let mut chars = s.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() && chars.all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))).then_some(s)
}

/// The URL text of a file system path: absolute paths give
/// `file:///...`, relative ones a relative reference. `dir` adds the
/// trailing slash a directory has.
pub fn from_path(path: &str, dir: bool) -> String {
    let mut p = String::from(path);
    if dir && !p.ends_with('/') {
        p.push('/');
    }
    if !dir && p.len() > 1 && p.ends_with('/') {
        p.pop();
    }
    let escaped = escape_path(&p);
    if p.starts_with('/') { alloc::format!("file://{escaped}") } else { escaped }
}

/// The path part of a URL string (after the scheme and authority, before a
/// query or fragment), still escaped.
pub fn path_part(url: &str) -> &str {
    let mut rest = url;
    if let Some(s) = scheme(url) {
        rest = &url[s.len() + 1..];
        if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('/').map_or(&r[r.len()..], |i| &r[i..]);
        }
    }
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    &rest[..end]
}

/// Where `part` (a slice of `whole`) starts in it.
fn offset(whole: &str, part: &str) -> usize {
    part.as_ptr() as usize - whole.as_ptr() as usize
}

/// `CFURLCopyFileSystemPath` with the POSIX style: the unescaped path
/// without its trailing slash (except `/` itself).
pub fn file_system_path(url: &str) -> Option<String> {
    let p = unescape(path_part(url))?;
    Some(if p.len() > 1 { String::from(p.trim_end_matches('/')) } else { p })
}

/// Resolve the reference `rel` against the absolute URL `base` (RFC 3986
/// section 5.2, paths only: these URLs carry no query here).
pub fn resolve(base: &str, rel: &str) -> String {
    if scheme(rel).is_some() {
        return String::from(rel);
    }
    let bpath = path_part(base);
    let prefix = &base[..offset(base, bpath)];
    let merged = if rel.starts_with('/') {
        String::from(rel)
    } else {
        let dir = match bpath.rfind('/') {
            Some(i) => &bpath[..=i],
            None => "/",
        };
        alloc::format!("{dir}{rel}")
    };
    let dir = merged.ends_with('/') || merged.ends_with("/.") || merged.ends_with("/..");
    let mut parts: Vec<&str> = Vec::new();
    for c in merged.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            c => parts.push(c),
        }
    }
    let mut path = String::new();
    for p in &parts {
        path.push('/');
        path.push_str(p);
    }
    if dir || path.is_empty() {
        path.push('/');
    }
    alloc::format!("{prefix}{path}")
}

/// Drop the last path component, keeping a directory's trailing slash
/// (`file:///a/b` and `file:///a/b/` both give `file:///a/`).
pub fn deleting_last_component(url: &str) -> String {
    let path = path_part(url);
    let prefix = &url[..offset(url, path)];
    let trimmed = path.trim_end_matches('/');
    let dir = match trimmed.rfind('/') {
        Some(i) => &trimmed[..=i],
        None if path.starts_with('/') => "/",
        None => "./",
    };
    alloc::format!("{prefix}{dir}")
}

/// Append a component (escaped) to a URL's path.
pub fn appending_component(url: &str, component: &str, dir: bool) -> String {
    let mut s = String::from(url);
    if !s.ends_with('/') {
        s.push('/');
    }
    s.push_str(&escape_path(component.trim_matches('/')));
    if dir {
        s.push('/');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls() {
        assert_eq!(from_path("/x/T.app", true), "file:///x/T.app/");
        assert_eq!(from_path("/a b/c", false), "file:///a%20b/c");
        assert_eq!(from_path("Contents/Resources", true), "Contents/Resources/");
        assert_eq!(file_system_path("file:///a%20b/T.app/").unwrap(), "/a b/T.app");
        assert_eq!(file_system_path("file:///").unwrap(), "/");
        assert_eq!(file_system_path("Contents/Resources/").unwrap(), "Contents/Resources");
        assert_eq!(scheme("file:///x"), Some("file"));
        assert_eq!(scheme("Contents/x"), None);
        assert_eq!(unescape("caf%C3%A9").unwrap(), "café");
    }

    #[test]
    fn relative_references() {
        assert_eq!(resolve("file:///x/T.app/", "Contents/Resources/"), "file:///x/T.app/Contents/Resources/");
        assert_eq!(resolve("file:///x/T.app/", "./"), "file:///x/T.app/");
        assert_eq!(resolve("file:///x/y", "../z"), "file:///z");
        assert_eq!(resolve("file:///x/", "/abs"), "file:///abs");
        assert_eq!(deleting_last_component("file:///bin/prog"), "file:///bin/");
        assert_eq!(deleting_last_component("file:///a/b/"), "file:///a/");
        assert_eq!(deleting_last_component("file:///a"), "file:///");
        assert_eq!(appending_component("file:///a/", "b c", false), "file:///a/b%20c");
    }
}
