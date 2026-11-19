//! `GetDriveTypeW`: which root a name asks about (`specs/windows/kernel32.dll/GetDriveTypeW.yaml`).
//!
//! The function answers for "the root directory of the drive"; Learn: "A trailing backslash is
//! required. If this parameter is NULL, the function uses the root of the current directory." The
//! sheet observed what that means in detail: a drive root in any spelling (`C:`, `C:/`, `C:\\`,
//! `C:\.`, `C:\..`, `C:\ `, the `\\?\`, `\\.\` and `\??\` prefixes with their backslash) answers
//! for the drive; a directory below a root answers for its drive only when the name ends in a
//! backslash and the directory exists; every other name is `DRIVE_NO_ROOT_DIR`. The function
//! decides the kind of name here; the caller looks at the file system.
//!
//! Constants: `winbase.h` (`DRIVE_UNKNOWN` 0, `DRIVE_NO_ROOT_DIR` 1, `DRIVE_FIXED` 3).

use alloc::vec::Vec;

/// `DRIVE_NO_ROOT_DIR` (`winbase.h`).
pub const DRIVE_NO_ROOT_DIR: u32 = 1;
/// `DRIVE_FIXED` (`winbase.h`).
pub const DRIVE_FIXED: u32 = 3;

/// What a name passed to `GetDriveTypeW` asks about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveQuery {
    /// The root of the drive with this letter (upper case).
    Root(u8),
    /// A directory below a root (`X:\dir\...\`, ending in a backslash): the drive's type when it
    /// exists, else `DRIVE_NO_ROOT_DIR`.
    Directory(Vec<u16>),
    /// No root: `DRIVE_NO_ROOT_DIR`.
    NoRoot,
}

const BACKSLASH: u16 = b'\\' as u16;

fn is_sep(c: u16) -> bool {
    c == BACKSLASH || c == u16::from(b'/')
}

fn letter(c: u16) -> Option<u8> {
    let up = c & !0x20;
    (u16::from(b'A')..=u16::from(b'Z')).contains(&up).then_some(up as u8)
}

/// The drive letter and the components of `path` made full against `cwd` (a full DOS path of the
/// current directory, `X:\...`): `.` and `..` and empty components resolved, and (for a name that
/// does not end in a separator) trailing blanks and dots of the last component dropped, as
/// `GetFullPathNameW` does. `None` for a name that has no drive (UNC, a bad letter).
fn full_components(path: &[u16], cwd: &[u16]) -> Option<(u8, Vec<Vec<u16>>)> {
    let (drive, rest, base): (u8, &[u16], &[u16]) = match path {
        [d, c, rest @ ..] if *c == u16::from(b':') => {
            let d = letter(*d)?;
            match rest.first() {
                Some(&s) if is_sep(s) => (d, rest, &[]),
                // `X:name`: relative to the drive's current directory; only the current drive's is
                // known, so another drive's is taken as its root.
                _ => {
                    let same = cwd.first().and_then(|&c| letter(c)) == Some(d);
                    (d, rest, if same { cwd.get(2..).unwrap_or(&[]) } else { &[] })
                }
            }
        }
        [s, t, ..] if is_sep(*s) && is_sep(*t) => return None,
        [s, ..] if is_sep(*s) => (letter(*cwd.first()?)?, path, &[]),
        _ => (letter(*cwd.first()?)?, path, cwd.get(2..).unwrap_or(&[])),
    };
    let mut comps: Vec<Vec<u16>> = Vec::new();
    let push = |comp: &[u16], comps: &mut Vec<Vec<u16>>| match comp {
        [] => {}
        [d] if *d == u16::from(b'.') => {}
        [a, b] if *a == u16::from(b'.') && *b == u16::from(b'.') => {
            comps.pop();
        }
        _ => comps.push(comp.to_vec()),
    };
    for comp in base.split(|&c| is_sep(c)) {
        push(comp, &mut comps);
    }
    let trailing = rest.last().is_some_and(|&c| is_sep(c));
    let parts: Vec<&[u16]> = rest.split(|&c| is_sep(c)).collect();
    let n = parts.len();
    for (i, comp) in parts.into_iter().enumerate() {
        if i + 1 == n && !trailing && comp != [u16::from(b'.')] && comp != [u16::from(b'.'), u16::from(b'.')] {
            let mut end = comp.len();
            while end > 0 && (comp[end - 1] == u16::from(b' ') || comp[end - 1] == u16::from(b'.')) {
                end -= 1;
            }
            push(&comp[..end], &mut comps);
        } else {
            push(comp, &mut comps);
        }
    }
    Some((drive, comps))
}

/// What `GetDriveTypeW(path)` asks about, with `cwd` the current directory as a full DOS path
/// (`None` is the NULL name: the current directory's root).
pub fn drive_query(path: Option<&[u16]>, cwd: &[u16]) -> DriveQuery {
    let Some(path) = path else {
        return cwd.first().and_then(|&c| letter(c)).map_or(DriveQuery::NoRoot, DriveQuery::Root);
    };
    if path.is_empty() {
        return DriveQuery::NoRoot;
    }
    let trailing = path.last().is_some_and(|&c| is_sep(c));
    // The device and long-name prefixes: only `X:\` (or a directory below it) after them.
    for prefix in ["\\\\?\\", "\\\\.\\", "\\??\\"] {
        let p: Vec<u16> = prefix.encode_utf16().collect();
        if let Some(rest) = path.strip_prefix(p.as_slice()) {
            return match rest {
                [d, c, s] if *c == u16::from(b':') && is_sep(*s) => {
                    letter(*d).map_or(DriveQuery::NoRoot, DriveQuery::Root)
                }
                [d, c, s, ..] if *c == u16::from(b':') && is_sep(*s) && trailing && letter(*d).is_some() => {
                    directory(rest, cwd)
                }
                _ => DriveQuery::NoRoot,
            };
        }
    }
    match path {
        [d, c] if *c == u16::from(b':') => letter(*d).map_or(DriveQuery::NoRoot, DriveQuery::Root),
        [d, c, ..] if *c == u16::from(b':') && letter(*d).is_none() => DriveQuery::NoRoot,
        _ if trailing => directory(path, cwd),
        _ => match full_components(path, cwd) {
            Some((d, comps)) if comps.is_empty() => DriveQuery::Root(d),
            _ => DriveQuery::NoRoot,
        },
    }
}

/// A name ending in a separator: its root, or the directory to look for.
fn directory(path: &[u16], cwd: &[u16]) -> DriveQuery {
    match full_components(path, cwd) {
        Some((d, comps)) if comps.is_empty() => DriveQuery::Root(d),
        Some((d, comps)) => {
            let mut full: Vec<u16> = alloc::vec![u16::from(d), u16::from(b':')];
            for c in comps {
                full.push(BACKSLASH);
                full.extend_from_slice(&c);
            }
            full.push(BACKSLASH);
            DriveQuery::Directory(full)
        }
        None => DriveQuery::NoRoot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn q(s: &str) -> DriveQuery {
        drive_query(Some(&w(s)), &w("C:\\Users\\poly\\scratch"))
    }

    #[test]
    fn the_system_drive_in_every_spelling() {
        for s in [
            "C:\\",
            "c:\\",
            "C:",
            "C:/",
            "C:\\\\",
            "C:\\.",
            "C:\\..",
            "C:\\ ",
            "\\",
            "\\\\?\\C:\\",
            "\\\\.\\C:\\",
            "\\??\\C:\\",
        ] {
            assert_eq!(q(s), DriveQuery::Root(b'C'), "{s}");
        }
        assert_eq!(drive_query(None, &w("C:\\Windows")), DriveQuery::Root(b'C'));
        assert_eq!(q("x:"), DriveQuery::Root(b'X'));
    }

    #[test]
    fn directories_need_the_trailing_backslash() {
        assert_eq!(q("C:\\Windows\\"), DriveQuery::Directory(w("C:\\Windows\\")));
        assert_eq!(q("C:\\nonexistent-dir-xyz\\"), DriveQuery::Directory(w("C:\\nonexistent-dir-xyz\\")));
        assert_eq!(q("C:\\Windows\\..\\Windows\\"), DriveQuery::Directory(w("C:\\Windows\\")));
        for s in
            ["C:\\Windows", "C:\\Windows\\System32\\kernel32.dll", "\\Windows", "Windows", " ", "CONOUT$", "NUL", ""]
        {
            assert_eq!(q(s), DriveQuery::NoRoot, "{s:?}");
        }
    }

    #[test]
    fn prefixes_unc_and_odd_letters() {
        for s in ["\\\\?\\C:", "\\\\.\\C:", "\\\\?\\GLOBALROOT\\", "\\\\no-such-host-xyz\\share\\", "1:\\", "[:\\"] {
            assert_eq!(q(s), DriveQuery::NoRoot, "{s}");
        }
        let long: alloc::string::String = core::iter::repeat_n('a', 299).collect();
        assert_eq!(q(&long), DriveQuery::NoRoot);
        let long = alloc::format!("C:\\{}", core::iter::repeat_n('a', 4996).collect::<alloc::string::String>());
        assert_eq!(q(&long), DriveQuery::NoRoot);
    }

    #[test]
    fn a_relative_name_at_the_root_is_the_root() {
        assert_eq!(drive_query(Some(&w(".")), &w("C:\\")), DriveQuery::Root(b'C'));
        assert_eq!(drive_query(Some(&w("sub\\")), &w("C:\\")), DriveQuery::Directory(w("C:\\sub\\")));
    }
}
