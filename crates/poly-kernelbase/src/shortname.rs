//! Short (8.3) names: `GetShortPathNameW`, `GetLongPathNameW` (sheets of `kernel32.dll`) and the
//! NT personality's lookup of a short name.
//!
//! Poly's file systems keep no second name for a file, so the short name of a long name is
//! generated from the directory's entries whenever it is asked for, by the rule the sheet of
//! `GetShortPathNameW` describes:
//!
//! - a name that already is a valid 8.3 name ([`is_short_form`]: a base of 1 to 8 and an
//!   extension of up to 3 characters of the allowed set) is its own short name, in its own case;
//! - another name gets the first six characters of its base in upper case (spaces and dots
//!   removed, characters an 8.3 name cannot have replaced by `_`), `~`, a number, and the first
//!   three characters of its extension (after the last dot) in upper case: `a long file
//!   name.text` -> `ALONGF~1.TEX`;
//! - the number counts up for the names of one directory that have the same generated start, in
//!   the order the entries were created (then by name), skipping a number whose name another
//!   entry of the directory has; from `~10` on the base keeps five characters.
//!
//! What differs from Windows: Windows stores the short name when the file is made, so it stays
//! when an earlier name with the same start is deleted; Poly's numbers are computed from the
//! entries there are now. A name of one or two letters with a long extension gets a hashed form
//! on Windows ("A7903~1.VER") whose digits the sheet does not give; Poly makes `A~1.VER`.

use crate::error::*;
use alloc::collections::BTreeSet;
use alloc::vec::Vec;

const DOT: u16 = b'.' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;

/// The characters besides letters and digits an 8.3 name may have (Learn: "Naming Files, Paths,
/// and Namespaces", short names).
const SPECIALS: &[u8] = b"!#$%&'()-@^_`{}~";

fn upcase(c: u16) -> u16 {
    if (u16::from(b'a')..=u16::from(b'z')).contains(&c) { c - 32 } else { c }
}

fn upper(s: &[u16]) -> Vec<u16> {
    s.iter().map(|&c| upcase(c)).collect()
}

fn eq_fold(a: &[u16], b: &[u16]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(&x, &y)| upcase(x) == upcase(y))
}

fn allowed(c: u16) -> bool {
    c < 0x80 && ((c as u8).is_ascii_alphanumeric() || SPECIALS.contains(&(c as u8)))
}

/// Whether `name` is a valid 8.3 name: 1 to 8 allowed characters, optionally a dot and 1 to 3
/// more.
pub fn is_short_form(name: &[u16]) -> bool {
    let (base, ext) = match name.iter().position(|&c| c == DOT) {
        Some(d) => (&name[..d], Some(&name[d + 1..])),
        None => (name, None),
    };
    let ext_ok = match ext {
        Some(e) => (1..=3).contains(&e.len()) && e.iter().all(|&c| allowed(c)),
        None => true,
    };
    (1..=8).contains(&base.len()) && base.iter().all(|&c| allowed(c)) && ext_ok
}

/// One character of a generated name: upper case, `_` for what an 8.3 name cannot have, nothing
/// for a space or a dot.
fn clean(part: &[u16]) -> Vec<u16> {
    part.iter()
        .filter(|&&c| c != u16::from(b' ') && c != DOT)
        .map(|&c| if allowed(c) { upcase(c) } else { u16::from(b'_') })
        .collect()
}

/// The generated start of a long name's short name: up to six characters of the base and up to
/// three of the extension.
pub fn basis(name: &[u16]) -> (Vec<u16>, Vec<u16>) {
    let (base, ext) = match name.iter().rposition(|&c| c == DOT) {
        Some(d) if d > 0 => (&name[..d], &name[d + 1..]),
        _ => (name, &[][..]),
    };
    let mut base = clean(base);
    base.truncate(6);
    if base.is_empty() {
        base.push(u16::from(b'_'));
    }
    let mut ext = clean(ext);
    ext.truncate(3);
    (base, ext)
}

/// `BASE~N.EXT` (the base shortened so that the name part stays within eight characters).
fn numbered(base: &[u16], n: u32, ext: &[u16]) -> Vec<u16> {
    let digits: Vec<u16> = alloc::format!("~{n}").encode_utf16().collect();
    let keep = base.len().min(8 - digits.len().min(7)).min(if n < 10 { 6 } else { 5 });
    let mut out = base[..keep].to_vec();
    out.extend_from_slice(&digits);
    if !ext.is_empty() {
        out.push(DOT);
        out.extend_from_slice(ext);
    }
    out
}

/// The short name of every entry of one directory (`entries`: name and creation time).
pub fn short_names(entries: &[(Vec<u16>, u64)]) -> Vec<Vec<u16>> {
    let mut out: Vec<Vec<u16>> = entries.iter().map(|(n, _)| n.clone()).collect();
    let mut used: BTreeSet<Vec<u16>> = entries.iter().map(|(n, _)| upper(n)).collect();
    let mut order: Vec<usize> = (0..entries.len()).filter(|&i| !is_short_form(&entries[i].0)).collect();
    order.sort_by(|&a, &b| (entries[a].1, &entries[a].0).cmp(&(entries[b].1, &entries[b].0)));
    for i in order {
        let (base, ext) = basis(&entries[i].0);
        for n in 1u32.. {
            let candidate = numbered(&base, n, &ext);
            if used.insert(upper(&candidate)) {
                out[i] = candidate;
                break;
            }
        }
    }
    out
}

/// The entry of a directory (`names`) whose short name is `component`, for a lookup that found no
/// entry of that name. `creation(i)` gives entry `i`'s creation time; it is asked only for the
/// long names whose generated start can make `component`.
pub fn alias_target(names: &[Vec<u16>], component: &[u16], mut creation: impl FnMut(usize) -> u64) -> Option<usize> {
    let tilde = component.iter().position(|&c| c == u16::from(b'~'))?;
    let prefix = upper(&component[..tilde]);
    let ext = match component.iter().rposition(|&c| c == DOT) {
        Some(d) if d > tilde => upper(&component[d + 1..]),
        _ => Vec::new(),
    };
    let candidate = |n: &Vec<u16>| {
        if is_short_form(n) {
            return false;
        }
        let (base, e) = basis(n);
        e == ext && (base.starts_with(&prefix) || prefix.starts_with(&base))
    };
    if !names.iter().any(candidate) {
        return None;
    }
    let entries: Vec<(Vec<u16>, u64)> =
        names.iter().enumerate().map(|(i, n)| (n.clone(), if candidate(n) { creation(i) } else { 0 })).collect();
    short_names(&entries).iter().zip(names).position(|(s, n)| !is_short_form(n) && eq_fold(s, component))
}

/// What a path conversion asks of the file system.
pub trait DirLookup {
    /// The name directory `dir` (a path ending in a separator, or empty for the current directory)
    /// holds for `name` without case, `None` when it has none.
    fn find(&mut self, dir: &[u16], name: &[u16]) -> Result<Option<Vec<u16>>, u32>;
    /// Every entry of `dir`: its name and creation time.
    fn list(&mut self, dir: &[u16]) -> Result<Vec<(Vec<u16>, u64)>, u32>;
}

/// The entry of `dir` that `component` names: its stored name and its short name. The stored name
/// is looked up first; a component that names no entry is compared with the short names.
fn resolve(l: &mut dyn DirLookup, dir: &[u16], component: &[u16], want_short: bool) -> Result<Option<Entry>, u32> {
    if let Some(stored) = l.find(dir, component)? {
        if !want_short || is_short_form(&stored) {
            return Ok(Some(Entry { short: stored.clone(), long: stored, by_alias: false }));
        }
        let entries = l.list(dir)?;
        let shorts = short_names(&entries);
        let short = entries.iter().zip(&shorts).find(|((n, _), _)| *n == stored).map(|(_, s)| s.clone());
        return Ok(Some(Entry { short: short.unwrap_or_else(|| stored.clone()), long: stored, by_alias: false }));
    }
    if !component.contains(&u16::from(b'~')) {
        return Ok(None);
    }
    let entries = l.list(dir)?;
    let shorts = short_names(&entries);
    Ok(entries.into_iter().zip(shorts).find(|(_, s)| eq_fold(s, component)).map(|((long, _), short)| Entry {
        long,
        short,
        by_alias: true,
    }))
}

struct Entry {
    long: Vec<u16>,
    short: Vec<u16>,
    by_alias: bool,
}

/// Convert every component of `path` (sheets of `GetShortPathNameW` and `GetLongPathNameW`): to
/// its short name (`short`), or to its long name. A long name keeps the case the caller wrote
/// unless it could be an 8.3 name (then the stored case); a short alias becomes the stored name.
/// The root (`C:\`, `\\?\C:\`, `\\server\share\`), `.`, `..`, empty components and the separators
/// (`\` or `/`) stay as written. An empty path is `ERROR_PATH_NOT_FOUND`, a wildcard
/// `ERROR_INVALID_NAME`, a missing entry `ERROR_FILE_NOT_FOUND` (the last component) or
/// `ERROR_PATH_NOT_FOUND`.
pub fn convert_path(path: &[u16], l: &mut dyn DirLookup, short: bool) -> Result<Vec<u16>, u32> {
    if path.is_empty() {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    let verbatim = path.len() >= 4 && path[..4] == [BACKSLASH, BACKSLASH, u16::from(b'?'), BACKSLASH];
    let root = if verbatim { 4 + crate::path::root_len(&path[4..]) } else { crate::path::root_len(path) };
    let root = root.min(path.len());
    let mut out: Vec<u16> = path[..root].to_vec();
    let mut walk: Vec<u16> = out.iter().map(|&c| if c == SLASH { BACKSLASH } else { c }).collect();
    let rest = &path[root..];
    // The components, each with the separator after it.
    let mut parts: Vec<(&[u16], Option<u16>)> = Vec::new();
    let mut start = 0;
    for (i, &c) in rest.iter().enumerate() {
        if c == BACKSLASH || c == SLASH {
            parts.push((&rest[start..i], Some(c)));
            start = i + 1;
        }
    }
    parts.push((&rest[start..], None));
    let last_named = parts.iter().rposition(|(c, _)| !c.is_empty()).unwrap_or(0);
    for (n, &(comp, sep)) in parts.iter().enumerate() {
        if comp.iter().any(|&c| c == u16::from(b'*') || c == u16::from(b'?')) {
            return Err(ERROR_INVALID_NAME);
        }
        let dots = !comp.is_empty() && comp.len() <= 2 && comp.iter().all(|&c| c == DOT);
        if comp.is_empty() || dots {
            out.extend_from_slice(comp);
            if comp.len() == 2 {
                // `..`: the directory looked in is the parent (never above the root).
                let mut end = walk.len();
                if end > root && walk[end - 1] == BACKSLASH {
                    end -= 1;
                }
                while end > root && walk[end - 1] != BACKSLASH {
                    end -= 1;
                }
                walk.truncate(end);
            }
            if let Some(sep) = sep {
                out.push(sep);
            }
            continue;
        }
        let Some(e) = resolve(l, &walk, comp, short)? else {
            return Err(if n == last_named { ERROR_FILE_NOT_FOUND } else { ERROR_PATH_NOT_FOUND });
        };
        if short {
            out.extend_from_slice(&e.short);
        } else if e.by_alias || is_short_form(comp) {
            out.extend_from_slice(&e.long);
        } else {
            out.extend_from_slice(comp);
        }
        walk.extend_from_slice(&e.long);
        if let Some(sep) = sep {
            out.push(sep);
            walk.push(BACKSLASH);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wide;
    extern crate std;

    fn names(list: &[&str]) -> Vec<(Vec<u16>, u64)> {
        list.iter().enumerate().map(|(i, n)| (wide(n), i as u64)).collect()
    }

    fn text(v: &[u16]) -> std::string::String {
        std::string::String::from_utf16_lossy(v)
    }

    #[test]
    fn short_forms() {
        for n in ["gp.txt", "GPUPPER.TXT", "GpMixed.Txt", "a", "abcdefgh.abc", "a~1.t"] {
            assert!(is_short_form(&wide(n)), "{n}");
        }
        for n in ["gp my file.txt", "abcdefghi", "a.abcd", "ab.c.d", ".x", "a.", "a+b"] {
            assert!(!is_short_form(&wide(n)), "{n}");
        }
    }

    #[test]
    fn generated_names_follow_the_sheet() {
        let e = names(&[
            "gp_long_directory",
            "a long file name.text",
            "gp my file.txt",
            "ab.c.d.txt",
            "gp_second_file_one.txt",
            "gp_second_file_two.txt",
            "gp.txt",
            "GpMixed.Txt",
        ]);
        let s: Vec<_> = short_names(&e).iter().map(|v| text(v)).collect();
        assert_eq!(
            s,
            [
                "GP_LON~1",
                "ALONGF~1.TEX",
                "GPMYFI~1.TXT",
                "ABCD~1.TXT",
                "GP_SEC~1.TXT",
                "GP_SEC~2.TXT",
                "gp.txt",
                "GpMixed.Txt"
            ]
        );
    }

    #[test]
    fn numbers_follow_creation_and_skip_taken_names() {
        let mut e = names(&["longname_b.txt", "longname_a.txt", "LONGNA~1.TXT"]);
        e[0].1 = 1;
        e[1].1 = 2;
        let s = short_names(&e);
        // b was made first, but ~1 is another entry's real name.
        assert_eq!((text(&s[0]), text(&s[1])), ("LONGNA~2.TXT".into(), "LONGNA~3.TXT".into()));
        let many: Vec<std::string::String> = (0..11).map(|i| std::format!("longname{i:02}.txt")).collect();
        let refs: Vec<&str> = many.iter().map(|s| s.as_str()).collect();
        let s = short_names(&names(&refs));
        assert_eq!(text(&s[9]), "LONGN~10.TXT");
    }

    #[test]
    fn aliases_find_their_entries() {
        let n: Vec<Vec<u16>> =
            ["gp_second_file_one.txt", "gp_second_file_two.txt", "gp.txt"].iter().map(|s| wide(s)).collect();
        let mut asked = Vec::new();
        let mut creation = |i: usize| {
            asked.push(i);
            i as u64
        };
        assert_eq!(alias_target(&n, &wide("gp_sec~2.txt"), &mut creation), Some(1));
        assert_eq!(asked, [0, 1], "only the candidates' times are read");
        assert_eq!(alias_target(&n, &wide("GP_SEC~3.TXT"), |i| i as u64), None);
        assert_eq!(alias_target(&n, &wide("GP_SEC~1.DOC"), |i| i as u64), None);
        assert_eq!(alias_target(&n, &wide("gp.txt"), |i| i as u64), None, "no tilde");
    }

    /// A directory tree: (directory, name, creation).
    struct Tree(Vec<(&'static str, &'static str)>);

    impl DirLookup for Tree {
        fn find(&mut self, dir: &[u16], name: &[u16]) -> Result<Option<Vec<u16>>, u32> {
            let d: std::string::String = text(dir).trim_start_matches(r"\\?\").into();
            Ok(self
                .0
                .iter()
                .find(|(td, n)| td.eq_ignore_ascii_case(&d) && eq_fold(&wide(n), name))
                .map(|(_, n)| wide(n)))
        }
        fn list(&mut self, dir: &[u16]) -> Result<Vec<(Vec<u16>, u64)>, u32> {
            let d: std::string::String = text(dir).trim_start_matches(r"\\?\").into();
            Ok(self
                .0
                .iter()
                .enumerate()
                .filter(|(_, (td, _))| td.eq_ignore_ascii_case(&d))
                .map(|(i, (_, n))| (wide(n), i as u64))
                .collect())
        }
    }

    fn tree() -> Tree {
        Tree(alloc::vec![
            (r"C:\", "PolyTests"),
            (r"C:\PolyTests\", "gp_long_directory"),
            (r"C:\PolyTests\", "gp.txt"),
            (r"C:\PolyTests\", "GpMixed.Txt"),
            (r"C:\PolyTests\gp_long_directory\", "a long file name.text"),
            (r"", "gp_long_directory"),
            (r"gp_long_directory\", "a long file name.text"),
        ])
    }

    #[test]
    fn short_and_long_paths() {
        let mut t = tree();
        let short = |t: &mut Tree, p: &str| convert_path(&wide(p), t, true).map(|v| text(&v));
        let long = |t: &mut Tree, p: &str| convert_path(&wide(p), t, false).map(|v| text(&v));
        let f = r"C:\PolyTests\gp_long_directory\a long file name.text";
        assert_eq!(short(&mut t, f), Ok(r"C:\POLYTE~1\GP_LON~1\ALONGF~1.TEX".into()));
        assert_eq!(long(&mut t, r"C:\POLYTE~1\GP_LON~1\ALONGF~1.TEX"), Ok(f.into()));
        assert_eq!(
            long(&mut t, r"c:\polyte~1\gp_lon~1\alongf~1.tex"),
            Ok(r"c:\PolyTests\gp_long_directory\a long file name.text".into())
        );
        // Long names keep the caller's case; 8.3-shaped ones get the stored case.
        assert_eq!(long(&mut t, r"C:\POLYTESTS\GP_LONG_DIRECTORY"), Ok(r"C:\POLYTESTS\GP_LONG_DIRECTORY".into()));
        assert_eq!(long(&mut t, r"C:\PolyTests\gpmixed.txt"), Ok(r"C:\PolyTests\GpMixed.Txt".into()));
        assert_eq!(short(&mut t, r"C:\PolyTests\GPMIXED.TXT"), Ok(r"C:\POLYTE~1\GpMixed.Txt".into()));
        // Separators, prefixes, dots, relative paths, trailing separators.
        assert_eq!(short(&mut t, "C:/PolyTests/gp_long_directory"), Ok("C:/POLYTE~1/GP_LON~1".into()));
        assert_eq!(short(&mut t, r"\\?\C:\PolyTests\gp.txt"), Ok(r"\\?\C:\POLYTE~1\gp.txt".into()));
        assert_eq!(
            short(&mut t, r"C:\PolyTests\gp_long_directory\..\gp.txt"),
            Ok(r"C:\POLYTE~1\GP_LON~1\..\gp.txt".into())
        );
        assert_eq!(short(&mut t, r"gp_long_directory\a long file name.text"), Ok(r"GP_LON~1\ALONGF~1.TEX".into()));
        assert_eq!(long(&mut t, r"GP_LON~1\ALONGF~1.TEX"), Ok(r"gp_long_directory\a long file name.text".into()));
        assert_eq!(short(&mut t, r"C:\PolyTests\gp_long_directory\"), Ok(r"C:\POLYTE~1\GP_LON~1\".into()));
        assert_eq!(long(&mut t, r"c:\"), Ok(r"c:\".into()));
        // Errors.
        assert_eq!(short(&mut t, r"C:\PolyTests\missing.txt"), Err(ERROR_FILE_NOT_FOUND));
        assert_eq!(short(&mut t, r"C:\PolyTests\nodir\missing.txt"), Err(ERROR_PATH_NOT_FOUND));
        assert_eq!(short(&mut t, ""), Err(ERROR_PATH_NOT_FOUND));
        assert_eq!(long(&mut t, r"C:\PolyTests\*"), Err(ERROR_INVALID_NAME));
    }
}
