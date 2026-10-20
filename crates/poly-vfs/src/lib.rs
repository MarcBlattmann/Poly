//! The virtual file system: everything between "a process names a path or a
//! file descriptor" and "a file system reads or writes an inode", without
//! hardware.
//!
//! - [`FileSystem`] is what a file system implements: inodes named by an
//!   [`Ino`], looked up by name in a directory, read and written at an
//!   offset. [`Tmpfs`] is one; ext2, devfs and procfs will be others.
//! - [`Vfs`] owns the mount table and the character device registry, and
//!   resolves paths with POSIX semantics (see `path.rs`): absolute or relative
//!   to a directory, `.` and `..` across mount points, symbolic links with a
//!   limit of [`MAX_SYMLINKS`] (`ELOOP`), trailing slashes, `ENOTDIR`,
//!   `ENOENT`, `ENAMETOOLONG`. Its methods are the path-based system calls:
//!   [`Vfs::open`], [`Vfs::mkdir`], [`Vfs::rename`], [`Vfs::stat`], ...
//! - [`NodeRef`] is a reference to an inode on a mount that keeps the inode
//!   alive (an unlinked file stays readable while a `NodeRef` to it exists).
//!   A process's root and working directory ([`FsContext`]) are `NodeRef`s.
//! - [`OpenFile`] is an open file description: the node, the offset, the
//!   access mode and status flags (`O_APPEND`, `O_NONBLOCK`). It is shared
//!   (`Arc`) between descriptors made by `dup` and between processes after
//!   `fork`.
//! - [`FdTable`] is a process's descriptor table: lowest-free allocation,
//!   `dup`/`dup2`/`dup3`, the close-on-exec flag, `close_on_exec` at exec.
//! - [`Cred`] carries the ids permission checks use (owner/group/other
//!   `rwx`, root bypass), passed to every call that checks permissions.
//! - [`CharDevice`] is how the kernel provides `/dev/console`, `/dev/null`
//!   and the like: registered by device number with
//!   [`Vfs::register_char_device`], reached by opening a character device
//!   node with that number. [`device`] has null, zero and full.
//! - Every error is an [`Errno`], with Linux's values, so the Linux
//!   personality returns them unchanged.
//!
//! # Allocation and locking
//!
//! Unlike the crates below it, this one uses `alloc` (`Arc`, `Vec`,
//! `BTreeMap`): file systems hold variable numbers of variable-sized names
//! and files, which fixed tables cannot describe, and the kernel has had a
//! heap since Phase 1. Shared state sits behind a small internal spinlock
//! that is never held across a call into a file system or a device, with one
//! documented exception (the offset of an open file, see `sync.rs`). File
//! systems do their own locking; [`FileSystem`] methods take `&self`.
//! Those that hold a lock across disk I/O take it as a type parameter,
//! a [`lock::RawLock`] (a spinlock by default; the kernel passes its
//! sleeping mutex) wrapped around their data by [`lock::Lock`].
//!
//! # How a Linux system call maps onto this
//!
//! ```ignore
//! // openat(dirfd, path, flags, mode)
//! let at = fds.at_for(dirfd, path)?;                 // AT_FDCWD or the fd's node
//! let file = vfs.open(&ctx, &cred, at, path, OpenFlags::from_bits(flags), mode)?;
//! let fd = fds.install(file, OpenFlags::from_bits(flags).contains(OpenFlags::CLOEXEC))?;
//! // write(fd, buf, len)
//! let n = fds.get(fd)?.write(kernel_copy_of_buf)?;
//! // on error: return e.to_syscall_return()
//! ```
//!
//! Buffers are kernel memory: copying from and to user space (and taking
//! page faults while doing so) happens in the caller, outside the VFS.

#![no_std]

extern crate alloc;

mod cred;
pub mod device;
mod errno;
mod fdtable;
mod file;
mod fs;
pub mod lock;
mod mount;
mod node;
mod path;
mod sync;
mod tmpfs;
mod vfs;

pub use cred::{Access, Cred};
pub use device::CharDevice;
pub use errno::Errno;
pub use fdtable::{AT_FDCWD, At, DEFAULT_FD_LIMIT, Fd, FdTable};
pub use file::{OpenFile, OpenFlags, Whence};
pub use fs::{Clock, DirEntry, FileSystem, FsInfo, NewNode, RenameFlags};
pub use mount::{Mount, MountFlags, MountId, MountInfo};
pub use node::{FsContext, NodeRef};
pub use path::{MAX_SYMLINKS, NAME_MAX, PATH_MAX};
pub use tmpfs::Tmpfs;
pub use vfs::Vfs;

/// The result of every fallible VFS operation.
pub type Result<T, E = Errno> = core::result::Result<T, E>;

/// An inode number, unique within one file system.
pub type Ino = u64;

/// File type bits of a mode (`S_IFMT` and friends, Linux UAPI `linux/stat.h`).
pub const S_IFMT: u32 = 0o170000;
pub const S_IFSOCK: u32 = 0o140000;
pub const S_IFLNK: u32 = 0o120000;
pub const S_IFREG: u32 = 0o100000;
pub const S_IFBLK: u32 = 0o060000;
pub const S_IFDIR: u32 = 0o040000;
pub const S_IFCHR: u32 = 0o020000;
pub const S_IFIFO: u32 = 0o010000;
/// Set-user-ID, set-group-ID and sticky bits (`linux/stat.h`).
pub const S_ISUID: u16 = 0o4000;
pub const S_ISGID: u16 = 0o2000;
pub const S_ISVTX: u16 = 0o1000;
/// Group execute permission (`linux/stat.h`).
pub const S_IXGRP: u16 = 0o010;
/// The permission part of a mode: `rwxrwxrwx` plus the three bits above.
pub const PERM_MASK: u16 = 0o7777;

/// What kind of object an inode is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileType {
    Regular,
    Directory,
    Symlink,
    CharDevice,
    BlockDevice,
    Fifo,
    /// A Unix domain socket's name in the file system. Sockets come in
    /// Phase 4; until then such nodes can exist but not be opened.
    Socket,
}

impl FileType {
    /// The `S_IF*` bits for this type.
    pub const fn mode_bits(self) -> u32 {
        match self {
            FileType::Regular => S_IFREG,
            FileType::Directory => S_IFDIR,
            FileType::Symlink => S_IFLNK,
            FileType::CharDevice => S_IFCHR,
            FileType::BlockDevice => S_IFBLK,
            FileType::Fifo => S_IFIFO,
            FileType::Socket => S_IFSOCK,
        }
    }

    /// The type a mode's `S_IFMT` bits name, if any.
    pub const fn from_mode(mode: u32) -> Option<FileType> {
        Some(match mode & S_IFMT {
            S_IFREG => FileType::Regular,
            S_IFDIR => FileType::Directory,
            S_IFLNK => FileType::Symlink,
            S_IFCHR => FileType::CharDevice,
            S_IFBLK => FileType::BlockDevice,
            S_IFIFO => FileType::Fifo,
            S_IFSOCK => FileType::Socket,
            _ => return None,
        })
    }

    /// The `d_type` value `getdents64` reports (`DT_REG`, `DT_DIR`, ...),
    /// which is the `S_IFMT` bits shifted down by 12 (`getdents(2)`).
    pub const fn dirent_type(self) -> u8 {
        (self.mode_bits() >> 12) as u8
    }
}

/// A device number: which driver (`major`) and which unit (`minor`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId {
    pub major: u32,
    pub minor: u32,
}

impl DeviceId {
    pub const fn new(major: u32, minor: u32) -> DeviceId {
        DeviceId { major, minor }
    }
}

/// A point in time: seconds and nanoseconds since the Unix epoch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timespec {
    pub sec: i64,
    pub nsec: u32,
}

impl Timespec {
    pub const fn new(sec: i64, nsec: u32) -> Timespec {
        Timespec { sec, nsec }
    }
}

/// An inode's attributes, as a file system reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attr {
    pub ino: Ino,
    pub kind: FileType,
    /// Permission bits including set-user-ID, set-group-ID and sticky
    /// ([`PERM_MASK`]).
    pub perm: u16,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    /// Bytes for regular files, the target's length for symbolic links.
    pub size: u64,
    /// The device a device node stands for.
    pub rdev: DeviceId,
    pub atime: Timespec,
    pub mtime: Timespec,
    pub ctime: Timespec,
    /// Storage used, in 512-byte units (`st_blocks`).
    pub blocks: u64,
    /// Preferred I/O size (`st_blksize`).
    pub blksize: u32,
}

impl Attr {
    /// Type and permission bits together, as `st_mode`.
    pub const fn mode(&self) -> u32 {
        self.kind.mode_bits() | self.perm as u32
    }
}

/// A time to set with [`SetAttr`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetTime {
    /// The file system's current time (`UTIME_NOW`).
    Now,
    At(Timespec),
}

/// Attribute changes for [`FileSystem::setattr`]; `None` leaves a field
/// alone. A file system sets `ctime` to now on every change unless `ctime`
/// is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SetAttr {
    pub perm: Option<u16>,
    pub uid: Option<u32>,
    pub gid: Option<u32>,
    pub size: Option<u64>,
    pub atime: Option<SetTime>,
    pub mtime: Option<SetTime>,
    pub ctime: Option<Timespec>,
}

/// What `stat` reports: an inode's attributes plus the device of the file
/// system it lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stat {
    /// The file system's device number (`st_dev`); anonymous file systems
    /// such as tmpfs get major 0 and a minor unique to the file system.
    pub dev: DeviceId,
    pub ino: Ino,
    /// Type and permission bits (`st_mode`).
    pub mode: u32,
    pub nlink: u32,
    pub uid: u32,
    pub gid: u32,
    pub rdev: DeviceId,
    pub size: u64,
    pub blksize: u32,
    pub blocks: u64,
    pub atime: Timespec,
    pub mtime: Timespec,
    pub ctime: Timespec,
}

impl Stat {
    pub(crate) fn new(dev: DeviceId, a: &Attr) -> Stat {
        Stat {
            dev,
            ino: a.ino,
            mode: a.mode(),
            nlink: a.nlink,
            uid: a.uid,
            gid: a.gid,
            rdev: a.rdev,
            size: a.size,
            blksize: a.blksize,
            blocks: a.blocks,
            atime: a.atime,
            mtime: a.mtime,
            ctime: a.ctime,
        }
    }

    /// The file type.
    pub fn kind(&self) -> Option<FileType> {
        FileType::from_mode(self.mode)
    }

    /// The permission bits.
    pub fn perm(&self) -> u16 {
        (self.mode & PERM_MASK as u32) as u16
    }
}

/// The largest file size and offset: `off_t` is a signed 64-bit integer.
pub const MAX_FILE_SIZE: u64 = i64::MAX as u64;

#[cfg(test)]
mod tests;
