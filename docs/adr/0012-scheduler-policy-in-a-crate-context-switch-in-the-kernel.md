# 0012: Scheduler policy in a host-tested crate; context switch and stacks in the kernel

**Status:** Accepted. Amended by [0016](0016-interrupt-dispatch-tick-and-kernel-locks.md):
with interrupts the idle thread halts instead of reporting a deadlock.

## Context

Phase 1 needs kernel threads and a preemptive round-robin scheduler. The
kernel has no heap and no timer interrupt yet, runs on one CPU, and every
thread will one day be preempted from an interrupt handler. The bugs that
hurt in schedulers are bookkeeping bugs (a thread on two queues, a lost
wakeup, a slot reused while someone still holds its id) and they are hard to
find in QEMU. The bugs that only the machine can show (a wrong register or
stack layout in the switch) are few and easy to test once.

## Decision

**Split policy from mechanism.** `crates/poly-sched` decides and never
touches a stack: `Scheduler<N>` owns a fixed table of `N` slots, the run
queue and the time-slice counter. Every call that changes the running thread
returns a `Switch { from, to }`. Its unit tests and a randomized check
against a reference model (std collections, same rules) cover the
bookkeeping. `kernel/src/thread.rs` owns stacks, saved stack pointers and the
call into `arch/x86_64/context.rs`, which only saves and restores the six
callee-saved registers and `rsp`.

**No allocation.** Queues are intrusive: one `next` link per slot, valid
because a thread is on at most one list (run queue, one wait queue, free
list). Wait queues are small values owned by whoever waits (a lock, a
device), which also keeps `poly-sched` free of any notion of what is being
waited for. Kernel stacks are order-2 buddy blocks; the table is static.

**Ids carry a generation.** `ThreadId` is slot + generation, and the
generation changes when the slot is freed, so joining or waking through a
stale id fails instead of hitting a new thread.

**Exit is two-phase.** `exit_current` cannot free the stack it runs on. The
thread becomes `Exited`; the next thread to run reaps it (frees the stack,
calls `reap`) right after the switch; the slot is freed when it is reaped
and also joined or detached.

**One lock, interrupts off, released before the switch.** All scheduler
state sits behind one spinlock that is only taken with interrupts disabled,
so a timer interrupt can never find it held on its own CPU. The lock is
released before the switch; on one CPU with interrupts off nothing can run
in between. `preempt_tick()` is the whole interface the timer handler needs.

**The idle thread reports deadlocks.** Until interrupts exist, the idle
thread only runs when every thread is blocked and nothing can wake one, so
it panics with a backtrace rather than hanging the test run.

## Consequences

- Most scheduler changes can be made and tested on the host; the in-kernel
  tests check only what the machine adds (switching, stacks, backtraces,
  freeing frames).
- SMP needs more than this: the outgoing thread is on the run queue before
  its registers are saved, so another CPU must not pick it (or reap it) until
  the switch is complete. Holding the lock across the switch, or an "on CPU"
  flag, are the options; per-CPU run queues come with the SMP milestone.
- A `WaitQueue` must not be dropped while threads wait on it, and must only
  be used with the scheduler its threads belong to; `poly-sched` cannot check
  either.
- Kernel stacks have no guard page until the kernel has its own page tables.
- Thread functions are `fn(usize) -> usize`; closures with captures need a
  heap and can come later as a wrapper.
