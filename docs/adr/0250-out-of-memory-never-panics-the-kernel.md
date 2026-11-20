# 0250: Out of memory never panics the kernel

**Status:** Accepted. Track `oom-no-panic`. Builds on [ADR 0050](0050-kernel-thread-stacks-with-guards.md) (thread
stacks) and the page cache's writeback thread.

## Context

chrome-feel found that about a dozen Chrome for Windows windows playing YouTube used up the 4 GB guest and the kernel
panicked: `memory allocation of 131072 bytes failed`, in `NtAllocateVirtualMemory` → `VirtualMemory::commit_range` →
`PageMap::fill`, which builds a new `Vec` of the allocation's runs.

The cause is not that one `Vec`:
- **One pool for everything.** The kernel heap, the page tables, thread stacks and DMA buffers take their frames from
  the same buddy allocator as user pages and the page cache.
- **User pages were taken until none were left.** Committed memory is demand-zero, so every page fault took a frame,
  as did every page the cache read in.
- **Then the kernel's next allocation failed.** Thousands of places in the kernel allocate infallibly (`Vec::push`,
  `Box::new`, map inserts). Any of them could be the one that panics, and making them all fallible is not possible.
- **No commit limit.** Poly had no system-wide limit for NT commits (only a per-process check against 1 TiB) and no
  policy for when memory runs out.

User page faults themselves already failed cleanly (`UserFrame::zeroed` returns `None`, and the thread gets its fault).

## Decision

1. **A reserve for the kernel** (`poly_mem::pressure`, `frames::allocate_user_frame`).
   - User pages and page cache pages are handed out only while more than the reserve is free:
     `UserFrame::zeroed`, `AddressSpace::map_zeroed` and the page cache's frames.
   - The reserve is 1/32 of RAM, at least 8 MiB and at most 128 MiB (128 MiB in a 4 GB guest).
   - The heap, page tables and DMA buffers may use the reserve. User memory never can, so no program can make a kernel
     allocation fail.
   - Thread stacks stop at half the reserve, so a storm of threads fails with `SpawnError::NoMemory`
     (`STATUS_NO_MEMORY` for NT) and leaves the heap its half.
2. **Reclaim before refusing.**
   - Below the low watermark (twice the reserve), a user allocation kicks the writeback thread, which drops clean page
     cache pages in the background, as it already did below 1/16 of RAM.
   - The page cache stays capped at a quarter of RAM, as before.
3. **NT system commit limit.**
   - `NtAllocateVirtualMemory` charges what it newly commits of private memory to the system's commit charge
     (`NtSystem::commit_total`).
   - `NtFreeVirtualMemory` gives back what it decommits or releases, and a process's remaining charge goes when it ends.
   - The limit is RAM less the reserve. Poly has no pagefile, so every committed page has a frame waiting for it, and a
     commit past the limit fails with `STATUS_COMMITMENT_LIMIT` (`ERROR_COMMITMENT_LIMIT`, 1455), as on Windows.
   - Measured with Chrome for Windows in a 4 GB guest: its commit charge (230–350 MiB) stays well below the frames user
     memory holds (550–680 MiB, which also counts images, sections and the desktop). The limit only bites when memory
     is really used up.
4. **Linux's heuristic.** `proc(5)` says the default overcommit mode refuses "obvious overcommits". A writable private
   anonymous `mmap` larger than all of RAM fails with `ENOMEM`. `PROT_NONE` and `MAP_NORESERVE` reservations
   (JavaScript engines) are not charged.
5. **An out-of-memory policy** (`kernel/src/oom.rs`).
   - When a user or page cache frame has been refused, the `oom` thread drops clean cache pages. If free frames are
     still at the low watermark, it ends the process holding the most memory (`choose_victim`):
     - for a Windows process, its own pages (resident less cache and shared pages: what ending it frees);
     - for others, their resident pages.
   - A Linux or macOS process is ended as by `SIGKILL`, a Windows one as by `NtTerminateProcess(STATUS_NO_MEMORY)`. It
     logs the process and waits for it to end (at most 5 s) before it ends another.
   - **The desktop goes first.** Poly's native programs (the compositor, the shell) and the first process are never
     ended. Their user memory may go down to half the reserve (the other programs stop at the whole), so the desktop
     still gets memory after a program has taken all it may.
   - **A refused page fault waits.** It wakes the policy and retries every 50 ms while memory is freed: up to 5 s for
     the desktop's programs, up to 2 s for others, and no longer once its own process is being ended. Then it fails as
     before (a signal or an exception). As on Linux, the allocating program waits for the killer, so the policy decides
     what goes, not whichever program faulted at that moment.
6. **Less heap traffic on commits.** `PageMap::fill` changes the run list in place, instead of copying it whole on every
   commit, and `commit_range` no longer clones the allocation's map.

7. **A crash the scenario found.** The launcher's text drawing (`poly-shell`'s `draw::fill`) panicked the compositor
   on a rectangle entirely right of the screen (a long query): such a rectangle now draws nothing.

`poly.mm.stats` logs, every 10 s, the free frames, user frames, page cache, heap, NT commit charge and limit, refusals
and ended processes.

## The Chrome scenario

Chrome for Windows in a 2 GB guest (`desktop --chrome-win --memory=2G poly.mm.stats`), with a new window playing a
YouTube video every 36 s:

| Build | What happened |
|---|---|
| Before | (chrome-feel, 4 GB) the kernel panicked |
| Reserve and commit limit only | No panic. Free memory went down to 129 MiB and Chrome renderers crashed on refused page faults. Then the compositor (pid 1) died on a refused page fault: the desktop was gone. |
| Desktop floor and waiting faults | No panic, but the compositor panicked in the launcher's drawing (item 7), and the policy did not see Windows processes' memory, so it ended nothing |
| All of the above | No panic. Free memory went down to 89 MiB. The policy ended a 288 MiB process and then one of 270 MiB (Chrome's browser, which closed Chrome). The compositor kept running and memory came back to 1.9 GB free. |

## Consequences

- **No panic from user memory.** Programs that keep allocating get failed commits, failed `mmap`s and failed page
  faults, and in the end the policy ends the largest one. The kernel's own allocations always find their reserve.
- **A kernel bug can still panic.** A kernel allocation larger than one buddy block (4 MiB), or a kernel leak that eats
  the reserve, still ends in a panic. That is a bug to fix, and `poly.selftest.oom` still checks how it is reported.
- **New tests:**
  - `oom::user_memory_stops_at_the_reserve`: user frames until refused; the heap still allocates; everything frees and
    works again.
  - `oom::a_storm_of_threads_fails_cleanly_and_recovers`.
  - `nt::test_kernel32::nt_commit_until_refused_then_again`: `k32test.exe --oom` commits until
    `ERROR_COMMITMENT_LIMIT`, frees, commits again, and four processes do it at once.
  - Host tests: the commit charge and its limit; the reserve, watermark and victim choice.
- **Not done:**
  - Swap and a pagefile.
  - Per-cgroup or per-job limits beyond the job objects' own.
  - Windows' low-memory notifications (`CreateMemoryResourceNotification`).
  - Making the remaining large kernel allocations fallible one by one.

## Addendum: which process goes first (track `oom-prefer-renderers`)

**Context.** The policy ended the process holding the most memory. With many Chrome windows that can be the browser
process, which takes every window with it (the scenario above did end Chrome's browser). Linux and Windows end a
renderer instead: one tab shows that it crashed and the rest stay.

**Decision.** The policy scores processes as Linux does (proc_pid_oom_score_adj(5)): a process's memory plus its
adjustment, -1000 to 1000, in thousandths of all memory; -1000 is never (`poly_mem::pressure::badness`). The highest score
goes. The desktop and the first process stay protected as before.

- **Linux processes:** their own `/proc/<pid>/oom_score_adj`, which Poly already stored but did not use. Chromium writes
  it for its children (renderers 300 and more, the browser stays 0); raising it is open to the owner, lowering it needs
  `CAP_SYS_RESOURCE`.
- **Windows processes** have no such file. Their adjustment comes from what Windows records about them, never from
  their names (`poly_personality_nt::oomadj`):
  - **+500 for a child its parent manages:** a process in a job object that another process made
    (`NtCreateJobObject` now records its creator) while that process is still running. That is how a browser keeps its
    renderers, GPU and utility processes; it sees one end through the job and can start it again, while ending the
    browser ends them all.
  - **+100 per memory priority level below normal** (`SetProcessInformation(ProcessMemoryPriority)`; Learn: pages of a
    lower memory priority are trimmed first).
  - Capped at 1000; nothing makes a Windows process less expendable than 0.
- The log line of an ended process names its adjustment.

**What it gives.** In a 4 GB guest an adjustment of 500 adds about 2 GB to a child's score, so any of the browser's
children goes before the browser, whatever their sizes. Measured on Chrome for Windows (`chrome-win-sandbox`): the
browser process and the other top-level process score 0; every renderer, GPU and utility process the browser started
in its jobs scores 500.

**Tests.**
- Host: `poly_mem::pressure` (`the_score_adjustment_makes_children_go_first`: a 100 MiB child at 300 goes before a
  270 MiB parent at 0; -1000 never goes), `poly_personality_nt::oomadj` (job made by a living parent, own job, parent
  gone, memory priority, the cap).
- Kernel: `nt_oom_ends_the_child_in_a_job_not_its_parent` (`k32test.exe --oom-job`): the parent holds 16 MiB and
  starts a copy of itself in a job it made, which takes memory until there is none (private commits, then section views
  the commit limit does not charge). The policy must end the child (`STATUS_NO_MEMORY`), and every page of the parent
  must be intact. So that it runs out after tens of megabytes on any machine and quickly under emulation (CI's TCG),
  the test raises the floor of user memory to 32 MiB above what is free (`frames::set_test_floor`; the policy counts it
  as the reserve). Run with the whole of a 256 MiB guest before the change, the policy ended the larger parent
  (128 MiB); with it, the child (66 MiB, adjustment 500).
- **One victim at a time.** Running again right after ending a large child, the policy found it "ending" (still giving
  its memory back) and took the next process, the parent. It now waits (at most 5 s) while a process that is ending
  still holds memory, as Linux's killer does not pick another while one is dying.

**Not done.** A way for a Windows program to set its own adjustment beyond its memory priority (Windows has none), and
the many-YouTube-windows run of the scenario above (it needs the network and half an hour; the kernel test reproduces
its decision).
