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
/// `FALLOC_FL_KEEP_SIZE` and `FALLOC_FL_PUNCH_HOLE` (Linux UAPI
/// `linux/falloc.h`): the fallocate(2) modes [`FileSystem::fallocate`]
/// speaks of.
pub const FALLOC_FL_KEEP_SIZE: u32 = 0x01;
pub const FALLOC_FL_PUNCH_HOLE: u32 = 0x02;

pub trait FileSystem: Send + Sync {
    /// Static facts about this file system.
    fn info(&self) -> FsInfo;

    /// The root directory.
    fn root(&self) -> Ino;

    /// The inode `name` names in directory `dir`. `..` is the directory's
    /// parent (the root's parent is the root itself); `.` is never asked.
    fn lookup(&self, dir: Ino, name: &[u8]) -> Result<Ino>;

    fn getattr(&self, ino: Ino) -> Result<Attr>;

    /// The attributes of `ino` for a path walk that goes through it: its
    /// type, and the owner and mode as far as the kernel checks search
    /// permission itself. The default is [`getattr`](Self::getattr); a file
    /// system that asks somebody else for attributes (FUSE) answers from
    /// what it knows, and with full permission where the checks are not the
    /// kernel's.
    fn walk_attr(&self, ino: Ino) -> Result<Attr> {
        self.getattr(ino)
    }

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
    /// An inode without names may get one only if it was made by
    /// [`create_tmpfile`](Self::create_tmpfile) (`ENOENT` otherwise).
    fn link(&self, dir: Ino, name: &[u8], ino: Ino) -> Result<()>;

    /// `O_TMPFILE` (open(2)): a regular file in directory `dir` without a
    /// name (`nlink` 0), freed when the last pin goes unless [`link`]
    /// gives it a name first. `EOPNOTSUPP` by default.
    ///
    /// [`link`]: Self::link
    fn create_tmpfile(&self, dir: Ino, node: &NewNode) -> Result<Ino> {
        let _ = (dir, node);
        Err(Errno::EOPNOTSUPP)
    }

    /// Removes a non-directory name (`EISDIR` for a directory).
    fn unlink(&self, dir: Ino, name: &[u8]) -> Result<()>;

    /// Removes an empty directory (`ENOTDIR`, `ENOTEMPTY`).
    fn rmdir(&self, dir: Ino, name: &[u8]) -> Result<()>;

    /// Moves `old_dir/old_name` to `new_dir/new_name` with `rename(2)` /
    /// `renameat2(2)` semantics.
    fn rename(&self, old_dir: Ino, old_name: &[u8], new_dir: Ino, new_name: &[u8], flags: RenameFlags) -> Result<()>;

    /// A symbolic link's target (`EINVAL` for anything else).
    fn readlink(&self, ino: Ino) -> Result<Vec<u8>>;

    /// Whether the symbolic link `ino` is a "magic" link: one that names an
    /// object the kernel knows rather than holding a path of its own
    /// (procfs's `/proc/<pid>/fd/<n>`, `exe`, `cwd`, `root`). openat2(2)'s
    /// `RESOLVE_NO_MAGICLINKS` refuses to follow them.
    fn is_magic_link(&self, ino: Ino) -> bool {
        let _ = ino;
        false
    }

    /// What the magic link `ino` names, for following it: the walk goes on
    /// at that file or directory itself, whatever the caller's root is
    /// (proc_pid_fd(5): the links "can be dereferenced ... even if the
    /// file has been deleted"; Chromium's sandboxed zygote, whose root is
    /// an empty directory, stats `/proc/self/fd/<n>`). `None` follows the
    /// link's text like any link's (files without a path: pipes, sockets).
    fn magic_target(&self, ino: Ino) -> Option<MagicTarget> {
        let _ = ino;
        None
    }

    /// Whether `ino` takes writes although the file system is read-only
    /// ([`FsInfo::read_only`]): a control file of a pseudo file system
    /// (procfs's `uid_map`). A read-only mount still refuses them.
    fn writable_file(&self, ino: Ino) -> bool {
        let _ = ino;
        false
    }

    /// The error of opening `ino` for writing, for a file that can never be
    /// written and says so with an error of its own (a namespace file of
    /// procfs: `EPERM`); it comes before the read-only file system's
    /// `EROFS`. `None` (the default): the usual checks.
    fn write_open_error(&self, ino: Ino) -> Option<Errno> {
        let _ = ino;
        None
    }

    /// Called when a regular file is opened for writing (or truncating),
    /// before the open file exists: a union file system copies the file to
    /// its writable layer here, so the copy exists even if nothing is
    /// written. An error fails the `open`. The default does nothing.
    fn prepare_write(&self, ino: Ino) -> Result<()> {
        let _ = ino;
        Ok(())
    }

    /// A remount changes the size limits (tmpfs(5) `size`, `nr_inodes`;
    /// 0: no limit). `EOPNOTSUPP` for a file system without such limits.
    fn set_limits(&self, bytes: Option<u64>, inodes: Option<u64>) -> Result<()> {
        let _ = (bytes, inodes);
        Err(Errno::EOPNOTSUPP)
    }

    /// Reads from a regular file at `offset`; 0 at or after the end.
    fn read(&self, ino: Ino, offset: u64, buf: &mut [u8]) -> Result<usize>;

    /// Writes to a regular file at `offset`, extending it as needed; bytes
    /// between the old end and `offset` read as zeros. `EFBIG` past
    /// [`MAX_FILE_SIZE`](crate::MAX_FILE_SIZE).
    fn write(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize>;

    /// Writes data a page cache holds back to the file: as
    /// [`write`](Self::write), but the file's times stay as they are. The
    /// cache set them when the program wrote; a later change of the times
    /// (utimensat(2), as `cp -p` does after copying) must survive the
    /// writeback. The default is `write`, for file systems without times
    /// of their own.
    fn write_back(&self, ino: Ino, offset: u64, buf: &[u8]) -> Result<usize> {
        self.write(ino, offset, buf)
    }

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

    /// Where a directory descriptor's position goes when a listing call
    /// finds nothing more to list (`getdents64` returns 0): `Some(end)`
    /// moves it there (tmpfs: `INT32_MAX`, so it stays at the end until
    /// lseek moves it), `None` leaves it where it was.
    fn readdir_end(&self) -> Option<u64> {
        None
    }

    /// Pins `ino` so it outlives its last name (`ENOENT` if it is gone).
    fn open(&self, ino: Ino) -> Result<()> {
        self.getattr(ino).map(|_| ())
    }

    /// Drops a pin taken by [`FileSystem::open`].
    fn release(&self, _ino: Ino) {}

    // ---- state per open file (ADR 0168) -----------------------------------
    //
    // A file system that keeps something for every `open` (FUSE: the
    // server's file handle) hands out a handle here; the VFS passes it back
    // with the calls made through that open file and gives it up when the
    // last reference to the open file is gone. The defaults keep nothing
    // and do the work by inode.

    /// Opens regular file or directory (`dir`) `ino` with `flags`; `Some`
    /// is the handle of this open. `O_TRUNC` is among the flags only when
    /// [`open_truncates`](Self::open_truncates) is [`OpenTrunc::InOpen`].
    /// An error fails the `open`.
    fn open_handle(&self, ino: Ino, flags: crate::OpenFlags, dir: bool) -> Result<Option<u64>> {
        let _ = (ino, flags, dir);
        Ok(None)
    }

    /// Creates regular file `name` in `dir` and opens it in one step:
    /// the new inode and the handle of the open. `None` (the default): the
    /// VFS calls [`create`](Self::create) and then
    /// [`open_handle`](Self::open_handle).
    fn create_open(
        &self,
        dir: Ino,
        name: &[u8],
        node: &NewNode,
        flags: crate::OpenFlags,
    ) -> Result<Option<(Ino, u64)>> {
        let _ = (dir, name, node, flags);
        Ok(None)
    }

    /// How `O_TRUNC` of an existing file is done.
    fn open_truncates(&self) -> OpenTrunc {
        OpenTrunc::IfNotEmpty
    }

    /// The last reference to the open file of `handle` is gone. Must not
    /// sleep.
    fn close_handle(&self, ino: Ino, handle: u64, flags: crate::OpenFlags, dir: bool) {
        let _ = (ino, handle, flags, dir);
    }

    /// A descriptor of the open file was closed (close(2), every time).
    /// With `wait` the call may sleep until the file system has seen it.
    fn flush_handle(&self, ino: Ino, handle: u64, wait: bool) -> Result<()> {
        let _ = (ino, handle, wait);
        Ok(())
    }

    /// [`read`](Self::read) through an open file.
    fn read_handle(&self, ino: Ino, handle: u64, offset: u64, buf: &mut [u8]) -> Result<usize> {
        let _ = handle;
        self.read(ino, offset, buf)
    }

    /// [`write`](Self::write) through an open file.
    fn write_handle(&self, ino: Ino, handle: u64, offset: u64, buf: &[u8]) -> Result<usize> {
        let _ = handle;
        self.write(ino, offset, buf)
    }

    /// [`append`](Self::append) through an open file.
    fn append_handle(&self, ino: Ino, handle: u64, buf: &[u8]) -> Result<(u64, usize)> {
        let _ = handle;
        self.append(ino, buf)
    }

    /// [`truncate`](Self::truncate) through an open file (ftruncate(2)).
    fn truncate_handle(&self, ino: Ino, handle: u64, size: u64) -> Result<()> {
        let _ = handle;
        self.truncate(ino, size)
    }

    /// [`readdir`](Self::readdir) through an open directory.
    fn readdir_handle(
        &self,
        dir: Ino,
        handle: u64,
        pos: u64,
        emit: &mut dyn FnMut(&DirEntry<'_>) -> bool,
    ) -> Result<()> {
        let _ = handle;
        self.readdir(dir, pos, emit)
    }

    /// fsync(2) (`data_only`: fdatasync(2)) of an open file or directory.
    fn sync_handle(&self, ino: Ino, handle: u64, data_only: bool, dir: bool) -> Result<()> {
        let _ = (ino, handle, data_only, dir);
        self.sync()
    }

    /// Whether `ino` has no file type: `stat` reports its mode without
    /// `S_IFMT` bits (Linux's anonymous inodes). The default is no.
    fn typeless(&self, _ino: Ino) -> bool {
        false
    }

    /// fallocate(2) on regular file `ino`, the range `[offset, offset +
    /// len)` checked (`len` > 0, the end within
    /// [`MAX_FILE_SIZE`](crate::MAX_FILE_SIZE)): `mode` 0 allocates the
    /// range and grows the size to its end, [`FALLOC_FL_KEEP_SIZE`]
    /// allocates without changing the size, `FALLOC_FL_KEEP_SIZE |`
    /// [`FALLOC_FL_PUNCH_HOLE`] frees the range so it reads as zeros.
    /// `ENOSPC` when the file system is full; `EOPNOTSUPP` for modes it
    /// does not support, which by default is every mode.
    fn fallocate(&self, _ino: Ino, _mode: u32, _offset: u64, _len: u64) -> Result<()> {
        Err(Errno::EOPNOTSUPP)
    }

    /// Makes everything written so far durable.
    fn sync(&self) -> Result<()> {
        Ok(())
    }

    /// Extended attribute `name` of `ino` (the full name, namespace
    /// prefix included; see [`crate::xattr`]): its value, `ENODATA` if the
    /// inode has none of that name. The default is a file system that
    /// keeps no extended attributes: `EOPNOTSUPP` for every one of these
    /// four methods. The VFS has checked the name and the caller's
    /// permissions.
    fn getxattr(&self, ino: Ino, name: &[u8]) -> Result<Vec<u8>> {
        let _ = (ino, name);
        Err(Errno::EOPNOTSUPP)
    }

    /// Sets attribute `name` of `ino` to `value` (`EEXIST` / `ENODATA` as
    /// `how` asks, `ENOSPC` when it does not fit), and the change time.
    fn setxattr(&self, ino: Ino, name: &[u8], value: &[u8], how: crate::xattr::SetMode) -> Result<()> {
        let _ = (ino, name, value, how);
        Err(Errno::EOPNOTSUPP)
    }

    /// The names of `ino`'s attributes, each followed by a NUL byte.
    fn listxattr(&self, ino: Ino) -> Result<Vec<u8>> {
        let _ = ino;
        Err(Errno::EOPNOTSUPP)
    }

    /// Removes attribute `name` of `ino` (`ENODATA` if it has none), and
    /// sets the change time.
    fn removexattr(&self, ino: Ino, name: &[u8]) -> Result<()> {
        let _ = (ino, name);
        Err(Errno::EOPNOTSUPP)
    }

    /// The pages of regular file `ino` in `[first, end)` (page indexes)
    /// that hold data, in order, for cachestat(2); holes of sparse files
    /// are left out. `None` (the default) if the file system cannot tell.
    fn data_pages(&self, _ino: Ino, _first: u64, _end: u64) -> Option<Vec<u64>> {
        None
    }

    /// Space and inode counts for `statfs(2)`. The default reports a file
    /// system without blocks or inode limits; the VFS fills in the magic
    /// number from the name when this leaves it 0 (see [`FsStats`]).
    fn statfs(&self) -> Result<FsStats> {
        Ok(FsStats { block_size: 4096, fragment_size: 4096, name_max: crate::NAME_MAX as u64, ..FsStats::default() })
    }

    /// `ino`'s inode flags (`FS_XFLAG_*`, [`crate::fileattr`]). The default
    /// is a file system without inode flags: `EOPNOTSUPP`, also from
    /// [`set_xflags`](Self::set_xflags).
    fn xflags(&self, ino: Ino) -> Result<u32> {
        let _ = ino;
        Err(Errno::EOPNOTSUPP)
    }

    /// Replaces `ino`'s inode flags (`EOPNOTSUPP` for a flag the file
    /// system does not keep), and sets the change time. The VFS has checked
    /// the caller's permissions.
    fn set_xflags(&self, ino: Ino, flags: u32) -> Result<()> {
        let _ = (ino, flags);
        Err(Errno::EOPNOTSUPP)
    }

    /// A persistent handle for `ino` (name_to_handle_at(2)). The default
    /// is a file system that cannot make handles: `EOPNOTSUPP`.
    fn handle(&self, ino: Ino) -> Result<crate::fileattr::FileHandle> {
        let _ = ino;
        Err(Errno::EOPNOTSUPP)
    }

    /// The live inode a handle of this file system names
    /// (open_by_handle_at(2)); `ESTALE` if there is none.
    fn handle_ino(&self, kind: i32, bytes: &[u8]) -> Result<Ino> {
        let _ = (kind, bytes);
        Err(Errno::ESTALE)
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
/// More `f_flags` bits (`sys/statvfs.h` as Linux reports them).
pub const ST_NOATIME: u64 = 1024;
pub const ST_NODIRATIME: u64 = 2048;
pub const ST_NOSYMFOLLOW: u64 = 0x2000;
pub const ST_RELATIME: u64 = 4096;

/// The `f_flags` bits a mount's extra attributes ([`Mount::attrs`],
/// Linux `MOUNT_ATTR_*` bits, `linux/mount.h`) add: the access time mode
/// (the field `0x70`: `MOUNT_ATTR_RELATIME` 0 gives `ST_RELATIME`,
/// `MOUNT_ATTR_NOATIME` `0x10` gives `ST_NOATIME`, `MOUNT_ATTR_STRICTATIME`
/// `0x20` neither, as mount_setattr's spec sheet observed),
/// `MOUNT_ATTR_NODIRATIME` (`0x80`), `MOUNT_ATTR_NOSYMFOLLOW` (`0x200000`).
///
/// [`Mount::attrs`]: crate::Mount::attrs
pub const fn st_flags_of_attrs(attrs: u32) -> u64 {
    let mut f = 0;
    match attrs & 0x70 {
        0 => f |= ST_RELATIME,
        0x10 => f |= ST_NOATIME,
        _ => {}
    }
    if attrs & 0x80 != 0 {
        f |= ST_NODIRATIME;
    }
    if attrs & 0x0020_0000 != 0 {
        f |= ST_NOSYMFOLLOW;
    }
    f
}

/// How a file system wants `open(O_TRUNC)` of an existing regular file
/// done ([`FileSystem::open_truncates`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTrunc {
    /// The VFS truncates before the open, unless the file is empty.
    IfNotEmpty,
    /// The VFS truncates before the open, also an empty file (the file
    /// system's own size may differ from the one the VFS saw).
    Always,
    /// The open truncates: [`FileSystem::open_handle`] gets `O_TRUNC`.
    InOpen,
}

/// Where a magic link leads ([`FileSystem::magic_target`]).
#[derive(Debug)]
pub enum MagicTarget {
    /// An open file (`/proc/<pid>/fd/<n>`, `exe`): its node, and the name
    /// it was opened by.
    File(alloc::sync::Arc<crate::OpenFile>),
    /// A directory (`/proc/<pid>/cwd`, `root`).
    Node(crate::NodeRef),
}

/// Magic numbers of the file systems Poly has (Linux UAPI `linux/magic.h`).
pub const TMPFS_MAGIC: u64 = 0x0102_1994;
pub const EXT2_SUPER_MAGIC: u64 = 0xef53;
pub const PROC_SUPER_MAGIC: u64 = 0x9fa0;
pub const SYSFS_MAGIC: u64 = 0x6265_6572;
pub const MQUEUE_MAGIC: u64 = 0x1980_0202;

/// The `f_type` of a file system by its name ([`FsInfo::name`]); 0 for
/// names without one here.
pub fn magic_for(name: &str) -> u64 {
    match name {
        "tmpfs" => TMPFS_MAGIC,
        "ext2" => EXT2_SUPER_MAGIC,
        "proc" => PROC_SUPER_MAGIC,
        "sysfs" => SYSFS_MAGIC,
        "mqueue" => MQUEUE_MAGIC,
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
