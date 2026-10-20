# 0018: MSI and MSI-X for PCI functions, interrupt-driven disks with sleeping waiters, a watchdog on the clock

**Status:** Accepted. Supersedes the waiting and watchdog parts of
[ADR 0017](0017-block-layer-in-the-kernel.md) (waiting by polling with the
disk's spinlock held; a watchdog that counts polls). Builds on
[ADR 0016](0016-interrupt-dispatch-tick-and-kernel-locks.md) (vector
allocation, `msi_message`, `WaitQueue`, `Mutex`).

## Context

The last Phase 1 milestone is "PCI/PCIe enumeration (ECAM), MSI/MSI-X
interrupts". Enumeration exists; nothing enables MSI or MSI-X yet. The
kernel has the other half since ADR 0016: device vectors from
`allocate_vector`, the x86 message from `msi_message`, handlers that may
wake a `WaitQueue`. `poly-pci` can parse and program both capabilities.

The block layer (ADR 0017) waits for its disk by polling with the disk's
`SpinLock` held, so interrupts are off for the whole of every disk
operation: the tick stalls, nothing is preempted, and a long request
blocks the CPU. ADR 0017 named the way out: an interrupt handler that runs
`RequestQueue::process` under a short interrupt-safe lock, waiters that
sleep, a `BufferCache` that can sleep, and a watchdog that measures time
instead of polls, since nobody polls in a loop any more.

## Decision

**MSI-X and MSI are generic PCI services in `kernel/src/pci.rs`.**
`Device::enable_msix` maps the table and the PBA (through `map_bar`, so a
BAR shared with the driver is mapped once), turns MSI and INTx off and
memory decoding on, and enables MSI-X with the function mask set and every
entry masked. The driver then `attach`es the entries it uses (each gets a
fresh kernel vector with its handler, and the local APIC message for it,
written unmasked) and clears the function mask: no interrupt can be sent
before its entry is programmed (PCI 3.0 §6.8.2). `MsixVectors` also masks
and unmasks entries, reads entries and pending bits back, and `disable`
undoes everything and frees the vectors. Plain MSI (`enable_msi`) is the
fallback for functions without MSI-X, with one vector: several would need
a block of consecutive kernel vectors aligned to its size (the function
fills in the low data bits), which the allocator does not hand out.
`enable_interrupt` picks MSI-X entry 0 or MSI. The message crosses from
`poly_irq::MsiMessage` to `poly_pci::Message` through `to_pci_message` /
`from_pci_message` (the crates do not know each other). Bus mastering stays
the driver's decision, although messages need it.

**virtio-blk maps its request queue to MSI-X entry 0, or polls.** The
function's MSI-X is enabled (masked) before `poly_virtio_blk::init`, which
maps the queue to entry 0 (`vector: Some(0)`). If the device refuses that,
it is brought up again without a vector, polled. Once the block layer has
made the disk, `virtio_blk::start` attaches entry 0 to the disk's handler,
asks the device for interrupts (`enable_interrupts`), clears the function
mask and switches the disk's waiters to sleeping. A function without MSI-X
stays polled. The boot log says which; INTx is never used.

**The disk's lock is split.** `block::Disk` keeps the request queue (with
the device) behind a `SpinLock`, held only to enqueue, run the queue or
collect a result, and taken by the interrupt handler; and the
`BufferCache` behind a sleeping `Mutex`, held across waits. Lock order:
cache, then queue; waiters are woken after the queue lock is released.

**The cache waits through a hook in `poly-block`.** `IoQueue` (enqueue,
wait, and read / write / flush built on them) is what `BufferCache` does
its I/O through. `RequestQueue` implements it by running itself and
spinning, as before, so host tests and single-threaded users are
unchanged; the kernel implements it for `block::Io`, a handle on a disk's
shared queue. The cache does not know how waiting happens.

**Waiters sleep on the disk's `WaitQueue`; whoever finishes requests wakes
them.** A waiter runs the queue once (which starts its request), and if
its result is not there sleeps until the disk's completion counter moves.
The interrupt handler runs the queue too; whenever running the queue
finished user requests (`QueueStats::finished`, new in `poly-block`), the
counter is bumped and every waiter woken, and each collects its own result
with its ticket. Because the waiter also wakes the others when it is the
one that harvested their completions, a completion that the device
reported without an interrupt (suppressed by EVENT_IDX because a waiter
had already seen it) is not lost. Polled disks (the RAM disk, virtio-blk
without MSI-X) run the queue and yield between tries. Either way
interrupts stay on while a thread waits.

**The watchdog measures time.** `poly_block::Watchdog` takes a `Clock`
(any `Fn() -> u64`; the kernel passes `time::now_ns`) and a limit, 10
seconds for virtio-blk. Silence starts when a request goes out with
nothing outstanding and restarts with every completion. It is checked on
every poll, which now happens from the interrupt handler and from waiters,
and `Watchdog::deadline` tells a sleeping waiter when to look: a lost
request raises no interrupt, so the waiter's sleep is a timed wait that
ends at the deadline at the latest, and its run of the queue lets the
watchdog abort the device.

**Timed waits are a scheduler service.** `WaitQueue::wait_until_deadline`
blocks like `wait_until` but gives up at a deadline. The thread table
keeps one `(deadline, queue)` entry per thread in a timed wait; the tick
wakes the whole queue of each entry that is due (the other waiters check
their conditions and sleep again), using the same earliest-deadline atomic
as `sleep`. The entry is removed, under the scheduler lock, before the wait
returns, so the queue it names is alive whenever the tick reads it.

**Fault injection and test devices.** `VirtioBlk::set_drop_notifications`
skips the doorbell, so a request is never seen by the device: the kernel
test brings up a spare virtio-blk device (QEMU's null block driver) as a
private disk, loses a request on it, and checks the reader sleeps until
the watchdog fails it. QEMU's `edu` device (MSI only) tests the MSI
fallback. A third self-test boot attaches the test disk and the spare
device without MSI-X (`vectors=0`), so the whole suite also runs polled.

## Consequences

- Disk I/O no longer masks interrupts: the tick keeps time and preempts
  threads, and a CPU-bound thread runs while a reader waits (the kernel
  test sees it run during every one of 40 reads).
- A woken waiter waits for the running thread's time slice to end (up to
  10 ms) when another thread is busy: there is no preemption on wakeup.
  An idle CPU switches to it at once.
- All threads share each disk's 32 request slots. A user with no request
  of its own outstanding that finds them all taken waits for them to free
  up; one with requests outstanding gets `Busy` and must collect its own
  first (the cache does).
- The interrupt handler runs the elevator with interrupts off; its scans
  are quadratic in the 32-entry table, a few microseconds.
- A due timed wait wakes every thread on that queue; fine for the few
  waiters a disk has, a per-thread wakeup in `poly-sched` would avoid it.
- MSI and MSI-X messages target the boot CPU; SMP needs a destination
  policy (and the held-lock and interrupt counters per CPU, ADR 0016).
- virtio configuration-change interrupts are not used (no config vector).
- The 10-second limit is guest time: a host that stalls the guest for
  longer breaks the disk for good, as before.
- QEMU's null block device can finish a request during the doorbell write,
  before the waiter sleeps; tests must not assume every request sleeps.
