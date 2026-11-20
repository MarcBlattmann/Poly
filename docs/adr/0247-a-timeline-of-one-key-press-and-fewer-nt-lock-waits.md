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
