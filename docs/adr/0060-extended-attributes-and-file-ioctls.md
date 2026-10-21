# 0060: Extended attributes stored by tmpfs and ext2/3/4, minimal ACLs as permission bits, and the file ioctls cp and cat probe

**Status:** Accepted. Builds on [0023](0023-vfs-tmpfs-and-cpio.md) (VFS,
tmpfs), [0032](0032-ext2-driver.md) and [0052](0052-ext4-extents-checksums-and-journal.md)
(ext2/ext4), [0057](0057-app-tests-host-programs-in-one-tree.md) (app
tests) and [0059](0059-bash-coreutils-python-gcc-from-the-persistent-disk.md),
whose `*xattr` calls answered `EOPNOTSUPP` for every existing file.

## Context

The app tests of `ls`, `cp` and `sed` (`specs/linux/apps/*.yaml`) show
what GNU programs need from extended attributes on the tmpfs a test tree
is on: `lgetxattr(path, "security.selinux")` is `ENODATA` and
`listxattr` returns 0 for every file `ls -l` lists (with an error ls
prints "Function not implemented" or "Operation not supported" lines);
`sed -i` reads `system.posix_acl_access` of the input (`ENODATA`) and sets
it on its temporary file (28 bytes: a minimal ACL, so the result keeps the
input's mode). cp probes `ioctl(dest, FICLONE, src)` before copying
(`EOPNOTSUPP` on tmpfs) and cat asks `ioctl(fd, FIONREAD)` of a regular
input (the bytes left). The spec also gives Linux tmpfs's directory size:
40 bytes plus 20 per entry.

## Decision

- **VFS** (`poly_vfs::xattr`): names are checked (1–255 bytes, `ERANGE`;
  prefix `user.`, `trusted.`, `security.`, `system.`, else `EOPNOTSUPP`)
  and permissions as xattr(7) describes (`user.` only on regular files
  and directories, read/write permission, sticky directories; `trusted.`
  needs `CAP_SYS_ADMIN` and is hidden without it; `security.` writes need
  `CAP_SYS_ADMIN` since Poly has no security module; read-only mounts
  `EROFS`). Storage is the file system's: four `FileSystem` methods,
  `EOPNOTSUPP` by default, passed through by the page cache's `CachedFs`.
- **tmpfs** keeps a sorted map per inode (`Xattrs`); the names of one
  inode fit in `XATTR_LIST_MAX`.
- **ext2/3/4** (`poly-ext2/src/xattr.rs`) reads and writes the on-disk
  format of "ext4 Data Structures and Algorithms": entries in the inode's
  extra space and in one attribute block, hashes, block checksums
  (`metadata_csum`), reference-counted shared blocks (copied on write, as
  a block other inodes use must not change), the `ext_attr` feature set on
  first use. Every change rewrites the inode's attributes, sorted, into
  the inode while they fit and the rest into the block. Tested against
  `debugfs ea_set`/`ea_get` and `e2fsck -fn`.
- **POSIX ACLs**: Poly enforces no ACLs beyond the permission bits, so it
  accepts exactly the ACLs those express. Setting a minimal access ACL
  sets the permission bits (as chmod(2), set-group-ID rules included) and
  stores nothing; reading `system.posix_acl_access` is then `ENODATA`.
  Extended access ACLs and default ACLs are `EOPNOTSUPP` rather than
  stored and silently not enforced. ACLs already on an ext4 volume (in
  ext4's version-1 form) are returned in the VFS's version-2 form.
- **The personality** (`sys/xattr.rs`, replacing ADR 0059's answers)
  copies names and values, applies the size rules of the man pages (size 0
  asks for the size, `ERANGE` for a small buffer, `E2BIG` past 64 KiB) and
  keeps ADR 0059's choices (`EINVAL` for both flags, `ERANGE` for an empty
  name).
- **File ioctls** (`sys/fileioctl.rs`, asked before the device ioctls):
  `FIONREAD` on a regular file is the size minus the file offset;
  `FICLONE`, `FICLONERANGE` and `FIDEDUPERANGE` check their descriptors
  (`EBADF`, `EISDIR`, `EINVAL` for special files, `EXDEV`) and answer
  `EOPNOTSUPP`: no Poly file system shares extents.
- **Output capture**: `pwrite64` to fds 1 and 2 of a traced process is
  reported in `POLY-OUT` like `write` (ADR 0027), so `copy_file_range`
  with an output offset, as well as `write`, `writev` and `sendfile`, is
  seen.
- **tmpfs directory size**: 40 bytes plus 20 per entry other than `.` and
  `..`, no blocks.
- **`pipeline polyrun`**: runs corpus programs and app tests on Poly only
  (no reference run), so implementer agents can see a visible test's
  output and app-mode events without the oracle.

## Consequences

- `ls -l`, `cp -p` and `sed -i` see the answers of a Linux tmpfs without
  SELinux; attributes set by programs persist on ext4 and are readable by
  Linux and e2fsprogs.
- Programs that set extended or default ACLs (`setfacl -m u:...`) get
  `EOPNOTSUPP`, as on a file system mounted without ACL support; ACL
  enforcement is future work (it needs the permission check in
  `Cred::check` to consult them).
- ext4 values larger than a block (`ea_inode`) stay unsupported (`ENOSPC`
  when written; volumes with the feature do not mount).
