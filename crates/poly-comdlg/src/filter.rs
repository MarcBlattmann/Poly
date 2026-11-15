//! `OPENFILENAME.lpstrFilter`: pairs of NUL-terminated strings, a
//! description and a semicolon-separated list of wildcard patterns, ended
//! by a second NUL (Learn, "OPENFILENAMEW structure"); and the wildcards.

use alloc::vec::Vec;

/// One filter: what the user sees and what it matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub description: Vec<u16>,
    /// Patterns separated by `;`, as given.
    pub patterns: Vec<u16>,
}

/// The pairs in `raw`, which holds the list up to (not necessarily
/// including) its final NUL. A description without a pattern ends the list.
pub fn parse(raw: &[u16]) -> Vec<Filter> {
    let mut out = Vec::new();
    let mut parts = raw.split(|&c| c == 0);
    while let (Some(d), Some(p)) = (parts.next(), parts.next()) {
        if d.is_empty() && p.is_empty() {
            break;
        }
        if p.is_empty() {
            break;
        }
        out.push(Filter { description: d.to_vec(), patterns: p.to_vec() });
    }
    out
}

/// A custom filter (`lpstrCustomFilter`) is the same: a description and a pattern.
pub fn lower(c: u16) -> u16 {
    match char::from_u32(u32::from(c)).map(|ch| ch.to_lowercase().next().unwrap_or(ch)) {
        Some(l) if (l as u32) < 0x1_0000 => l as u16,
        _ => c,
    }
}

/// Whether `name` matches one wildcard `pattern` (`*` any run, `?` one character), ignoring case.
/// `*.*` matches every name, with a period or without (as the file dialogs treat it).
pub fn wild(pattern: &[u16], name: &[u16]) -> bool {
    if pattern == [b'*' as u16, b'.' as u16, b'*' as u16] {
        return true;
    }
    fn go(p: &[u16], n: &[u16]) -> bool {
        match p.split_first() {
            None => n.is_empty(),
            Some((&c, rest)) if c == b'*' as u16 => (0..=n.len()).any(|i| go(rest, &n[i..])),
            Some((&c, rest)) => match n.split_first() {
                Some((&x, nr)) if c == b'?' as u16 || lower(c) == lower(x) => go(rest, nr),
                _ => false,
            },
        }
    }
    go(pattern, name)
}

/// Whether `name` matches any of the `;`-separated `patterns`.
pub fn matches(patterns: &[u16], name: &[u16]) -> bool {
    patterns.split(|&c| c == b';' as u16).map(trim).any(|p| !p.is_empty() && wild(p, name))
}

fn trim(mut s: &[u16]) -> &[u16] {
    while let Some((&f, r)) = s.split_first() {
        if f == b' ' as u16 {
            s = r;
        } else {
            break;
        }
    }
    while let Some((&l, r)) = s.split_last() {
        if l == b' ' as u16 {
            s = r;
        } else {
            break;
        }
    }
    s
}

/// The extension (without the period) the first pattern of a filter implies, for the default
/// extension: `*.txt` gives `txt`; nothing for `*.*` or a pattern with wildcards in the extension.
pub fn default_extension(patterns: &[u16]) -> Option<Vec<u16>> {
    let first = trim(patterns.split(|&c| c == b';' as u16).next()?);
    let dot = first.iter().rposition(|&c| c == b'.' as u16)?;
    let ext = &first[dot + 1..];
    if ext.is_empty() || ext.iter().any(|&c| c == b'*' as u16 || c == b'?' as u16) {
        return None;
    }
    Some(ext.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn parse_pairs() {
        let raw = w("Text\0*.txt;*.log\0All\0*.*");
        // The caller gives the list with its NULs; the last NUL pair is the end.
        let mut raw = raw;
        raw.extend_from_slice(&[0, 0]);
        let f = parse(&raw);
        assert_eq!(f.len(), 2);
        assert_eq!(String::from_utf16(&f[0].description).unwrap(), "Text");
        assert_eq!(String::from_utf16(&f[0].patterns).unwrap(), "*.txt;*.log");
        assert_eq!(String::from_utf16(&f[1].patterns).unwrap(), "*.*");
        assert!(parse(&w("")).is_empty());
    }

    #[test]
    fn wildcards() {
        assert!(wild(&w("*.txt"), &w("Notes.TXT")));
        assert!(!wild(&w("*.txt"), &w("notes.txt.bak")));
        assert!(wild(&w("a?c"), &w("abc")));
        assert!(!wild(&w("a?c"), &w("ac")));
        assert!(wild(&w("*.*"), &w("README")));
        assert!(wild(&w("*"), &w("")));
        assert!(wild(&w("f*e*"), &w("file.exe")));
        assert!(matches(&w("*.c; *.h"), &w("x.h")));
        assert!(!matches(&w("*.c;*.h"), &w("x.o")));
    }

    #[test]
    fn default_extensions() {
        assert_eq!(default_extension(&w("*.txt;*.log")), Some(w("txt")));
        assert_eq!(default_extension(&w("*.*")), None);
        assert_eq!(default_extension(&w("*")), None);
        assert_eq!(default_extension(&w("data.t?t")), None);
    }
}
