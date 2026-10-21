# 0061: Per-CPU interrupt stacks; switching threads only on the way out

**Status:** Accepted. Amends [0016](0016-interrupt-dispatch-tick-and-kernel-locks.md)
(handlers no longer run on the interrupted thread's stack; the tick's
preemption moves to the interrupt's way out),
[0050](0050-kernel-thread-stacks-with-guards.md) (a second stack area, for
the interrupt stacks) and [0053](0053-smp-per-cpu-data-scheduling-and-tlb-shootdowns.md)
(a per-CPU field the entry path reads, `gs:[24]`).

## Context

Under heavy host load the `dyn-threads` boot test (a dynamically linked
pthreads program as `poly.init`) once ended in a kernel stack overflow in
the timer tick. With a load harness (40 boots at a time next to busy loops)
it came back in about 1 boot in 15, always the same way:

```text
CPU exception 8 (double fault), rip ...on_timer_tick+0x2d, rsp 0xffffc0800003bc90, cr2 0xffffc0800003bc88
kernel stack overflow in thread user (slot 7): hit the guard below its stack
  #0 poly_kernel::thread::on_timer_tick+0x2d
  #1 poly_kernel::arch::x86_64::idt::trap_dispatch+0xe1
  #2 isr_common+0x42
  #3 <poly_kernel::process::Process>::attach_thread_as+0x235
  #4 poly_kernel::user::start_thread+0x47
  #5 poly_kernel::user::spawn_child+0x1f9
  #6 poly_kernel::linux::task::clone_task+0x956
  #7 poly_personality_linux::sys::task::clone3+0x37c
  ...
  #14 syscall_entry+0x4c
```

Two things added up. Interrupts ran on the interrupted thread's kernel
stack (ADR 0016, so the tick could switch threads with its frames still on
that stack), so every thread stack had to hold its deepest system call
*plus* the deepest interrupt path. And both were large in the dev build
(`opt-level = 1`, the CI configuration):

- `clone3` of a thread went 15,792 of the 16,384 bytes deep by itself
  (measured by painting the stacks, below): `clone3` had a page-sized
  buffer on its frame for a rarely used check (4.5 KiB frame),
  `clone_task` kept both the thread and the process branch's locals in
  one 6.6 KiB frame, and `LinuxState::add_thread` built a whole
  `LinuxProcess` on its 2.8 KiB frame for the first thread only.
- The tick then needed 1.6 KiB on top: the trap frame, `trap_dispatch`,
  and `on_timer_tick` with `wake_sleepers` inlined and its 1 KiB array of
  due deadlines in the frame on every tick, though almost every tick has
  none. A tick that preempted the thread added the switch, and the thread
  switched to then reaped exited threads on *its* stack (`finish_switch`
  dropping address spaces), wherever that stack happened to be.

Load only made it likelier: a tick has to land in the few microseconds the
clone path spends at its deepest, and a host that holds the vCPU off makes
ticks come due in bursts at arbitrary points.

## Decision

**Each CPU has an interrupt stack**, 16 KiB above an unmapped 16 KiB guard,
in a second stack area right after the thread stacks (one more page table
made at boot, `paging::INTERRUPT_STACKS_BASE`; `irqstack.rs`). The boot CPU
gets its stack in `init_interrupts`, each AP before it is started. The top
is in the per-CPU area at `gs:[24]`.

**External interrupts and IPIs enter through `irq_common`**, which pushes
the trap frame where the CPU put it (the thread's stack, as before),
switches to the CPU's interrupt stack for the handler (`irq_entry` →
`irq::dispatch`), switches back, and runs the way out there (`irq_exit`).
Exceptions keep `isr_common` on the thread's stack: a page fault may
block. No IST is used for interrupts: an IST stack is reset on every entry,
and the trap frame must stay with the thread it belongs to.

**Switching threads happens only on the way out**, on the thread's own
stack. The tick's handler counts the tick, sends the EOI and wakes due
sleepers (on the interrupt stack), then leaves "check the time slice" in
the per-CPU area; the reschedule IPI leaves "switch". `irq::run_deferred`
does that after the switch back (`preempt_tick`, `on_reschedule`), then a
return to ring 3 gets its signals. So an interrupt costs the interrupted
thread its trap frame and the switch path, nothing a handler does.

**Reaping runs on the interrupt stack too.** `finish_switch` (and the reap
at the start of `spawn`) runs through `irqstack::run_on`, which needs
interrupts off (they are, on every switch) and switches the stack for the
call: freeing stacks and dropping exited threads' address spaces no longer
lands on whatever thread was switched to, deep in a system call or on its
way out of an interrupt.

**Nothing nests on an interrupt stack.** Interrupt gates keep interrupts
off in handlers, and `run_on` requires them off. An interrupt that arrives
on the interrupt stack all the same (a bug) stays on it, is counted
(`irqstack::nested`, checked by a kernel test) and does not switch threads.

**The large frames on the clone path are gone**: `clone3` checks the bytes
past the fields it knows in 64-byte pieces, `clone_task` builds each kind
of child in a frame of its own, `add_thread` resets a first thread's state
out of line, and `wake_sleepers`' scan is out of line (`#[cold]`).

**Measuring.** Thread and interrupt stacks are painted when allocated
(`poly_mem::STACK_PAINT`); `mm::stack_used` reads how deep one was ever
used. The reaper records the deepest thread stack;
`poly.debug.stack_report` prints that and each interrupt stack's at the
end of a test run. `poly.debug.thread_stack_kib=<n>` maps only the top
`n` KiB of every thread stack (the rest becomes guard), to find paths that
come close to the limit. Backtraces follow the frame chain from an
interrupt stack into the interrupted one (`poly_backtrace::FrameWalker`
moves between known stacks, at most twice per walk), and an overflow of
an interrupt stack is reported as such.

The stacks stay 16 KiB: with the frames above trimmed, the deepest thread
stack use in the boot tests is RESULT, and the worst an interrupt adds is
INTERRUPT bytes (`INTERRUPT_STACK_BUDGET`, 1 KiB, is checked by a kernel
test). Raising the size would have hidden the next large frame instead of
showing it.

## Consequences

- An interrupt costs a thread's stack at most `INTERRUPT_STACK_BUDGET`
  (1 KiB) below wherever it was, independent of the handlers;
  `thread::interrupts_take_little_of_the_interrupted_threads_stack`
  checks that deterministically (a thread spinning with just over 1 KiB
  left is preempted, resumed and switched to while threads are reaped,
  over and over; before this change it ran into its guard).
- Each CPU uses 16 KiB more memory; the area has room for 64 CPUs, the
  per-CPU table for 16.
- `thread::preempt_tick` and `on_reschedule` are no longer called by the
  handlers but by the way out; code that wants to switch from an interrupt
  defers in the same way (`irq::defer`).
- Handlers are still not allowed to block or switch: they run on a stack
  the next interrupt reuses.
- System calls that go deep still compete only with themselves; painting
  shows how close they come (`poly.debug.stack_report`), and
  `poly.debug.thread_stack_kib` turns "close" into a reported overflow
  with a backtrace.
