# 0014: Block layer: submit/poll devices, an ordering request queue and a pinning buffer cache

**Status:** Accepted

## Context

Phase 1 ends with "first real disk I/O" on virtio-blk and "a block layer with
request queue and buffer cache". Phase 3 puts ext2/ext4 with a page cache on
top. The kernel polls devices today and will take interrupts soon, and it has
no heap yet, so the block layer must work without allocation and must not
bake in either way of waiting.

Disks (virtio-blk among them) complete requests in any order, have limits on
request size and scatter-gather, and may keep completed writes in a volatile
cache until flushed. A file system needs reads that see earlier writes, and
a journal needs "these writes are durable before that one starts".

## Decision

- **`poly-block`** (`no_std`, no dependencies, no allocation) holds the block
  layer; device drivers depend on it, not the other way round. The virtio
  driver is a separate crate, **`poly-virtio-blk`**, on top of `poly-block`
  and `poly-virtio`, following ADR 0009 (one crate per device class), so RAM
  disks and future drivers do not pull in virtio.
- **`BlockDevice`** is submit / kick / poll: requests carry a caller-chosen
  id and come back from `poll` in any order, exactly once, also when the
  device fails. A polling kernel calls `poll` in a wait loop; an
  interrupt-driven one calls it from the interrupt handler. `kick` lets
  drivers batch doorbell writes. Requests carry raw `IoBuf` segments, so
  `submit` is `unsafe` (the memory belongs to the device until completion);
  safe synchronous wrappers hold a borrow and wait.
- **`RequestQueue`** owns one device. It accepts requests of any length,
  holds back any request that conflicts with an earlier unfinished one
  (overlapping read/write, write/write, and flushes as write barriers), so
  results equal running them in submission order; then it picks the next one
  by a one-way elevator, merges neighbours and splits what exceeds the
  device's limits. Users get tickets and collect results (`take`, `wait`, or
  `finished` for whoever wakes waiters). All tables are caller-provided
  slices; decisions are linear scans, quadratic in the table size, which is
  fine for the dozens of slots a disk needs.
- **`BufferCache`** keeps blocks in caller-provided memory, found through a
  hash table threaded through its slots, evicts by exact LRU over unpinned
  slots, writes dirty blocks back on eviction, and makes everything durable
  with `sync` (batched writes, then a flush). Pinned buffers are handles that
  cannot be copied, so a double release does not compile. I/O is synchronous
  through the queue for now.
- **virtio-blk** puts headers and status bytes in a driver-owned DMA area
  (one slot per queue entry), data straight in the caller's buffers, and
  translates addresses through a `Dma` trait the kernel implements. It
  treats `seg_max` as counting header and status descriptors, since the
  specification does not say; that is safe either way. On a protocol
  violation it resets the device before failing requests in flight, so
  freed memory is never written afterwards.

## Consequences

- Block I/O, ordering, merging, splitting, caching and the virtio-blk
  protocol are all tested on the host in milliseconds, against simulated
  disks that complete out of order, fail on purpose and lose unflushed
  writes in a simulated power cut.
- The kernel side is: map the virtio regions, provide DMA memory and the
  tables, implement `Dma` and `Mmio`, and poll (later: route the queue's
  interrupt to `RequestQueue::process`).
- The buffer cache waits for its own I/O, so it suits polling and a single
  thread; with a scheduler, `get` will want to sleep on the ticket instead of
  spinning, which the ticket API already allows.
- A buffer handed to the block layer must be contiguous for the device too
  (true of the kernel's direct map); the queue may join buffers that are
  adjacent for the CPU into one segment. `Dma` refuses what it cannot
  translate, so a violation fails a request instead of corrupting memory.
- If a broken device never finishes its reset, its requests never complete:
  their memory stays lent out rather than being reused while the device
  might still write it.
