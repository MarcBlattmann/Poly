//! The interface between the VFS and a file system.

use crate::{Attr, DeviceId, Errno, FileType, Ino, Result, SetAttr, Timespec};
use alloc::vec::Vec;

/// A file system: a tree of inodes, each named by an [`Ino`].
///
/// The VFS does path resolution, mount crossing, symbolic links, permission
/// checks, read-only mounts and descriptor bookkeeping; a file system only
/// sees single-directory operations on inode numbers and answers them
/// atomically. It does its own locking (all methods take `&self`) and must
/// not call back into the VFS.
///
/// # Contract
///
/// - Names passed in are never empty, `.` or `..` (except to [`lookup`],
///   see there), contain no `/` and are at most [`NAME_MAX`](crate::NAME_MAX)
///   bytes.
/// - Errors are the ones POSIX documents for the matching system call:
///   `ENOENT` (no such name, or the directory was removed), `EEXIST`,
///   `ENOTDIR`, `EISDIR`, `ENOTEMPTY`, `ENOSPC`, `EINVAL`, `EPERM`.
/// - Inodes stay alive while their link count is non-zero **or** they are
///   pinned by [`open`]; a removed directory or an unlinked file that is
///   still pinned keeps answering [`getattr`] (link count 0), reads and
///   writes. `lookup` and creation in a removed directory fail with `ENOENT`.
/// - [`rename`] replaces an existing target atomically and refuses (with
///   `EINVAL`) to move a directory below itself.
/// - Inode numbers are not reused while pinned.
///
/// [`lookup`]: FileSystem::lookup
/// [`open`]: FileSystem::open
/// [`getattr`]: FileSystem::getattr
/// [`rename`]: FileSystem::rename
pub trait FileSystem: Send + Sync {
    /// Static facts about this file system.
    fn info(&self) -> FsInfo;

    /// The root directory.
    fn root(&self) -> Ino;

    /// The inode `name` names in directory `dir`. `..` is the directory's
    /// parent (the root's parent is the root itself); `.` is never asked.
    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino>;

    fn getattr(&self, ino: Ino) -> Result<Attr>;

    /// Changes attributes; returns the new ones. A size change is a
    /// truncate (regular files only: `EISDIR` / `EINVAL` otherwise).
    fn setattr(&self, ino: Ino, set: &SetAttr) -> Result<Attr>;

    /// Creates a non-directory, non-symlink node (`EINVAL` for those two
    /// kinds) named `name` in `dir`.
    fn create(&self, dir: Ino, name: &[u8], node: &NewNode) -> Result<Ino>;

    fn mkdir(&self, dir: Ino, name: &[u8], perm: u16, uid: u32, gid: u32) -> Result<Ino>;

    /// A symbolic link to `target` (any bytes but empty), mode `0777`.
    fn symlink(&self, dir: Ino, name: &[u8], target: &[u8], uid: u32, gid: u32) -> Result<Ino>;

    /// Another name for `ino`, which must not be a directory (`EPERM`).
    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()>;

    /// Removes a non-directory name (`EISDIR` for a directory).
    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()>;

    /// Removes an empty directory (`ENOTDIR`, `ENOTEMPTY`).
    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()>;

    /// Moves `old_dir/old_name` to `new_dir/new_name` with `rename(2)` /
    /// `renameat2(2)` semantics.
    fn rename(&self, old_dir: Ino, old_name: &[u8], new_dir: Ino, new_name: &[u8], flags: RenameFlags) -> Result<()>;

    /// A symbolic link's target (`EINVAL` for anything else).
    fn readlink(&self, ino: Ino) -> Result<Vec<u8>>;

    /// Reads from a regular file at `offset`; 0 at or after the end.
    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize>;

    /// Writes to a regular file at `offset`, extending it as needed; bytes
    /// between the old end and `offset` read as zeros. `EFBIG` past
    /// [`MAX_FILE_SIZE`](crate::MAX_FILE_SIZE).
    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize>;

    /// Writes at the end of the file, atomically with respect to other
    /// appends; returns the offset written at and the byte count. The
    /// default is not atomic; file systems that can should override it.
    fn append(&self, ino: Ino, buf: &[u8]) -> Result<(u64, usize)> {
        let at = self.getattr(ino)?.size;
        Ok((at, self.write(ino, at, buf)?))
    }

    /// Sets a regular file's size.
    fn truncate(&self, ino: Ino, size: u64) -> Result<()> {
        self.setattr(ino, &SetAttr { size: Some(size), ..SetAttr::default() }).map(|_| ())
    }

    /// Lists directory `dir` from position `pos` (0 is the start; every
    /// entry says where the next one is). `emit` returns `false` when it
    /// has no room for an entry; that entry is not consumed and listing
    /// stops. `.` and `..` come first. `emit` may sleep or fault, so it must
    /// be called without holding locks. Positions stay valid while entries
    /// are added and removed; an entry added or renamed meanwhile may or
    /// may not be listed (as POSIX allows).
    fn readdir(&self, dir: Ino, pos: u64, emit: &mut dyn FnMut(&DirEntry<'_>) -> bool) -> Result<()>;

    /// Pins `ino` so it outlives its last name (`ENOENT` if it is gone).
    fn open(&self, ino: Ino) -> Result<()> {
        self.getattr(ino).map(|_| ())
    }

    /// Drops a pin taken by [`FileSystem::open`].
    fn release(&self, _ino: Ino) {}

    /// Makes everything written so far durable.
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    /// Space and inode counts for `statfs(2)`. The default reports a file
    /// system without blocks or inode limits; the VFS fills in the magic
    /// number from the name when this leaves it 0 (see [`FsStats`]).
    fn statfs(&self) -> Result<FsStats> {
        Ok(FsStats { block_size: 4096, fragment_size: 4096, name_max: crate::NAME_MAX as u64, ..FsStats::default() })
    }
}

/// What `statfs(2)` reports about a file system (`struct statfs`, Linux
/// UAPI `asm-generic/statfs.h`). Sizes count blocks of `block_size` bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FsStats {
    /// The file system type (`f_type`): the magic numbers of Linux UAPI
    /// `linux/magic.h`. 0 lets the VFS pick it from [`FsInfo::name`]
    /// ([`magic_for`]).
    pub magic: u64,
    /// `f_bsize`: the optimal transfer block size.
    pub block_size: u64,
    /// `f_blocks`, `f_bfree`, `f_bavail` (free for unprivileged users).
    pub blocks: u64,
    pub blocks_free: u64,
    pub blocks_avail: u64,
    /// `f_files`, `f_ffree`: inodes in all and free.
    pub files: u64,
    pub files_free: u64,
    /// `f_namelen`: the longest file name.
    pub name_max: u64,
    /// `f_frsize`: the fragment size.
    pub fragment_size: u64,
    /// `f_flags`: the mount's `ST_*` flags (filled in by the VFS from the
    /// mount; see [`ST_RDONLY`] and friends).
    pub flags: u64,
    /// `f_fsid`, filled in by the VFS: the file system's device number.
    pub fsid: [u32; 2],
}

/// `f_flags` bits, equal to the `MS_*` mount flags of Linux UAPI
/// `linux/mount.h` (statfs(2), statvfs(3)).
pub const ST_RDONLY: u64 = 1;
pub const ST_NOSUID: u64 = 2;
pub const ST_NODEV: u64 = 4;
pub const ST_NOEXEC: u64 = 8;

/// Magic numbers of the file systems Poly has (Linux UAPI `linux/magic.h`).
pub const TMPFS_MAGIC: u64 = 0x0102_1994;
pub const EXT2_SUPER_MAGIC: u64 = 0xef53;
pub const PROC_SUPER_MAGIC: u64 = 0x9fa0;
pub const SYSFS_MAGIC: u64 = 0x6265_6572;

/// The `f_type` of a file system by its name ([`FsInfo::name`]); 0 for
/// names without one here.
pub fn magic_for(name: &str) -> u64 {
    match name {
        "tmpfs" => TMPFS_MAGIC,
        "ext2" => EXT2_SUPER_MAGIC,
        "proc" => PROC_SUPER_MAGIC,
        "sysfs" => SYSFS_MAGIC,
        _ => 0,
    }
}

/// Static facts about a file system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FsInfo {
    /// Its type, as `/proc/mounts` would show it (`"tmpfs"`).
    pub name: &'static str,
    /// Refuses all changes; the VFS reports `EROFS` before asking.
    pub read_only: bool,
    /// Whether the VFS may remember name lookups (the dentry cache). Only
    /// for file systems whose directories change solely through the VFS;
    /// a procfs whose entries come and go on their own says no, and so does
    /// tmpfs, where a lookup is as cheap as the cache.
    pub cache_lookups: bool,
}

/// A node to create with [`FileSystem::create`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NewNode {
    pub kind: FileType,
    pub perm: u16,
    pub uid: u32,
    pub gid: u32,
    /// For device nodes.
    pub rdev: DeviceId,
}

/// One directory entry, as [`FileSystem::readdir`] lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DirEntry<'a> {
    pub ino: Ino,
    pub kind: FileType,
    pub name: &'a [u8],
    /// Where the entry after this one is (`d_off`).
    pub next: u64,
}

/// `renameat2(2)` flags (values from Linux UAPI `linux/fs.h`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenameFlags {
    /// `RENAME_NOREPLACE`: fail with `EEXIST` instead of replacing.
    pub noreplace: bool,
    /// `RENAME_EXCHANGE`: swap the two names; both must exist.
    pub exchange: bool,
}

impl RenameFlags {
    pub const LINUX_NOREPLACE: u32 = 1 << 0;
    pub const LINUX_EXCHANGE: u32 = 1 << 1;
    pub const LINUX_WHITEOUT: u32 = 1 << 2;

    /// From `renameat2`'s flags argument. `RENAME_WHITEOUT` (overlay file
    /// systems only) and unknown bits are `EINVAL`, and so is asking for
    /// both of the others.
    pub fn from_linux(bits: u32) -> Result<RenameFlags> {
        if bits & !(Self::LINUX_NOREPLACE | Self::LINUX_EXCHANGE) != 0 {
            return Err(Errno::EINVAL);
        }
        let f =
            RenameFlags { noreplace: bits & Self::LINUX_NOREPLACE != 0, exchange: bits & Self::LINUX_EXCHANGE != 0 };
        if f.noreplace && f.exchange { Err(Errno::EINVAL) } else { Ok(f) }
    }
}

/// Where file systems get the time for timestamps.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timespec;
}
