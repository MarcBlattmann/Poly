# 0121: futex2, process access, namespaces, seccomp filters, protection keys without PKU, and the rest of `spec-linux-d`

**Status:** Accepted. Delivers the Linux syscall entries handed from
`spec-linux-d` to `impl-linux-d` (`specs/HANDOFF_LOG.md`). Refines
ADR 0031 (futexes), ADR 0054 (seccomp, namespaces) and ADR 0064 (memory
locks).

## Context

The 43 spec sheets of this hand-off cover the futex2 interface
(`futex_wait`, `futex_wake`, `futex_waitv`, `futex_requeue`),
`epoll_pwait2`, credentials and signal odds (`getgroups`, `getresgid`,
`setregid`, `setfsgid`, `tkill`, `rt_tgsigqueueinfo`, `pause`,
`restart_syscall`, `personality`, `execveat`), file calls (`fadvise64`,
`readahead`, `sync_file_range`, `mknod`, `mknodat`, `openat2`,
`fchmodat2`, `futimesat`, `chroot`), memory and scheduling (`mlock2`,
`mlockall`, `munlockall`, `pkey_alloc`, `pkey_free`, `pkey_mprotect`,
`sched_getattr`, `sched_setattr`, `ioprio_get`, `ioprio_set`), access to
other processes (`kcmp`, `pidfd_getfd`, `process_vm_readv`,
`process_vm_writev`) and isolation (`sethostname`, `setdomainname`,
`unshare`, `setns`, `seccomp`). The inputs were the spec sheets, their
visible tests, the man pages and the UAPI headers (CLEAN_ROOM.md).

## Decision

### futex2 shares the futex(2) queues

- **One table.** The futex2 calls (`personalities/linux/src/sys/futex2.rs`)
  use the wait queues of futex(2) (`crate::futex::FutexTable`): a futex2
  wake finds classic `FUTEX_WAIT` / `FUTEX_WAIT_BITSET` waiters of the
  same kind and word and the other way round, as the spec sheets require.
  `FUTEX2_PRIVATE` maps onto the private key, no flag onto the shared key
  (the memory object and offset for shared mappings, so two mappings of
  one memfd page name one futex).
- **A waiter may wait on several keys.** A queue entry now carries a
  slot (the `futex_waitv` index; 0 for a single wait). `enqueue_all`
  queues one thread on every entry at once; the first wake of any entry
  takes the thread off all of them and records the slot, which
  `futex_waitv` returns. A requeue moves only the entry of the source
  futex and updates that slot's key, so a later wake of the target
  returns the entry's original index.
- **Requeue onto itself** moves nobody and returns the number of
  waiters it selected (futex_requeue's spec sheet; the visible tests use
  `FUTEX_CMP_REQUEUE` onto the same word to count sleepers). The classic
  `FUTEX_CMP_REQUEUE` shares the rule.
- **Checks in the spec sheets' order.** flags (size `FUTEX2_SIZE_U32`,
  only `PRIVATE`, `NUMA`, `MPOL`), then the clock (only with a timeout:
  `CLOCK_MONOTONIC` or `CLOCK_REALTIME`), the 32-bit width of values,
  the timeout, the mask and alignment, the address, the value, the
  deadline. Deadlines are absolute, so an interrupted wait restarts as
  the same call after an `SA_RESTART` handler (`EINTR` otherwise).
  `futex_wake` on a private futex does not look at memory (NULL with
  `FUTEX2_PRIVATE` returns 0); a negative count wakes one.
- **One NUMA node.** With `FUTEX2_NUMA` the futex and the 32-bit node
  word after it form an 8-byte aligned pair; node `-1` is written back
  as node 0, any node but 0 is `EINVAL`. `FUTEX2_MPOL` changes nothing.

### epoll_pwait2 with a zero timeout only polls

With a `{0, 0}` timeout the temporary signal mask is installed and put
back without delivering anything, which is the same as not installing
it: a pending signal the mask would unblock stays pending and the call
returns 0 or the ready count. A call that may sleep still returns
`EINTR` after the handler ran. Its arguments are checked in the spec
sheet's order: the timeout, `sigsetsize`, the mask, whether `epfd` is
open (asked of the descriptor table, so `EBADF` comes before a bad
`maxevents`), `maxevents`, the range `events + maxevents * 12` within
user space (`EFAULT`), then the kernel's epoll check (`EINVAL`).

### Shared directories, and what tracers and waiters see

- **`CLONE_FS` between processes shares** the root, working directory
  and umask (`Files` holds them behind a shared reference, like the
  descriptor table of `CLONE_FILES`); unshare(2) `CLONE_FS`, and
  `CLONE_NEWNS` and `CLONE_NEWUSER`, which imply it, give the caller a
  private copy. `KCMP_FS` compares that reference.
- **A call seccomp kills still reports a result** to the trace: the
  registers are as at entry (`rax` the call's number), as after
  `SECCOMP_RET_TRAP`.
- **wait4 / waitid in a pid namespace** report the child's number there,
  taken while the child is still in the process tree (reaping frees the
  number, which used to make the result 0).

### Access to other processes: one rule, kernel object identities

- **One access rule** serves `kcmp`, `pidfd_getfd`, `process_vm_readv`
  and `process_vm_writev` (`personalities/linux/src/sys/procaccess.rs`):
  the caller's own process always; another process if the caller's real
  uid and gid equal all three of its uids and gids and it is dumpable, or
  with `CAP_SYS_PTRACE`. Poly does not tell ptrace(2)'s "read" access from
  "attach" access and has no Yama-like restriction. Processes are looked
  up in the caller's pid namespace; a thread id names its process.
- **kcmp** compares identities the kernel hands out
  (`LinuxKernel::kcmp_id`): the addresses of the open file description,
  the address space, the descriptor table, the file system context and
  the signal state. Only "equal or not" is meaningful (the man page says
  the ordering is not); `KCMP_IO` and `KCMP_SYSVSEM` always compare
  equal. `KCMP_EPOLL_TFD` finds the n-th registration of a descriptor
  number in an epoll set (`poly_vfs::Epoll::target`).
- **process_vm_readv / writev** copy in pieces that never cross a page
  boundary on either side, so a partial transfer ends exactly at the
  first byte that cannot be reached; the count so far is returned, or
  `EFAULT` when it is 0. The target's page protections apply (no forced
  writes to read-only memory).
- **A process is gone** for `pidfd_getfd` and `process_vm_*` once every
  thread of it has started to exit (`Process::is_ending`), not only once
  it is reaped: a parent that sees the child's `clear_child_tid` cleared
  gets `ESRCH`.

### Memory locks are marks on regions; protection keys without PKU

- **Locks.** Poly never pages memory out, so a lock (`poly_vm` region
  flag) keeps nothing in; it is recorded for `VmLck` in
  `/proc/<pid>/status`, for `RLIMIT_MEMLOCK` accounting, for
  `MCL_FUTURE` (new mappings, heap growth and the stack inherit it) and
  for fork (a child gets no locks and no future setting). Locking brings
  the pages in unless it is "on fault". Without `CAP_IPC_LOCK` a limit
  of 0 is `EPERM` and going over it `ENOMEM` (also `mlockall
  (MCL_CURRENT)` when the address space is larger); `mmap` under
  `MCL_FUTURE` beyond the limit is `EAGAIN`. Regions with different lock
  flags do not merge.
- **pkeys** behave as on a CPU without protection keys, which is what
  the oracle and QEMU's default CPU are: `pkey_alloc` checks its
  arguments then fails with `ENOSPC`, `pkey_free` is always `EINVAL`, and
  `pkey_mprotect` accepts only key `-1`, which makes it `mprotect`. The
  kernel's `mprotect` now rejects `PROT_GROWSUP`, and `PROT_GROWSDOWN`
  outside a stack region, with `EINVAL`.

### Scheduling attributes and I/O priorities are recorded, not honoured

`sched_setattr` / `sched_getattr` read and write the 56-byte `struct
sched_attr` with the spec sheets' size rules (`E2BIG` writes 56 back
into `size`; a larger buffer's tail is zeroed) and keep the deadline
parameters, the reclaim and overrun flags and a fair-policy time slice
per thread next to ADR 0113's policy and priority. There is no deadline
admission control (`EBUSY` never happens), utilisation clamping is
`EOPNOTSUPP`, and a deadline task without reset-on-fork cannot fork
(`EAGAIN`). `ioprio_set` / `ioprio_get` keep a 16-bit value per thread
(copied to children); for process groups and users a thread without a
class counts as best effort at level `(nice + 20) / 5` and the best
value wins. Neither changes what the scheduler or the block layer do.

### Namespaces: cgroup and time as identities, pins for open ns files

- **Two more kinds.** Cgroup and time namespaces exist as identities
  only (Poly has no cgroups and no clock offsets). Like pid, time is a
  "for children" namespace: `unshare(CLONE_NEWTIME)` and `setns` into a
  time namespace change `time_for_children`, and `pid_for_children` /
  `time_for_children` files are `ENOENT` until the new namespace has its
  first process. clone(2) still rejects the two flags.
- **unshare** checks the privilege before anything is unshared (a
  refused call leaves a shared descriptor table shared), allows
  `CLONE_FILES`, `CLONE_FS` and `CLONE_SYSVSEM` in a multithreaded
  process, requires a single thread for `CLONE_THREAD`, `CLONE_SIGHAND`
  and `CLONE_NEWUSER`, refuses `CLONE_VM` when the memory has other users
  (`LinuxKernel::memory_shared`) and a second `CLONE_NEWPID`.
  `CLONE_SYSVSEM` applies the semaphore adjustments at once.
- **Lifetime.** The namespace registry keeps weak references; an open
  `/proc/<pid>/ns/*` file holds a strong pin, counted through the VFS's
  open and release hooks, so a namespace lives as long as a file names
  it. A namespace file's `st_ino` is the namespace's inode number.
- **setns through a pidfd** joins the user namespace first and the
  others with the capabilities it gives, all or nothing (`ESRCH` for a
  reaped target); a time namespace in a multithreaded process is
  `EUSERS`, checked before the privilege.
- **Host and domain names** are also writable through
  `/proc/sys/kernel/hostname` and `domainname` (up to the first newline
  or NUL, at most 64 bytes, with sethostname(2)'s privilege), for the
  writer's UTS namespace.

### seccomp: the spec sheet's order, a listener that is never notified

seccomp(2) checks the operation, the flags (`WAIT_KILLABLE_RECV` needs
`NEW_LISTENER`; `TSYNC` with `NEW_LISTENER` needs `TSYNC_ESRCH`), then
reads and validates the program, then the privilege (`EACCES`), then
the listener (`EBUSY`). `SECCOMP_FILTER_FLAG_NEW_LISTENER` returns a
close-on-exec, `O_RDWR` descriptor shown as `anon_inode:seccomp notify`;
the flag lives on the filter, so a chain has one listener at most. No
notification is ever delivered on it (`SECCOMP_RET_USER_NOTIF` still
fails the call with `ENOSYS`); `USER_NOTIF` is reported as available
because the spec sheet's oracle reports it. After `SECCOMP_RET_TRAP`
the registers are as at entry, so a handler that returns makes the call
appear to return its own number.

### File calls: restricted path walks, checks after the lookup

- **openat2's `RESOLVE_*` restrictions live in the VFS path walker**
  (`poly_vfs::Resolve`, `Walk::restricted`): `IN_ROOT` makes the start
  directory the walk's root (also for absolute paths); `BENEATH`
  remembers the start directory and refuses an absolute path, an
  absolute link target or `..` there with `EXDEV`; `NO_SYMLINKS` and
  `NO_MAGICLINKS` are checked where every link is followed, so links in
  the middle, at the end and under `O_CREAT` are covered; `NO_XDEV`
  refuses to enter or climb out of a mount. `RESOLVE_CACHED` has no
  effect (every Poly lookup is "cached"), except that it refuses
  creating or truncating opens with `EAGAIN`. File systems say which of
  their links are "magic" (`FileSystem::is_magic_link`; procfs's
  `fd/<n>`, `exe`, `cwd`, `root`, `ns/<kind>`). The size rules of `struct
  open_how` follow the spec sheet (`EINVAL` below 24 bytes, `E2BIG` above
  a page or for non-zero bytes past the known structure).
- **Capabilities after the lookup.** For `mknod` and `chroot` the
  personality still decides the capability, but passes the decision
  down; the kernel drops the capability from the VFS credentials, so
  `ENOENT`, `EEXIST` and `EACCES` win over `EPERM` as in the spec sheets.
  `getcwd` shows a working directory outside the root as
  `(unreachable)` followed by its path from the topmost root.
- **Hints are checked, not acted on.** `readahead`, `sync_file_range`
  and `fadvise64` validate their arguments in the spec sheets' order and
  return 0: Poly's page cache takes no hints.
- `fchmodat2` honours `AT_SYMLINK_NOFOLLOW` (a symlink cannot have its
  mode changed: `EOPNOTSUPP`, before the ownership check) and
  `AT_EMPTY_PATH` (also on `O_PATH` descriptors).

### Exec, personality and signal odds

- **execveat** looks the program up before it reads `argv` and `envp`
  (a separate `exec_lookup` that changes nothing), names it
  `/dev/fd/N[/path]` for `AT_EXECFN`, a script's argument and `comm`
  when reached through a descriptor, and fails with `ENOENT` for a
  script reached through a close-on-exec descriptor (the interpreter
  could not open it). The VFS counts writers per inode, so executing a
  file open for writing is `ETXTBSY`, the mirror of the existing rule for
  writers. `AT_EXECFN` and `comm` now come from the requested name for
  every exec.
- **personality(2)** is per thread, inherited by clone and fork, and
  kept by exec except `READ_IMPLIES_EXEC`. Only `PER_LINUX32` (uname's
  machine `i686`), `UNAME26` (release `2.6.(60 + minor)`) and
  `READ_IMPLIES_EXEC` (`PROT_READ` implies `PROT_EXEC` in `mmap` and
  `mprotect`) have effects.
- **pause** returns `EINTR` after a handler even with `SA_RESTART` and
  sleeps on through stops and ignored signals. A restart block now lives
  until `rt_sigreturn`, so `restart_syscall` called inside a handler
  resumes the interrupted relative sleep or futex wait.
- **tkill / rt_tgsigqueueinfo** check in the spec sheets' order; a
  thread waiting in `rt_sigtimedwait` for a thread-directed signal is
  now woken by it.

## Gaps

- A `CLONE_SIGHAND` child that is a separate process gets a copy of the
  signal handlers, not the same table: kcmp(2) reports them shared (the
  child takes its creator's `LinuxProcess::sighand` number, execve(2)
  gives a fresh one), but a sigaction(2) in one is not seen by the other.
- No seccomp notification is ever delivered; no deadline admission
  control; pkeys are never allocatable (as without PKU).

## Consequences

- futex2 programs (Wine's fsync, newer C libraries) find their calls,
  and they interoperate with classic futex users on the same words.
- Every entry of the hand-off whose visible tests pass on Poly is green
  in `coverage/linux/syscall/x86_64.json`; the held-out tests decide the
  rest.
