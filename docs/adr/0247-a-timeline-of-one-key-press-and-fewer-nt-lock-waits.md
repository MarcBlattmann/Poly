# 0247: A timeline of one key press, and fewer NT lock waits

**Status:** Accepted. Track `chrome-timeline`. Builds on ADR 0148 (the profile), ADR 0067 (NT file I/O
and its completions), ADR 0232 (a frame draws only what shows) and ADR 0242 (user-mode locks). Inputs:
Poly's own records of Chrome for Windows and Debian's Chromium running on Poly. No third-party source.

## Context

QA round 2 measured Chrome for Windows on Poly at about twice Linux Chromium's latency for Ctrl+Tab
(about 177 against 63 ms) and the address bar (105 against 56 ms), with screenshots taken from outside
the guest. The profile (ADR 0148) samples the kernel and times Linux system calls; it cannot say which
of Chrome's threads, in which processes, ran, waited for a CPU or waited for each other between a key
and the frame that shows it.

## Decisions

1. **A timeline in guest memory.** `poly.debug.timeline[=<MiB>]` keeps a ring of 40-byte records
   (`crates/poly-timeline`, host-tested; `kernel/src/timeline.rs`): context switches with what the
   outgoing thread became, wake-ups with the waking thread (or an interrupt), system calls of every
   personality (entry with the first argument, exit with the result), blocks on wait queues (the
   NT state's lock and the I/O drain's are named), keys, display frames, and a user-mode instruction
   pointer at every timer tick. A record is a handful of stores; nothing is printed while the ring
   runs. The Pause key freezes it; the host copies it out of guest memory with QEMU's `pmemsave`.
   Names go to the log once, as they become known: kernel threads, Windows thread names
   (`SetThreadDescription` hands them to the kernel through `PolyDebugPrint` with a private prefix,
   never logged), Linux ones (`prctl(PR_SET_NAME)`), command lines, mapped images
   (`Backend::image_mapped`) and system call names. `tools/timeline/timeline.py` turns ring and log
   into the chain of wake-ups from a key to its first frame, per-thread run / CPU-wait / blocked
   time, per-call times and user-mode samples by module (docs/DEBUGGING.md).
2. **`NtQueryPerformanceCounter` takes no lock.** It reads the clock and writes two words; the
   kernel runs it before its NT bookkeeping (`dispatch_unlocked`) and skips the end-of-call work,
   which has nothing to do for it. Chrome calls it about 80,000 times a second while switching tabs;
   each call took the NT state's lock up to seven times and the I/O drain's lock once.
3. **Which processes are NT processes is known without the lock.** Every NT system call first asked
   the NT state whether its process is an NT process. A bitmap of process ids, set and cleared with
   the table under the lock, answers it.
4. **Only the thread that queued a completion waits for its delivery.** The end of every NT system
   call took the I/O drain's lock (ADR 0067: a call that completed a request returns only after the
   status block is written), so every NT thread of every process queued on it whenever Chrome's
   pipes had a completion in flight. Now a state-lock section that queues a completion marks its
   thread, and only that thread drains or waits for the delivery in progress; any other thread
   delivers when the drain's lock is free and never waits for it. Completions popped and not yet
   delivered are counted under the state lock, in the look that pops them.

## Consequences

Measured with the timeline on Chrome for Windows at 2560x1600, 200 %, polyd with KVM: 32 Ctrl+Tab
switches in one session, the fixes switched off and on in blocks of 8 (a temporary switch, removed),
so Chrome, the tabs and the host load were the same:

| Ctrl+Tab, key to the first frame | Decisions 2–4 off | On |
|---|---|---|
| First frame, median (range) | 123.5 ms (82–194) | **93.0 ms** (79–130) |
| CPU used until the frame, median | 277 ms | 210 ms |
| Threads blocked on the I/O drain's lock, median per switch | 115 ms | 19 ms |
| `NtQueryPerformanceCounter` blocked on a lock (500 ms, before) | 3,800 times, 360 ms | none |

Typing in the address bar did not change (key to first frame, median 20.6 against 22.8 ms over 16
keys each).

What is left, from the same tool on Debian's Chromium in the same guest:

- Ctrl+Tab reaches its first frame in about 50 ms there (47–56). Chrome for Windows uses more CPU
  for the same switch (about 210 against 120–195 ms), and **90 % of its user-mode samples are in
  `chrome.dll`**: Chrome's own code. Poly's DLLs (`ntdll`, `user32`, `d3d11`, `kernelbase`) are
  under 5 %. With 4 virtual CPUs busy, threads wait for a CPU (CPU-wait about 80 ms summed over
  threads per switch).
- Address-bar keys reach their first frame in about 17 ms on Linux Chromium and 21–23 ms on
  Chrome for Windows: the 40 ms the QA saw is not in the guest's key-to-frame time.
- Memory calls are the costliest Poly calls on Chrome's threads during a switch:
  `NtAllocateVirtualMemory` (up to 5 ms for one call), `NtUnmapViewOfSection`,
  `NtFreeVirtualMemory`; about 40 ms of CPU per switch together. Linux Chromium's `munmap`,
  `madvise` and `mmap` cost it about the same.
- The browser's UI thread makes about 1,500 `PolyUserCall`s per switch; they take the state lock
  but no longer wait for the drain.

The timeline costs nothing while off (one load of a flag per hook). On, a busy Chrome writes about
60,000 records a second; the ring's 32 MiB hold about 14 s.

## Addendum (2026-11-20): kernel samples, and the kernel's writes into a process (`chrome-typing-latency`)

**The timeline samples the kernel too.** A timer tick that interrupts kernel code records the
interrupted function and its caller (the frame's return address) in the sample record's `b`
(user-mode samples keep `b` = 0). `timeline.py --samples --kernel-syms FILE [--callers]` names
them from `nm -C --defined-only` of the kernel ELF and sums them by function and by the system
call the thread was in. `--at MS` reads a window without a key press. The samples cost about a
sixth of the ring (35–60 s of a busy Chrome in 32 MiB).

**What it showed.** Inside `NtWriteFile` on Chrome's IPC pipes, `AddressSpace::protect` called from
`Memory::resolve_action`. Every kernel copy into another process's memory (`Memory::read`/`write`:
an I/O completion's data and status block, written by the thread whose call completed it) ran the
page fault decision for each page, also for a page the page table already mapped with the needed
rights; the decision for a present page is "set its permissions", which re-wrote the same rights
and flushed the page from the TLB of every CPU running that process (an IPI and a wait for the
acknowledgements), and reset the process's PCIDs on the others. While Chrome's renderer and GPU
processes run, that is a shootdown per completed pipe read.

**Decision.** `Memory::access` copies a page that the page table maps with the rights the access
needs (present for a read, writable for a write) without resolving it: the copy goes through the
direct map anyway. Pages that are absent, read-only for copy on write, page-cache pages not yet
writable, parked (`PROT_NONE`) or forced (a debugger's write) are mapped without those rights and
are resolved as before. Kernel test: `kernel_writes_keep_writable_pages_and_still_copy_shared_ones`.

**Measured** (same session, the old path switched on and off in four rounds of 10+10 keys by a
temporary key, removed; Chrome for Windows, a page text field, key to the next display frame):

| | Old path | Fast path |
|---|---|---|
| Median | 15.9 ms | 15.1 ms |
| Mean | 42.6 ms | 25.9 ms |
| 90th percentile | 153.7 ms | 45.3 ms |
| `NtWriteFile`, kernel running time, median / p90 / p99 | 97 µs / 1.19 ms / 15.4 ms | 74 µs / 0.21 ms / 4.5 ms |

The address bar's median went from 29.5 to 23.5 ms; its tail (Chrome's suggestion work) did not
change.

**About the QA's typing number.** The QA driver's `lat … type X` counts its own key press, hold and
gap (60–80 ms) before its first screendump, and one screendump of a 2560x1600 screen takes 40–50 ms:
it cannot report less than about 100 ms, for any browser. Its "105 ms" for the address bar was the
suggestion box changing, measured that way; Linux Chromium measured the same way in the same week
gives 107–123 ms. The guest's own key-to-frame time is the timeline's (or `--compositor-stats`'s
`key_to_frame`).

## Addendum (2026-11-20): the address bar's slow keys (`chrome-addressbar-tail`)

Chrome for Windows' address bar, keys 400 ms apart, two sessions (10 rounds of 12 keys in each
state, the order alternating): key to the next display frame median 24–30 ms, p90 38–80 ms; the
slowest keys take 0.1–0.9 s. Over their windows (key to frame), summed over threads, they are:

- **Chrome's bursts:** 2.7 s of running and 1.3 s of waiting for a CPU in an 870 ms window, with
  the data decoder service busy and a utility process starting (`NtCreateNamedPipeFile`); in
  others the browser's UI thread runs 150–200 ms in `chrome.dll` (the omnibox providers) and
  presents nothing.
- **The virtual machine paused.** Each CPU goes 3–30 ms without a timer tick several times a
  second (about 3 % of the time on this host, ~18 of whose 32 cores ran other agents' builds);
  two thirds of those gaps every CPU shares: the host stopped the whole VM. They account for the
  NT state lock's long waits (blocks over 1 ms: half of the blocked time, p99 2–5 ms): the holder
  stops with everyone else. `timeline.py --gaps` reports them.
- **Not lock-holder preemption by Poly's scheduler.** Keeping the time slice from switching away
  from a thread holding the NT state lock (tried, in-session A/B over both sessions) left the long
  waits as they were (blocks over 1 ms: 4.9 → 7.5 s and 5.9 → 6.0 s); not kept.
- **Writeback's dirty check walked the whole page cache.** `writeback_pass` asked
  `CachedFs::page_counts()` every pass (every 5 s) only to know whether a page is dirty; that
  counts every cached page of the file system with the page cache's index locked (one session: 0.8
  s of CPU in it over 70 s, page-cache users waiting meanwhile). `CachedFs::has_dirty` looks at the
  first dirty entry instead (disk and memfs writeback). Its effect on keys was not measurable in
  the two A/B sessions, whose caches never made the walk show up.

**Viz's software composite (about 7 ms a frame).** Its samples while typing: 44 % `chrome.dll`
(Skia), 46 % kernel, 7 % Poly's `d3d11.dll`. The kernel part: kernel time outside system calls
(28 %: the system-call entry path, page-fault handling and interrupts; zero-filling fresh pages is a small part
of it, ADR 0263), `NtFreeVirtualMemory`/`NtAllocateVirtualMemory` (25 %: buffers freed and committed again
every frame), `NtWriteFile` (24 %, mostly waking the reader). Poly's present path (the copy of the
dirty rectangles, `PolyUserPresentBuffer`) is a few per cent: no cheap win there; the candidate is
the memory churn (keeping pages of a recommitted range, a pool of zeroed frames), a change of its
own.
