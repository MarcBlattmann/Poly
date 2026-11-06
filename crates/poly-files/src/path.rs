//! Path strings: `/`-separated, UTF-8, absolute once normalized.
//!
//! - [`normalize`] removes `.`, empty segments (`//`) and a trailing `/`,
//!   and resolves `..` lexically (`/a/b/../c` is `/a/c`; `..` above the
//!   root stays at the root). Symbolic links are not looked at, so this is
//!   the path the user typed, not necessarily where it leads.
//! - [`join`], [`parent`], [`file_name`].
//! - [`extension`]: lowercased, with the compound archive extensions
//!   (`tar.gz`, `tar.xz`, `tar.bz2`, `tar.zst`) kept whole so MIME
//!   detection and "copy" names treat `archive.tar.gz` as one extension;
//!   [`split_ext`] splits a name the same way.
//! - [`is_hidden`]: dot files, by the Unix convention.
//! - [`crumbs`]: the path bar's segments, with everything up to `$HOME`
//!   collapsed into one "Home" crumb.
//! - [`is_inside`]: whether a path is a folder or below it (used to refuse
//!   pasting a folder into itself).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// `path` with `.`, `..`, repeated and trailing slashes resolved. A
/// relative path stays relative (leading `..` kept); an empty one is `.`.
pub fn normalize(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut segs: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if segs.last().is_some_and(|s| *s != "..") {
                    segs.pop();
                } else if !absolute {
                    segs.push("..");
                }
            }
            s => segs.push(s),
        }
    }
    let mut out = String::with_capacity(path.len());
    if absolute {
        out.push('/');
    }
    out.push_str(&segs.join("/"));
    if out.is_empty() {
        out.push('.');
    }
    out
}

/// `name` resolved against directory `base` (an absolute `name` replaces
/// it), normalized.
pub fn join(base: &str, name: &str) -> String {
    if name.starts_with('/') {
        return normalize(name);
    }
    let mut s = String::with_capacity(base.len() + name.len() + 1);
    s.push_str(base);
    s.push('/');
    s.push_str(name);
    normalize(&s)
}

/// The folder containing normalized `path`: `/a/b` → `/a`, `/a` → `/`;
/// `None` for `/` and for a relative name without a slash.
pub fn parent(path: &str) -> Option<&str> {
    let path = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    if path == "/" || path.is_empty() {
        return None;
    }
    match path.rfind('/') {
        Some(0) => Some("/"),
        Some(i) => path.get(..i),
        None => None,
    }
}

/// The last component: `/a/b.txt` → `b.txt`; `/` → `""`.
pub fn file_name(path: &str) -> &str {
    let path = path.trim_end_matches('/');
    match path.rfind('/') {
        Some(i) => path.get(i + 1..).unwrap_or(""),
        None => path,
    }
}

/// Extensions that span two dots.
const COMPOUND: [&str; 4] = ["tar.gz", "tar.xz", "tar.bz2", "tar.zst"];

/// `name` split before its extension: `("photo", ".JPG")`,
/// `("archive", ".tar.gz")`, `(".bashrc", "")`, `("README", "")`. A dot at
/// the start (a hidden file) or the end does not start an extension.
pub fn split_ext(name: &str) -> (&str, &str) {
    // `.tar.gz` at the end, with something before it.
    let ends_with = |suffix: &str| {
        name.len() > suffix.len() + 1
            && name
                .get(name.len() - suffix.len() - 1..)
                .and_then(|tail| tail.strip_prefix('.'))
                .is_some_and(|t| t.eq_ignore_ascii_case(suffix))
    };
    for c in COMPOUND {
        if ends_with(c) {
            let at = name.len() - c.len() - 1;
            if let (Some(stem), Some(ext)) = (name.get(..at), name.get(at..)) {
                return (stem, ext);
            }
        }
    }
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (name.get(..i).unwrap_or(name), name.get(i..).unwrap_or("")),
        _ => (name, ""),
    }
}

/// The lowercased extension of `name` without the dot (`tar.gz` for
/// `x.tar.gz`), if it has one.
pub fn extension(name: &str) -> Option<String> {
    let (_, ext) = split_ext(file_name(name));
    let ext = ext.strip_prefix('.')?;
    Some(ext.to_ascii_lowercase())
}

/// Whether `name` is hidden by default (starts with a dot).
pub fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// One segment of the path bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Crumb {
    /// What the segment shows: a folder name, "Home" or "Computer".
    pub label: String,
    /// Where clicking it goes.
    pub path: String,
}

/// The label of the root crumb (and of the root place in the sidebar).
pub const ROOT_LABEL: &str = "Computer";
/// The label of the home crumb and place.
pub const HOME_LABEL: &str = "Home";

/// The path bar's segments for `path`: `/usr/share` is Computer › usr ›
/// share; a path at or below `home` starts with one Home crumb
/// (`~/Documents/x` is Home › Documents › x).
pub fn crumbs(path: &str, home: Option<&str>) -> Vec<Crumb> {
    let path = normalize(path);
    let mut out = Vec::new();
    let home = home.map(normalize).filter(|h| h.starts_with('/') && h != "/");
    let rest = match &home {
        Some(h) if is_inside(&path, h) => {
            out.push(Crumb { label: HOME_LABEL.into(), path: h.clone() });
            path.get(h.len()..).unwrap_or("").to_string()
        }
        _ => {
            out.push(Crumb { label: ROOT_LABEL.into(), path: "/".into() });
            path.clone()
        }
    };
    let mut cur = out.last().map(|c| c.path.clone()).unwrap_or_default();
    for seg in rest.split('/').filter(|s| !s.is_empty()) {
        cur = join(&cur, seg);
        out.push(Crumb { label: seg.into(), path: cur.clone() });
    }
    out
}

/// Whether `child` is `ancestor` or lies below it (lexically, after
/// normalizing both).
pub fn is_inside(child: &str, ancestor: &str) -> bool {
    let (c, a) = (normalize(child), normalize(ancestor));
    if c == a || a == "/" && c.starts_with('/') {
        return true;
    }
    c.len() > a.len() && c.starts_with(a.as_str()) && c.as_bytes().get(a.len()) == Some(&b'/')
}

/// `path` as a `file://` URI (RFC 8089), bytes outside the unreserved set
/// and `/` percent-encoded.
pub fn file_uri(path: &str) -> String {
    let mut s = String::from("file://");
    s.push_str(&percent_encode(path));
    s
}

/// Percent-encodes everything except RFC 2396's unreserved characters
/// (letters, digits, `-_.!~*'()`) and `/`, as uppercase `%XX` per UTF-8
/// byte.
pub fn percent_encode(s: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()/".contains(&b) {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[usize::from(b >> 4)] as char);
            out.push(HEX[usize::from(b & 15)] as char);
        }
    }
    out
}

/// Undoes `%XX` escapes; a malformed escape stays as it is. Bytes that do
/// not form UTF-8 are replaced.
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |x: u8| (x as char).to_digit(16);
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|&x| hex(x)), b.get(i + 2).and_then(|&x| hex(x)))
        {
            out.push((h * 16 + l) as u8);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizing() {
        assert_eq!(normalize("/a//b/./c/"), "/a/b/c");
        assert_eq!(normalize("/a/b/../c"), "/a/c");
        assert_eq!(normalize("/../.."), "/");
        assert_eq!(normalize("/"), "/");
        assert_eq!(normalize(""), ".");
        assert_eq!(normalize("a/../../b"), "../b");
        assert_eq!(join("/home/u", "docs"), "/home/u/docs");
        assert_eq!(join("/home/u", "/etc"), "/etc");
        assert_eq!(join("/", ".."), "/");
    }

    #[test]
    fn parents_and_names() {
        assert_eq!(parent("/a/b"), Some("/a"));
        assert_eq!(parent("/a"), Some("/"));
        assert_eq!(parent("/"), None);
        assert_eq!(parent("x"), None);
        assert_eq!(file_name("/a/b.txt"), "b.txt");
        assert_eq!(file_name("/"), "");
        assert_eq!(file_name("rel"), "rel");
    }

    #[test]
    fn extensions() {
        assert_eq!(extension("Photo.JPG").as_deref(), Some("jpg"));
        assert_eq!(extension("archive.TAR.gz").as_deref(), Some("tar.gz"));
        assert_eq!(extension("x.gz").as_deref(), Some("gz"));
        assert_eq!(extension(".bashrc"), None);
        assert_eq!(extension("README"), None);
        assert_eq!(extension("trailing."), None);
        assert_eq!(extension("/dir.d/file").as_deref(), None);
        assert_eq!(split_ext("archive.tar.gz"), ("archive", ".tar.gz"));
        assert_eq!(split_ext(".tar.gz"), (".tar", ".gz"));
        assert_eq!(split_ext("a.b.c"), ("a.b", ".c"));
        assert_eq!(split_ext("é.ü"), ("é", ".ü"));
        assert!(is_hidden(".git") && !is_hidden("git"));
    }

    #[test]
    fn breadcrumbs() {
        let c = crumbs("/home/u/Documents/x", Some("/home/u/"));
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Documents", "x"]);
        assert_eq!(c[2].path, "/home/u/Documents/x");
        assert_eq!(c[0].path, "/home/u");
        let c = crumbs("/usr/share", Some("/home/u"));
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Computer", "usr", "share"]);
        assert_eq!(c[1].path, "/usr");
        assert_eq!(crumbs("/", None).len(), 1);
        assert_eq!(crumbs("/home/u", Some("/home/u")).len(), 1);
        // A sibling that shares the prefix is not inside home.
        assert_eq!(crumbs("/home/u2", Some("/home/u"))[0].label, "Computer");
    }

    #[test]
    fn inside() {
        assert!(is_inside("/a/b/c", "/a/b"));
        assert!(is_inside("/a/b", "/a/b/"));
        assert!(!is_inside("/a/bc", "/a/b"));
        assert!(is_inside("/x", "/"));
        assert!(!is_inside("/a", "/a/b"));
    }

    #[test]
    fn percent() {
        assert_eq!(percent_encode("/home/u/My File ü.txt"), "/home/u/My%20File%20%C3%BC.txt");
        assert_eq!(percent_decode("/home/u/My%20File%20%C3%BC.txt"), "/home/u/My File ü.txt");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
        assert_eq!(file_uri("/a b"), "file:///a%20b");
    }
}
