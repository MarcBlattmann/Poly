//! cpio "newc" archives (the initramfs format Linux documents in
//! `Documentation/driver-api/early-userspace/buffer-format.rst`): a 110-byte
//! ASCII header per file (magic `070701`, thirteen 8-digit hex fields), the
//! NUL-terminated name, the data, each padded to 4 bytes; a `TRAILER!!!`
//! entry ends the archive.
//!
//! Archives are reproducible: timestamps are the entries' own (0 unless
//! set), owners are root, inode numbers count up from 1 in the order given.

use crate::util::Result;

const MAGIC: &str = "070701";
const TRAILER: &str = "TRAILER!!!";
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFLNK: u32 = 0o120000;
const S_IFMT: u32 = 0o170000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Path inside the archive, without a leading `/` (`test-binary`, `dev`).
    pub name: String,
    /// File type and permission bits (`S_IFREG | 0o755`).
    pub mode: u32,
    /// File contents, or a symbolic link's target.
    pub data: Vec<u8>,
    /// Modification time, seconds since 1970.
    pub mtime: u32,
}

impl Entry {
    pub fn file(name: &str, mode: u32, data: Vec<u8>) -> Entry {
        Entry { name: name.into(), mode: S_IFREG | mode, data, mtime: 0 }
    }

    pub fn dir(name: &str) -> Entry {
        Entry::dir_mode(name, 0o755)
    }

    pub fn dir_mode(name: &str, mode: u32) -> Entry {
        Entry { name: name.into(), mode: S_IFDIR | mode, data: Vec::new(), mtime: 0 }
    }

    pub fn symlink(name: &str, target: &str) -> Entry {
        Entry { name: name.into(), mode: S_IFLNK | 0o777, data: target.as_bytes().to_vec(), mtime: 0 }
    }

    /// The same entry with modification time `mtime`.
    pub fn at(mut self, mtime: u32) -> Entry {
        self.mtime = mtime;
        self
    }
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

fn header(out: &mut Vec<u8>, ino: u32, mode: u32, nlink: u32, mtime: u32, size: usize, name: &str) {
    // ino mode uid gid nlink mtime filesize devmajor devminor rdevmajor rdevminor namesize check
    let fields = [ino, mode, 0, 0, nlink, mtime, size as u32, 0, 0, 0, 0, name.len() as u32 + 1, 0];
    out.extend_from_slice(MAGIC.as_bytes());
    for f in fields {
        out.extend_from_slice(format!("{f:08x}").as_bytes());
    }
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    pad4(out);
}

/// The archive holding `entries`, in order. Directories must come before
/// what they contain.
pub fn write(entries: &[Entry]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        if e.name.is_empty() || e.name.starts_with('/') || e.name == TRAILER {
            return Err(format!("bad cpio entry name {:?}", e.name));
        }
        if u32::try_from(e.data.len()).is_err() {
            return Err(format!("{}: too large for cpio newc", e.name));
        }
        let nlink = if e.mode & S_IFMT == S_IFDIR { 2 } else { 1 };
        header(&mut out, i as u32 + 1, e.mode, nlink, e.mtime, e.data.len(), &e.name);
        out.extend_from_slice(&e.data);
        pad4(&mut out);
    }
    header(&mut out, 0, 0, 1, 0, 0, TRAILER);
    Ok(out)
}

/// Read an archive back (for tests and `pipeline initramfs --list`).
pub fn read(mut bytes: &[u8]) -> Result<Vec<Entry>> {
    let total = bytes.len();
    let mut out = Vec::new();
    loop {
        let offset = total - bytes.len();
        if bytes.len() < 110 || &bytes[..6] != MAGIC.as_bytes() {
            return Err(format!("no newc header at offset {offset}"));
        }
        let field = |i: usize| -> Result<u32> {
            let s = std::str::from_utf8(&bytes[6 + i * 8..14 + i * 8]).map_err(|e| e.to_string())?;
            u32::from_str_radix(s, 16).map_err(|e| format!("offset {offset}: {e}"))
        };
        let (mode, mtime, size, namesize) = (field(1)?, field(5)?, field(6)? as usize, field(11)? as usize);
        let name_end = 110 + namesize;
        let data_start = name_end.next_multiple_of(4);
        if namesize == 0 || bytes.len() < data_start + size {
            return Err(format!("truncated entry at offset {offset}"));
        }
        let name = String::from_utf8_lossy(&bytes[110..name_end - 1]).into_owned();
        if name == TRAILER {
            return Ok(out);
        }
        out.push(Entry { name, mode, data: bytes[data_start..data_start + size].to_vec(), mtime });
        bytes = &bytes[(data_start + size).next_multiple_of(4).min(bytes.len())..];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::process::{Command, Stdio};

    fn sample() -> Vec<Entry> {
        vec![
            Entry::dir("dev"),
            Entry::file("test-binary", 0o755, b"\x7fELF hello".to_vec()),
            Entry::file("a", 0o644, vec![]),
            Entry::symlink("l", "/a").at(1_700_000_000),
        ]
    }

    #[test]
    fn layout_is_newc() {
        let a = write(&sample()).unwrap();
        assert!(a.starts_with(b"07070100000001000041ed"));
        assert_eq!(a.len() % 4, 0);
        let trailer = a.windows(10).position(|w| w == b"TRAILER!!!").unwrap();
        // 10 name bytes, the NUL and 3 bytes of padding end the archive.
        assert_eq!(trailer, a.len() - 14);
        // Reproducible: same input, same bytes.
        assert_eq!(write(&sample()).unwrap(), a);
    }

    #[test]
    fn roundtrip() {
        assert_eq!(read(&write(&sample()).unwrap()).unwrap(), sample());
        assert!(read(b"junk").is_err());
        assert!(write(&[Entry::dir("/abs")]).is_err());
    }

    /// GNU cpio, where installed, must read what we write.
    #[test]
    fn host_cpio_can_list_and_extract() {
        let Some(cpio) = crate::util::which("cpio") else {
            eprintln!("skipped: cpio not installed");
            return;
        };
        let dir = std::env::temp_dir().join(format!("poly-cpio-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("initramfs.cpio");
        std::fs::write(&archive, write(&sample()).unwrap()).unwrap();
        let out = Command::new(&cpio)
            .args(["-i", "-d", "--quiet", "-F"])
            .arg(&archive)
            .current_dir(&dir)
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(out.status.success(), "cpio failed: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(std::fs::read(dir.join("test-binary")).unwrap(), b"\x7fELF hello");
        assert!(dir.join("dev").is_dir());
        let list = Command::new(&cpio).args(["-t", "--quiet", "-F"]).arg(&archive).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&list.stdout), "dev\ntest-binary\na\nl\n");
        assert_eq!(std::fs::read_link(dir.join("l")).unwrap(), Path::new("/a"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
