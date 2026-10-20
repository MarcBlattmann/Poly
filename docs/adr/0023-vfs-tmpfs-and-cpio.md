# 0023: The VFS in a host-tested crate with `alloc`, tmpfs beside it, cpio unpacked through the VFS

**Status:** Accepted. Relaxes the "allocation-free" consequence of
[ADR 0003](0003-workspace-layout.md) for `poly-vfs` and `poly-cpio`.

## Context

Phase 3 needs "VFS core: inodes, dentries, mounts, file descriptor tables,
path lookup" and "Initial RAM disk (cpio) mounted as the root file system;
tmpfs" before the first Linux program can `write(1, ...)` and before
`execve` can find a binary. The Linux personality is being written in
parallel against a kernel-services trait and will call into the VFS for
`openat`, `read`, `write`, `getdents64` and the rest, so the VFS has to
speak Linux's semantics and error numbers directly. Later the NT and XNU
personalities, ext2, devfs and procfs plug into the same core.

Path resolution, descriptor tables and file system bookkeeping are pure
logic with many edge cases (`..` across mount points, symbolic link loops,
trailing slashes, `dup2` onto itself, unlinked open files), exactly what
ADR 0003 says belongs in host-tested crates. Unlike the drivers, though,
the VFS holds variable numbers of variable-sized things (names, files,
directories), and the kernel has had a heap since Phase 1.

The behaviour comes from POSIX and the Linux manual pages
(`path_resolution(7)`, `open(2)`, `rename(2)`, `dup(2)`, `chown(2)`, ...);
constants and error numbers from the public Linux UAPI headers; the cpio
format from public format documentation. No kernel source was read.

## Decision

**`crates/poly-vfs` is `no_std` with `alloc`.** It uses `Arc`, `Vec` and
`BTreeMap`; it has no other dependencies. Errors are an `Errno` enum with
Linux's values (`asm-generic/errno-base.h`, `errno.h`), and `OpenFlags`,
`RenameFlags` and `Whence` take Linux's bit values, so the Linux
personality passes arguments through and returns `-errno` unchanged; the
other personalities translate.

**Layers.**
- `FileSystem` (trait, `&self` methods, own locking): single-directory
  operations on inode numbers: `lookup`, `create`, `mkdir`, `symlink`,
  `link`, `unlink`, `rmdir`, `rename` (with `RENAME_NOREPLACE` and
  `RENAME_EXCHANGE`), `readlink`, `read`/`write`/`append` at an offset,
  `truncate`, `getattr`/`setattr`, `readdir` with stable positions, and
  `open`/`release` pins that keep an inode alive after its last name.
- `Vfs`: the mount table, the character device registry, path resolution
  and permission checks, and the path-based calls (`open`, `mkdir`,
  `mknod`, `symlink`, `link`, `unlink`, `rmdir`, `rename`, `stat`,
  `readlink`, `access`, `chmod`, `chown`, `truncate`, `chdir`, `chroot`,
  `getcwd`, `mount`, `bind`, `umount`, `open_exec`).
- `NodeRef`: an inode seen through a mount, pinned while it exists.
  `FsContext` (root, working directory, umask) holds two; `OpenFile` holds
  one, plus the offset and the status flags; `FdTable` holds
  `Arc<OpenFile>`s with a close-on-exec bit each. `Clone` on `FsContext`
  and `FdTable` is what `fork` does.

**Path resolution is iterative.** A symbolic link is spliced into the rest
of the path instead of recursing, so a 40-link chain (Linux's limit,
`ELOOP` beyond) costs no kernel stack. The walk hands the last component
back unresolved, so each operation decides about following, creating and
trailing slashes the way its manual page says. `..` stays at the process
root and climbs out of a mount at the mount's root; a name whose inode is
a mount point continues at the root of the topmost mount there.

**"Dentries" are locations plus a lookup cache, not a tree of cached
nodes.** A place in the tree is `(mount, inode)`; parents come from the
file system's `..`, names for `getcwd` from scanning the parent. This
stays correct when directories are renamed under a working directory,
with nothing to keep coherent. What a dentry cache is for, not asking the
file system again, is a per-superblock map of positive name lookups, opt-in
per file system (`FsInfo::cache_lookups`): every change to such a file
system goes through the VFS, which forgets the names it changes, and `..`
is never cached. tmpfs opts out (a lookup is as cheap as the cache); ext2
will opt in. The superblock is shared by every mount of one file system,
bind mounts included.

**Mounts stack.** Mounting on a mount point covers what was there; the last
mount wins and unmounting it uncovers the one below. Mounting on `/` covers
the root for every process, which is how a disk root will replace the
initramfs later (no `pivot_root` yet). Bind mounts are mounts of a
directory of an existing superblock. `rename` and `link` refuse to cross
mounts (`EXDEV`), even of one file system, as Linux does. A mount is busy
(`EBUSY`) while anything is mounted below it or anything else holds it (an
open file, a working directory), measured by the `Arc`'s reference count;
lazy unmount detaches a whole subtree at once.

**Permission checks live in the VFS**, from the attributes the file system
reports and a `Cred` (uid, gid, groups) passed into every call: one class
of `rwx` bits, root bypassing read/write/search and needing one execute
bit to execute; search permission on every directory walked; write and
search on a directory to change it; the sticky bit; set-group-ID
directories; `chmod`/`chown`/`utimensat` ownership rules; `EROFS` for
read-only mounts; `nodev` and `noexec`. File systems never see credentials.

**Character devices are a registry keyed by device number** (`CharDevice`:
read, write, seekable, ioctl). Opening a character device node finds the
device there (`ENXIO` without one). `null`, `zero` and `full` are in the
crate; the kernel registers the console. Opening FIFOs, sockets and block
devices is `ENXIO` until pipes, sockets and a block-device file exist.

**Locking.** One small internal spinlock type, held only for in-memory
work and never across a call into a file system or device, except the
offset lock of an open file, which is held across the transfer (and the
`readdir` callback) because POSIX makes offset updates atomic per open file
description. Devices are called without it unless they are seekable.

**tmpfs lives in `poly-vfs`.** It is the reference `FileSystem` and every
VFS test runs on it. Files are sparse (4 KiB pages allocated on first
write; holes read as zeros), directories keep insertion-ordered sequence
numbers that make `readdir` positions stable across insertions and
removals, inode numbers are never reused, and an optional size limit gives
`ENOSPC`. One lock per instance; `readdir` copies entries out and calls
back with the lock released.

**`crates/poly-cpio`** parses `newc` (070701) and `crc` (070702) without
allocating and never panics on any input: concatenated archives with NUL
padding (as an initramfs is built), a missing final padding, uppercase hex.
`crc` checksums are verified for regular files only, because GNU cpio
writes 0 for symbolic links. A `Writer` builds archives (tests now; an
xtask initramfs builder later, since the host may have no `cpio`). The
unpacker goes through a `Vfs` as root, so it works on any `FileSystem` and
gets symbolic links in paths, hard links (`link` to the first name seen,
whichever entry carries the data), replacement of existing names and
missing parent directories right; owners, modes and times are applied
exactly (umask 0), directory times last; names with `..` are refused.

## Consequences

- The VFS, tmpfs and the cpio code are tested on the host in milliseconds:
  path resolution edge cases, mount crossing, descriptor semantics,
  permissions, rename variants, unlinked open files, readdir under
  concurrent change, every truncation and single-byte mutation of an
  archive, and archives made by the host's GNU cpio (stored in the repo,
  and regenerated live when `cpio` is installed).
- The kernel side is: build a `Tmpfs` with a clock, `unpack_fs` the
  initramfs into it, `Vfs::new` it, register the console and the memory
  devices, give the first process `root_context()` and an `FdTable` with
  the console on 0, 1 and 2. Buffers are kernel memory: the kernel copies
  to and from user space outside the VFS (a page fault in user memory
  never happens with a VFS lock held).
- The internal spinlock does not mask interrupts and spins rather than
  sleeps. That is correct as long as no interrupt handler enters the VFS
  and the kernel never calls the VFS holding an interrupt-masking spinlock
  of its own; a file system that sleeps (ext2) makes only threads sharing
  the same open file description spin. If that shows up in profiles, the
  offset lock becomes a parameter the kernel fills with its sleeping mutex.
- `FileSystem` methods are per directory and synchronous; asynchronous I/O
  and a page cache (Phase 4, ext2) sit below it, not in the VFS.
- Not yet there: `O_TMPFILE`, `statfs`, `pivot_root`, mount propagation,
  extended attributes, access-time policies, pipes and sockets as files,
  block device files, `RLIMIT_FSIZE`.
