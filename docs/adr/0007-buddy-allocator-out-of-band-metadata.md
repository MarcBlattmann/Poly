# 0007: Buddy frame allocator with out-of-band metadata

**Status:** Accepted. Amended by [0010](0010-kernel-address-space.md): the
metadata buffer may now come from any usable RAM, reached through the direct map.

## Context

The kernel needs a physical frame allocator for all RAM that can free, hands
out physically contiguous blocks (page tables, DMA buffers, 2 MiB pages) and
is fast. It must work before the kernel has a heap or its own page tables:
the boot mapping only reaches physical memory below 1 GiB. Logic should live
in a host-tested `no_std` crate.

A classic buddy allocator keeps its free lists inside the free blocks. That
needs every free frame mapped, which we cannot promise yet, and it cannot be
exercised on the host without simulating physical memory.

## Decision

`poly_mem::BuddyAllocator` is a binary buddy allocator with orders 0 to 10
(4 KiB to 4 MiB). All of its state lives in a `&mut [u64]` the caller
provides, sized by `BuddyAllocator::metadata_words(&map)`:

- per order, a *free* bitmap (one bit per block slot of the span from the
  lowest to the highest usable address) with summary levels above it, so the
  lowest free block is found with one `trailing_zeros` per level;
- per order, an *allocated* bitmap, so `free` can reject double frees and
  frees with the wrong order or address instead of corrupting state.

The kernel carves this buffer from usable RAM below 1 GiB and removes it from
the map it hands to the allocator.

## Consequences

- The allocator never touches managed memory, so it manages RAM that is not
  mapped and is fully tested on the host, including a randomized stress test.
- `allocate` and `free` are `O(MAX_ORDER × levels)`, independent of RAM size.
- Metadata is about 4 bits per 4 KiB frame of *span* (1/8192): 256 KiB for
  2 GiB. A map with a huge hole between two usable ranges pays for the hole;
  that is unusual on PCs and can be fixed with per-range zones if needed.
- `free` must be given exactly the order the block was allocated with;
  freeing part of a block is not supported.
- Single global lock for now; per-CPU frame caches come with SMP.
