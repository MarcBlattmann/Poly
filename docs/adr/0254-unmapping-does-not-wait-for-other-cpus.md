# 0254: Unmapping does not wait for other CPUs

**Status:** Accepted. Track `tlb-shootdown`. Builds on the address spaces and TLB shootdowns of
`kernel/src/arch/x86_64/addrspace.rs` and `tlb.rs`, and on ADR 0247 / 0249 (the timeline, which found
the cost). Inputs: Intel SDM Vol. 3A §4.10 (TLBs, paging-structure caches, INVLPG, PCIDs). No
third-party source.

## Context

Every unmap of a page that another CPU might have cached a translation for sent a TLB shootdown and
waited, with interrupts off and the space's page-table lock held, until every CPU running the same
address space had acknowledged. Shootdowns ran one at a time system-wide. Under load the wait was
about 1.3 ms per unmap (the target vCPUs must be scheduled by the host to answer), 25–40 ms of CPU per
Ctrl+Tab in Chrome for Windows, and every other thread that needed the space's lock (a page fault)
spun meanwhile. Linux programs pay the same for `munmap`.

## Decisions

1. **Mailboxes instead of one request at a time** (`poly_smp::tlb::Mailboxes`, host-tested): a
   mailbox per CPU, any number of requests in flight. A request gets a sequence number in each
   target's box; a ticket is complete once each target has emptied its box past that number. A full
   box becomes one flush of everything. The kernel's synchronous `shoot` is post + wait.
2. **Unmapping does not wait** (`AddressSpace::unmap_async`, used by `Memory::release`: `munmap`,
   NT decommit and view unmaps). The pages are gone from the tables, this CPU flushes, the other
   CPUs' PCIDs for the space are forgotten as before, and the shootdown is posted *quietly*: no IPI.
   The targets carry it out at their next timer tick, context switch or wait loop (at most a tick),
   together with whatever else came: requests posted in a burst are batched.
3. **Nothing is reused while a translation to it may live.** The frames and page tables an unmap
   frees, and the page-cache pages and shared frames its regions held (`defer_drop`), wait in a list
   until their ticket is complete, then the next unmap or the `tlb-reaper` thread frees them. Past
   512 waiting entries an unmapping thread waits for the oldest first.
4. **A new mapping waits for old translations of its addresses.** `map_zeroed`, `map_frames` and
   `map_owned` (which page faults use too) first wait, under the space's lock, for unmaps of the same
   range whose tickets are not complete, interrupting their targets then (`hurry`, one IPI per CPU
   at most while one is on its way). So no CPU can see a new page through a translation from before
   the unmap.
5. **Narrowing still waits.** `protect`, copy on write after `fork`, `take` and kernel mappings keep
   the synchronous shootdown: a stale writable entry must not reach a frame that is shared from then
   on.
6. **Only CPUs running the space are targets** (the space's CPU mask, as before; a CPU leaves it when
   it loads another space or the kernel's tables, and the switch serves its mailbox).

## Consequences

**Correctness.** A kernel test (`a_remapped_page_is_never_seen_through_a_stale_translation`) runs
readers on every other CPU that read the published one of four slots again and again, while one CPU
takes the slot published four rounds ago out without waiting, maps a fresh page there with the next
generation and publishes it, 3,000 times. A reader that finds another generation in a slot whose
publication did not change saw it through a stale translation. It passes on KVM with 2, 4, 8 and 16
CPUs and on TCG with 4. With decision 4 switched off (temporarily) it fails, with quiet posts and
also with no IPIs at all; with decision 4 and no IPIs it passes (each remap then waits about a
tick). All 300 kernel tests pass in each configuration.

**Cost.** The same test times an allocator-like pattern while the other CPUs run the space (unmap
page *i* of 64, map page *i* + 32):

| Other CPUs in the space | Unmap + map, waiting | Not waiting |
|---|---|---|
| 3 | 26.9 µs | **3.0 µs** |
| 7 | 76.2 µs | **7.8 µs** |
| 15 | 193.7 µs | **14.5 µs** |

Chrome for Windows, Ctrl+Tab (timeline, one session, blocks of 8 with unmaps waiting and not, a
temporary switch, removed): time inside `NtFreeVirtualMemory`, `NtUnmapViewOfSection` and
`NtAllocateVirtualMemory` per switch 44.2 → **32.0 ms**, running inside them 30.4 → **20.6 ms**
(`NtFreeVirtualMemory` 12.3 → 6.7 ms, `NtUnmapViewOfSection` 11.8 → 8.1 ms). What is left is the
page-table work of large releases. Key → first frame did not change beyond the host's noise in that
session (it was heavily loaded: 223 against 253 ms medians, both far above quiet sessions).

Linux Chromium scrolling `chrome://credits` (Page Down, same method): `munmap` / `madvise` / `mmap`
are few there (3–7 per key, under 1 ms each, about 0.85 ms per call in both modes) and key → frame
was within noise (55 against 65 ms medians). Its renderer is single-threaded for these calls, so it
had little to gain; the change is for multi-threaded unmappers.

Frames freed by an unmap are reused up to a tick later than before. Memory is not otherwise held:
the waiting list is bounded.
