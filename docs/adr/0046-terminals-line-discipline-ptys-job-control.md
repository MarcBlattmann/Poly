# 0046: Terminals: the line discipline, ptys and job control in `poly-tty`, the kernel's devices sharing the console with the kernel shell

**Status:** Accepted. Builds on [0021](0021-kernel-shell.md) (the kernel
shell and its serial input), [0035](0035-screen-terminal.md) (the screen
terminal, which already encodes keys as terminal bytes "for a TTY layer
later"), [0037](0037-vfs-and-initramfs-in-the-kernel.md) (the console
device whose reads were end of file "until the TTY layer"),
[0031](0031-linux-signals-and-futexes.md) (signals, restarts),
[0041](0041-processes-threads-fork-exec-wait.md) (process groups and
sessions) and [0042](0042-readiness-pipes-poll-epoll-and-event-files.md)
(wait queues, blockers).

## Context

Phase 4 lists "TTY layer: line discipline, pseudo-terminals, job
control", on the way to "BusyBox shell boots as init and runs
interactively". Until now `/dev/console` and `/dev/tty` wrote to the
kernel log, read end of file and answered every ioctl with `ENOTTY`; the
serial port's input and the keyboard belonged to the kernel shells. A
shell needs a terminal: line editing with echo for `read`, `isatty`
(glibc decides stdout's buffering with `TCGETS`), raw mode for its own
line editor, `^C` / `^Z` as signals for the foreground job, `tcsetpgrp`,
`SIGTTIN` / `SIGTTOU` for background jobs, window sizes, and ptys for
`script`, terminal emulators and tests.

Sources: termios(3), ioctl_tty(2) (tty_ioctl(4)), pty(7), pts(4),
credentials(7), tty(4), console(4), ttyS(4), grantpt(3), unlockpt(3),
posix_openpt(3), tcsetpgrp(3), tcgetpgrp(3), bootparam(7) (`console=`),
the GNU coreutils manual (`stty sane`), POSIX.1-2024 (General Terminal
Interface, `_exit`, orphaned process groups); constants and layouts from
the UAPI headers `asm-generic/termbits-common.h`, `asm-generic/termbits.h`,
`asm-generic/termios.h`, `asm-generic/ioctls.h`, `asm-generic/ioctl.h`,
`asm-generic/signal.h` and `linux/tty.h`. No Linux or C library source
was read and nothing was observed on a real system; points the documents
leave open are marked "to be confirmed by the diff runner" in the code.

## Decision

**Everything but the machine is in a host-tested crate, `poly-tty`**, on
`poly-vfs`'s readiness types:

- `termios`: `struct termios` / `termios2` / `termio` / `winsize`, the
  flags, special characters and baud codes, with their byte layouts.
- `ldisc::LineDiscipline`, plain data: input processing (`ISTRIP`,
  `IUCLC`, `LNEXT`, `IXON` / `IXANY`, `ISIG` with the queues flushed
  unless `NOFLSH`, `IGNCR` / `ICRNL` / `INLCR`), canonical editing
  (`ERASE`, `KILL`, `WERASE`, `REPRINT`, `EOF`, `EOL`, `EOL2`; lines of
  4095 characters, termios(3)), echo (`ECHO`, `ECHOE`, `ECHOK`, `ECHONL`,
  `ECHOCTL`, `ECHOKE`, erasing tabs and `^X` by their columns, UTF-8 with
  `IUTF8`), `IXOFF`, a 4096-byte input buffer, reads with `MIN` / `TIME`
  (it returns a deadline to wait for), and output processing (`OPOST`,
  `ONLCR`, `OCRNL`, `ONOCR`, `ONLRET`, `OLCUC`, `TAB3`). It returns
  `Effects` (bytes to echo, signals, wake-ups) instead of acting.
- `Tty`: a discipline between a `Driver` (the device: `write`, `room`,
  `queued`, `flush`, open/close notices) and the processes, with the
  session and foreground group, and the ioctls of ioctl_tty(2): `TCGETS`
  and `TCSETS*` (also the `termios2` and `termio` forms), `TIOCGWINSZ` /
  `TIOCSWINSZ` (`SIGWINCH`), `TIOCGPGRP` / `TIOCSPGRP`, `TIOCSCTTY` /
  `TIOCNOTTY`, `TIOCGSID`, `FIONREAD`, `TIOCOUTQ`, `TCFLSH`, `TCXONC`,
  `TCSBRK`, `TIOCSTI`, `TIOCEXCL` / `TIOCNXCL`, `TIOCGETD` / `TIOCSETD`.
  A `TtyFile` per open file counts opens (closing is dropping it).
- **Job control** follows POSIX: a session leader without a controlling
  terminal that opens a terminal no session has, without `O_NOCTTY`,
  acquires it with its group in the foreground (opening `/dev/console`
  or `/dev/tty` never does: implementation-defined, and what lets a
  shell on the console run without job control until it takes the
  terminal with `TIOCSCTTY`). A background reader gets `SIGTTIN` sent to
  its group and the read is interrupted; `EIO` instead when it blocks or
  ignores `SIGTTIN` or its group is orphaned. A background writer with
  `TOSTOP`, and a background process changing the terminal at any time,
  get `SIGTTOU` the same way, except that blocking or ignoring it lets the
  call go ahead. The session table (which session has which terminal)
  is the crate's `Ttys`.
- `pty`: `/dev/ptmx` opens a new pair: a master and a slave `Tty` whose
  driver is the master's 64 KiB buffer. Master writes are typed into the
  slave and wait for room in its input; slave writes wait for room in the
  master's buffer. New slaves are locked (`TIOCSPTLCK`, `TIOCGPTLCK`;
  `TIOCGPTN` gives the number), owned by the opener with mode `0620`.
  Every slave closed again: master reads fail with `EIO`, poll says
  `POLLHUP`. The master closing hangs the slave up: `SIGHUP` to the
  controlling process, `SIGHUP` and `SIGCONT` to the foreground group, end
  of file for reads, `EIO` for writes, and the number is free again.
- `devpts`: the file system on `/dev/pts`, one node `<n>` per open master
  (character device 136, `n`).
- `device`: the devices behind the nodes: a terminal's own (`/dev/ttyS0`,
  `/dev/tty1`), `/dev/console` (the console terminal) and `/dev/tty`
  (the caller's controlling terminal, `ENXIO` without one).

**The kernel provides a `Host`**: the caller (pid, group, session, ids,
privilege) from the running user thread and the process tree, whether
it blocks or ignores a signal, signals sent by the kernel (`SI_KERNEL`, no
permission check; init only gets those it handles, kill(2) NOTES), the
session of a group and whether it is orphaned (the personality's new
`task::jobs`, host-tested on `ProcessTree`), a `Blocker` (park / unpark,
so signals interrupt waits), user copies and the clock. The crate's
locks are spinlocks held for memory work and the driver's `write`, never
across a user copy, a block or a signal, so drivers must not sleep: the
serial driver writes to the UART under the console lock, the screen's
pushes into a ring the terminal's render thread drains.

**A device may open as another one.** `poly-vfs`'s `CharDevice` gets one
method, `open(flags)`, called when its node is opened, which may return
the device the open file then uses: `/dev/ptmx` returns a new master,
`/dev/tty` the caller's terminal, and every terminal node a `TtyFile`.
Without it `/dev/ptmx` and `/dev/tty` would need a hook in the VFS's open
path that knows processes.

**How the kernel shell and programs share a device.** A device's input
goes to its terminal while a program has the terminal open, else to the
kernel shell on that device: the serial ring has two readers (the shell
and the `tty-serial` thread, one at a time under a lock), the keyboard
likewise (`shell-screen` and `tty-keys`). The shells work as before when
no program runs; `run <program>` gives the program the shell's own
terminal (`/dev/ttyS0` or `/dev/tty1`, which it acquires as its
controlling terminal, leading a session of its own) and the device goes
back to the shell when the program and whatever it started that keeps the
terminal open have ended. `poly.init=` programs get `/dev/console`, the
terminal `console=` names (`ttyS0` by default, or `tty1`, for which the
screen terminal is brought up before the first program). Program output
on the serial port erases a shown shell prompt first, like a log line.
Rejected: a fixed owner per device (the shell only until `poly.init`
starts), which would leave no way back to the kernel shell, and kernel
shell commands talking through a pty, which would make the debug console
depend on the code it debugs.

**Traced test runs keep the old console.** A process traced for the diff
runner (`poly.test.capture`) gets the kernel log's console on fds 0-2
(writes to the log, reads end of file, no terminal), so the corpus's
recorded behaviour on Linux, where the tracer captures the output, does
not depend on Poly's console.

**Process ends.** The kernel tells the terminals when a process ends: a
session leader that had a controlling terminal hangs its session up
(`SIGHUP` to the foreground group, the terminal free again), and groups
the end orphans that have a stopped member get `SIGHUP` then `SIGCONT`
(POSIX `_exit`). An ioctl interrupted by job control restarts under
`SA_RESTART`, as reads and writes on slow devices do.

## Consequences

- Programs read lines from the console and from ptys with echo and
  editing; `isatty` is true on the console (glibc line-buffers stdout);
  `^C`, `^Z` and `^\` reach foreground groups; shells get job control
  once they lead a session on a terminal.
- `/dev/console` output on the serial port no longer goes through the
  kernel log: it is not mirrored on the screen (the screen has `tty1`).
- The input threads look every 50 ms whether their terminal is still
  open; a byte typed just as a program closes the terminal may go to
  either side.
- Not done: the `open`/`openat` system calls themselves (the file-call
  work; the kernel side, `Files::open`, does everything above), packet
  mode (`TIOCPKT`), `TIOCGPTPEER`, modem lines (`TIOCM*`), `ECHOPRT` and
  `XCASE`, `VDISCARD`, breaks and parity, a real drain for `TCSETSW` /
  `tcdrain` (output is synchronous), `SIGIO` (`O_ASYNC`), per-process
  `TIOCNOTTY` surviving `fork` (it is recorded per pid), a
  `CLD_CONTINUED` report for a `SIGCONT` the kernel sends from a kernel
  thread (it only sends one from process ends, which report it), more
  virtual consoles than `tty1`, and resizing `tty1` with the display.
- Orders and edge cases the documents leave open (errors of a locked
  slave, `/dev/tty` without a terminal, the pty device major, who gets
  which hangup signal, `O_NONBLOCK` with `MIN` 0) are Poly's choices until
  the diff runner confirms or corrects them.
