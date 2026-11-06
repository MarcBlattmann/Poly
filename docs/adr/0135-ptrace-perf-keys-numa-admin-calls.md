# 0135: ptrace stops made by the tracee itself, software perf counters, key rings, and the administration calls as the oracle's unprivileged or unsupported cases

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

### Key rings, NUMA, the clock discipline, LSM (`keys.rs`, `sys/keys.rs`, `sys/numa.rs`, `sys/procmem.rs`, `sys/timex.rs`, `sys/lsm.rs`)

- **Keys** (keyrings(7)): one table of keys under random 31-bit serials,
  keyrings as lists of linked keys, thread, process, session and per-user
  keyrings, possession by search from the caller's keyrings, `add_key`,
  `request_key` and the `keyctl` commands with their permission bytes.
  Poly has no `/sbin/request-key`, so a request that finds nothing is
  `ENOKEY`.
- **NUMA and memory**: Poly has one node, node 0. The calls keep their
  policies (per thread for `set_mempolicy`, copied by clone and fork and
  kept by exec; per page range for `mbind`, copied by fork and dropped by
  exec and by unmapping) and check their arguments; nothing ever moves.
  `remap_file_pages` is emulated, `process_madvise` only checks other
  processes' ranges, `process_mrelease` answers and leaves the freeing to
  the process's end.
- **Clock discipline**: `adjtimex` and `clock_adjtime` keep the NTP model
  as bookkeeping (what is set is reported back, `ADJ_SETOFFSET` steps
  `CLOCK_REALTIME`); the clock stays `STA_UNSYNC` until something clears
  it. `settimeofday` and `clock_settime` mark it unsynchronised again.
- **LSM calls**: the active modules are `capability` and Landlock;
  neither has task attributes, so getting one is `EOPNOTSUPP`.

### The administration calls (`sys/syslog.rs`, `klog.rs`, `sys/fsinfo.rs`, `sys/hostadmin.rs`, `sys/admin.rs`, `sys/mountapi.rs`)

- **syslog** keeps a kernel log buffer in the personality (`klog.rs`): a
  ring of 256 KiB of `<level>[seconds.micros] message` lines with a reader
  position (`READ`), a clear position (`READ_ALL`) and the console level.
  `kernel.dmesg_restrict` is 0: `READ_ALL` and `SIZE_BUFFER` are open to
  everyone, everything else needs `CAP_SYSLOG` or `CAP_SYS_ADMIN`, before
  any argument. The buffer starts with two lines of Poly's own; the kernel
  can add lines with `klog::log`. `READ` cannot wait for new text and
  answers `EAGAIN` when there is none.
- **sysfs / ustat**: `sysfs` numbers the same list `/proc/filesystems`
  shows (`sys::fsinfo::FILE_SYSTEM_TYPES`, which procfs now reads, so the
  two cannot differ). `ustat` finds the mounted file system by its
  `st_dev` (a new `LinuxFs::statfs_dev`) and fills `struct ustat` from its
  statfs; no such device is `EINVAL`, before the buffer is looked at.
- **swap, quotas, modules**: Poly has none, and the calls refuse in the
  order of the sheets. `swapon`/`swapoff`: flags, `CAP_SYS_ADMIN`, the path,
  then `EISDIR` for a directory and `EINVAL` for anything else. `quotactl`:
  the type, `Q_SYNC` without a device syncs, other commands without a
  device are `ENODEV`, a device must be a block device (`ENOTBLK`) and has
  no quotas (`ENODEV`); `quotactl_fd`: `EBADF`, the type, then `ENOSYS`.
  `init_module`: `CAP_SYS_MODULE`, `ENOMEM` for lengths past 1 GiB, every
  byte of the image must be readable (`EFAULT`), then `ENOEXEC`.
  `finit_module`: capability, flags, the descriptor (`EBADF`; also
  `O_PATH` and write-only), a regular file with data (`EINVAL`), no
  writers (`ETXTBSY`, from a new `NodeRef::has_writers`), compressed files
  `EOPNOTSUPP`, then `ENOEXEC`. `delete_module`: capability, the name
  (cut at 55 bytes), then `ENOENT`.
- **reboot** checks `CAP_SYS_BOOT` in the user namespace that owns the
  caller's pid namespace first, then the magic numbers, then the command.
  In a pid namespace other than the initial one the machine is never
  touched: `RESTART`, `RESTART2`, `HALT` and `POWER_OFF` end the
  namespace (its init is killed by `SIGHUP`, or `SIGINT` for the last two,
  and the others by `SIGKILL`), every other command is `EINVAL`.
- **open_tree_attr** is `open_tree` followed by `mount_setattr` on the
  descriptor, with the checks in the sheet's order and the descriptor
  closed again when an attribute is refused. `MOUNT_ATTR_IDMAP` with a
  `userns_fd` that is not open is `EBADF`, with one that is open `EINVAL`
  (Poly has no id-mapped mounts).
- **vhangup** needs `CAP_SYS_TTY_CONFIG`. The terminal counts its hang-ups
  and every open file remembers the count it was opened at
  (`poly_tty::TtyFile`): a file from before is revoked (reads return end of
  file, writes and ioctls `EIO`), files opened afterwards work. The
  session leader and foreground group get the signals of a master's
  hang-up and the session loses the terminal.
- **acct** (`acct.rs`): BSD accounting belongs to a pid namespace, kept as
  the file the kernel opened for appending (`LinuxKernel::acct_open`). A
  process that ends appends a 64-byte `struct acct_v3` to the file of its
  own namespace and of every namespace above it that has one, before its
  parent can wait for it (`task::process_exited`); naming a file again, or
  `acct(NULL)`, writes the caller's record to the file being left. Times
  and fault counts come from the kernel's accounting; `ac_tty` is 0,
  `ASU` is never set, and the file of a namespace whose init ended is
  closed. A FIFO is `EACCES` at once instead of blocking in the open.

### bpf (`bpf.rs`, `sys/bpf.rs`, `linux/bpf.rs`)

Poly has no BPF interpreter. The maps that need none exist: array maps
(every element exists, zeroed, no deletion) and hash maps, with the update
flags, `E2BIG` for a full map, `BPF_MAP_FREEZE`, `BPF_F_RDONLY` and
`BPF_F_WRONLY`, `BPF_OBJ_GET_INFO_BY_FD` (the first 104 bytes of
`struct bpf_map_info`) and fdinfo lines (a new `FileObject::fdinfo`).
`BPF_PROG_LOAD` takes socket filters through a straight-line verifier (an
`exit` is needed at the end and `r0` must be set before it; the log goes to
the caller's buffer); a loaded program is never run. The attribute is
copied the way the sheet says: `EFAULT` for NULL, `E2BIG` past a page or
for non-zero bytes past the 160 bytes Poly's union has. With
`kernel.unprivileged_bpf_disabled` 2, creating a map or loading a program
needs `CAP_BPF` or `CAP_SYS_ADMIN`, after the attribute passed.

### Known limits

- `modify_ldt`'s descriptors are not loaded into the CPU (above), so
  `f_modify_ldt_write` fails at its `load_es(7)`: loading an LDT selector
  needs `lldt` and an LDT descriptor in each CPU's GDT, which is
  `unsafe` architecture code a human has to write.
- `syslog(READ)` does not block; `acct` does not set `ASU` and reports no
  controlling terminal; `bpf` has no other map types, no program
  attachment and no pinning.
