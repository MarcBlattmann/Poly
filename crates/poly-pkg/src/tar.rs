//! tar archives as far as a `.deb`'s `control.tar` needs them (POSIX
//! ustar; pax(1), "ustar Interchange Format"): 512-byte headers with the
//! name at 0, the size in octal at 124, the type at 156 and the name's
//! prefix at 345, each followed by the data padded to 512 bytes; two zero
//! blocks end the archive. Only regular files are returned.

use crate::Error;
use alloc::string::String;
use alloc::vec::Vec;

const BLOCK: usize = 512;

fn octal(field: &[u8]) -> Option<u64> {
    let text = core::str::from_utf8(field).ok()?;
    let digits = text.trim_matches(|c: char| c == '\0' || c == ' ');
    if digits.is_empty() { Some(0) } else { u64::from_str_radix(digits, 8).ok() }
}

fn text(field: &[u8]) -> String {
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// The regular files of the archive `data`: path (without a leading `./`
/// or `/`) and contents.
pub fn files(data: &[u8]) -> Result<Vec<(String, &[u8])>, Error> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(h) = data.get(at..at + BLOCK) {
        if h.iter().all(|&b| b == 0) {
            break;
        }
        let size = octal(&h[124..136]).and_then(|s| usize::try_from(s).ok()).ok_or(Error::Corrupt)?;
        let body = data.get(at + BLOCK..at + BLOCK + size).ok_or(Error::Truncated)?;
        at += BLOCK + size.div_ceil(BLOCK) * BLOCK;
        if !matches!(h[156], b'0' | 0 | b'7') {
            continue;
        }
        let mut name = text(&h[..100]);
        let prefix = text(&h[345..500]);
        if &h[257..262] == b"ustar" && !prefix.is_empty() {
            name = alloc::format!("{prefix}/{name}");
        }
        let name = name.trim_start_matches("./").trim_start_matches('/');
        if !name.is_empty() {
            out.push((name.into(), body));
        }
    }
    Ok(out)
}

/// A ustar archive of `files` (path, contents) below `./`, with the
/// directory entry dpkg-deb writes first: for tests and fixtures.
pub fn build(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn entry(out: &mut Vec<u8>, name: &str, kind: u8, body: &[u8]) {
        let mut h = [0u8; BLOCK];
        h[..name.len()].copy_from_slice(name.as_bytes());
        h[100..108].copy_from_slice(b"0000644\0");
        h[108..116].copy_from_slice(b"0000000\0");
        h[116..124].copy_from_slice(b"0000000\0");
        h[124..136].copy_from_slice(alloc::format!("{:011o}\0", body.len()).as_bytes());
        h[136..148].copy_from_slice(b"00000000000\0");
        h[148..156].copy_from_slice(b"        ");
        h[156] = kind;
        h[257..263].copy_from_slice(b"ustar\0");
        h[263..265].copy_from_slice(b"00");
        let sum: u32 = h.iter().map(|&b| u32::from(b)).sum();
        h[148..156].copy_from_slice(alloc::format!("{sum:06o}\0 ").as_bytes());
        out.extend_from_slice(&h);
        out.extend_from_slice(body);
        out.resize(out.len().next_multiple_of(BLOCK), 0);
    }
    let mut out = Vec::new();
    entry(&mut out, "./", b'5', b"");
    for (name, body) in files {
        entry(&mut out, &alloc::format!("./{name}"), b'0', body);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_files_are_read_and_directories_skipped() {
        let big = [b'x'; 700];
        let t = build(&[("control", b"Package: a\n"), ("md5sums", &big), ("empty", b"")]);
        let fs = files(&t).unwrap();
        let names: Vec<&str> = fs.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["control", "md5sums", "empty"]);
        assert_eq!(fs[0].1, b"Package: a\n");
        assert_eq!(fs[1].1.len(), 700);
        assert!(fs[2].1.is_empty());
    }

    #[test]
    fn a_cut_archive_is_truncated_and_a_bad_size_corrupt() {
        let t = build(&[("control", &[b'c'; 900])]);
        assert_eq!(files(&t[..512 + 512 + 600]), Err(Error::Truncated));
        let mut bad = t.clone();
        bad[512 + 124] = b'9';
        assert_eq!(files(&bad), Err(Error::Corrupt));
        // Nothing at all is an empty archive.
        assert_eq!(files(&[]).unwrap().len(), 0);
    }

    #[test]
    fn a_ustar_prefix_is_part_of_the_name() {
        let mut t = build(&[("name", b"1")]);
        t[512 + 345..512 + 348].copy_from_slice(b"dir");
        let fs = files(&t).unwrap();
        assert_eq!(fs[0].0, "dir/./name");
    }
}
