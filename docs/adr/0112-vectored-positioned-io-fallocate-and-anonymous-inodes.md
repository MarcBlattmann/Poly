# 0112: preadv/pwritev and their `2` forms in the personality, fallocate through the file system, and anonymous inodes without a type

**Status:** Accepted. Delivers spec-linux-b's file-system entries
(`specs/linux/x86_64/`, hand-off of 2026-10-31 in `specs/HANDOFF_LOG.md`).

## Context

spec-linux-b's 30 entries were mostly implemented already (bash and
coreutils run on Poly), but their visible tests found four calls missing
(`preadv`, `pwritev`, `preadv2`, `pwritev2`: `ENOSYS`), one more missing
(`fallocate`), and a handful of differences in what existed: the order of
`unlink`'s and `rmdir`'s errors, the `fs.protected_hardlinks` rule of
`link`, `chown(-1, -1)` on another user's set-user-ID file, `truncate`'s
negative length, `flock`'s `LOCK_MAND` and `O_PATH`, `copy_file_range`'s
order and `EOVERFLOW`, and `fchmod` of an event file. The inputs were the
spec sheets, their visible tests, the man pages and the UAPI headers
(`linux/fs.h` for `RWF_*`, `linux/falloc.h` for `FALLOC_FL_*`,
`asm-generic/fcntl.h` for `LOCK_MAND`).

## Decision

- **Vectored I/O at an offset lives in the personality**
  (`personalities/linux/src/sys/rwv.rs`): the checks in the spec's order
  (offset, descriptor, flags, `ESPIPE`, the iovec array, `EISDIR`), then
  the transfer through the existing readv/writev machinery of
  `sys/io.rs`, generalised by a `How` (explicit offset or the file
  offset, `RWF_APPEND`/`RWF_NOAPPEND`, `RWF_NOWAIT`). Plain transfers use
  `LinuxFs::pread`/`pwrite` and `LinuxKernel::read`/`write` as before.
  Two `LinuxFs` methods with defaults (`EOPNOTSUPP`) carry the flags the
  existing calls cannot: `read_nowait` and `write_flags`. The kernel
  implements them with `OpenFile::write_with` (one write that appends, or
  does not, whatever `O_APPEND` says, without changing the descriptor's
  flags) and the open file's waits with `Timeout::Now`. `RWF_NOWAIT` on a
  file on a file system is `EOPNOTSUPP` (tmpfs has no non-blocking
  buffered I/O); `RWF_ATOMIC` and `RWF_DONTCACHE` are `EOPNOTSUPP`;
  `RWF_DSYNC`/`RWF_SYNC` sync after the write; `RWF_NOSIGNAL` suppresses
  `SIGPIPE`.
- **fallocate is a `FileSystem` operation** (`FileSystem::fallocate`,
  default `EOPNOTSUPP`): tmpfs allocates zero pages (mode 0 grows the
  size, `KEEP_SIZE` does not; `ENOSPC` against the mount's `size=`) and
  frees them for `KEEP_SIZE | PUNCH_HOLE` (edge bytes zeroed); the page
  cache zeroes the old tail when the size grows and makes its copy of a
  punched range read as zeros. ext2 says `EOPNOTSUPP` (C libraries fall
  back for `posix_fallocate`). The personality (`sys/falloc.rs`) checks
  the order of the spec, `RLIMIT_FSIZE` included (`EFBIG` with `SIGXFSZ`,
  as `truncate` now also sends).
- **Anonymous inodes have no type.** Event files, timers, epoll sets and
  signal files get `Vfs::anon_inode`: `st_mode` is `0600` without
  `S_IFMT` bits and `fchmod` is `EOPNOTSUPP` (`FileSystem::typeless`,
  default false; the anonymous file system keeps the set). Pipes and
  sockets keep their typed anonymous inodes.
- **Permission rules in `poly-vfs`**: `fs.protected_hardlinks = 1` is
  always on (a caller without `CAP_FOWNER` who does not own the file may
  only link a regular file it may read and write that is not set-user-ID
  or executable set-group-ID); chown clearing a set-id bit counts as a
  mode change (a non-owner's `-1, -1` is `EPERM`), and only on regular
  files; unlink and rmdir check a read-only mount before the last name
  and the parent's write permission before the victim's type.

## Consequences

- 29 of the 30 entries are green; `fchmod` stays claimed: its visible test
  also needs `memfd_create`, which belongs to impl-linux-a's entries.
- `fs.protected_hardlinks` cannot be turned off (there is no sysctl yet).
- A mapping of a page that a punched hole drops from the page cache keeps
  the old frame until it is unmapped.
