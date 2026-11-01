# 0113: Extended attributes in the spec sheets' order, per-thread scheduling attributes with real-time policies recorded, pipes that keep their buffers, System V message queues

**Status:** Accepted. Delivers 35 Linux syscall entries handed from
`spec-linux-c` to `impl-linux-c` (`specs/HANDOFF_LOG.md`): the twelve
extended attribute calls, System V IPC (`semget`, `semctl`,
`semtimedop`, `msgget`, `msgsnd`, `msgrcv`, `msgctl`, `shmctl`, `shmdt`),
`splice`, `tee`, `vmsplice`, `sendmmsg`, `recvmmsg` and the scheduling
calls (`sched_*param`, `sched_*scheduler`, `sched_get_priority_max/min`,
`sched_rr_get_interval`, `getpriority`, `setpriority`). Refines ADR 0059
(xattr flags), ADR 0051/0053 (priorities) and ADR 0012 (the scheduler
still ignores priorities).

## Context

The spec sheets of these calls pin down behaviour Poly had either
guessed differently or not implemented: the order in which extended
attribute calls report their errors, the bare prefixes (`user.`), both
`XATTR_CREATE` and `XATTR_REPLACE` together, pipes and sockets as
targets; per-thread nice values and scheduling policies with the
permission rules of sched(7); splice and tee between pipes, where a
pipe's number of buffers bounds a transfer; System V message queues,
which Poly did not have. The inputs were the spec sheets, their visible
tests, the man pages and the UAPI headers (CLEAN_ROOM.md).

## Decision

### Extended attributes

- **Order.** The personality checks the flags, the name, the size and
  the value before it looks at the file (a bad name with a NULL path or
  a bad descriptor is `ERANGE`). The VFS (`poly_vfs::xattr`) then checks,
  for changes: `EROFS`; `EPERM` (`trusted.` without `CAP_SYS_ADMIN`,
  `user.` on anything but a regular file or directory, a sticky
  directory the caller does not own); `EACCES` (write permission, for
  `user.` names and unknown prefixes); `EPERM` (`security.` without
  `CAP_SYS_ADMIN`); `EOPNOTSUPP` (unknown prefix, a `system.` name other
  than the ACLs); `EINVAL` (a bare prefix); `EEXIST` / `ENODATA`. Reads
  follow the same pattern with `ENODATA` for hidden `trusted.` names and
  for `user.` names on special files, and read permission.
  `poly_vfs::xattr::Name` classifies a name without failing on an
  unknown or bare prefix, so those errors come in their place.
- **Both flags** are valid and fail either way (`EEXIST` if the name
  exists, `ENODATA` if not): ADR 0059's `EINVAL` was a guess the spec
  sheet corrected.
- **File objects** (pipes, sockets, event counters: the anonymous file
  system's inodes) keep no attributes: `user.` names are `EPERM` /
  `ENODATA` whatever kind `fstat` reports, other names `EOPNOTSUPP`, and
  they list nothing. A Unix socket descriptor has one read-only
  attribute, `system.sockprotoname` (`UNIX-STREAM`, `UNIX`), answered by
  the personality.
- **Sizes.** tmpfs no longer limits the names of one inode to
  `XATTR_LIST_MAX` bytes; listxattr returns the full length for size 0,
  `E2BIG` for a buffer of 64 KiB or more that is still too small, and
  `ERANGE` for a smaller one. Buffer sizes above the limits count as the
  limits.

### Scheduling attributes

- The nice value, the policy, the static priority and
  `SCHED_RESET_ON_FORK` are **per thread** (`task::sched::Sched` in
  `LinuxThread`; `LinuxProcess::nice` is gone). New threads and fork
  children copy their creator's; `SCHED_RESET_ON_FORK` resets a
  real-time policy to `SCHED_OTHER` with nice 0 and a negative nice to 0.
  A zombie keeps its last thread's values, so it can still be read.
- **Real-time policies are accepted and recorded** (`SCHED_FIFO`,
  `SCHED_RR` with priorities 1 to 99) under sched(7)'s permission rules:
  `CAP_SYS_NICE` (initial user namespace) for anything; otherwise the
  caller's effective uid must be the target's real or effective uid and
  the caller must hold the target's permitted capabilities, real-time
  priorities stay within `RLIMIT_RTPRIO`, leaving `SCHED_IDLE` and lower
  nice values within `RLIMIT_NICE`, and `SCHED_RESET_ON_FORK` cannot be
  cleared. Before, Poly refused real-time policies with `EPERM` because
  its round-robin scheduler cannot honour them; it still cannot (ADR
  0012), but the spec sheets show programs (and tests) that only set and
  read the values back, and refusing breaks them. Honouring priorities
  is the scheduler's work for later; the recorded values are what it
  will read.
- `sched_rr_get_interval` reports 100 ms under `SCHED_RR` and 0 under
  every other policy (the spec sheet allows 0 for the fair policies).

### Pipes keep their buffers; splice, tee, vmsplice

- A pipe's pages remember whether a later write may add to them. Pages
  that splice, tee or vmsplice put in stay buffers of their own, so the
  free-buffer count (16 in a new pipe) bounds what one call moves, as on
  Linux. `poly_vfs::pipe::Pipe::transfer` moves (splice) or copies (tee)
  whole buffers or their prefixes between two pipes, locking both in
  address order.
- splice from a pipe into a file or socket peeks, writes, then consumes,
  so an error of the output leaves the bytes in the pipe; into a pipe
  from a file, memory device or socket at most the free pages' bytes go.
- **vmsplice copies** the caller's memory at the call. Linux's pipe
  refers to the caller's pages until they are read (a change before the
  read is seen); Poly's pipes hold bytes, and referring to user pages
  would need page pinning in the memory manager. Known difference.

### Sockets: sendmmsg, recvmmsg

- Per entry: read the header, send or receive as sendmsg/recvmsg, store
  `msg_len`; an error after the first entry ends the call with the
  count. recvmmsg keeps such an error on the socket for its next
  receive (`LinuxNet::keep_error`, implemented for AF_UNIX), and an
  interruption by a signal after a message as error 512, which is what
  the spec sheet observed user space getting.

### System V IPC

- **Message queues** (`ipc::msg`, handlers in `sys::msg`) belong to the
  IPC namespace beside semaphore sets and segments. Like semaphore
  sleepers, a sleeping sender or receiver never retries: **the caller
  that changes the queue completes it.** A message sent while a matching
  receiver sleeps goes straight to it and never counts in `msg_qnum`; a
  receiver whose buffer is too small wakes with `E2BIG` and the message
  is queued; a receive or an `IPC_SET` that raises `msg_qbytes` sends
  the messages of sleeping senders that now fit, re-checking their write
  permission; `IPC_RMID` fails every sleeper with `EIDRM`. `msg_qbytes`
  bounds both the bytes and the number of messages. The message is
  copied out after it has left the queue, so `EFAULT` loses it.
- **semtimedop** shares semop's code; the deadline is on the monotonic
  clock and an expired (or `{0,0}`) one never sleeps (`EAGAIN`).
- **`IPC_INFO`, `*_INFO`, `*_STAT`, `*_STAT_ANY`** for semaphores,
  segments and queues: the "index" is `id & 0x7fff` with Poly's
  increasing ids, `IPC_INFO` returns the highest index in use. The
  `IPC_64` bit in a command is `EINVAL` (the spec sheets' bad-command
  rule), no longer ignored.
- **`SHM_LOCK` / `SHM_UNLOCK`** only set and clear `SHM_LOCKED`: Poly
  never swaps. Without `CAP_IPC_LOCK` the segment must fit in the
  caller's `RLIMIT_MEMLOCK` (compared with this segment's size, not with
  everything the user has locked).

## Consequences

- The xattr checks now live in one place in the VFS in the documented
  order; file systems only store.
- Code that read `LinuxProcess::nice` reads the thread's `sched.nice`.
- The scheduler can later honour policies and nice values from the
  per-thread state without changing the system call layer.
- Known differences, recorded in the module docs: vmsplice copies; the
  kept-error mechanism exists only for AF_UNIX sockets.
