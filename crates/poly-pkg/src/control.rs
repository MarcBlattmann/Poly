//! Debian control data (deb-control(5), deb822(5)): paragraphs of
//! `Field: value` lines; a line that starts with a space or tab continues
//! the field above it, a line starting with `#` is a comment, an empty
//! line ends the paragraph. Field names compare without regard to case.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// One paragraph: its fields in the order they were written.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Paragraph {
    /// Name and value. A value of several lines keeps its line breaks; each
    /// continuation line keeps what followed its first space or tab.
    pub fields: Vec<(String, String)>,
}

impl Paragraph {
    /// The value of field `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    /// The value of field `name` on one line (continuation lines joined
    /// with spaces): relation fields are written folded.
    pub fn folded(&self, name: &str) -> Option<String> {
        self.get(name).map(|v| v.lines().map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" "))
    }
}

/// The paragraphs of `text`.
pub fn paragraphs(text: &str) -> Vec<Paragraph> {
    let mut out = Vec::new();
    let mut cur = Paragraph::default();
    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.starts_with('#') {
            continue;
        }
        if line.trim().is_empty() {
            if !cur.fields.is_empty() {
                out.push(core::mem::take(&mut cur));
            }
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, v)) = cur.fields.last_mut() {
                v.push('\n');
                v.push_str(&line[1..]);
            }
        } else if let Some((k, v)) = line.split_once(':') {
            cur.fields.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    if !cur.fields.is_empty() {
        out.push(cur);
    }
    out
}

/// The first paragraph of `text` (a package's `control` file has one).
pub fn first(text: &str) -> Option<Paragraph> {
    paragraphs(text).into_iter().next()
}

/// A description field as its synopsis (the first line) and the long text:
/// a continuation line holding only `.` is an empty line, lines that start
/// with a further space are kept as they are (deb-control(5),
/// "Description").
pub fn description(value: &str) -> (String, String) {
    let mut lines = value.lines();
    let synopsis = lines.next().unwrap_or("").trim().to_string();
    let mut long = String::new();
    for line in lines {
        if !long.is_empty() {
            long.push('\n');
        }
        if line.trim() != "." {
            long.push_str(line.trim_end());
        }
    }
    (synopsis, long)
}

/// The package names of a relation field (`Depends`, ...), one per group
/// of alternatives: the first alternative's name, without its version
/// restriction and architecture qualifier (Debian Policy 7.1).
pub fn relation_names(field: &str) -> Vec<String> {
    field
        .split(',')
        .filter_map(|group| {
            let first = group.split('|').next()?.trim();
            let name = first.split([' ', '(', '[', '<']).next()?;
            let name = name.split(':').next()?;
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTROL: &str = "Package: galculator\n\
Version: 2.1.4-2\n\
# a comment\n\
Installed-Size: 1297\n\
Depends: libc6 (>= 2.34), libgtk-3-0t64 (>= 3.21.5) | libgtk-3-0,\n libpango-1.0-0:any (>= 1.14.0)\n\
Description: scientific calculator\n galculator is a GTK 3 calculator.\n .\n It has three modes:\n   - basic\n";

    #[test]
    fn fields_are_found_whatever_their_case() {
        let p = first(CONTROL).unwrap();
        assert_eq!(p.get("Package"), Some("galculator"));
        assert_eq!(p.get("installed-size"), Some("1297"));
        assert_eq!(p.get("Homepage"), None);
        assert_eq!(p.fields.len(), 5, "the comment is no field");
    }

    #[test]
    fn a_folded_field_is_one_line() {
        let p = first(CONTROL).unwrap();
        assert_eq!(
            p.folded("Depends").unwrap(),
            "libc6 (>= 2.34), libgtk-3-0t64 (>= 3.21.5) | libgtk-3-0, libpango-1.0-0:any (>= 1.14.0)"
        );
        assert_eq!(relation_names(&p.folded("Depends").unwrap()), ["libc6", "libgtk-3-0t64", "libpango-1.0-0"]);
        assert!(relation_names("").is_empty());
    }

    #[test]
    fn a_description_has_a_synopsis_and_paragraphs() {
        let p = first(CONTROL).unwrap();
        let (synopsis, long) = description(p.get("Description").unwrap());
        assert_eq!(synopsis, "scientific calculator");
        assert_eq!(long, "galculator is a GTK 3 calculator.\n\nIt has three modes:\n  - basic");
    }

    #[test]
    fn paragraphs_end_at_empty_lines() {
        let ps = paragraphs("Package: a\nStatus: install ok installed\n\n\nPackage: b\r\nVersion: 1\r\n");
        assert_eq!(ps.len(), 2);
        assert_eq!(ps[1].get("Version"), Some("1"));
        assert!(paragraphs("\n\n").is_empty());
    }
}
