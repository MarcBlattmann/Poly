# 0111: inotify on the VFS, memfd with seals, pidfds, rseq, membarrier, shared descriptor tables and the prctl odds

**Status:** Accepted. Supersedes the `rseq` decision of
[ADR 0043](0043-dev-proc-sys-and-file-system-calls.md) and
[ADR 0049](0049-dynamic-linking-program-interpreter.md) ("`rseq` stays
`ENOSYS`"): the owner approved implementing it as the spec sheet describes.

## Context

Spec agent `spec-linux-a` handed over 17 Linux system calls
(`specs/HANDOFF_LOG.md`): `memfd_create`, `inotify_init1`, `inotify_init`,
`inotify_add_watch`, `inotify_rm_watch`, `rseq`, `set_robust_list`,
`get_robust_list`, `membarrier`, `getcpu`, `sched_getaffinity`,
`sched_setaffinity`, `close_range`, `clone3`, `pidfd_open`,
`pidfd_send_signal` and `prctl`. Several already worked (robust lists,
affinity, `getcpu`, most of `close_range`, `clone3` without `CLONE_PIDFD`,
most of `prctl`); the others needed new pieces: inotify on the VFS, memfd
files with seals, pidfds, restartable sequences, membarrier registrations,
descriptor tables shared between processes, and a few `prctl` options.
The implementer (`impl-linux-a`) worked from the spec sheets, their visible
tests, the man pages and the uapi headers (`linux/inotify.h`,
`linux/memfd.h`, `linux/fcntl.h`, `linux/rseq.h`, `linux/membarrier.h`,
`linux/sched.h`, `linux/pidfd.h`, `linux/prctl.h`) only (CLEAN_ROOM.md).

glibc registers an `rseq` area for every thread it starts, so Poly's
`ENOSYS` was the one remaining difference of the `libc-hello` corpus test
(ADR 0049 kept it because registering without ever aborting would break
the contract the first time a thread is preempted inside a critical
section).

## Decision 1: inotify is a VFS service

inotify lives in the host-tested `poly_vfs::inotify`. An instance
(`Inotify`) is a `FileObject`: a queue of events and its watch numbers,
which start at 1 per instance and are never handed out again. One registry
maps a watched inode (file system identity and inode number) to the watches
of every instance; watches do not keep their inode alive, and a global
count lets every hook return at once while nothing is watched.

The VFS reports what happens, so every personality's calls produce the
events:

- `open`/`O_CREAT`, `mkdir`, `mknod`, `symlink` and `link`: `IN_CREATE`
  (`IN_ISDIR` for directories); `link` also `IN_ATTRIB` on the inode;
- `unlink`/`rmdir`: `IN_ATTRIB` if the inode lives on, then `IN_DELETE`;
- `rename`: `IN_MOVED_FROM`/`IN_MOVED_TO` with one non-zero cookie, and
  `IN_MOVE_SELF`;
- open files: `IN_OPEN`; reads and writes of `OpenFile` `IN_ACCESS` /
  `IN_MODIFY`; `ftruncate`/`truncate` `IN_MODIFY`; the last close
  (`OpenFile`'s drop) `IN_CLOSE_WRITE` or `IN_CLOSE_NOWRITE`;
- attribute changes (chmod, chown, utimens): `IN_ATTRIB`;
- an inode freed by unlink, rmdir, a replacing rename or the last
  `NodeRef` (an unlinked file closed for the last time): its watches end
  with `IN_DELETE_SELF` (if asked for) and `IN_IGNORED`.

An event goes, without a name, to the inode's own watches, and with the
entry's name to the watches of the directory it was found in (the open
file's origin or the looked-up parent). Reads return whole records (16-byte
header, names NUL-padded to a multiple of 16); a buffer too small for the
next record is `EINVAL` and consumes nothing; an empty queue is `EAGAIN` or
a wait on the instance's `WaitQueue`; an event equal to the last queued one
is merged; after 16384 queued events one `IN_Q_OVERFLOW` ends the queue.
`IN_ONESHOT`, `IN_MASK_ADD` and `IN_MASK_CREATE` (`EEXIST`) are the
instance's. The personality's `sys::inotify` checks the mask, then the
descriptor, then the path; the kernel's `FileIo` looks the path up
(`IN_DONT_FOLLOW`), checks read permission (`EACCES`) and `IN_ONLYDIR`
(`ENOTDIR`). A new `FileObject::anon_name` gives the `/proc/<pid>/fd` link
(`anon_inode:inotify`) and makes `fstat` report the mode without file type
bits (0600), as the spec observed; `FIONREAD` now asks file objects first.
Not done: `IN_EXCL_UNLINK` has no effect, no `IN_UNMOUNT`, no
`max_user_watches`/`max_user_instances` limits; `fchmod`-style changes
through a descriptor notify only the inode's own watches.

## Decision 2: memfd files are unnamed tmpfs files with seals

The personality's `sys::memfd` checks the flags (any bit outside 0x1f, a
huge page size without `MFD_HUGETLB`, `MFD_EXEC | MFD_NOEXEC_SEAL`:
`EINVAL`), then the name (read into a 250-byte buffer: no NUL in it is
`EINVAL`, an unreadable byte `EFAULT`), then asks `LinuxFs::memfd_create`
for the file with its mode and first seals (0777 and `F_SEAL_SEAL`; 0777
and none with `MFD_ALLOW_SEALING`; 0666 and `F_SEAL_EXEC` with
`MFD_NOEXEC_SEAL`). The kernel makes it with `Vfs::memfd_file`: the
`O_TMPFILE` path (`create_tmpfile`) on a page-cache tmpfs mounted where no
path reaches it (`Vfs::detached_mount`, made at the first call), so the file
is an ordinary regular file without links: `MAP_SHARED`, `dup` and fork
share it as they share any tmpfs file. It is open `O_RDWR | O_LARGEFILE`,
ignores the umask and can never be named. `poly_vfs::memfd::Memfd` holds
the name and the seals and travels with the `OpenFile` every descriptor and
mapping shares; `Vfs::file_path` names it `/memfd:<name> (deleted)`. The
seal rules (fcntl(2) `F_ADD_SEALS` 1033 / `F_GET_SEALS` 1034, `EINVAL` on
other files) are enforced in `OpenFile::write`, `write_at`, `truncate` and a
new `OpenFile::chmod`, so every path is covered: `F_SEAL_WRITE` /
`F_SEAL_FUTURE_WRITE` refuse writes and new writable shared mappings,
`F_SEAL_GROW` growing truncates and writes past the end, `F_SEAL_SHRINK`
shrinking truncates, `F_SEAL_EXEC` a change of an execute bit,
`F_SEAL_SEAL` any further seal. Writable shared mappings are counted by a
guard each mapping's `FileSource` holds (split regions and a fork child's
copies share it), so adding `F_SEAL_WRITE` is `EBUSY` while one exists.
Not done: `MFD_HUGETLB` gives an ordinary memfd; `mprotect` adding
`PROT_WRITE` to a read-only shared mapping of a write-sealed file is not
refused; reopening through `/proc/self/fd/N` loses the seals;
`vm.memfd_noexec` is always 0.

## Decision 3: pidfds are file objects; `clone3` makes them too

A pidfd is a kernel `FileObject` (`kernel/src/linux/pidfd.rs`, `PidFd`)
naming a process by its global id (with `PIDFD_THREAD`, one thread).
`process::alloc_pid` never hands an id out twice, so the id alone keeps
naming the process after it was reaped. It sits on an anonymous inode with
mode 0700, open `O_RDWR` (`O_NONBLOCK` with `PIDFD_NONBLOCK`, `O_EXCL`
reported for thread pidfds) and always close-on-exec; its `anon_name`
`[pidfd]` gives the link `anon_inode:[pidfd]` and a mode without type bits.
It is readable once its process is a zombie or gone (`task::exited_or_gone`);
every change of the process tree and every thread exit notifies one wait
queue all pidfds share, as signal files do.

The personality's `sys::pidfd` checks `pidfd_open`'s flags, then the pid
(`EINVAL` for 0 or negative), then looks it up: a process in the tree
(alive or a zombie) works, another existing thread is `ENOENT` unless
`PIDFD_THREAD`, nothing is `ESRCH`. `pidfd_send_signal` checks the flags
(0, `PIDFD_SIGNAL_THREAD`, `_THREAD_GROUP`, `_PROCESS_GROUP`), the
descriptor (`FileIo::pidfd_target`: a pidfd, or a `/proc/<pid>` directory
that `poly_procfs::process_dir` recognises; anything else `EBADF`), the
signal number, the siginfo (`EFAULT`, `si_signo` mismatch `EINVAL`,
`si_code >= 0` or `SI_TKILL` to another process `EPERM`), and then sends
through the same `deliver::send` as kill(2), so a zombie accepts signals
without effect and a reaped process is `ESRCH`; `_PROCESS_GROUP` needs the
target to lead its group. waitid(2) `P_PIDFD` waits for the pidfd's child
(`EBADF`, `EINVAL` for another kind of file, `ECHILD` once reaped).
`CLONE_PIDFD` is accepted: clone3 stores the new pidfd at `args.pidfd`,
clone(2) at `parent_tid` (so not together with `CLONE_PARENT_SETTID`);
`CLONE_DETACHED` or `CLONE_THREAD` with it are `EINVAL`, and clone3 refuses
`CLONE_DETACHED` alone. The kernel installs the pidfd after the child's
table was copied and stores it after its memory was, both before the child
runs; a failed store fails the clone with `EFAULT` and leaves nothing
behind. Not done: thread pidfds' readiness is approximate (ended when the
thread is gone), `set_tid` is still `EINVAL`, no `pidfd_getfd` or pidfd
ioctls.

## Decision 4: rseq aborts on the way back to user mode

The registration is per thread, in the personality
(`LinuxThread::rseq: Option<Registration { area, len, sig }>`): a fork
child keeps its parent thread's, the kernel clears it for a `CLONE_VM`
child that is not a thread, new threads start without one and execve drops
it. `sys::rseq::check` (host-tested) takes `len`, `flags` and `sig` as 32
bits and decides in the spec's order: flags other than 0 and
`RSEQ_FLAG_UNREGISTER` are `EINVAL`; while registered, another area or
length is `EINVAL`, another signature `EPERM`, the same again `EBUSY` (or
the unregistration); unregistered, unregistering is `EINVAL`, a length
below 32 or an area not 32-byte aligned `EINVAL`, an unreadable area
`EFAULT`. Registering writes the current CPU to `cpu_id_start` and
`cpu_id` and 0 to `node_id` and `mm_cid`; if the area cannot be written
the call still returns 0 and the thread gets `SIGSEGV` on its way back.
Unregistering writes `cpu_id_start` 0 and `cpu_id` 0xffffffff
(`RSEQ_CPU_ID_UNINITIALIZED`).

The abort (`sys::rseq::fixup`) reads `rseq_cs`; if it is not 0 it reads the
`struct rseq_cs` (version and flags must be 0, the section must not wrap
and `abort_ip` must lie outside it), and if the instruction pointer is in
`[start_ip, start_ip + post_commit_offset)` the four bytes before
`abort_ip` must equal the registered signature (else `SIGSEGV`) and the
thread continues at `abort_ip`. `rseq_cs` is always cleared and the CPU
fields refreshed; any user access that faults is `SIGSEGV`. It runs

- in `signal::deliver::exit_to_user` before every signal frame is built,
  so a handler returns to `abort_ip`;
- through `Linux::rseq_resume` when a registered thread returns to user
  mode after it was switched out: `thread::switch` calls
  `UserThread::switched_out`, which sets the thread's `rseq_notify` flag
  only if `ThreadFlags::rseq` says it is registered (set through
  `LinuxKernel::rseq_registered`; threads without a registration pay one
  relaxed load per switch), and `return_to_user` and the system call exit
  path call `rseq_resume` while the flag is set, before signals.

So a preempted, migrated or signalled thread never resumes inside a
critical section, which is the contract ADR 0049 did not want to break.
`mm_cid` is always 0 (the spec saw it only in a single-threaded process),
and a system call return inside a critical section is aborted like any
other return instead of being treated as a bug. The boot tests (`dynamic`,
`rootdisk`) no longer tolerate an `rseq` "not implemented" line, and
`libc-hello` (glibc registers `rseq` for its main thread) is in
`tests/linux/poly-pass.txt`. App mode still leaves `rseq` out of its
comparison (`APP_DROPPED_CALLS`), which changes nothing now that both sides
answer 0.

## Decision 5: membarrier registrations live with the memory

`sys::membarrier` (host-tested) decides a command: `QUERY` answers every
command Poly has (0x3ff), anything that is not one known command is
`EINVAL`, flags must be 0 except `MEMBARRIER_CMD_FLAG_CPU` with
`PRIVATE_EXPEDITED_RSEQ` (checked before the registration), and the three
private commands need their `REGISTER_` command first (`EPERM`).
Registrations belong to the memory, as the man page says ("the process"):
the kernel's `Memory` keeps them as bits, so every task that shares it
(`CLONE_VM`, threads) sees them, `Memory::fork` copies them and execve,
which makes a new memory, starts without. A private barrier sends a TLB
request for page 0, which no user program maps, to every CPU that has the
address space loaded and waits for the acknowledgements; the global one
does the same for the kernel's mappings on every CPU. Taking an interrupt
and returning from it is serializing, so every CPU running the memory has
ordered its accesses before the call returns.

## Decision 6: `CLONE_FILES` shares the descriptor table between processes

A process's `Files` held its descriptor table directly, so `CLONE_FILES`
without `CLONE_THREAD` copied it (ADR 0041's "not done yet"). `Files` now
holds a reference to the table (`SpinLock<Arc<Mutex<FdTable>>>`, cloned for
each use so no lock is held across a call): `Files::share_table` gives a
`CLONE_FILES` child the same table, `Files::unshare_table` replaces it by a
private copy (close_range(2) `CLOSE_RANGE_UNSHARE`, unshare(2)
`CLONE_FILES`, through `LinuxFs::unshare_fd_table`), and `close_all` at a
process's end only drops its reference while others still use the table.
Record locks are owned by the table (fcntl(2): locks belong to the
process, and processes sharing a table share them); the last process to
let go of a table releases its locks under the table's id. A table shared by the
threads of one process still cannot be split (`EINVAL`, as before).

## Decision 7: the rest of `prctl`, `/proc/<pid>/task`, the small fixes

- `PR_GET_DUMPABLE` / `PR_SET_DUMPABLE` (0 or 1): a per-process flag
  (`LinuxProcess::dumpable`), inherited by fork, set again by execve. It
  takes part in the ptrace access check `get_robust_list` makes (same user
  *and* dumpable, or `CAP_SYS_PTRACE`).
- `PR_SET_PDEATHSIG` / `PR_GET_PDEATHSIG` (0..=64): per thread, not
  inherited. When a process ends, every thread of its children that set
  one gets the signal (from the kernel), after the children were
  reparented. Linux sends it when the creating *thread* exits; Poly sends
  it when the parent process does, which is the same for single-threaded
  parents.
- `/proc/<pid>/task/<tid>/comm` (procfs `ProcSource::threads` and
  `thread_comm`): each thread's own name (`PR_SET_NAME`). A task directory
  holds only `comm` so far.
- `get_robust_list` of a zombie reads NULL (the list was cleared at its
  exit) instead of `ESRCH`.
- `getcpu` stores `cpu` and `node` independently: an unwritable one is
  `EFAULT`, but the other is still written.
- `sched_setaffinity` reads the mask first (`EFAULT` even for a missing
  thread), then looks the thread up (`ESRCH`, also for an empty mask), then
  judges the mask (`EINVAL`); both affinity calls take `cpusetsize` as
  32 bits.

## Consequences

- All 45 visible tests of the 17 entries pass on Poly; the entries are
  green in the coverage map, so `pipeline polycheck` keeps their visible
  tests passing in CI.
- New host tests: `poly_vfs` (inotify queue, watches and events through a
  real VFS; memfd seals), `sys::membarrier`, `sys::rseq`, `sys::pidfd`,
  `sys::memfd`, `prctl` dumpable and parent-death signal, procfs task
  directories.
- The kernel's `Memory` carries membarrier registrations; `Files` holds a
  shareable reference to its descriptor table; `ThreadFlags` carries the
  rseq flags; `Process` creation hands an inherited rseq registration to
  the fork child's thread.
- Follow-ups: `set_tid` in clone3, `pidfd_getfd`, `IN_EXCL_UNLINK`, inotify
  limits, `MFD_HUGETLB` as a hugetlbfs file, `mm_cid` for threaded
  processes, the full `/proc/<pid>/task/<tid>` directory, the
  parent-death signal per creating thread, and `CLONE_FS`/`CLONE_SIGHAND`
  sharing between processes.
