# 0017: The block layer in the kernel: poly-virtio-blk on direct-map DMA, a watchdog, a queue and a cache per disk

**Status:** Accepted. Supersedes the virtio-blk driver parts of
[ADR 0015](0015-pci-and-virtio-in-the-kernel.md) (the kernel's own request
code, its bounce buffer and `virtio_blk/protocol.rs`); PCI, DMA allocation
and polling stay as decided there.

## Context

ADR 0015 brought up virtio-blk with a small driver of the kernel's own: one
request at a time through a one-page bounce buffer, with its own copy of
the request format. ADR 0014 built the reusable pieces since: `poly-block`
(the `BlockDevice` interface, `RequestQueue`, `BufferCache`, `RamDisk`) and
`poly-virtio-blk` (a full driver behind `BlockDevice`, taking the caller's
buffers through a `Dma` trait). Phase 1 ends with "a block layer with
request queue and buffer cache" in the kernel, which the file systems of
Phase 3 will sit on. Interrupts are being added in parallel; until they
exist, the kernel polls.

Two things the libraries leave to the kernel need deciding: which memory
the device may be handed directly, and what happens when a request never
comes back (the old driver had a spin budget; `RequestQueue::wait` spins
forever).

## Decision

**`poly-virtio-blk` is the driver.** The kernel's part
(`kernel/src/virtio_blk.rs`) is discovery and mapping (as in ADR 0015), a
reset before bus mastering, DMA memory for a 128-entry virtqueue and the
request headers (`DmaBuffer`), its tables from the heap, and the address
translation. Everything is kept for the kernel's lifetime (leaked): there
is no hot-unplug, and memory the device has seen is never reused without a
reset. The hand-written request code and protocol module are gone.

**DMA straight into the caller's memory, from the direct map only.**
`dma::bus_address` (and `KernelDma`, the `Dma` implementation) translates a
range only if all of it lies in the direct map, which maps physical memory
linearly, so the range is physically contiguous. Everything else (the
kernel image with its statics and the boot stack, MMIO mappings, holes)
gets `None`, and the request fails with `BadBuffer` before it reaches the
device. Heap memory is always eligible: the heap's page source takes runs
of frames from one buddy block each and reaches them through the direct
map, a large allocation is one such run, and a small object sits in one
slot of one slab, which is one run too. So a heap buffer that crosses a
page boundary still crosses into the physically next frame. Thread stacks
and `DmaBuffer`s are direct-mapped as well. Without an IOMMU, bus address =
physical address.

**A watchdog instead of a spin budget in the driver.** `poly_block::Watchdog`
wraps any device that implements `poly_block::Abort` and aborts it when
requests are outstanding but a number of polls in a row bring nothing back.
`VirtioBlk` implements `Abort` as it already handles a protocol violation:
reset the device (which takes the in-flight buffers back), set FAILED, fail
what was in flight with `DeviceFailed`, and refuse everything after. The
budget is 20 million polls, about 30 seconds under TCG; generous on purpose,
since a false alarm breaks the disk for good. Both pieces are host-tested.

**One request queue and one buffer cache per disk, behind one lock.**
`kernel/src/block.rs` gives each device a `RequestQueue` (32 user requests;
4 device requests of up to 16 segments, which fit the virtqueue) and a
64 KiB `BufferCache`, with all tables and the cache's buffers from the
heap. The cache's buffers are one heap allocation, so DMA into them is
allowed. A `Disk` holds the pair behind a spinlock and offers whole-block
`read_block`/`read`/`write_block`, `sync` and `stats`; `lock()` gives the
rest (pinned buffers, invalidation, uncached I/O). Writes are write-back:
durable only after `sync`. `poly-block` gained `BufferCache::invalidate`
(drop clean, unpinned blocks), used to check that data really is on the
disk.

**Named disks, and a RAM disk always.** The virtio-blk disk is `virtio0`
(`block::root()`, what a file system mounts); a 256 KiB `RamDisk` from the
heap is `ram0` (`block::ram()`), so file system work can be tested without
a disk. Devices are a `block::Device` enum (generics, not `dyn`, per ADR
0009). The kernel tests run one suite against both.

## Consequences

- No bounce-buffer copy and several requests in flight; requests are merged
  and split by the queue. In exchange, callers must pass direct-mapped
  memory: a buffer on the boot stack fails with `BadBuffer` (a kernel test
  checks this), which matters for code that runs on thread 0.
- Every block operation waits with the disk's spinlock held, polling. Fine
  with one CPU and no interrupts. Once `SpinLock` masks interrupts, a disk
  wait would mask them for its whole duration, and an interrupt handler
  could never take the lock the waiter holds. The interrupt step therefore
  has to split the lock: the queue under a short interrupt-safe lock that
  the handler takes to run `RequestQueue::process`, and waiters that sleep
  on their tickets without it. `BufferCache` waits inside `get`/`sync`
  through `RequestQueue::wait`, so it needs a way to sleep instead (a wait
  hook in `poly-block`, or ticket-based cache I/O) and a sleeping lock of
  its own. See the steps in `kernel/src/virtio_blk.rs`.
- The watchdog counts polls, not time: under KVM it fires after a second or
  two of silence, which a stalled host could cause. It becomes a time limit
  once the timer exists.
- Tables and DMA memory are never freed, even when bring-up fails after
  allocating them (a few KiB, once at boot).
- The cache is per disk and separate from the page cache Phase 3 will
  bring; whether the two merge is decided then.
