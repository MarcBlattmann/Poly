# 0263: Idle CPUs zero frames ahead of page faults

**Status:** Proposed: the measurement found no visible gain; whether to keep it is open. Track `prezeroed-pages`.
Builds on the frame allocator and its reserve (ADR 0250), the deferred frees of ADR 0254 and the timeline's kernel samples (ADR 0247 addenda), which found the cost. Works beside
ADR 0262 (a lock per NT process). No third-party source.

## Context

A page fault on fresh anonymous memory (a Linux `mmap`, the first touch of an NT commit, a Mach `vm_allocate`)
takes a frame from the allocator and zeroes its 4 KiB before the program goes on (`UserFrame::zeroed`). Chrome for
Windows' display compositor (`VizCompositorThread`) frees and commits buffers every frame; the timeline showed page
faults among its kernel time while typing (ADR 0247 addendum, "the address bar's slow keys"). That addendum first
called the whole bucket of kernel time outside system calls (28 %) "zero-filling"; it is mostly the system-call entry
path, fault handling and interrupts (corrected there).

## Decisions

1. **A bounded pool of zeroed frames** (`kernel/src/zeropool.rs`; the rules in `poly_mem::zeropool`, host-tested).
   At most 1/256 of RAM, between 64 frames and 4096 (16 MiB). Global, behind a spinlock; last in, first out.
2. **Idle CPUs fill it.** When a CPU has nothing to run, its idle loop zeroes one frame for the pool and looks again
   (interrupts on), before it halts. A thread made runnable meanwhile preempts it at once (its reschedule interrupt),
   so the zeroing costs the running programs nothing. No separate thread: there are no priorities to keep one from
   competing with programs, and the idle loop is exactly "when CPUs are idle". Not while the kernel tests run (they
   count free frames); the pool's own tests fill it explicitly.
3. **Faults take a zeroed frame when there is one** (`UserFrame::zeroed`, every personality), else zero one inline
   as before.
4. **The allocator's free frames are the dirty list.** Freed frames (ADR 0254's deferred ones included) go back to
   the allocator as they were; only the idle loop moves a frame into the pool, after zeroing it. A frame in the pool
   belongs to no one.
5. **The reserve stays the kernel's (ADR 0250).**
   - The pool refills only while the allocator has more than the low watermark plus a whole pool free.
   - A user frame comes from the pool under the reserve rule with the pool's frames counted as free.
   - At the low watermark (`frames::after_user_allocation`) the pool goes back to the allocator, so the heap, page
     tables and DMA buffers (which take from the allocator only) never find frames parked in it.
   - The out-of-memory policy and the free memory reported (`/proc/meminfo`, NT's memory information, `poly.nt.mem`)
     count the pool as free.
6. **Locks.** The pool's spinlock and the allocator's are leaves, never held together, and the pool takes no other
   lock: it fits the NT state's lock, ADR 0262's per-process locks and the memory map's in any order they are held.

## Not done: keeping recommitted pages mapped

Chrome's compositor decommits (`MEM_DECOMMIT`) and commits the same ranges again every frame; each round unmaps the
frames (a TLB flush, ADR 0254), frees them and faults fresh ones in. Keeping the frames mapped across a decommit would
need:

- ADR 0262's own-memory path for `NtFreeVirtualMemory`/`NtAllocateVirtualMemory` (the process's lock alone), so the
  decision is local to the process;
- a per-allocation record of decommitted-but-kept frames (counted against the commit charge as free, given back under
  pressure like the pool);
- zeroing on recommit (a decommitted page must read as zero when committed again): swapping each kept frame for a
  pooled one at the first touch, or zeroing it then;
- protection: a kept page must not stay accessible while decommitted (`PAGE_NOACCESS` in the page table until the
  recommit).

## Consequences

**Measured** (in-session A/B, the pool switched off and on by a temporary key, removed; the timeline's kernel samples;
order alternating; VM pauses checked with `--gaps`):

| | Pool off | Pool on |
|---|---|---|
| A zeroed frame (kernel micro-benchmark, 512 frames) | 2,263 ns (cold frames zeroed inline) | 371 ns |
| Chrome for Windows, address bar, 6 rounds: zero-fill samples per 1000 ticks | 0.1 | 0.2 |
| ... Viz's kernel share / its fault samples per 1000 ticks | 31 % / 2.3 | 35 % / 2.5 |
| ... key to frame, keys without a VM pause (median / p90 / p99) | 30.2 / 60.2 / 120.3 ms | 28.5 / 66.3 / 108.7 ms |
| Linux Chromium scrolling, bursts 2–11 (the first and last are outliers either way): kernel / faults per 1000 ticks | ≈107 / ≈4.6 | ≈103 / ≈5.0 |

A fresh-page fault saves about 1–2 µs. Zeroing was under 0.1 % of the CPU in both workloads, below what the
samples resolve, so neither workload shows a difference beyond noise. What the pool buys is bounded by that; what it
costs is code in the idle loop and the fault path (covered by the tests below).

## Tests

- `poly_mem::zeropool` (host): capacity bounds, refill and drain thresholds, the reserve rule with pooled frames,
  the stack.
- Kernel tests (`zeropool::kernel_tests`): pooled frames are zero even after dirty frees; a process never sees
  another's old pages in fresh memory; the pool drains at the watermark, refills, and loses no frame.
