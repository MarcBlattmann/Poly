# 0076: One order of path errors for every path call; a two-path call's second path reported in its place

**Status:** Accepted. Amends the path resolution of
[0023](0023-vfs-tmpfs-and-cpio.md) and the path calls of
[0043](0043-dev-proc-sys-and-file-system-calls.md).

## Context

spec-a specified, for every call that takes a path, which error wins when a path has
more than one problem: symlinkat's `precedence` and the `path_order` of chdir,
faccessat, faccessat2, fchmodat, fchownat, truncate (`ftruncate.yaml`), linkat,
mkdirat, newfstatat, openat, readlinkat, renameat2, statfs, statx, unlinkat and
utimensat, plus `path_order_two_paths` for linkat and renameat2. In short: the string
as a whole first (`EFAULT`, `PATH_MAX` bytes or more `ENAMETOOLONG`, empty `ENOENT`),
then a relative path's directory descriptor (`EBADF`, `ENOTDIR`), then the components
left to right as the walk reaches them, the first failure deciding; a name over 255
bytes is reported only when every directory before it exists.

Poly's walk checked each component's length before it looked at the directory the
component is in, so `D/file/<256 bytes>` and `symlinkat("x", <fd on a file>,
<256 bytes>)` were `ENAMETOOLONG` where the sheets say `ENOTDIR` (the visible tests
`file_before_long_name_enotdir`, `file_before_long_middle_enotdir`,
`file_dirfd_before_long_name_enotdir`). And linkat and renameat2 copied both strings
and looked up both descriptors before walking either path, while the sheets say
linkat resolves oldpath completely before anything of newpath, and renameat2 checks
both paths' directories before either last name (`renameat2(D/nope/x, <4096-byte
newpath>)` is `ENOENT`, `renameat2(D/<256 bytes>, D/nope/x)` is `ENOENT`).

Sources: those spec sheets and their visible tests in `specs/linux/x86_64/`, the man
pages path_resolution(7), link(2), rename(2), symlink(2) and the UAPI
`linux/limits.h` for `PATH_MAX` and `NAME_MAX`. No Linux or C library source was read
and no Linux system was run or traced.

## Decision

**The walk** (`poly-vfs`, `path.rs`, whose module documentation states the order).
For each component, the directory it is in comes first (not a directory `ENOTDIR`,
no search permission `EACCES`), then the name (over `NAME_MAX` `ENAMETOOLONG`, then
the lookup). Every path call goes through this one walk, so they all follow it.
`Walk::dirs` resolves everything but the last name's own length; `Walk::parent` is
`dirs` plus `Parent::check_name`. Search permission before the name's length is our
choice (the sheets do not separate them; their runs are as root).

**A path argument whose errors are not reported yet**, `poly_vfs::PathArg`: the
string or why it could not be read, and the start or why the descriptor names
nothing. `PathArg::begin` reports them in their order (string, then the start for a
relative path only, so an absolute path ignores even a bad descriptor). The two-path
calls take one per path:

- `Vfs::link_paths`: oldpath's string, start and every component including the last,
  then newpath's string, start and components, then `EEXIST` and the link's own
  errors; `Vfs::link_node` (`AT_EMPTY_PATH`) takes the new path as a `PathArg` too.
- `Vfs::rename_paths`: (1) oldpath up to its last name, (2) newpath up to its last
  name, a `/`, `.` or `..` last component on either side `EBUSY`, (3) oldpath's last
  name (length, then the lookup: `ENOENT`), (4) newpath's last name's length, then the
  rename's own errors (`EXDEV` now comes after them).
- `Vfs::link` and `Vfs::rename` keep their signatures (good paths) and call these.

For one path the order of steps 1 and 2 follows from who does them: the personality
copies the string (`EFAULT`, `ENAMETOOLONG`, `ENOENT`) before it calls the kernel,
and the kernel looks the descriptor up before the walk. For linkat and renameat2 the
personality copies the second string but hands the result over
(`LinuxFs::linkat`/`renameat` take `new: Result<&[u8], Errno>`), and the kernel
(`Files::linkat`/`renameat`) looks both descriptors up into `PathArg`s without
returning early, so the VFS puts those errors in their place.

## Consequences

- Every path call reports the first failing component; the visible tests of all 17
  entries pass on Poly, symlinkat's three failing checks included.
- A rename across mounts with a missing source is now `ENOENT`, not `EXDEV`.
- Host tests: `poly-vfs` `tests/path_order.rs` checks each rule for every path call
  through one table, and the two-path orders; the personality's
  `second_path_errors_come_in_their_place` checks the syscall level.
