# 0122: Native AIO and io_uring run in the submitting thread, POSIX message queues, the new mount API over movable mounts, Landlock domains in the credentials, fanotify on inotify's hooks

**Status:** Accepted. Delivers 42 Linux syscall entries handed from
`spec-linux-e` to `impl-linux-e` (`specs/HANDOFF_LOG.md`): native AIO
(`io_setup`, `io_submit`, `io_getevents`, `io_destroy`, `io_cancel`,
`io_pgetevents`), io_uring (`io_uring_setup`, `io_uring_enter`,
`io_uring_register`), POSIX message queues (`mq_open`, `mq_unlink`,
`mq_timedsend`, `mq_timedreceive`, `mq_notify`, `mq_getsetattr`), the
mount API (`open_tree`, `move_mount`, `mount_setattr`, `fsopen`,
`fsconfig`, `fsmount`, `fspick`, `statmount`, `listmount`) and
`pivot_root`, file handles (`name_to_handle_at`, `open_by_handle_at`),
the `*xattrat` calls and inode flags (`getxattrat`, `setxattrat`,
`listxattrat`, `removexattrat`, `file_getattr`, `file_setattr`),
`fanotify_init`, `fanotify_mark`, Landlock (`landlock_create_ruleset`,
`landlock_add_rule`, `landlock_restrict_self`), `memfd_secret`,
`userfaultfd`, `mseal` and `cachestat`. Builds on ADR 0042 (file
objects), ADR 0051 (mount(2)), ADR 0054 (namespaces) and ADR 0113
(extended attributes).

## Context

These calls are the "newer" Linux interfaces: asynchronous I/O through
shared rings, a file-descriptor based mount API, unprivileged sandboxing,
file-system wide notification and memory sealing. Their spec sheets and
visible tests also need support around them: statx with
`STATX_MNT_ID_UNIQUE`, mount namespaces whose root can be pivoted,
mount attributes and propagation flags, a tmpfs `/dev/shm` with file
handles and inode flags, blocking user faults for `CLONE_VM` children,
and real Landlock enforcement of opens, TCP binds and signals. The
inputs were the spec sheets, their visible tests, the man pages, the
kernel's public user-space API documentation and the UAPI headers
(`linux/aio_abi.h`, `linux/io_uring.h`, `linux/mqueue.h`,
`linux/mount.h`, `linux/fs.h`, `linux/xattr.h`, `linux/fcntl.h`,
`linux/fanotify.h`, `linux/landlock.h`, `linux/userfaultfd.h`,
`linux/mman.h`) (CLEAN_ROOM.md).

## Decision

### Native AIO (`personalities/linux/src/aio`, `sys/aio.rs`)

- **The context id is the ring's address.** `io_setup` maps a shared
  anonymous read-write ring of whole pages with the `aio_abi.h` header
  (magic `0xa10a10a1`). As the spec sheet observed (383 slots for any
  request up to 128), the ring is sized for twice the larger of
  `nr_events` and 128; unreaped events plus pending polls may fill all
  slots but one, so a context of 1 takes 382 requests before `io_submit`
  says `EAGAIN` (a batch gets as many as fit). Events user space reaped
  by advancing the ring's head free their slots for the next submit. One
  system-wide table, keyed by (address space, ring address) and
  recording the owner pid, holds the kernel's head and tail and mirrors
  them into the header; it reads the user's head back before reaping,
  so user-space reaping (libaio) stays in step.
- **Teardown** on `io_destroy`, on a `munmap` overlapping the ring (a
  hook in the munmap handler), on `execve` and at process exit; address
  space ids are reused page-table frames, so stale contexts must go. A
  forked child has another address space and gets `EINVAL`.
- **Requests run in `io_submit`.** `PREAD`, `PWRITE`, `PREADV`,
  `PWRITEV`, `FSYNC` and `FDSYNC` reuse the read/write/fsync paths; I/O
  errors land in the event's `res`, submission errors follow the spec
  sheet's order. A `PREAD`/`PWRITE` buffer is range-checked at submit:
  `[buf, buf + min(count, MAX_RW_COUNT))` past the end of user space
  (`0x7ffffffff000`) is `EFAULT` with nothing queued; whether it is
  mapped shows only in the event. Only `IOCB_CMD_POLL` stays pending; it completes when
  the process next enters an AIO call, and `io_getevents` waits by
  polling the pending polls' descriptors (with `io_pgetevents`' mask
  installed like ppoll's). `IOCB_FLAG_RESFD` signals an eventfd.
- **Limits:** `aio-max-nr` 65536 events (`EAGAIN`), rings over 128 MiB
  `EINVAL`.

### io_uring (`personalities/linux/src/uring`, `kernel/src/linux/uring.rs`)

- **No worker thread.** Requests run in the thread that calls
  `io_uring_enter`, mostly by calling the matching system call handler,
  so errors are that call's own as `-errno` in `res`. A request on a
  stream that is not ready (pipe or socket read/write, `ACCEPT`, `RECV`,
  `RECVMSG`), `POLL_ADD` and `TIMEOUT` are parked and retried on every
  enter; `GETEVENTS` waits by polling the parked requests' descriptors
  until the earliest timeout (an expired `EXT_ARG` timeout is `ETIME`).
- **Completion without another enter.** Linux completes parked requests
  asynchronously, so a program may find the CQE in the ring right after
  the system call that made the file ready (`write` to the peer of a
  waiting `RECV`). Poly retries a process's parked requests on each of
  its threads' way back to user mode: after every system call, and after
  interrupts while anything is parked anywhere (a global count gates
  it). A parked request belongs to the process that submitted it (its
  descriptor numbers are that process's), and only that process's
  threads retry it in the background. The kernel watches each parked
  request's file (a waker on its wait queue): a wake-up counts and kicks
  the instance, which wakes the ring file's own wait queue; a kicked
  ring reads as readable (and `POLLPRI`, which `GETEVENTS` asks of the
  ring itself), so `poll`/`epoll` on the ring fd wake and the poller's
  way back to user mode posts the CQE. The ring file's queue is also
  woken when a CQE is posted. A multishot `POLL_ADD` fires once per
  wake-up of its file (also while the file stays ready), as Linux posts
  one CQE per wake-up. Kept-back CQEs still move into the ring only in
  io_uring_enter. `GETEVENTS` rechecks every 10 ms for CQEs other
  threads post.
- **Ring memory** is two kernel shared regions: the SQ and CQ rings,
  CQEs and SQ array together (`IORING_FEAT_SINGLE_MMAP`), and the SQEs;
  `mmap` of the ring fd at `IORING_OFF_SQ_RING`/`CQ_RING`/`SQES` maps
  them, and the mappings keep them alive after close and across fork.
  The personality reads and writes them through a `UringKernel` trait.
- **Links, cancellation, timeouts, overflow:** `IOSQE_IO_LINK` chains
  break on a failure or short transfer (`-ECANCELED`), `HARDLINK` never
  breaks; `ASYNC_CANCEL`, `POLL_REMOVE`, `TIMEOUT_REMOVE` find parked
  requests by `user_data`; CQEs that do not fit are kept in order with
  `IORING_SQ_CQ_OVERFLOW` set.
- **Registrations:** fixed buffers, fixed files (opened on a temporary
  descriptor for each request), an eventfd, `ENABLE_RINGS`, and a probe
  that lists only the opcodes Poly runs.
- **Simplified:** `SQPOLL` has no poll thread (`NEED_WAKEUP` stays set);
  `IOSQE_IO_DRAIN` and `IOSQE_ASYNC` are ignored; `BUFFER_SELECT`,
  `NO_MMAP` and registered rings are `EINVAL`; a `TIMEOUT` that falls
  due while its process sleeps outside io_uring_enter (in `poll` on the
  ring fd) completes only when one of its threads runs again.

### POSIX message queues (`ipc/mqueue.rs`, `sys/mqueue.rs`, `kernel/src/linux/mqueue.rs`)

- **Names per IPC namespace** in a table keyed by the namespace's inode
  number (never reused). `mq_unlink` removes only the name; open
  descriptors keep the queue.
- **Order:** a map from priority to a FIFO, highest first. Blocking
  uses the System V message queues' ticket scheme: a sender reaching an
  empty queue hands its message to the first blocked receiver, a
  receive that frees a slot takes the first blocked sender's message,
  so a woken caller never loses to a newcomer. Waits use an absolute
  `CLOCK_REALTIME` deadline and restart under `SA_RESTART`.
- **Notification:** one registration per queue, fired when a message
  reaches an empty queue no receiver waits on, then removed; dropped
  with the open file that registered it or at its process's exit.
  `SIGEV_THREAD` is not supported.
- **The descriptor** is a file object on an anonymous regular-file inode
  with the queue's mode and owner, close-on-exec; reading it gives the
  `QSIZE:... NOTIFY:...` line, `poll` works.
- **Simplified:** no `RLIMIT_MSGQUEUE` accounting; the status read
  ignores the file position.

### The mount API and pivot_root (`poly_vfs::mountapi`, `personalities/linux/src/mountapi.rs`)

- **Mounts can move.** A mount's mount point is now mutable
  (`Mount::parent()`); after a move the table is reordered so every
  mount follows the one it sits on, ids stay stable. `mount(2)` gained
  `MS_MOVE` and records propagation (`MS_SHARED`/`PRIVATE`/`SLAVE`/
  `UNBINDABLE`, with `MS_REC`); nothing propagates, so a slave is
  private in effect.
- **Detached mounts** (`fsmount`, `open_tree(OPEN_TREE_CLONE)`) stay out
  of the table; the `O_PATH` files on their root hold them, and closing
  the last drops the mount. `move_mount` attaches one. A recursive clone
  (`OPEN_TREE_CLONE | AT_RECURSIVE`) copies the mounts below the source
  directory at any depth; the detached top keeps the copies in a list
  (`Mount::subtree`), the copies point back to their parents weakly (no
  reference cycle), path lookups find them through the top's list, and
  attaching the top moves them into the table.
- **pivot_root** moves the namespace's root mount onto `put_old` and
  makes `new_root` the base; since namespaces are named by their first
  mount, the table keeps an alias from the old name. The caller's root
  and working directory move, and so do those of every process that
  used the old root.
- **Attributes:** `MOUNT_ATTR_RDONLY`/`NOSUID`/`NODEV`/`NOEXEC` are the
  existing per-mount flags; atime modes and `NODIRATIME` are stored and
  reported (`statfs` `ST_RELATIME` for the default relatime mode,
  `ST_NOATIME` for noatime, neither for strictatime), not enforced;
  `NOSYMFOLLOW` is enforced: path resolution does not follow a symbolic
  link on such a mount (`ELOOP`). Superblocks can be read-only
  as a whole (remount, `fsconfig` reconfigure), and per-mount and
  per-superblock writer counts give the `EBUSY` cases.
- **File system contexts** (`fsopen`, `fspick`) hold the parameters, a
  state and a message log read one message per `read(2)`; only tmpfs
  creation and `ro`/`rw` reconfiguration take effect, other parameters
  are validated and ignored.
- **statmount/listmount and statx:** the personality encodes `struct
  statmount` and does listmount's ordering and paging. Unique mount ids
  are 2^31 plus the mount id (`statx` `STATX_MNT_ID_UNIQUE`,
  `name_to_handle_at` `AT_HANDLE_MNT_ID_UNIQUE`, statmount).
- **The boot mounts** are closer to a Linux system's: besides `/proc`,
  `/sys`, `/dev/pts` and `/dev/shm`, the POSIX message queue file system
  on `/dev/mqueue` (a tmpfs named `mqueue`, `MQUEUE_MAGIC`; the queues
  themselves are not files in it) and tmpfs on `/run` and `/run/lock`
  (`/tmp` stays on the root file system). Programs that list or count
  mounts (listmount's test fills an 8-entry buffer) see a usual layout.
- **Simplified:** a mount below the top of a detached tree cannot be
  attached on its own; `MOVE_MOUNT_SET_GROUP`, `MOVE_MOUNT_BENEATH` and `MOUNT_ATTR_IDMAP` are
  `EINVAL`.

### `*xattrat`, inode flags, file handles (`poly_vfs::fileattr`, `sys/{xattrat,fileattr,handle}.rs`)

- **`*xattrat`** reuse the xattr calls with a new `XattrTarget::At`
  target and a generic copy-in of extensible structures (`EINVAL` below
  the first version's size, `E2BIG` above a page or with a non-zero
  tail), which `file_setattr` shares. All argument checks come before
  the lookup.
- **Inode flags** are `FS_XFLAG_*` stored by the file system: tmpfs keeps
  `IMMUTABLE`, `APPEND`, `NOATIME`, `NODUMP` on regular files and
  directories. Changes are checked unknown bit, owner/`CAP_FOWNER`,
  `CAP_LINUX_IMMUTABLE`, read-only mount, unsupported field, in that
  order. Opening an append-only file for writing without `O_APPEND`,
  opening an immutable one for writing or truncation, and unlinking
  either are `EPERM`; rename, chmod and creation in an immutable
  directory are not blocked yet.
- **File handles** come from the file system: a tmpfs handle is the
  inode number and a per-instance number; handles of another instance,
  corrupted ones and deleted files nothing holds are `ESTALE`.
  `open_by_handle_at` follows `open`'s rules; `CAP_DAC_READ_SEARCH` is
  checked after the handle size.

### Landlock (`personalities/linux/src/landlock.rs`)

- **Domains live in the credentials.** A ruleset collects rules until it
  is enforced; `landlock_restrict_self` stacks a frozen copy as a layer
  onto the thread's domain in `Creds` (inherited by fork and clone, kept
  across execve, at most 16 layers). ABI version 7.
- **File accesses:** the kernel hands the personality the chain of
  objects from the target up to the process root, crossing mount
  points; a handled access is allowed if a rule on any object in the
  chain allows it, in every layer. Rules follow inodes, so they survive
  renames and bind mounts. Checked: open, create, mkdir, mknod,
  symlink, unlink, rmdir, truncate, execve, rename and link (`EXDEV`
  across directories without `REFER`).
- **TCP:** bind and connect of stream inet sockets are checked against
  port rules. **Signal scope:** a sender's scoped layers must all be
  part of the target's domain, else `EPERM` whatever its capabilities.
- **Not enforced yet:** `ftruncate` on already open files, `IOCTL_DEV`,
  the refer rule against gaining rights, abstract unix socket scope; the
  `RESTRICT_SELF_LOG_*` flags are accepted and nothing is logged.

### fanotify (`poly_vfs::fanotify`)

- **Groups and marks** (inode, mount, file system; event and ignore
  masks) in one registry; events reuse inotify's VFS hooks, whose bits
  have the same values. Directory entry events go only to groups that
  report file ids.
- **Reading:** a descriptor group opens the object "quietly" in the
  reader's process (a quiet file causes no events); a file-id group
  reports `FAN_NOFD` with FID/DFID/DFID_NAME records whose fsid is
  statfs's `f_fsid` and whose `struct file_handle` is the one
  name_to_handle_at gives for the object (the file system's handle; the
  32-bit inode number, `FILEID_INO32_GEN`, only where the file system
  makes none). Mount marks are keyed by the mount's id and identity.
  Events merge with
  the last queued one when they match; the queue holds 16384 events,
  then one overflow event.
- **Not implemented:** permission events never block.

### Memory: mseal, userfaultfd, memfd_secret, cachestat

- **mseal** marks VMAs sealed (`poly_vm`); unmap, protect, remap and
  `MAP_FIXED` over any sealed page fail with `EPERM` and change nothing;
  discarding advice on sealed read-only private anonymous memory is
  `EPERM`. Fork copies the flag.
- **userfaultfd** registers regions by object id; a missing page in a
  registered region makes the faulting thread (also a `CLONE_VM`
  child) queue a message and wait until a wake covers its page
  (`UFFDIO_WAKE`, `UFFDIO_COPY`/`ZEROPAGE` without `DONTWAKE`, which
  wake only the filled range), the range is unregistered or the object
  is closed; with `UFFD_FEATURE_SIGBUS` it gets `SIGBUS` instead. A
  fault the kernel takes on user memory inside a system call is not
  handled by an object made with `UFFD_USER_MODE_ONLY`: the call fails
  with `EFAULT` and no message is queued. The descriptor is read-only
  (`F_GETFL` shows `O_RDONLY`, plus `O_NONBLOCK` if asked for). The
  handshake, queue and waiters are host-tested in the personality.
  Write-protect mode is accepted, nothing is protected.
- **memfd_secret** is a sealed memfd marked secret: `read`/`write` are
  `EINVAL`, `MAP_PRIVATE` and a second size change are refused. Nothing
  leaves the direct map (Poly never swaps).
- **cachestat** asks the file system which pages hold data
  (`FileSystem::data_pages`): tmpfs its sparse pages, the page cache its
  inner file system's pages plus dirty ones. Only `nr_cache` is non-zero.

## Consequences

- All 91 visible tests of the 42 entries pass on Poly; the held-out tests
  decide the guesses listed above as simplified.
- AIO completes deferred polls only when the process enters the
  interface again. io_uring completes parked requests on the owning
  process's way back to user mode (after any system call, or an
  interrupt), and the ring fd wakes `poll`/`epoll` waiters.
- `Mount::parent` is private behind a lock now; code walking mounts uses
  `Mount::parent()`.
- fanotify's file ids and name_to_handle_at's handles are the same
  bytes, so a reader can match an event to a file it knows.
