# 0053: SMP: starting the CPUs, per-CPU data, per-CPU run queues, TLB shootdowns

**Status:** Accepted. Amends [0012](0012-scheduler-policy-in-a-crate-context-switch-in-the-kernel.md)
(the scheduler gets CPUs), [0016](0016-interrupt-dispatch-tick-and-kernel-locks.md)
("interrupts off" no longer excludes other CPUs; a tick per CPU) and
[0022](0022-user-mode-address-spaces-and-system-calls.md) (address spaces
track the CPUs that have them loaded). Amended by
[0061](0061-per-cpu-interrupt-stacks.md): `gs:[24]` holds the CPU's
interrupt stack.

## Context

Until now the kernel ran on the boot CPU only; the MADT's other processors
stayed in their wait-for-SIPI state. Much of the kernel leaned on that:
"interrupts off" was a lock, the running thread was a global, the
`syscall` entry used one per-CPU block reached through GS but only one
existed, `invlpg` after a page-table change was enough, the futex
compare-and-exchange read and wrote the word in two steps, and tests
counted frames or ticks assuming nothing else ran at the same time.

Sources: Intel SDM Vol. 3A (§8.4 MP initialisation, §10/§11 APIC and
IPIs, §4.10 TLBs and PCIDs, §6.14 IST), AMD APM Vol. 2 (§15.x, start-up
and `SWAPGS`), the ACPI specification (MADT), the MultiProcessor
Specification (INIT-SIPI-SIPI timing).

## Decision

### Bring-up

- **Plan in a crate.** `poly-smp` (host-tested, `no_std`) holds everything
  that is not hardware: which processors to start (`plan`: the boot CPU
  first, then the MADT's enabled processors in order, without duplicates,
  up to `poly.cpus=<n>` and the kernel's table size, none with
  `poly.smp=off`; APIC IDs above 254 are unreachable in xAPIC mode), the
  trampoline's layout and parameter block (`trampoline`), the INIT-SIPI-SIPI
  sequence with the SDM's delays (10 ms after INIT, 200 µs after each SIPI,
  no second SIPI once the AP answered; `startup`, against a trait the
  kernel implements with the local APIC and the clock), and the
  shootdown mailbox (`tlb`).
- **Trampoline.** Four pages below 1 MiB (the lowest page-aligned block
  of usable RAM between 4 KiB and 640 KiB, found in the firmware memory
  map before any allocator exists and marked `Kernel`; the SIPI vector is
  its page number) hold the 16-bit/32-bit/64-bit
  code, its GDT and parameter block, and page tables mapping the first
  2 MiB 1:1 plus the kernel's upper half. The AP goes real mode →
  protected mode → long mode on those tables, then calls `ap_entry` on the
  stack the parameter block names. APs are started one at a time; the
  boot CPU waits up to two seconds per step and leaves out (and logs) one
  that does not answer.
- **Per AP.** `ap_entry` loads the kernel PML4, its own GDT and TSS (each
  with its own double-fault and NMI/machine-check IST stacks), the shared
  IDT, its GS base (its per-CPU area), then does what the boot CPU did for
  itself: EFER.NXE, PAT, CR4.PGE, SMEP/SMAP/PCIDs, FPU/XSAVE, the
  `syscall` MSRs, IA32_TSC_AUX = CPU number (for `RDPID`/`RDTSCP` in the
  vDSO's `getcpu`), its local APIC and a periodic tick. The tick reuses the
  boot CPU's calibration (same divider and count): the APs share the
  boot CPU's bus clock, and calibrating on each would cost boot time and
  add noise.
- **CPU numbers** are dense (0 = boot CPU, then start order) and are what
  the scheduler, `CpuSet`s, sysfs, `/proc/cpuinfo` and `getcpu` use; APIC
  IDs stay in the per-CPU area for IPIs.

### Per-CPU data

Each CPU has a `PerCpu` area (`arch/x86_64/percpu.rs`, at most 16 CPUs), its
address in IA32_GS_BASE while in the kernel. Fixed offsets at the start
serve the assembly: `gs:[0]` the kernel stack for `syscall`, `gs:[8]` the
user RSP scratch slot, `gs:[16]` a pointer to the area itself. The rest is
Rust fields: CPU number, APIC ID, current thread, spinlocks held, interrupt
nesting and "interrupted ring 3", reaping flag, tick count, FPU owner,
active address space and PCID table, a pointer to the CPU's TSS.

The `syscall`/interrupt paths `swapgs` on entry from ring 3, as before.
NMI, double fault and machine check use IST stacks and may arrive with
either GS: they do not guess. Each CPU stores its per-CPU pointer in the top
word of each of its IST stacks; the paranoid entry loads GS base from
there, keeps the old value and restores it on exit.

### Scheduling

- **Policy stays in `poly-sched`.** `Scheduler<N, C>` has a FIFO run queue
  per CPU, one idle thread per CPU, a current thread per CPU, and per-thread
  affinity (`CpuSet`, a 64-bit mask). All of it is behind the kernel's one
  `THREADS` spinlock. One lock for all CPUs is simple and correct, and with
  at most 16 CPUs and short critical sections it is not the bottleneck yet;
  per-CPU locks can come later behind the same API.
- **Placement.** A new or woken thread goes to its previous CPU if that is
  idle, else to an idle CPU it may run on, else to the least loaded one it
  may run on (previous CPU on ties). When a thread lands on an idle CPU's
  queue, the scheduler records that CPU (`take_kicks`); the kernel sends it
  a reschedule IPI (vector 0xf1) after unlocking, which ends its `hlt`.
- **Balancing by stealing.** When a CPU picks its next thread and its own
  queue is empty, or another CPU's queue is at least two longer than its
  own, it takes the first thread there that may run on it. No periodic
  balancer thread.
- **Preemption per CPU.** Each CPU's tick runs `preempt_tick` for that CPU
  (10-tick slices as before; a lone thread keeps running). Sleep deadlines
  are kept globally; whichever CPU's tick sees one due wakes it (claimed
  with a compare-and-exchange so only one does).
- **Switching away safely.** The lock is dropped before the context switch,
  so another CPU could otherwise resume (or reap) a thread whose registers
  are not saved yet. The scheduler marks the outgoing thread as being
  saved; it cannot be picked, stolen or reaped until the CPU that switched
  away calls `saved(cpu)`, first thing on the incoming thread (or implicitly
  at that CPU's next scheduler operation). That call, under the lock, also
  publishes the saved stack pointer.
- **Affinity.** `spawn_on`, `set_affinity` (a running thread outside its
  new set leaves at its CPU's next tick, at once if it is the caller),
  `pin_current` (tests), and Linux `sched_setaffinity`/`sched_getaffinity`
  (inherited by `clone`/`fork`). `smp::Exclusive` drains every other CPU
  (their threads migrate) and parks them in their idle loops with
  interrupts off, for tests that need the old "nothing else runs".
- **FPU state** is saved on every switch away from a user thread and
  restored only if this CPU's registers do not already hold it (per-CPU
  owner plus the CPU the state was last loaded on), because a thread that
  migrated may have stale registers left on its old CPU.

### Locks

`SpinLock` is a real SMP lock (atomic exchange, Acquire/Release) that also
masks interrupts on its CPU while held; the heap's lock likewise. Code that
relied on interrupts-off as a lock took a spinlock instead (CMOS, the
keyboard ring's producer side, the per-CPU tick and IRQ bookkeeping moved
into `PerCpu`). Lock order where locks nest: heap → kernel or user
page-table lock → `FRAMES`; `THREADS` is a leaf (it takes `FRAMES` only
with `try_lock`) and the shootdown mailbox comes after everything.

### TLB shootdowns

- **One request at a time.** `poly_smp::tlb::Shootdowns` is a mailbox: a
  sender takes it, writes the request (an address space's PML4 address, or
  0 for kernel mappings, and a `Flush`: a few page ranges, or everything
  past 32 pages), sets a pending flag per target, sends IPI 0xf2, and waits
  until every target cleared its flag. A target applies the request if it
  concerns the space loaded there (`invlpg` per page, or a CR3 reload that
  flushes the current PCID) or kernel mappings (`invlpg`, or toggling
  CR4.PGE for everything, globals included).
- **No deadlocks with interrupts off.** The sender waits with interrupts
  off and may hold a page-table spinlock; a target may be spinning on that
  very lock, or sending its own shootdown, with interrupts off. So every
  wait loop that runs with interrupts off (spinlock spins, waiting for
  acknowledgements, a parked CPU) serves its own mailbox
  (`arch::wait_hook`). Both sides make progress.
- **Who gets one.** Each address space has a CPU mask of where it is
  loaded (`activate` sets the bit *before* loading CR3, switching away
  clears it). With PCIDs, translations of a space survive on a CPU after it
  switched away; a change therefore first forgets the space's PCID on
  every other CPU (their next load flushes), then reads the mask and shoots
  only CPUs that have it loaded. That order guarantees each CPU either gets
  the IPI or flushes when it next loads the space.
- **When.** Every change that removes or narrows a user translation:
  `munmap`, `mprotect`, taking a page out (page cache page-out, `PROT_NONE`
  parking), copy-on-write's write-protection of the parent at `fork`, and
  page tables freed when a range empties. Frames and page tables freed by
  an operation are collected and freed only after its shootdown
  (`Reclaim`), and the pages of one operation are one shootdown; `fork`
  batches all its write-protections into one at the end. A space that is
  dropped is
  loaded nowhere; translations its PCIDs left on other CPUs are flushed
  when those PCIDs are handed out again. Kernel mappings: unmapping a
  thread's stack (`flush_kernel` to every CPU) before its frames are
  reused.

### Other single-CPU assumptions fixed

- The futex compare-and-exchange is one `lock cmpxchg` on the user word,
  with the same fault fixup as the copy routines.
- `notify_signal`/group exit/stop kick the target threads' CPUs
  (reschedule IPI) so a thread spinning in ring 3 on another CPU sees the
  signal at once, not at its next tick; `ThreadFlags` records the kernel
  thread of each Linux thread.
- Process exit wakes the parent's waiters only after the whole exit is
  done (it could run concurrently with the waiter on another CPU).
- A zombie is waitable only once its parent's `SIGCHLD` is pending
  (`ProcessTree::exit_announced_later` / `announce`): a parent on another
  CPU could otherwise reap the child and go on before the signal came,
  which then interrupted it later (a shell redrawing its prompt).
- MSI messages and I/O APIC routes target the boot CPU's APIC ID from the
  per-CPU area: reading the APIC's ID register gave the running CPU's.
- CPU time accounting reads the running thread per CPU.
- Panics stop the other CPUs with an NMI before printing.
- The status bar, `/proc/cpuinfo`, `/sys/devices/system/cpu/*`, `getcpu`
  (system call and vDSO, via IA32_TSC_AUX) report the CPUs that run.

## Consequences

- `cargo xtask test` boots on 1, 2 and 4 CPUs (and with `poly.cpus=2` /
  `poly.smp=off`); every kernel test passes on each. Tests that assumed
  one CPU now pin themselves (`pin_current`, `spawn_here`) or take
  `smp::Exclusive`, and tests that wait for another CPU wait in time with
  generous deadlines, not in a fixed number of yields.
- One global scheduler lock limits scaling beyond a handful of CPUs;
  replacing it with per-CPU locks and a lock-free kick is a contained
  change inside `thread.rs` and `poly-sched`.
- Interrupts from devices still go to the boot CPU (MSI/MSI-X and the I/O
  APIC target APIC ID of CPU 0); spreading them is future work.
- No CPU hot-plug: the CPU set is fixed after boot (sysfs's `online`,
  `possible` and `present` are equal).
- A shootdown is synchronous and serialised through one mailbox: simple
  and easy to reason about, slower than per-CPU queues under heavy
  `munmap` traffic. Lazy shootdowns are a later optimisation.
