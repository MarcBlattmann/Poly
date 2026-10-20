# 0016: Interrupt dispatch, a 1 kHz periodic tick, and kernel locks that mask interrupts

**Status:** Accepted. Amends [0012](0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md):
the idle thread halts instead of reporting a deadlock once interrupts are
set up; a self-deadlock on a `Mutex` is reported instead.

## Context

[ADR 0013](0013-interrupt-controllers-and-time-base.md) put the interrupt
controllers and the time base in `poly-irq`. The kernel now has to wire them
up: take external interrupts, give drivers (PCI MSI-X next) a way to get a
vector, preempt threads from the timer, let threads sleep, and make every
lock safe against an interrupt handler that takes the same lock. All of this
runs on one CPU for now, but should not have to be redone for SMP.

## Decision

**One IDT stub per vector, one dispatcher.** Vectors 32-255 get assembly
stubs 16 bytes apart (generated with `.rept`, checked at boot) that push the
same `TrapFrame` as the exceptions. `arch::irq::dispatch` sends the timer
vector to the tick, the local APIC's error vector to a logger, the spurious
vector (0xff) nowhere (no EOI, SDM §11.9), and the 8259's remapped vectors
(0x20-0x2f, all masked) to a spurious-IRQ check. Everything else is a device
vector: its handler runs, then the EOI. All gates are interrupt gates, so
handlers never nest, and none uses an IST stack: the timer may switch
threads from inside its handler, so the trap frame must stay on the
interrupted thread's stack.

**Vectors are allocated, not assigned.** `allocate_vector(handler, data)`
claims one of 0x30-0xef with a compare-exchange on a per-vector slot (no lock,
no heap) and installs `handler(data)`; `free_vector` releases it. The caller
then points something at it: `msi_message(vector)` (MSI / MSI-X, encoded by
`poly_irq::msi`, host-tested) or `route_gsi` / `route_isa_irq` (I/O APIC,
ISA routes from the MADT overrides). 0xf0-0xff are fixed: tick, IPIs later,
error, spurious.

**A periodic 1 kHz tick from the local APIC timer.** The APIC timer is
calibrated against the HPET (or the ACPI PM timer) like the TSC and runs in
periodic mode. 1 kHz makes the 10-tick time slice 10 ms and lets `sleep`
wake within a millisecond, for one interrupt per millisecond. TSC-deadline
mode and a tickless idle are later optimisations; neither changes the
interfaces (`preempt_tick`, `sleep_until`).

**The tick's order of work** is fixed: count the tick (and read a narrow
reference counter so the clock can extend it), EOI, then
`thread::on_timer_tick`, which wakes due sleepers and calls `preempt_tick`.
EOI comes before any thread switch, because the thread switched to may run
for a long time and a new thread never returns through the handler.

**Every spinlock masks interrupts while held** (`InterruptsOff` inside the
guard; `RawIrqSpinLock` for the heap). An interrupt handler can then take
any spinlock without deadlocking against the code it interrupted, and a
spinlock holder is never preempted. The kernel counts held spinlocks and the
scheduler refuses to switch while any is held, or from inside an interrupt
handler, which turns "slept while holding a spinlock" into a panic with a
backtrace instead of a hang.

**A sleeping `Mutex` on the scheduler's wait queues.** `lock` tries a
compare-exchange and otherwise waits with the acquisition itself as the
wait condition, which `WaitQueue::wait_until` checks with the scheduler
locked, so a release cannot be lost. It is not FIFO-fair (a running thread
may take a just-released mutex). It records its owner, so locking a mutex
the thread already holds panics with "deadlock" and a backtrace.

**Sleep is a deadline and one shared wait queue.** `sleep_until(deadline)`
waits until the clock passes the deadline and registers the earliest
deadline in an atomic; the tick wakes all sleepers only when that deadline
has passed, and the ones not yet due go back to sleep. Few sleepers exist;
a timer wheel or heap replaces this when many do.

**Deadlock reporting.** With interrupts, "every thread is blocked" is normal
(everyone sleeps), so the idle thread halts (`sti; hlt`) instead of
panicking, except before interrupts are set up, when nothing could wake a
thread. The deadlock negative boot test now locks a `Mutex` twice.

## Consequences

- Drivers get interrupts with three calls (allocate, build the MSI message
  or route the GSI, free) and no knowledge of the APIC.
- A long console write holds the console spinlock, and thus interrupts off,
  for as long as the UART takes; at 115200 baud that is about 10 ms per 100
  characters on real hardware. A buffered console fixes that later.
- A thread that waits for a device to be woken by an interrupt handler uses
  a `WaitQueue`: handlers may wake threads but never block.
- SMP needs: per-CPU held-lock and interrupt-depth counters, per-CPU local
  APIC setup and tick on each application processor, IPIs (0xf1-0xfd) for
  rescheduling and TLB shootdown, the scheduler lock (or an "on CPU" flag)
  held across the switch (see ADR 0012), I/O APIC and MSI destinations
  beyond the boot CPU, and a check that the TSCs agree across CPUs before
  the clock relies on them.
- Under QEMU without KVM the TSC is not invariant, so the clock reads the
  HPET (one MMIO read); with KVM or on hardware with an invariant TSC it
  reads the TSC. QEMU 8.2's TCG has no x2APIC, so that mode is covered by
  `poly-irq`'s host tests (both backends against one simulated APIC) but not
  by the boot tests.
