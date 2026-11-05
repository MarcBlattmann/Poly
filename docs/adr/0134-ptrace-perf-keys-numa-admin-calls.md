# 0134: ptrace stops made by the tracee itself, software perf counters, key rings, and the administration calls as the oracle's unprivileged or unsupported cases

**Status:** Accepted. Delivers 39 Linux syscall entries handed from
`spec-linux-f` to `impl-linux-f` (`specs/HANDOFF_LOG.md`): `ptrace`,
`uprobe`, `uretprobe`, `modify_ldt`, `vhangup`; the NUMA and memory calls
`get_mempolicy`, `set_mempolicy`, `mbind`, `move_pages`, `migrate_pages`,
`set_mempolicy_home_node`, `remap_file_pages`, `process_madvise`,
`process_mrelease`; the key calls `add_key`, `keyctl`, `request_key`; the
LSM calls `lsm_get_self_attr`, `lsm_set_self_attr`, `lsm_list_modules`;
the clock calls `settimeofday`, `clock_settime`, `adjtimex`,
`clock_adjtime`; `syslog`, `sysfs`, `ustat`, `acct`, `reboot`, `swapon`,
`swapoff`, `quotactl`, `quotactl_fd`, `init_module`, `finit_module`,
`delete_module`, `open_tree_attr`, `perf_event_open` and `bpf`. Builds on
ADR 0031 (signals), ADR 0041 (the process tree and wait), ADR 0121
(process access) and ADR 0122 (io_uring's shared ring memory).

## Context

The inputs were the spec sheets, their visible tests, the man pages and
the UAPI headers (`linux/ptrace.h`, `asm/ptrace-abi.h`, `asm/user.h`,
`linux/elf.h`, `linux/audit.h`, `linux/perf_event.h`, `asm/ldt.h`,
`linux/keyctl.h`, `linux/lsm.h`, `linux/mempolicy.h`, `linux/timex.h`,
`linux/reboot.h`, `linux/bpf.h`, ...) (CLEAN_ROOM.md). Several of these
calls administer hardware or kernel features Poly does not have (kernel
modules, swap devices, disk quotas, a BPF JIT, a performance monitoring
unit, NUMA nodes); for those, the sheets record what the oracle answers
an unprivileged caller or a caller asking for something it lacks, and
Poly answers that.

## Decision

### ptrace (`personalities/linux/src/task/ptrace.rs`, `sys/ptrace.rs`)

- **Links in the process tree.** Who traces whom is kept in the
  `ProcessTree`, keyed by the tracee (Poly traces processes as single
  threads; a tracee's new threads are not attached). The tracer's
  wait(2) looks at its tracees as at its children in one pass: a
  ptrace-stop is `ChildEvent::Trapped(sig | event << 8)` (status
  `sig << 8 | event << 16 | 0x7f`), reported once and without
  `WUNTRACED`; a tracee's end goes to a tracer that is not its parent
  first, and only then can the parent reap it (it waits meanwhile).
- **A tracee stops itself.** At a stop the tracee saves its registers
  (as `struct user_regs_struct`, with `orig_rax` from the system call it
  is in), its FPU area and its signal mask into its link, tells the
  tracer (`SIGCHLD`, `CLD_TRAPPED`) and blocks on the tree's generation.
  The tracer's requests (`GETREGS`, `POKEUSER`, `GETSIGINFO`,
  `SETSIGMASK`, `GET_SYSCALL_INFO`, ...) read and change that copy under
  the tree lock; the restarting request leaves a `Resume` and the tracee
  loads what changed. Nothing reaches into another thread's kernel state.
  `SIGKILL` (or a group exit) ends any stop.
- **Where it stops:** signal delivery (every signal but `SIGKILL`, also
  ignored ones: a traced process keeps them, like the test run's own
  tracing); `PTRACE_INTERRUPT` and the first stop of an auto-attached
  child (`PTRACE_EVENT_STOP`; a child of an `ATTACH`ed tracer gets
  `SIGSTOP`), on the tracee's next way to user mode or into the kernel
  (the kernel notifies it); system call entry and exit after
  `PTRACE_SYSCALL` (`Linux::dispatch_with`; `orig_rax` -1 skips the call);
  fork, vfork and clone events (a kernel hook in `clone_task`, before a
  vfork parent waits; the child is linked to the tracer as it enters the
  tree, before it runs); `VFORK_DONE`; exec (`PTRACE_EVENT_EXEC`, or a
  `SIGTRAP` with `SI_USER`); exit (`PTRACE_EVENT_EXIT` with the wait
  status).
- **Memory.** `PEEK*` read the tracee's memory (`EIO` where unmapped);
  `POKE*` write it with `Memory::write_forced`: a private region the
  process may not write gets a private copy of the page that keeps its
  protection (`VmMap::fault_forced`); a read-only shared mapping is
  refused.
- **Permissions:** the ptrace access check of process_vm_readv (ADR 0121)
  plus Yama's `ptrace_scope` 1 (the oracle's): an unprivileged tracer
  attaches only to its descendants. A process traced by the test run's
  tracer counts as traced (`EPERM`), and `CLONE_UNTRACED` children are not
  followed by the test run either.
- Not done: multi-threaded tracees, `PTRACE_LISTEN` (always `EIO`), group
  stops of tracees in detail, seccomp events, hardware breakpoints
  (`POKEUSER` of a debug register only accepts 0). `PTRACE_SINGLESTEP`
  sets the trap flag and clears it at the next stop.

### perf_event_open (`personalities/linux/src/perf.rs`, `sys/perf.rs`)

Poly has no PMU, so only software events exist (hardware, cache and raw
events are `ENOENT`, as on a machine without a PMU). A counter is a
running difference of what the kernel accounts per process anyway: CPU
time for task-clock and cpu-clock, page faults for page-faults (and
-min; -maj stays 0); the other software events open and stay 0.
`read_format`, groups, ids, `ENABLE`/`DISABLE`/`RESET` (with
`PERF_IOC_FLAG_GROUP`), inheritance (children's counts are added when
they exit) and prctl's `PR_TASK_PERF_EVENTS_*` work. The checks follow
the sheet's order with `perf_event_paranoid` 2. The mmap ring is
io_uring's shared ring memory with only the header's data offset and
size filled in; no samples are ever written.

### modify_ldt, uprobe, uretprobe

The LDT is kept per process in the personality (threads share it, fork
copies it, exec drops it) and read and written exactly as the sheet
records, but it is not loaded into the CPU: that needs new
architecture code (an LDT descriptor in each CPU's GDT, `lldt` at
context switches), which is not agent work (AGENT_PIPELINE.md, safety
rules). A program that loads a selector of its LDT gets `SIGSEGV`.
uprobe is always `ENXIO`; uretprobe forces `SIGILL` (`SI_KERNEL`) and
returns -1, as on Linux for any caller outside the kernel's trampolines.
