# 0031: Linux signals and futexes: personality-owned state, kernel-provided blocking

**Status:** Accepted

## Context

Phase 4 needs signal delivery (handlers, `sigaltstack`, `rt_sigreturn`,
process groups) and futexes (thread libraries, robust mutexes,
`set_tid_address`). Both are mostly rules from the man pages and the UAPI
headers (queues, masks, default actions, frame layout, futex operation
encodings, the robust list), which ADR 0001 and 0025 want host-tested in
the personality. But both also need things only the kernel has: other
threads and processes, blocking and waking, the full user register frame
and the FPU state, atomic access to user memory, and a lock that works
across CPUs. ADR 0025's trait gave the personality the signal mask and
dispositions as plain `&mut` accessors, which cannot be shared safely once
other threads send signals.

## Decision

- **The personality owns the state, the kernel stores and locks it.** One
  `LinuxProcess` per Linux process holds the dispositions, the shared
  pending queue and per thread (`LinuxThread`) the mask, the private
  queue, the alternate stack, the rt_sigsuspend mask, the pending restart
  and restart block, the robust list head and `clear_child_tid`. The
  kernel creates it (`new`, `fork`), tells it about threads (`add_thread`,
  `remove_thread`, `exec`) and lends it out under its lock through
  `LinuxKernel::with_process(target, f)`: closure-scoped, so the lock is
  never held across a block or while another process's state is used.
  The old `signal_mask`, `signal_actions` and `set_clear_child_tid`
  methods are gone (no kernel implemented them yet).
- **Blocking is "park/unpark".** `block(deadline)` returns `Woken`,
  `TimedOut` or `Signal`; `unblock(tid)` before `block` is not lost; the
  per-thread signal notification (`notify_signal`, cleared by the thread)
  also ends a block. Callers re-check what they wait for, so spurious
  returns are harmless. With this, futex waits are lost-wakeup free
  without holding the futex lock while asleep: compare and enqueue under
  the lock, unlock, block; wakers dequeue under the lock and unblock.
- **Futexes.** One system-wide `FutexTable` (FIFO queues of thread ids with
  bitsets, keyed by `FutexKey::Private { space, addr }` or
  `Shared { object, offset }`) lives in the kernel and is lent out by
  `with_futexes(f)`; `f` gets the kernel back for `futex_load` /
  `futex_cmpxchg` (atomic, non-sleeping user accesses). `futex_key`
  defaults to private keys, which is right until shared mappings exist.
  PI operations return `ENOSYS` (futex(2) documents that for kernels
  without PI support). The table is a module of the personality, not a
  crate, so the kernel's lock file does not change; it has no Linux
  encodings and can move to `crates/` when NT or XNU want it.
- **Delivery happens on the way out.** The kernel calls
  `Linux::exit_to_user(k)` before every return to user mode. It drops
  ignored signals, stops (`stop_process`) or kills (`kill_process`) per the
  default actions, and builds the x86-64 frame (return address to
  `sa_restorer`, `struct ucontext` with `struct sigcontext`, `siginfo_t`,
  the FXSAVE/XSAVE area with `_fpx_sw_bytes` and `FP_XSTATE_MAGIC2`) into a
  buffer from `user_context()` and `fpu_state()`, copies it to the user
  stack and installs the handler's registers with `set_user_context`.
  rt_sigreturn validates the frame (canonical `rip`/`rsp`, the current
  64-bit `cs`, `ss` per `asm/ucontext.h`'s `UC_STRICT_RESTORE_SS` rule,
  only user-changeable `rflags` bits) and restores registers, mask, FPU
  state (the kernel checks the image) and alternate stack; a bad frame is
  `SIGSEGV`.
- **Interrupted system calls.** Handlers return one of four internal
  errnos (`Errno::RESTART_*`, values far above Linux's user errnos). The
  dispatcher puts `-EINTR` in `rax` and records the policy; `exit_to_user`
  decides once it knows whether a handler runs (signal(7)): restart unless
  a handler without `SA_RESTART` runs, always, only without a handler, or
  through restart_syscall(2) with saved state (only without a handler,
  since restarting a relative timeout after a handler would extend it).
  Restarting rewinds `rip` by 2 and puts the number back in `rax`.
- **Sending.** `kill`, `tkill`, `tgkill`, `rt_sigqueueinfo` and
  `rt_tgsigqueueinfo` check permissions per kill(2) (real or effective uid
  against real or saved uid; `SIGCONT` within the session; effective uid 0
  stands in for `CAP_KILL`), queue (standard signals merge, real-time ones
  queue with siginfo, up to a per-queue limit), apply the generation rules
  (ignored signals discarded, stop/continue cancel each other, init only
  gets handled signals) and notify the thread(s) that should act.
  `process_ids(selector)` gives the members of a process group or every
  process.
- **Thread exit.** `Linux::thread_exit(k, shares_memory)` walks the robust
  list (bounded by `ROBUST_LIST_LIMIT`, stopping at unreadable links,
  `FUTEX_OWNER_DIED` plus a one-waiter wake for owned locks) and handles
  `clear_child_tid` (write 0, wake one waiter). Kernel faults that become
  signals go through `Linux::fault_signal(k, info)`.
- **Clean room.** Behaviour from signal(7), sigaction(2), sigaltstack(2),
  sigreturn(2), restart_syscall(2), kill(2), tgkill(2), sigprocmask(2),
  sigpending(2), sigsuspend(2), sigwaitinfo(2), rt_sigqueueinfo(2),
  futex(2), futex(7), set_robust_list(2), set_tid_address(2) and POSIX;
  layouts and constants from `asm/signal.h`, `asm/sigcontext.h`,
  `asm/ucontext.h`, `asm-generic/ucontext.h`, `asm-generic/siginfo.h`,
  `asm-generic/signal-defs.h` and `linux/futex.h`. No Linux or libc source
  was read and nothing was observed on a real system. Every point the
  documents leave open is marked "to be confirmed by the diff runner" in
  the code.

## Consequences

- The kernel implements about twenty more trait methods, most of them
  thin (ids, notifications, register frame access, a futex lock, park and
  unpark), and must call `exit_to_user` on every return to user mode and
  `thread_exit` on every thread exit. Returns after `set_user_context`
  must use `iret`, since `rcx` and `r11` are restored.
- Locking is per process for signal state and global for futexes; both are
  only held inside closures that never block, so a spinlock suffices. A
  global futex lock may need hashing into buckets once SMP makes it hot.
- Parent notification on stop and continue (`SIGCHLD`, wait status), core
  dumps and the orphaned-process-group rule for `SIGTSTP`/`SIGTTIN`/`SIGTTOU`
  are left to the kernel and the TTY milestone.
- Several orders and edge cases (delivery order among standard signals,
  frame placement, `oldmask`, error order) are Poly's choices until the
  diff runner confirms or corrects them.
