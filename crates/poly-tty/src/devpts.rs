//! devpts: the file system mounted on `/dev/pts`, one character device
//! node per open pty master, named after its number (pts(4)). Nodes come
//! and go with the masters; nothing can be created or removed by name.
//! `chmod` and `chown` of a node change the slave's owner and mode (what
//! grantpt(3) may do).

use crate::pty::{Owner, Ptys};
use alloc::sync::Arc;
use alloc::vec::Vec;
use poly_vfs::{
    Attr, DeviceId, DirEntry, Errno, FileSystem, FileType, FsInfo, Ino, NewNode, RenameFlags, Result, SetAttr, Timespec,
};

const ROOT: Ino = 1;
/// The inode of pty `n` is `n + FIRST`.
const FIRST: Ino = 2;

/// devpts over the pseudo-terminals of `ptys`.
pub struct DevPts {
    ptys: Arc<Ptys>,
}

impl core::fmt::Debug for DevPts {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DevPts").finish_non_exhaustive()
    }
}

impl DevPts {
    pub fn new(ptys: Arc<Ptys>) -> DevPts {
        DevPts { ptys }
    }

    fn index(ino: Ino) -> Option<u32> {
        ino.checked_sub(FIRST).and_then(|i| u32::try_from(i).ok())
    }
}

fn attr(ino: Ino, kind: FileType, perm: u16, uid: u32, gid: u32, rdev: DeviceId) -> Attr {
    Attr {
        ino,
        kind,
        perm,
        nlink: if kind == FileType::Directory { 2 } else { 1 },
        uid,
        gid,
        size: 0,
        rdev,
        atime: Timespec::default(),
        mtime: Timespec::default(),
        ctime: Timespec::default(),
        blocks: 0,
        blksize: 1024,
    }
}

/// Pty number `name` (decimal, no leading zeros).
fn parse(name: &[u8]) -> Option<u32> {
    if name.is_empty() || (name.len() > 1 && name[0] == b'0') || !name.iter().all(u8::is_ascii_digit) {
        return None;
    }
    core::str::from_utf8(name).ok()?.parse().ok()
}

impl FileSystem for DevPts {
    fn info(&self) -> FsInfo {
        FsInfo { name: "devpts", read_only: false, cache_lookups: false }
    }

    fn root(&self) -> Ino {
        ROOT
    }

    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        if name == b".." {
            return Ok(ROOT);
        }
        let n = parse(name).ok_or(Errno::ENOENT)?;
        self.ptys.get(n).map(|_| Ino::from(n) + FIRST).ok_or(Errno::ENOENT)
    }

    fn getattr(&self, ino: Ino) -> Result<Attr> {
        if ino == ROOT {
            return Ok(attr(ROOT, FileType::Directory, 0o755, 0, 0, DeviceId::default()));
        }
        let pair = Self::index(ino).and_then(|n| self.ptys.get(n)).ok_or(Errno::ENOENT)?;
        let o = pair.owner();
        Ok(attr(ino, FileType::CharDevice, o.perm, o.uid, o.gid, pair.device()))
    }

    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr> {
        if set.size.is_some() {
            return Err(Errno::EINVAL);
        }
        if ino == ROOT {
            return Err(Errno::EPERM);
        }
        let pair = Self::index(ino).and_then(|n| self.ptys.get(n)).ok_or(Errno::ENOENT)?;
        let o = pair.owner();
        pair.set_owner(Owner {
            uid: set.uid.unwrap_or(o.uid),
            gid: set.gid.unwrap_or(o.gid),
            perm: set.perm.unwrap_or(o.perm),
        });
        self.getattr(ino)
    }

    fn create(&self, _: Ino, _: &[u8], _: &NewNode) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn mkdir(&self, _: Ino, _: &[u8], _: u16, _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn symlink(&self, _: Ino, _: &[u8], _: &[u8], _: u32, _: u32) -> Result<Ino> {
        Err(Errno::EPERM)
    }

    fn link(&self, _: Ino, _: &[u8], _: Ino) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn unlink(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn rmdir(&self, _: Ino, _: &[u8]) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn rename(&self, _: Ino, _: &[u8], _: Ino, _: &[u8], _: RenameFlags) -> Result<()> {
        Err(Errno::EPERM)
    }

    fn readlink(&self, _: Ino) -> Result<Vec<u8>> {
        Err(Errno::EINVAL)
    }

    fn read(&self, _: Ino, _: u64, _: &mut [u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    fn write(&self, _: Ino, _: u64, _: &[u8]) -> Result<usize> {
        Err(Errno::EINVAL)
    }

    /// `.`, `..`, then the ptys in number order; the position of pty `n`'s
    /// entry is `n + 2`, so entries that come and go leave the others'
    /// positions alone.
    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()> {
        if dir != ROOT {
            return Err(Errno::ENOTDIR);
        }
        let dots: [&[u8]; 2] = [b".", b".."];
        for (i, name) in dots.iter().enumerate().skip(pos as usize) {
            if !emit(&DirEntry { ino: ROOT, kind: FileType::Directory, name, next: i as u64 + 1 }) {
                return Ok(());
            }
        }
        for n in self.ptys.indices() {
            let ino = Ino::from(n) + FIRST;
            if ino < pos {
                continue;
            }
            let name = alloc::format!("{n}");
            if !emit(&DirEntry { ino, kind: FileType::CharDevice, name: name.as_bytes(), next: ino + 1 }) {
                break;
            }
        }
        Ok(())
    }
}
