# 0045: Credentials and capabilities: per-thread `Creds` in the Linux personality, capability-based checks in the VFS

**Status:** Accepted. Builds on [0023](0023-vfs-tmpfs-and-cpio.md) /
[0037](0037-vfs-and-initramfs-in-the-kernel.md) (the VFS's `Cred`),
[0043](0043-dev-proc-sys-and-file-system-calls.md) (the `LinuxFs` file
system services),
[0031](0031-linux-signals-and-futexes.md) (per-process Linux state, kill
permissions) and [0041](0041-processes-threads-fork-exec-wait.md) (clone,
fork, execve).

## Context

Phase 4 asks for "users, groups, credentials, file permissions, `umask`".
Every process ran as root: `fs::Files` held one `Cred::root()` per process,
the VFS treated user id 0 as privileged, `getuid` and friends read that
`Cred`, and kill(2)'s permission check compared ids the kernel always
reported as 0. Nothing could change an id, programs could not drop
privileges, and set-user-ID bits did nothing.

Linux's model has three parts the man pages describe separately:
credentials(7) (real, effective, saved and file system user and group ids,
supplementary groups), capabilities(7) (privilege split into some forty
capabilities in effective, permitted, inheritable, bounding and ambient
sets, with rules that tie them to user id changes and execve(2)), and
path_resolution(7) (which capability overrides which permission check).
credentials(7) also says where they live: "At the kernel level, user IDs
and group IDs are a per-thread attribute", while POSIX wants them shared by
the process, which the C library arranges by running every `set*id` call
on all threads (nptl(7)).

Sources: credentials(7), capabilities(7), path_resolution(7), setuid(2),
setgid(2), setreuid(2), setresuid(2), setfsuid(2), setfsgid(2),
getgroups(2), capget(2), prctl(2), execve(2), access(2), chmod(2),
chown(2), utimensat(2), umask(2), unlink(2), rename(2), open(2), mknod(2),
inode(7), kill(2), getauxval(3), POSIX `write()`, truncate(2), passwd(5),
group(5); constants from the UAPI headers `linux/capability.h`,
`linux/prctl.h`, `linux/securebits.h`, `linux/fcntl.h`, `linux/stat.h` and
`linux/limits.h`. No Linux or C library source was read and nothing was
observed on a real system.

## Decision

**Credentials are per thread and live in the personality.** The
personality's `cred::Creds` holds the eight ids, the supplementary groups
(`Arc<[u32]>`, so copies are cheap), the five capability sets,
`keep_caps` (`SECBIT_KEEP_CAPS`) and `no_new_privs`. Each `LinuxThread`
(ADR 0031) has one, next to its signal mask, and a thread name (`comm`).
`LinuxProcess::add_thread` and `fork` copy the creating thread's, so
clone(2) with or without `CLONE_THREAD`, fork(2) and vfork(2) inherit
without the kernel doing anything; execve(2) keeps the calling thread's and
the kernel then installs the transformed ones. The `set*id`, `setgroups`,
`capset` and `prctl` calls change only the calling thread, as the kernel
side of Linux does; making a change process-wide is the C library's job.
Every rule is a method of `Creds` that changes nothing when it fails,
host-tested paragraph by paragraph against the man pages: the privileged
and unprivileged forms of each `set*id` call, the saved-id rule of
setreuid(2), setfsuid(2)'s silent refusal, capabilities(7)'s four "effect
of user ID changes" rules (with `keep_caps`), capset(2)'s subset rules, the
bounding and ambient sets, and execve(2)'s transformation.

**Privilege is a capability, everywhere.** "Privileged (Linux: has
CAP_X)" in a man page is `Creds::capable(CAP_X)`, the effective set. Root
is user 0 with full permitted and effective sets; the uid rules keep them
in step, so a program that drops root loses its capabilities and one that
drops only `CAP_DAC_OVERRIDE` stays user 0 without it. kill(2) needs
`CAP_KILL` or matching ids, get_robust_list(2) `CAP_SYS_PTRACE`, and both
read the target thread's (or the process's main thread's) real
credentials from its `LinuxProcess`, which keeps the last thread's when a
process becomes a zombie.

**The VFS checks capabilities, not user 0.** `poly_vfs::Cred` gained a
`CapSet` (Linux's numbers, like its `Errno`): `CAP_DAC_OVERRIDE` and
`CAP_DAC_READ_SEARCH` in the access check as path_resolution(7) describes
them, `CAP_FOWNER` for chmod, explicit times, the sticky bit and
`O_NOATIME`, `CAP_CHOWN`, `CAP_FSETID` (set-group-ID kept on chmod and on
new files, set-id bits kept on writes and truncation), `CAP_MKNOD`,
`CAP_SYS_CHROOT` and `CAP_SYS_ADMIN` for mounts. New in the VFS:
`NodeRef::clear_setid` (a write or truncation by a caller without
`CAP_FSETID` clears set-user-ID, and set-group-ID when the group may
execute), used by `O_TRUNC`, `truncate` and, in the kernel, `write`,
`pwrite` and `ftruncate`; `Vfs::utimens`; `O_NOATIME`'s `EPERM`; and
utimensat(2)'s rule that only "both now" may be done with mere write
access. A personality without capabilities gives its superuser
`CapSet::ALL`; `Cred::root()` still means "the kernel itself".

**The kernel asks the running thread.** `fs::Files` no longer stores a
`Cred`: `Files::cred()` builds one from the calling thread's file system
ids, groups and effective capabilities (`linux::cred::current_vfs_cred`),
and the kernel's own threads (shell, tests, the initramfs) are root. So
every file operation, whoever added it, checks as the thread that asked.
The file calls take one path, ADR 0043's `LinuxFs`: chown and utimensat
became `LinuxFs::fchownat`, `fchown` and `utimensat` beside its `fchmodat`
and `umask`, and its `accessat(.., effective, ..)` picks the ids in the
kernel: access(2) checks with the real ids ("for the root user ... the set
of permitted capabilities ...; for non-root users, ... an empty set"),
faccessat2's `AT_EACCESS` with the effective ones (`Creds::access_ids`,
`Creds::fs_ids`). Privilege checks elsewhere in the personality use
capabilities too (`CAP_SYS_RESOURCE` for prlimit, `CAP_SYS_TIME` for
setting clocks), and `/proc/<pid>/status` shows the main thread's four
user and group ids.

**execve transforms in two steps.** Before loading, the kernel asks
`Creds::exec` for the new credentials from the executable's owner, group,
mode and mount (`nosuid`): set-user-ID and set-group-ID (the latter only
with group execute, since without it the bit marks mandatory locking,
inode(7)) unless `no_new_privs` or `nosuid`; saved and file system ids
from the effective ones; `keep_caps` cleared; capabilities by
capabilities(7)'s formula with empty file sets, except that for a real or
effective user 0 the file sets count as full (and the effective bit as set
for an effective 0). The ids go into the auxiliary vector (`AT_UID`...,
`AT_SECURE` when real and effective ids differ); the credentials and the
thread name (the program's file name, cut to 15 bytes) are installed past
the point of no return. For `#!` scripts the interpreter's bits count.

**System calls.** setuid, setgid, setreuid, setregid, setresuid,
setresgid, getresuid, getresgid, setfsuid, setfsgid, getgroups, setgroups,
capget, capset (versions 1 to 3; an unknown version gets the preferred one
written back with `EINVAL`), prctl (`PR_SET`/`GET_KEEPCAPS`,
`PR_CAPBSET_READ`/`DROP`, `PR_SET`/`GET_NO_NEW_PRIVS`, `PR_CAP_AMBIENT`,
`PR_GET_SECUREBITS`, `PR_SET`/`GET_NAME`, which `/proc/<pid>/comm` shows)
in `sys/cred.rs` and `sys/prctl.rs`; chown, fchown, lchown, fchownat and
utimensat (with the null-path form futimens(3) uses) in `sys/perm.rs`,
registered with ADR 0043's file system calls. access, faccessat,
faccessat2, chmod, fchmod, fchmodat and umask are ADR 0043's handlers.

**A user database is user space's.** The default initramfs carries a
minimal `/etc/passwd` and `/etc/group` (root and nobody, passwd(5) and
group(5) formats) for C libraries and BusyBox; the kernel never reads them.

## Consequences

- Host tests cover the rules in `cred` and `poly-vfs`, and every call
  through the dispatcher against the fake kernel; ring-3 kernel tests drop
  root and lose access to a 0600 file, run a set-user-ID-root program (and
  the same with `no_new_privs`, and without the bit), lose and regain
  capabilities across execve, keep the permitted set with `keep_caps`,
  meet the sticky `/tmp`, chown and chmod rules and a write clearing
  set-group-ID, and inherit credentials through fork while a thread's
  changes stay its own.
- Each file operation costs a lookup of the calling thread's credentials
  (the process's spinlock, an `Arc` clone), and each write to a regular
  file one `getattr` to see whether set-id bits need clearing.
- Code that kept a `Cred` field in `Files` now calls `Files::cred()`;
  `LinuxFs` implementations implement `fchownat`, `fchown` and
  `utimensat`.
- Not done yet: user namespaces (every id is valid but `-1`), file
  capabilities (`security.capability`), securebits beyond `keep_caps`
  (`PR_SET_SECUREBITS` is `EINVAL`), `PR_SET_DUMPABLE` and the
  parent-death signal reset at a set-user-ID exec, `fchmodat2`, the
  `protected_hardlinks`/`protected_symlinks` policies, `RLIMIT_NPROC` on
  setuid, and updating the process tree's uid (`si_uid` of `SIGCHLD`) when
  a child changes its real uid.
- Orders the man pages leave open are marked "to be confirmed by the diff
  runner" in the code: setgroups(2)'s `EINVAL`/`EPERM`/`EFAULT` order,
  capget(2) with a good version and a null data pointer (0), the ambient
  set under `keep_caps`, and capset(2) ignoring bits of capabilities that
  do not exist.
