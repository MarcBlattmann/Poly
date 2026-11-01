# 0062: Signals reported as a tracer sees them; FIFOs; pipe pages; shared futex keys; loopback TCP within the sending call

**Status:** Accepted. Amends [0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
and [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md)
(`POLY-SIGNAL`), [0031](0031-linux-signals-and-futexes.md) (ignored
signals, futex keys), [0042](0042-readiness-pipes-poll-epoll-and-event-files.md)
(pipes, FIFOs) and [0048](0048-sockets-inet-unix-netlink.md) (loopback).

## Context

The spec agent's 58 entries on signals, futexes, pipes and FIFOs, poll /
select / epoll, the event files, Unix and loopback sockets, credentials,
terminals and mount came with 75 visible tests; on Poly 50 of them passed
their own checks, and fewer passed the diff runner, which compares the
syscall and signal sequence too. Most failures were single behaviours
(one errno, one flag; fixed where they are, with host tests), but a few
were structural:

- The Linux tracer records a `--- SIGx ---` event at every
  signal-delivery-stop (ptrace(2): any signal but `SIGKILL`, when a thread
  takes it), handled or not; Poly's `POLY-SIGNAL` covered faults and fatal
  signals only, and also reported `SIGKILL`. Default-ignored signals
  (`SIGCHLD` above all) are in the reference traces too (the app mode
  of the diff runner drops `SIGCHLD` for that reason).
- A call a signal interrupts shows the kernel's internal restart number at
  its exit under ptrace (512, 513, 514, 516); the runner's normaliser
  removes a restarted attempt only when it sees one. Poly recorded `EINTR`.
- FIFOs could be made but not opened (`ENXIO`).
- Pipe room was a byte count; Linux's is pages (a partly read page takes no
  new bytes), and `F_GETPIPE_SZ` / `F_SETPIPE_SZ` did not exist.
- Futex keys were always `(address space, address)`, so processes sharing
  `MAP_SHARED` memory never found each other's waiters.
- Small writes over TCP loopback reached the reader one at a time: Nagle
  held the next segment back until a delayed acknowledgement 40 ms later,
  and looped packets waited for the network thread.

Sources: ptrace(2) (signal-delivery-stop), signal(7), sigaction(2),
pipe(7), fifo(7), fcntl(2) (`F_SETPIPE_SZ`), futex(2), tcp(7), the spec
sheets and visible tests of the entries, and the pipeline's own tracer and
normaliser (`tools/pipeline/src/ptrace.rs`, `trace.rs`). No Linux or C
library source was read and nothing was run on Linux.

## Decision

- **`POLY-SIGNAL` is a signal-delivery-stop.** The personality tells the
  kernel about every signal a thread takes from its pending set to act on
  it (`LinuxKernel::signal_delivered`, called by `exit_to_user` for each
  dequeued signal but `SIGKILL`): handled, stopping, continuing, fatal and
  ignored ones, in delivery order, before the action. Signals taken by
  rt_sigtimedwait or a signal file are not delivered and not reported. The
  kernel writes the record for traced processes; the fault path and
  `kill_process` no longer write their own.
- **A traced process keeps ignored signals until a thread takes them**
  (`LinuxProcess::traced`, set by the kernel for `poly.test.capture`
  processes, inherited by fork), as a traced Linux process does, so the
  taking is seen. An untraced process discards an ignored signal at
  generation unless it is blocked (a blocked `SIG_IGN` signal now stays
  pending too, as rt_sigprocmask's visible test shows).
- **Traced results show restart numbers.** `Linux::traced_result` gives
  the record of an interrupted call the number ptrace shows (by restart
  policy: 512 / 513 / 514 / 516), which the runner's normaliser expects.
- **A child is waitable before its parent hears of `SIGCHLD`**: the signal
  is queued, the child announced, then the parent's threads are notified,
  so a parent in wait4 returns the child before taking the signal (the
  order a Linux trace shows) instead of being interrupted and restarted.
- **FIFOs**: `Vfs::open` of a FIFO inode returns an end of the inode's one
  `Pipe` (`poly_vfs` `fifo.rs`, a table by file system and inode, the pipe
  gone with its last end) and never blocks; `O_NONBLOCK` write opens
  without a reader are `ENXIO`. A blocking open waits in the kernel
  (`linux/fifo.rs`, `PipeEnd::wait_for_peer` with the thread's blocker),
  restarted under `SA_RESTART`. The VFS stays free of blocking.
- **Pipe room is counted in pages** of 4096 bytes (16 by default): a
  write puts the part of it that is not whole pages (its length modulo
  4096) into the last page if all of that part fits there, the rest into
  new pages (amended after the runner's third pass, below); a page is free
  again once every byte of it was read. `F_SETPIPE_SZ` rounds up to a power of
  two pages, refuses to go below the pages in use (`EBUSY`), and above
  1 MiB needs `CAP_SYS_RESOURCE` (`EPERM`).
- **Shared futex keys**: a futex without `FUTEX_PRIVATE_FLAG` in a
  `MAP_SHARED` mapping is `Shared { object, offset }`: the object is the
  mapped `PageSource` (a file by device and inode, `PageSource::object_id`;
  shared anonymous memory by its one `AnonShared` object, which fork
  shares), the offset its offset in it. Everything else is keyed by
  address space and address, a shared futex there as `Unshared`, apart
  from the private futex at the same address (amended, below).
- **Loopback TCP completes within the sending call**: the interface polls
  the stack again while looped packets wait (at most 8 rounds), and a TCP
  peer on this host is acknowledged at once instead of after the delayed
  acknowledgement timer. Datagrams to this host may be as large as an IP
  packet (65507 bytes of UDP), as on Linux's `lo`.
- **`int3` in user mode is `SIGTRAP`** (`SI_KERNEL`): the breakpoint gate
  has DPL 3, the only user-raisable vector.

## Consequences

- Of the 75 visible tests, 73 pass their checks on Poly. The two left,
  `b_futex_wait_errors` and `b_timerfd_settime_basic`, take
  `CLOCK_MONOTONIC` minus one second as a deadline in the past; Poly runs
  the test about half a second after boot, so that time is negative and
  `EINVAL`, which Linux answers too on a machine up for less than a second.
  Whether Poly's monotonic clock should start at an offset (it counts from
  boot, as Linux's does) or the tests should not depend on the uptime is
  the owner's and the spec agent's call.
- Traced runs now report `SIGCHLD`, `SIGPIPE`, `SIGCONT` and the stop
  signals; the app mode still drops `SIGCHLD`, whose place depends on
  scheduling.
- A traced process behaves as a traced Linux process does where they
  differ from an untraced one: ignored signals wake blocked calls (which
  restart). Untraced processes are unchanged.
- Loopback sends do the receiver's work in the sender's call; a burst over
  loopback holds the interface lock a little longer.

## Amendments after the runner's third pass

- **Pipe pages.** A 5000-byte write into a pipe holding 15 full pages
  and one with 1000 bytes free takes 904 bytes on the reference
  (`b_pipe2_capacity`), not the 1000 Poly took by filling the last page
  first: only the write's length modulo a page joins the last page, and
  only if all of it fits. Writes of at most `PIPE_BUF` stay all or
  nothing (a page's worth never joins), and a blocking write's later
  rounds always start new pages, as one write on the reference does.
- **Private and shared futexes are apart** (futex's spec sheet: "a
  private waiter is not found by a shared wake"). The personality gives a
  shared futex in private memory `FutexKey::Unshared`
  (`poly_personality_linux::futex::key`). The kernel's own wakes
  (`clear_child_tid`, a dead owner's robust locks) are shared wakes, as
  set_tid_address(2) describes, and take a private waiter when no shared
  one is there. Every operation reads its words first: `EFAULT` for an
  unmapped `uaddr` or `uaddr2` also in `FUTEX_WAKE` and the requeues.
- **A hang-up signals before it shows.** Closing a pty master sent
  `SIGHUP` after marking the slave hung up and waking its readers; a
  reader on another CPU could read end of file and exit before taking the
  signal (`b_ioctl_ctty`, 1 run in 6). `Tty::hangup` now sends the
  signals first.
- **`/tmp` in a test's initramfs is 1777.** The diff runner's spec-test
  image made it 0755, so a test that gave up root could not create files
  there on Poly (the reference's `/tmp` is sticky and world-writable, and
  the app tests' tree already was).
- **An ignored signal ends no call.** A kept ignored signal still wakes a
  blocked call, but no call fails because of one: the program sees what
  an untraced process sees (`b_epoll_wait_block`'s child exit, 1 CI run
  in 3). The calls signal(7) has fail with `EINTR` (epoll_wait, semop,
  semtimedop, msgrcv, msgsnd, rt_sigtimedwait, and reads, writes, accept,
  connect on a socket with `SO_RCVTIMEO` / `SO_SNDTIMEO`) do so only
  while a pending signal acts on delivery (a handler, a stop, a kill:
  `LinuxProcess::has_acting`). The ones that sleep in the personality
  (semop, msgrcv, msgsnd, rt_sigtimedwait) sleep on, keeping their
  timeout, and the signal is taken when they return; the ones whose
  kernel wait ended restart (`signal::deliver::eintr_if_acting`), a
  socket's timeout counting from the restart.
