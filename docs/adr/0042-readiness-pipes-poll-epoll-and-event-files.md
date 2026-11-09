# 0042: Readiness, pipes, poll / select / epoll and the event files: wait queues and file objects in `poly-vfs`, waits in the personality, park / unpark in the kernel

**Status:** Accepted. Builds on [ADR 0023](0023-vfs-tmpfs-and-cpio.md)
(the VFS), [0025](0025-linux-syscall-table-and-kernel-services-trait.md)
(the `LinuxKernel` trait), [0031](0031-linux-signals-and-futexes.md)
(signals, park / unpark, restart codes), [0037](0037-vfs-and-initramfs-in-the-kernel.md)
and [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md)
(per-process files, `LinuxKernel` per thread). [0039](0039-networking-in-the-kernel.md)
left sockets a readiness generation for `poll`; this ADR gives files the
general mechanism sockets will join.

## Context

Phase 4 lists "Pipes, `poll` / `select`, `epoll`, `eventfd`, `timerfd`,
`signalfd`". All of them are about one thing: an open file whose
readiness changes (data arrives, room frees up, a timer expires, a signal
is queued) and threads that wait for that, either in a blocking `read` /
`write` on the file or in `poll`, `select` or `epoll_wait` over many
files. The VFS so far had only files that never block (tmpfs, ext2, the
memory devices, a console that reads end of file), and `OpenFile` always
sat on a file system inode. A blocked wait must also end for a signal,
with `EINTR` or a restart as signal(7) says per call, which ADR 0031's
restart codes and per-thread signal notification already express. The
console, the TTY layer and sockets will need the same mechanism.

Sources: pipe(7), pipe(2), fifo(7), poll(2), select(2), select_tut(2),
epoll(7), epoll_create(2), epoll_ctl(2), epoll_wait(2), eventfd(2),
timerfd_create(2), signalfd(2), signal(7), write(2), ioctl(2) /
`FIONREAD`; constants and layouts from the UAPI headers
(`asm-generic/poll.h`, `linux/eventpoll.h` (packed `epoll_event` on
x86-64), `asm-generic/fcntl.h`, `linux/timerfd.h`, `linux/signalfd.h`,
`linux/time_types.h`). No Linux or C library source was read and nothing
was observed on a real system; every point the manual pages leave open is
marked "to be confirmed by the diff runner" in the code.

## Decision

**Readiness lives in `poly-vfs` (`poll.rs`), host-tested.**

- `PollEvents`: the event bits with Linux's values, shared by `poll`,
  `select` and `epoll`.
- `WaitQueue`: the wakers waiting on one file, each with the events it
  cares about (`POLLERR` / `POLLHUP` always included). Whoever changes a
  file's readiness calls `notify(events)`; the queue copies the matching
  wakers out and calls them with its lock released, so a waker may take
  its own locks and notify further queues (an epoll set does).
  `register` returns a `Registration` that takes the waker off when
  dropped.
- `Waker` (`wake(events)`) and `Blocker` (`waker()`, `block(until)` →
  `Woken` / `TimedOut` / `Interrupted`): the calling thread's side, which
  the kernel implements with park / unpark. A wake before the block is not
  lost; spurious returns are allowed, callers check again.
- The waiting pattern is always: try; register; try again; block; loop.
  `poll::poll` does it over many files, `wait_until_done` for one
  operation, `Epoll::wait` on a set's ready list. Timeouts are monotonic
  nanoseconds (`Timeout::Now` / `At` / `Never`).

**Open files that are not on a file system are `FileObject`s** (`read`,
`write`, `poll`, `wait_queue`, `ioctl`, `bytes_readable`), a new backend
of `OpenFile`. Their operations never block: one that would returns
`EAGAIN`. `OpenFile::read_wait` / `write_wait` block on the object's
queue unless the description is `O_NONBLOCK`; a blocking write keeps
writing until everything is in (pipe(7)), and returns the partial count
when a signal or the last reader's close stops it after some bytes.
`fstat` needs an inode: `Vfs::anon_node` makes one on an anonymous file
system mounted nowhere (`AnonFs`), alive while an open file pins it; a
pipe's two ends share theirs. `CharDevice` gets `poll` / `wait_queue`
with "always ready, never changes" defaults, which the console uses until
the TTY layer gives it a queue. Regular files and directories are always
ready (and refused by epoll with `EPERM`, as epoll_ctl(2) documents).

**The objects are in `poly-vfs` too**, each host-tested with threads:

- `pipe`: a 64 KiB buffer, read and write ends counted; end of file
  without writers, `EPIPE` without readers; writes up to `PIPE_BUF`
  (4096) all or nothing, longer ones take what fits; packet mode
  (`O_DIRECT`); `POLLHUP` on the read end, `POLLERR` on the write end
  when the other side is gone; `FIONREAD`.
- `eventfd`: the counter, semaphore mode, the `0xfffffffffffffffe`
  maximum.
- `timerfd`: expirations counted from the clock whenever the timer is
  looked at (nothing is lost to a late wake-up), and a `TimerService`
  trait (clocks, `arm(at, Weak<target>)`, `cancel`) so waiters hear of an
  expiration when it happens. A realtime timer is armed at the
  equivalent monotonic time.
- `epoll`: interest entries keyed by descriptor and open file
  description, each a `Waker` on its file's queue that puts it on the
  set's ready list; `wait` re-polls ready entries: level-triggered ones
  go back on the list while ready, edge-triggered ones once per
  notification, `EPOLLONESHOT` ones are disabled until `MOD`. Nesting is
  allowed, loops and depth above 5 are `ELOOP`; entries whose
  description is closed everywhere are dropped. `EPOLLEXCLUSIVE` is
  accepted with its documented restrictions and wakes every waiter (the
  page allows "one or more"); `EPOLLWAKEUP` is ignored.

**The personality owns the system calls and the wait rules**
(`sys/pipe.rs`, `poll.rs`, `epoll.rs`, `eventfd.rs`, `timerfd.rs`,
`signalfd.rs`), through a second trait, `fileio::FileIo`, reached with
`LinuxKernel::file_io()` (default `None`: those calls are `ENOSYS`). One
accessor instead of a dozen trait methods keeps `LinuxKernel` and the
kernel's `impl` of it nearly unchanged; the kernel implements `FileIo`
for the same `Thread` in its own file. The personality checks flags and
sizes, turns timeouts into deadlines, converts `select`'s descriptor sets
(readable for `POLLIN`/`RDNORM`/`RDBAND`/`HUP`/`ERR`, writable for
`POLLOUT`/`WRNORM`/`WRBAND`/`ERR`, exceptional for `POLLPRI`, from
select(2)'s "will not block" definitions; `poll` asks the kernel for
exact events in that case), writes back the time left (`ppoll`,
`select`, `pselect6`), and decides interruption:

- A wait first clears the thread's signal notification and only checks
  (no blocking) if a signal is deliverable already; nothing ready and a
  signal there is an interruption.
- `ppoll`, `pselect6` and `epoll_pwait` install their mask through
  rt_sigsuspend's `saved_mask`: put back at once when the call returns,
  left for the signal's delivery when it was interrupted (so the handler
  runs with the call's mask and the old one returns after it).
- `poll` and `select` variants: `RESTART_IF_NO_HANDLER` (signal(7):
  never restarted after a handler, restarted after a stop). `epoll_wait`
  and `epoll_pwait`: plain `EINTR` (in both of signal(7)'s lists) when
  a pending signal acts (a handler, a stop, a kill); a wait ended only by
  signals whose delivery does nothing (the default-ignored `SIGCHLD` a
  traced process keeps until it is taken) is restarted. Reads
  and writes interrupted before any byte moved: `RESTART_IF_SA_RESTART`
  ("slow" devices).
- `write` / `writev` failing with `EPIPE` raise `SIGPIPE` for the writing
  thread (write(2)), with `SI_KERNEL` (to be confirmed).
- `FIONREAD` is answered from `FileIo::bytes_readable` for files that
  know it.

**Signal files are a kernel object over the personality's state.** The
kernel's `SignalFd` reads by taking the *reading* thread's pending signals
of its mask out of its `LinuxProcess` (`dequeue_from`, as rt_sigtimedwait
does, blocked or not) and encoding them with the personality's
`siginfo_bytes` (`struct signalfd_siginfo`, the members sigaction(2)
names for each signal and code). A new trait method,
`LinuxKernel::signal_queued(pid)`, called by the personality whenever a
signal is queued, blocked or not, notifies one queue all signal files
share; readers then look at their own pending signals.

**The kernel's part is small** (`fs/poll.rs`, `linux/services/fileio.rs`,
`linux/signalfd.rs`): `ThreadBlocker` (park / unpark; a signal
notification or the end of the process interrupts), `FileIo` for
`Thread` (descriptor lookups, objects on anonymous inodes, `Files::install`),
`LinuxKernel::read` / `write` going through `read_wait` / `write_wait`,
`mmap` refusing objects (`ENODEV`), and the timer service: a `timers`
kernel thread, started at the first use, that sleeps until the earliest
callback is due and runs callbacks with no lock held (never in an
interrupt handler, so they may notify wait queues and allocate).

## Consequences

- Every blocking file now has one mechanism: a queue, notifications on
  change, and a blocker that signals interrupt. Sockets can become file
  objects whose queue the network thread notifies (instead of, or on top
  of, ADR 0039's generation), and the TTY layer gives the console a
  queue; `poll` and `epoll` then work on them unchanged.
- Readiness is per object, so a write to one pipe wakes only that pipe's
  waiters. Every signal queued anywhere wakes every signal-file waiter
  (they re-check); fine until many processes use signal files.
- `epoll_wait` re-polls only ready entries; the interest list is walked
  only by `ctl`. Wake-ups take a heap allocation (the copied waker list),
  so `notify` must not run in an interrupt handler; the timer thread is
  why timer files need none.
- Not done: FIFOs (`mkfifo` / `mknod` and opening them, with blocking
  opens) wait for the path-based file calls; pipe size changes
  (`F_SETPIPE_SZ`), `O_NOTIFICATION_PIPE`, `splice`/`tee`, `SO_*` socket
  readiness, `TFD_TIMER_CANCEL_ON_SET` actually cancelling (the wall clock
  cannot be set yet), `CLOCK_BOOTTIME` differing from the monotonic clock
  (nothing suspends), the nesting depth counted above the set being
  changed (only the depth below it is checked), and restart_syscall for
  `poll`'s millisecond timeout after a stop (it restarts from the full
  timeout; the timespec variants continue with the time they wrote back).
