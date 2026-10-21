# 0070: CPU time split by measured mode changes, not by tick samples

**Status:** Accepted. Amends [0044](0044-time-syscalls-timers-and-the-vdso.md)
(how `cputime.rs` splits a thread's run time into user and system time).

## Context

ADR 0044 measured each thread's run time with the monotonic clock at
context switches and split it into user and system time *in the ratio of
tick samples*: every tick counted one sample for the running thread, user
if it had interrupted ring 3. A read clamped the split so neither part ever
went down.

Under host load the time test's `busy_loops_are_user_time` check failed in
about 5 of 60 boots (two CPUs, 256 MiB, five boots at a time next to two
busy loops on a four-CPU host that other work kept at a load of 20 to 35):
up to 60% of a 200 ms busy loop in ring 3 came out as system time. Logging
every sample of the test's process showed two faults of the method, both
real accounting errors rather than a test that expected too much:

- **New time was split by old samples.** The ratio is of the thread's
  whole history. A busy host merges ticks that come due while QEMU's vCPU
  is held off, so the 200 ms loop collected only 10 to 20 samples, against
  about 200 from the earlier, system-heavy part of the run (exec, the vDSO
  checks, sleeps and timers). The loop's 200 ms were therefore split almost
  in the history's ratio, mostly system time, although nearly all of its
  own samples were user samples (`4/0 2/0 4/0` user/system per 50 ms).
- **The samples were biased.** Under QEMU's TCG every clock read is an HPET
  access that takes QEMU's global lock, which the main loop also holds when
  it raises the timer interrupt. A tick that comes due during a clock read
  is delivered right after it, in the kernel: the most frequent interrupted
  address of system samples was the instruction after the HPET counter
  read, even unloaded (74 of 138 system samples in one run).

## Decision

Measure instead of sampling. Each CPU keeps a `poly_sched::cputime::Stint`
for the thread it runs: the mode the thread is in since a mark (a TSC
reading) and the cycles it used in each mode before. The architecture's
entry code reports every mode change: `cputime::entered_kernel` at the
start of the `syscall` handler, of `irq_entry` and of `trap_dispatch` for
anything from ring 3 (not NMI, machine check or double fault, which leave
through their own exit); `cputime::returning_to_user` before SYSRET and in
`trap_return` for every return to ring 3 (interrupts off from there to the
IRETQ). At a context switch the stint's cycles move to the outgoing
thread's slot and a new stint starts. A thread's measured run time is then
split in the ratio of its cycles (`poly_sched::cputime::Split`, which keeps
the never-goes-down clamp), so time counts in the mode it was spent in.

The mode changes take no lock: a CPU's stint is written only by that CPU
with interrupts off, and read by other CPUs (for a thread running there)
through a sequence count. The total stays on the monotonic clock, so the
CPU-time clocks agree with `CLOCK_MONOTONIC` as before; the TSC only gives
the ratio, so it need not be calibrated or invariant (a reading behind the
mark, as another CPU's TSC may be, counts as no time). The tick no longer
takes part, and the per-CPU "interrupted ring 3" flag is gone.

## Consequences

- With the same load harness, the time test failed in 14 of 300 baseline
  boots (12 of them `busy_loops_are_user_time`) and in none of 109 with
  this change and the test changes below, on two and on four CPUs.
  A thread that ran less than a tick no longer shows all of its time as
  system time.
- A running thread is still charged for the time the host holds its vCPU
  off, in the mode it was in. Under TCG that is mostly system time: a
  system call that reads the clock can wait tens of milliseconds for
  QEMU's global lock on a busy host (stints of the time test's thread of
  60 ms with 12 ticks, 57 ms of it in the kernel). Measured precisely, a
  busy loop that read the clock through a system call every few
  milliseconds was sometimes more system time than user time on four
  CPUs. So the time test's busy loop now does a fixed amount of work with
  no system calls, and its sleep check
  (`thread_cpu_clock_stands_still_while_asleep`, which saw up to 120 ms
  charged around a 50 ms sleep) compares the CPU time charged with the
  clock time that encloses it, over up to three attempts: a thread charged
  for its sleep fails all three, a stall spoils one.
- Every entry from and return to ring 3 costs a TSC read and a few stores
  to per-CPU data.
