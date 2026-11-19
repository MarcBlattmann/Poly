//! The names `GetVolumeInformationW` and `GetVolumePathNameW` take (sheets
//! `kernel32.dll/GetVolumeInformationW` and `kernel32.dll/GetVolumePathNameW`;
//! Learn's pages of both): which volume a name means, before the volume is
//! asked anything. The drive itself is checked by the caller.

use crate::error::{ERROR_BAD_NETPATH, ERROR_DIR_NOT_ROOT, ERROR_INVALID_NAME, ERROR_PATH_NOT_FOUND};
use alloc::vec::Vec;

const BS: u16 = b'\\' as u16;
const SLASH: u16 = b'/' as u16;
const COLON: u16 = b':' as u16;

/// Which root a name means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Root {
    /// The root of the current directory's drive.
    Current,
    /// The root of drive `letter`, written `prefix` + `X:` + `\` (the prefix
    /// `\\?\` or `\\.\` kept as given, or none; the letter in its case).
    Drive { prefix: Vec<u16>, letter: u16 },
    /// A device's own root (`NUL` gives `\\.\NUL\`).
    Device(Vec<u16>),
}

impl Root {
    /// The root as `GetVolumePathNameW` writes it (with its backslash), the
    /// current drive's letter given.
    pub fn text(&self, current: u16) -> Vec<u16> {
        match self {
            Root::Current => alloc::vec![current, COLON, BS],
            Root::Drive { prefix, letter } => {
                let mut v = prefix.clone();
                v.extend_from_slice(&[*letter, COLON, BS]);
                v
            }
            Root::Device(name) => {
                let mut v: Vec<u16> = "\\\\.\\".encode_utf16().collect();
                v.extend_from_slice(name);
                v.push(BS);
                v
            }
        }
    }

    /// The drive letter, if the root is a drive's (`current` for
    /// [`Root::Current`]).
    pub fn letter(&self, current: u16) -> Option<u16> {
        match self {
            Root::Current => Some(current),
            Root::Drive { letter, .. } => Some(*letter),
            Root::Device(_) => None,
        }
    }
}

fn is_letter(c: u16) -> bool {
    (u16::from(b'A')..=u16::from(b'Z')).contains(&c) || (u16::from(b'a')..=u16::from(b'z')).contains(&c)
}

fn sep(c: u16) -> bool {
    c == BS || c == SLASH
}

/// `\\?\` or `\\.\` at the start of `name`, if there.
fn device_prefix(name: &[u16]) -> Option<usize> {
    (name.len() >= 4
        && sep(name[0])
        && sep(name[1])
        && (name[2] == u16::from(b'?') || name[2] == u16::from(b'.'))
        && sep(name[3]))
    .then_some(4)
}

/// The name of `GetVolumeInformationW`'s `lpRootPathName` (`None` for
/// NULL): a drive's root with its backslash (any case, also after `\\?\` or
/// `\\.\`), NULL and `\` (the current drive). Failures: an empty name or a
/// drive that is not there `ERROR_PATH_NOT_FOUND` (the drive is the caller's
/// check), a drive without its backslash, a relative name or a file name
/// `ERROR_INVALID_NAME`, a folder (a name ending in a backslash below the
/// root) `ERROR_DIR_NOT_ROOT`, a UNC name `ERROR_BAD_NETPATH` (Poly has no
/// network shares).
pub fn information_root(name: Option<&[u16]>) -> Result<Root, u32> {
    let Some(name) = name else { return Ok(Root::Current) };
    if name.is_empty() {
        return Err(ERROR_PATH_NOT_FOUND);
    }
    if name == [BS] {
        return Ok(Root::Current);
    }
    let (prefix, rest) = match device_prefix(name) {
        Some(n) => (name[..n].to_vec(), &name[n..]),
        None if name.len() >= 2 && sep(name[0]) && sep(name[1]) => return Err(ERROR_BAD_NETPATH),
        None => (Vec::new(), name),
    };
    if rest.len() >= 4 && rest[..4].iter().map(|&c| c | 0x20).eq("unc\\".encode_utf16().map(|c| c | 0x20)) {
        return Err(ERROR_BAD_NETPATH);
    }
    if rest.len() < 2 || !is_letter(rest[0]) || rest[1] != COLON {
        return Err(ERROR_INVALID_NAME);
    }
    match &rest[2..] {
        [] => Err(ERROR_INVALID_NAME),
        [c] if sep(*c) => Ok(Root::Drive { prefix, letter: rest[0] }),
        more if sep(more[0]) && more.last().is_some_and(|&c| sep(c)) => Err(ERROR_DIR_NOT_ROOT),
        _ => Err(ERROR_INVALID_NAME),
    }
}

/// The root of the volume that holds `name` (`GetVolumePathNameW`): the
/// name need not exist; a relative name or one rooted without a drive is on
/// the current drive; the `\\?\` and `\\.\` prefixes are kept; a DOS device
/// name (`NUL`) is its own root. An empty name, a UNC name and a
/// `\\?\Volume{...}` name are `ERROR_INVALID_NAME` (Poly has no network
/// shares and no volume names).
pub fn path_root(name: &[u16]) -> Result<Root, u32> {
    if name.is_empty() {
        return Err(ERROR_INVALID_NAME);
    }
    let (prefix, rest) = match device_prefix(name) {
        Some(n) => (name[..n].to_vec(), &name[n..]),
        None if name.len() >= 2 && sep(name[0]) && sep(name[1]) => return Err(ERROR_INVALID_NAME),
        None => (Vec::new(), name),
    };
    if rest.len() >= 2 && is_letter(rest[0]) && rest[1] == COLON {
        return Ok(Root::Drive { prefix, letter: rest[0] });
    }
    if !prefix.is_empty() {
        // `\\?\Volume{...}`, `\\?\UNC\...` and other device names.
        return Err(ERROR_INVALID_NAME);
    }
    let upper: Vec<u16> = rest.iter().map(|&c| if (0x61..=0x7a).contains(&c) { c - 0x20 } else { c }).collect();
    for device in ["NUL", "CON", "PRN", "AUX"] {
        if upper.iter().copied().eq(device.encode_utf16()) {
            return Ok(Root::Device(device.encode_utf16().collect()));
        }
    }
    Ok(Root::Current)
}

/// What `GetVolumePathNameW` writes into a buffer of `capacity` units for
/// `root` (the text with its backslash): `Ok(units with the NUL)` or
/// `Err(error)`. A buffer one unit short takes the root without its last
/// backslash (`C:` for `C:\`); a smaller one is
/// `ERROR_FILENAME_EXCED_RANGE`, and none at all `ERROR_INVALID_PARAMETER`.
pub fn path_into(root: &[u16], capacity: usize) -> Result<Vec<u16>, u32> {
    if capacity == 0 {
        return Err(crate::error::ERROR_INVALID_PARAMETER);
    }
    let mut out = if capacity > root.len() {
        root.to_vec()
    } else if capacity == root.len() && root.last() == Some(&BS) {
        root[..root.len() - 1].to_vec()
    } else {
        return Err(crate::error::ERROR_FILENAME_EXCED_RANGE);
    };
    out.push(0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    fn drive(prefix: &str, letter: char) -> Root {
        Root::Drive { prefix: w(prefix), letter: letter as u16 }
    }

    #[test]
    fn information_roots() {
        assert_eq!(information_root(Some(&w("C:\\"))), Ok(drive("", 'C')));
        assert_eq!(information_root(Some(&w("c:\\"))), Ok(drive("", 'c')));
        assert_eq!(information_root(Some(&w("\\\\?\\C:\\"))), Ok(drive("\\\\?\\", 'C')));
        assert_eq!(information_root(Some(&w("\\\\.\\C:\\"))), Ok(drive("\\\\.\\", 'C')));
        assert_eq!(information_root(None), Ok(Root::Current));
        assert_eq!(information_root(Some(&w("\\"))), Ok(Root::Current));
        assert_eq!(information_root(Some(&w("C:"))), Err(ERROR_INVALID_NAME));
        assert_eq!(information_root(Some(&w(""))), Err(ERROR_PATH_NOT_FOUND));
        assert_eq!(information_root(Some(&w("C:\\Windows\\"))), Err(ERROR_DIR_NOT_ROOT));
        assert_eq!(information_root(Some(&w("x"))), Err(ERROR_INVALID_NAME));
        assert_eq!(information_root(Some(&w("C:\\x.txt"))), Err(ERROR_INVALID_NAME));
        assert_eq!(information_root(Some(&w("\\\\polyserver\\share\\"))), Err(ERROR_BAD_NETPATH));
        assert_eq!(information_root(Some(&w("Z:\\"))), Ok(drive("", 'Z')));
    }

    #[test]
    fn path_roots() {
        let text = |s: &str| path_root(&w(s)).map(|r| alloc::string::String::from_utf16(&r.text(b'C' as u16)).unwrap());
        assert_eq!(text("C:\\Windows\\System32\\notepad.exe").as_deref(), Ok("C:\\"));
        assert_eq!(text("C:").as_deref(), Ok("C:\\"));
        assert_eq!(text("C:/Windows/System32").as_deref(), Ok("C:\\"));
        assert_eq!(text("c:\\windows").as_deref(), Ok("c:\\"));
        assert_eq!(text("C:file").as_deref(), Ok("C:\\"));
        assert_eq!(text("\\\\?\\C:\\").as_deref(), Ok("\\\\?\\C:\\"));
        assert_eq!(text("\\\\?\\C:\\Windows").as_deref(), Ok("\\\\?\\C:\\"));
        assert_eq!(text("\\\\.\\C:\\Windows").as_deref(), Ok("\\\\.\\C:\\"));
        assert_eq!(text(".").as_deref(), Ok("C:\\"));
        assert_eq!(text("relative\\path").as_deref(), Ok("C:\\"));
        assert_eq!(text("\\").as_deref(), Ok("C:\\"));
        assert_eq!(text("\\Windows").as_deref(), Ok("C:\\"));
        assert_eq!(text("NUL").as_deref(), Ok("\\\\.\\NUL\\"));
        assert_eq!(text(""), Err(ERROR_INVALID_NAME));
        assert_eq!(text("\\\\server\\share\\dir\\file"), Err(ERROR_INVALID_NAME));
        assert_eq!(text("\\\\?\\UNC\\server\\share\\dir"), Err(ERROR_INVALID_NAME));
        assert_eq!(text("\\\\?\\Volume{00000000-0000-0000-0000-000000000000}\\x"), Err(ERROR_INVALID_NAME));
    }

    #[test]
    fn path_buffers() {
        let root = w("C:\\");
        assert_eq!(path_into(&root, 0), Err(ERROR_INVALID_PARAMETER));
        assert_eq!(path_into(&root, 1), Err(ERROR_FILENAME_EXCED_RANGE));
        assert_eq!(path_into(&root, 2), Err(ERROR_FILENAME_EXCED_RANGE));
        assert_eq!(path_into(&root, 3), Ok(alloc::vec![b'C' as u16, COLON, 0]));
        assert_eq!(path_into(&root, 4), Ok(alloc::vec![b'C' as u16, COLON, BS, 0]));
    }
}
