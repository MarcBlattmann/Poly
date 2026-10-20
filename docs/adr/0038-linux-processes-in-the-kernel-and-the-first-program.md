# 0038: Linux processes in the kernel: `LinuxKernel` per thread, process-wide exits, signals on the way out, programs from the VFS, the test-run protocol

**Status:** Accepted. Builds on [0022](0022-user-mode-address-spaces-and-system-calls.md)
(user mode, the syscall path, FPU state), [0025](0025-linux-syscall-table-and-kernel-services-trait.md)
(the `LinuxKernel` trait), [0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
(the test-run protocol), [0031](0031-linux-signals-and-futexes.md)
(signals and futexes), [0036](0036-demand-paging-and-exec.md) (memory maps
and exec) and [0037](0037-vfs-and-initramfs-in-the-kernel.md) (the VFS).

## Context

Phase 3 ends with a static Linux "hello world" running and exiting cleanly,
and with the diff runner comparing the `tests/linux/` corpus on real Linux
and on Poly. The personality had its handlers and the `LinuxKernel` trait;
the kernel had user threads, memory maps, exec of static ELF files and a
VFS, but no implementation of the trait, no notion of a process's threads
(so `exit_group` could not end them), no way to start a program from a
file, and ignored `poly.init`. Signal delivery needs the full user
register set and the FPU state, which the syscall path did not expose.

## Decision

**`LinuxKernel` is implemented per call, for the running thread.**
`kernel/src/linux/services.rs` has `Thread<'a>`: the process (borrowed), its
`LinuxState`, the thread id and flags, and what one system call or return
to user mode accumulates (the result in `rax`, an exit request, whether the
registers were replaced, the FPU image once saved). Files go to the
process's `fs::Files`, memory calls to `user::current_memory()` (no lock is
held while user memory is copied), FS/GS to the MSRs, clocks to the
monotonic clock and the RTC, random bytes to RDRAND or the TSC.
`syscall::dispatch` routes a Linux process that has personality state to
`linux::syscall`, which runs `Linux::dispatch_with`, records the call for a
traced process, turns `exit` / `exit_group` / a fatal signal into
`user::request_exit`, and calls `Linux::exit_to_user` before the return.
Bare process records (`Process::new`, used by the kernel tests with their
own hooks) keep the old behaviour: no services (`-ENOSYS`), and a fault
kills only the faulting thread.

**A process knows its threads; a thread only ever ends itself.** Every user
thread registers with its `Process` when it starts (`attach_thread`) and
gets an id from the pid space (the first thread's id is the pid; pids
count from 1, so `poly.init` is pid 1). `user::end_thread` does the
personality's thread-exit duties (robust list, `clear_child_tid`), leaves
the process, and the last thread ends it: files closed, status set, waiters
woken (`Process::wait`). Ending a whole process (`exit_group`, a fatal
signal) sets `group_exit` and wakes parked threads; each thread ends itself
when it next returns to user mode: after its system call (`user::syscall`),
or after an interrupt or exception in ring 3 (`user::return_to_user`,
called by `trap_dispatch`), so a thread spinning in ring 3 leaves at the
next tick. The kernel thread scheduler needs no way to kill another thread.
A process table (pid to `Weak<Process>`) serves the calls that name other
processes and threads.

**Signals on every way out.** After a system call, `Linux::exit_to_user`
always runs (it also settles interrupted calls). After an interrupt or
exception it runs only when the thread was notified of a signal (every
signal sent sets the notification of the threads that should act), or the
process is stopped or exiting, which keeps the timer tick cheap. A CPU
exception in ring 3 that demand paging does not resolve goes to
`user::fault`: for a Linux process the personality's
`signal::fault::fault_info` names the signal and code (`SIGSEGV` with
`SEGV_MAPERR` / `SEGV_ACCERR` and the address, `SIGBUS`, `SIGILL`,
`SIGFPE`, `SIGTRAP`), `Linux::fault_signal` queues it and `exit_to_user`
either builds a handler frame or kills the process. A failed return to ring 3 (an IRETQ
that faults) kills a Linux process as by `SIGSEGV`.

**The user registers live in the trap frame at the top of the kernel
stack.** Both the `syscall` entry and an interrupt from ring 3 put the
same `TrapFrame` there, so `user::registers()` / `user::set_registers()`
read and replace it in place (CS, SS and RFLAGS sanitised). After a
replacement the system call returns with IRETQ, restoring RCX and R11, and
takes only `rax` from the call (`SyscallReturn::restore_all`); the
`syscall` handler and `trap_dispatch` take the frame as a raw pointer so
the rewrite does not alias a live reference. The FPU image for a signal
frame is the thread's save area, saved from the registers first if the
thread owns them (`FpuArea::snapshot`); `rt_sigreturn` loads an FXSAVE or
standard-format XSAVE image only after checking what would make FXRSTOR /
XRSTOR fault (reserved MXCSR bits against the CPU's MXCSR_MASK, a header
with XCOMP_BV or reserved bytes set, components outside XCR0, a short
image); a refused image is `EINVAL`, which the personality turns into
`SIGSEGV`.

**Blocking, stops and futexes.** `block` / `unblock` are `process::park` /
`unpark` on one wait queue; a park ends on an unpark (not lost if it came
first), the thread's signal notification, the end of its process or the
deadline (a `CLOCK_REALTIME` deadline is converted to the monotonic clock;
the wall clock cannot be set yet). `stop_process` stops the process; its
threads wait at their next return to user mode until `continue_process`.
The futex table is one global spinlock; `futex_cmpxchg` brings the page in
first when it may block, then reads, compares and writes with interrupts
off, which is atomic on one CPU (a `lock cmpxchg` with a fixup replaces it
with SMP).

**Programs come from the VFS.** `fs::cache::FileSource` (ADR 0040) makes a
file a `PageSource` (pages read at fault time through the file system, or
the page cache's own pages on a disk mount); exec and file `mmap` use it.
`program::exec_path(path, argv, envp, traced)` opens the file with
`Vfs::open_exec` (regular, executable), reads its first 4 KiB, decides the
personality (`poly-binfmt`), and for a static Linux ELF builds the memory
with `exec::load` and starts a Linux process with the console on fds 0-2.
PE, Mach-O, `#!` scripts and `PT_INTERP` executables are refused with a
reason until their loaders exist. The shell's `run <program> [args]` uses
it and waits.

**The test-run protocol (ADR 0027) is implemented.** `poly.init=<path>`
runs that program as the first process right after the file systems are
up, before the self-test: `POLY-INIT: exec <path>` before its first
instruction (or `POLY-INIT: unsupported <path>: <reason>`), then
`POLY-EXIT: pid=1 code=N` (or `signal=N`); with `poly.qemu_exit` QEMU then
exits with the success status and nothing else runs. `poly.test.capture`
marks the process traced: one `POLY-SYSCALL` record per system call
(number, six raw arguments, the result or `ret=?` for calls that do not
return; no `data<i>` fields or times yet, so pointer arguments match
anything), `POLY-OUT` records (hex, at most 256 bytes each) for bytes
written to fds 1 and 2, and `POLY-SIGNAL` for a fault turned into a signal
and for a fatal signal. Records go to the serial port only, each as one
line; the program's console output goes through the kernel log as usual.

**A wall clock.** `CLOCK_REALTIME` reads the CMOS RTC once
(`arch/x86_64/rtc.rs`; decoding and calendar arithmetic in the host-tested
`poly-rtc` crate, the century register from the FADT) and adds the
monotonic clock since. It cannot be set yet.

**Test programs and checks.** `tests/linux/nolibc/hello.rs` is a hello world
without any C library, built by `cargo xtask` with the pinned toolchain
alone (`rustc --target x86_64-unknown-none`, static, not PIE): it uses
`brk`, `mmap`, `munmap`, `arch_prctl`, `write` and `exit_group`. The C
hellos (freestanding and statically linked with the host's C library) are
built by the pipeline's corpus build. The boot tests run each as
`poly.init=/bin/...` and check the output and exit status 0 (the C ones are
skipped with a message where the host has no C compiler or static C
library); the self-test boots carry `/bin/hello` in their initramfs for the
kernel's exec tests and the shell test types `run /bin/hello`.
`cargo xtask ci` runs `pipeline polycheck`: the corpus on Poly against
Linux, where every test listed in `tests/linux/poly-pass.txt` must pass
(about ten seconds; it skips itself like `selfcheck` without a C compiler,
ptrace or QEMU).

**One behaviour change from a visible test.** The corpus test `mmap.c`
expects `EINVAL` for `MAP_SHARED | MAP_PRIVATE | MAP_ANONYMOUS`; those type
bits are `MAP_SHARED_VALIDATE`, so the personality now refuses
`MAP_SHARED_VALIDATE` with `MAP_ANONYMOUS` with `EINVAL` (after the
unknown-flag check). File mappings keep `MAP_SHARED_VALIDATE`'s documented
behaviour.

**Clean room.** Behaviour comes from the man pages (exit_group(2),
_exit(2), mmap(2), signal(7), sigaction(2), clock_gettime(2), execve(2)),
the psABI, the Intel SDM (exception vectors, FXSAVE/XSAVE formats and their
fault conditions), the MC146818 data sheet, the public x86 UAPI headers and
the corpus's visible tests. No Linux or C library source was read and no
Linux system was traced to decide behaviour; the diff runner was run as a
tool, and its Linux side was only used through the test verdicts.

## Consequences

- A static glibc "hello world" runs and exits 0; it gets `ENOSYS` for
  `rseq`, `prlimit64` and `readlinkat`, which glibc tolerates. Of the
  corpus, 11 of 12 tests match Linux exactly; `libc-hello` differs at
  `rseq`.
- Not done yet, each marked where it is: parent notification on stop and
  continue (`wait4` does not exist), core dumps (`WCOREDUMP` is never
  set), shared mappings of files outside the page cache (tmpfs:
  `ENODEV`), `MAP_32BIT` placement (a
  mapping that lands high is refused with `ENOMEM`), `PROT_GROWSDOWN`
  (applied to the given range only), CPU-time clocks (`EINVAL`),
  `POLY-SIGNAL` for signals that a handler takes (only faults and fatal
  signals are reported), `data<i>` fields in syscall records. Kernel
  waits other than `park` (a mutex, a sleep) are not interrupted by a
  group exit; the thread leaves when the wait ends.
- `fork`, `clone`, `execve` and `wait4` (Phase 4) build on the process
  record: thread registration, group exit and the status are there; a
  parent link and zombies are not.
- Every return from an interrupt in ring 3 now looks up the running
  thread's process (one lock of the thread table and a few atomic loads).
- A process's open files are closed by its last thread, in its own
  context, before it is reaped. The scheduler drops a reaped thread's
  state wherever it switches (maybe in an interrupt handler); so that the
  process's last reference (its file system context, possibly on a disk
  mount whose locks sleep) is never dropped there, a thread buried while
  reaping hands a reference to its process to the writeback thread
  (`fs::cache::release_later`, ADR 0040).
