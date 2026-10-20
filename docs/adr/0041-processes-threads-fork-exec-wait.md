# 0041: Processes and threads: copy-on-write fork with shared frames, clone in the kernel, a process tree in the personality

**Status:** Accepted. Builds on [0031](0031-linux-signals-and-futexes.md)
(signal state, futexes, `clear_child_tid`), [0036](0036-demand-paging-and-exec.md)
(`user::Memory`, exec), [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md)
(Linux processes in the kernel) and [0040](0040-page-cache-and-ext2-on-virtio-blk.md)
(page-cache pages in mappings).

## Context

Phase 4 starts with "processes and threads: `clone`, `fork` (copy on
write), `execve`, `exit`, `wait4`" and "thread-local storage, `futex`,
robust futex lists, `set_tid_address`". The kernel had one `Memory` per
program, threads that registered with their process and a group exit,
but no way to make a second thread or process from user space, no parent
links, no zombies and nothing to wait with. `PageState::Present { refs }`
in `poly-vm` and its `CopyOnWrite` action were there for fork, unused.
The futex and robust-list logic, and `clear_child_tid` at thread exit,
existed in the personality (ADR 0031).

Sources: clone(2), fork(2), vfork(2), execve(2), _exit(2), exit_group(2),
wait(2) / wait4(2), setpgid(2), setsid(2), getsid(2), getppid(2),
set_tid_address(2), sigaction(2) (`SA_NOCLDSTOP`, `SA_NOCLDWAIT`),
signal(7), credentials(7), mmap(2), the x86-64 psABI (initial process
state, the TLS register), and the UAPI headers `linux/sched.h` (clone
flags, `struct clone_args`), `linux/wait.h`, `asm-generic/siginfo.h`
(`CLD_*`, `_sigchld`), `linux/resource.h` (`struct rusage`) and
`linux/limits.h`; the status word layout is POSIX `<sys/wait.h>`. No Linux
or C library source was read and nothing was observed on a real system.

## Decision

**Copy on write with shared frames, counted by `Arc`.** A frame that more
than one memory maps is a `SharedFrame` (a `UserFrame` behind an `Arc`);
each memory keeps the ones it maps in a table next to its page-cache
pages, maps them without owning them (`map_frames`) and unmaps them before
dropping its reference, so a frame is freed exactly when its last user
goes. `Memory::fork` clones the region map into a new address space and,
for every present page of a private region, turns the parent's own frame
into a shared frame (`AddressSpace::disown`: the entry stays, read-only,
without the ownership bit) mapped read-only in both, shares frames that
were shared already, maps the parent's page-cache pages read-only in the
child, and shares parked (`PROT_NONE`) frames unmapped. Shared regions are
not copied at all: the child's faults find the same pages through the
page cache or the shared-memory object. A fault passes the frame's `Arc`
count to `VmMap::fault` as `refs`; a write to a shared frame with other
users is copied, and the last user takes the frame back
(`AddressSpace::adopt`) instead of copying. A global table of reference
counts per physical frame was the alternative; it would need a lock taken
under the frame allocator's (heap allocations are not allowed there) or a
fixed array over all of RAM, and the per-memory tables were already there
for page-cache pages.

**Shared anonymous memory is an object.** `MAP_SHARED | MAP_ANONYMOUS`
maps an `AnonShared` (a `PageSource` whose pages are shared frames, made
zeroed at the first fault) as a shared file mapping, which is how
`poly-vm` describes it, so every process that inherits the mapping finds
the same pages.

**The kernel creates threads and processes; the personality decides.**
The personality decodes clone(2) / clone3(2) / fork(2) / vfork(2)
(`task::clone`: flag rules, `CSIGNAL`, `struct clone_args` and its size
rules, namespaces, pidfds, cgroups and `set_tid` refused with `EINVAL`)
into a `CloneRequest` and calls `LinuxKernel::clone_task`. The kernel
starts the child with the caller's trap frame (rax 0, the new stack if
given) and FPU state (`user::spawn_child`, `enter_user_regs`), the TLS
base if given, stores the child's tid at `parent_tid` and `child_tid`
(in the child's own memory when it has one) and sets its
`clear_child_tid` before it runs. A thread joins the caller's process and
memory; a process gets a copy of the memory (or the same one with
`CLONE_VM`), of the open files (`Files::fork`) and of the signal state
(`LinuxProcess::fork`, with `CLONE_CLEAR_SIGHAND` honoured). `CLONE_VFORK`
makes the caller wait until the child execs or ends; vfork(2) is
`CLONE_VM | CLONE_VFORK`, not fork: the waiting makes sharing the memory
safe, as vfork(2) documents. Sharing files, file-system context or signal
dispositions between two *processes* (`CLONE_FILES`, `CLONE_FS`,
`CLONE_SIGHAND` without `CLONE_THREAD`) copies them instead: a process
owns its `Files` and `LinuxProcess`.

**The process tree is the personality's, the kernel lends it.** Like the
futex table, `task::tree::ProcessTree` (parents, children, process groups,
sessions, zombies, unreported stops and continues, a generation counter)
is host-tested personality code, and the kernel keeps one behind a
spinlock (`LinuxKernel::with_tree`). The rules live there: a child starts
in its creator's group and session; a process the kernel started has
parent 0 and is reaped at once; an ended process stays a zombie until its
parent waits, unless the parent ignores `SIGCHLD` or set `SA_NOCLDWAIT`;
orphans go to init (pid 1) while it lives, else to the kernel; setpgid(2)
and setsid(2) follow their man pages. wait4(2) and waitid(2) (selectors,
`WNOHANG`, `WUNTRACED`, `WCONTINUED`, `WNOWAIT`, `__WCLONE`, `__WALL`, the
status word, `siginfo_t`) poll the tree and block in
`LinuxKernel::wait_tree` until the generation moves, interrupted by
signals and restarted with `SA_RESTART` (signal(7)). `rusage` is zero (no
accounting yet).

**Exits, stops and continues tell the parent.** When the last thread of a
Linux process leaves it (`user::end_thread`), the kernel calls
`task::process_exited`: the tree records the zombie (or reaps it) and
re-parents the children, and the parent gets the child's exit signal with
`CLD_EXITED` / `CLD_KILLED` / `CLD_DUMPED`; init gets the same for orphans
that had already ended. `stop_process` and `continue_process` report
`CLD_STOPPED` / `CLD_CONTINUED` unless the parent set `SA_NOCLDSTOP`. A
zombie's process record stays alive (and findable, so kill(2) to a zombie
succeeds) until its parent reaps it.

**execve in the kernel, arguments in the personality.** The personality
reads the path, `argv` and `envp` (`EFAULT`, `ENAMETOOLONG`,
`MAX_ARG_STRLEN`, a total limit, `E2BIG`) and parses `#!` lines; the
kernel opens the file through the process's own working directory
(`Files::open_exec_at`), follows up to four scripts (`ELOOP` beyond), loads
the ELF, and only then (the point of no return) ends every other thread
(a per-thread `killed` flag that ends parks and makes the thread leave at
its next return to user mode; the exec'ing thread waits until they are
gone), takes over the pid if a thread other than the main one called it,
swaps the memory and address space of the running thread, closes
close-on-exec descriptors, resets the signal state (`LinuxProcess::exec`),
the FPU state and the FS/GS bases, releases a vfork parent and sets the
registers for the entry (psABI: all zero but `rsp` and `rip`).

**Thread exit.** `clear_child_tid` is cleared and woken when a thread ends
while its memory has other users (other threads, or another process with
`CLONE_VM`), which is what joining a thread through
`CLONE_CHILD_CLEARTID` and `FUTEX_WAIT` relies on.

## Consequences

- `fork` costs a walk of the page tables that exist (`PageTables::mapped`,
  host-tested in `poly-paging`) and one shared-frame record per resident
  private page, with no copy; the first write to each page after it costs
  a copy (or nothing for the last user).
- `user::Memory` now has three kinds of frames: its own (owned by the
  address space), page-cache pages and shared frames. Every path that
  drops a mapped one unmaps it first; `resident_pages` counts all three.
- The `LinuxKernel` trait has four more methods (`clone_task`, `execve`,
  `with_tree`, `wait_tree`) with default bodies (`ENOSYS`), so other
  implementations keep compiling; `getppid`, `getpgrp` and the group and
  session ids come from the tree.
- Kernel waits used by `wait4` and vfork share the process event queue
  with `park`, so signals, group exits and `execve` interrupt them.
- Not done yet: sharing descriptor tables, file-system context and
  dispositions between processes; pidfds; namespaces; `rusage` and CPU
  time; core dumps (`WCOREDUMP` is never set); `PR_SET_CHILD_SUBREAPER`;
  `execveat`; `#!` and dynamic executables for the kernel's own
  `program::exec_path` (the `execve` system call follows `#!` lines).
- Orders of checks the man pages leave open (clone flags, setpgid errors,
  the `#!` line length, storing the tids, `EFAULT` after reaping) are
  marked "to be confirmed by the diff runner" in the code.
