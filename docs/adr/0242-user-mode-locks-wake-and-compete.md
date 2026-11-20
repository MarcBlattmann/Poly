# 0242: User-mode locks wake a waiter and let it compete (no convoys)

**Status:** Accepted. Track `lock-convoys`. Follows ADR 0225's addendum (chrome-feel-2, #329:
the process heap's lock was a convoy). Inputs: the spec sheets of the SRW lock, condition
variable, run-once and critical-section entries (`specs/windows/ntdll.dll/RtlAcquireSRWLockExclusive.yaml`
and its relatives, `RtlSleepConditionVariableCS.yaml`, `RtlWakeConditionVariable.yaml`,
`specs/windows/kernel32.dll/AcquireSRWLockExclusive.yaml`) with their visible tests; Microsoft
Learn ("Slim Reader/Writer (SRW) Locks", "Critical Section Objects",
"InitializeCriticalSectionAndSpinCount"). No third-party source.

## What was there

`poly_rtl::sync::Lot` (SRW locks for `ntdll`'s `Rtl*` names and `kernel32`'s) and
`poly_rtl::critsec::CriticalSection` both **handed the lock to a sleeping waiter** on release:
the releaser wrote the word as "held by the first waiter" (SRW) or set the event that made the
waiter the owner (critical sections), then woke it. Until the waiter had been scheduled nobody
could take the lock, also not the threads that were running and wanted it for a moment. Under
contention every acquisition paid a full wake-up: a **lock convoy**. #329 found one on the
process heap (about 13 of Chrome's threads queued behind each other) and gave heaps their own
unfair lock (`HeapLock`); the SRW locks (Chrome's `base::Lock`, `std::mutex`) and the
critical sections (C runtimes, the loader lock, programs) kept the hand-off.

The convoy benchmark (`tests/windows/bench/lock_convoy.rs`, 4 vCPUs, KVM; N threads, a short
critical section, 400 ms each) on `origin/main`: SRW 98 acquisitions/ms with one thread, **30
with 8 and 23 with 16**, the median wait **256 µs and 1 ms**; critical sections the same.

## Decisions

1. **Release, wake, compete.** A release frees the lock and wakes the first waiter (for SRW
   locks: or all the readers at the head of the queue); the woken thread takes the lock like
   any other, and a thread that is running when the lock comes free takes it at once.
2. **Bounded unfairness for SRW locks.** A woken waiter that finds the lock taken again goes
   back to the head of the queue, marked; the next release hands the lock to it directly (the
   old hand-over, now only for that waiter). Nobody loses twice in a row.
3. **The rules of the sheets stay.** A waiting writer still holds back readers that come later
   (`RtlTryAcquireSRWLockShared` FALSE, a shared acquire blocks); a woken reader may join the
   readers or take a free lock although writers are queued behind it (it was queued before
   them); several waiting writers are served one at a time; the queued readers at the head
   enter together; the lock words without waiters (`0`, `1`, `(n << 4) | 1`) and the
   condition variables' wake order are unchanged. A free lock with waiters is the word `2`
   (`WAITERS` without `LOCKED`), only for the moment between a release and the next
   acquisition; `RtlTryAcquireSRWLockExclusive` takes it, the shared try does not.
4. **Critical sections: `OwningThread` is the lock.** A thread owns the section when it moved
   `OwningThread` from 0 to its id. `LockCount` keeps the observable values (free -1, held once
   -2, one lower for each nested enter and each waiter); the last leave frees the section and,
   when others are counted, sets the event. A woken waiter that loses sleeps again; the
   winner's leave wakes it. (No bound like decision 2: the structure has no room for a queue.)
5. **Spinning first.** SRW acquirers look at the word 256 times before they queue; an enter
   spins its spin count, at least 128 times (Learn: waiting threads spin briefly on a
   multiprocessor). `HeapLock` (ADR 0225) is unchanged.
6. **Nothing in the kernel.** Condition variables, `RtlWaitOnAddress` and run-once only wake
   (their sleepers take their lock again through the paths above); kernel waits and `park`
   are chrome-feel's and stay as they are.

## Consequences

- The benchmark with this change: SRW **158–228 acquisitions/ms with 8 threads and 128–180 with
  16** (5–8×), the median wait 1–2 µs; critical sections 73–256 with 8 and 80–251 with 16.
  The longest single wait grows (a waiter can lose to running threads; tens of ms with 16
  threads on 4 processors), the 99th percentile stays about where it was.
- Chrome for Windows (chrome-qa's driver, 2560x1600 at 200 %, KVM; two sessions with
  `origin/main`'s `poly-rtl` and two with this change, the same steps): **Ctrl+Tab no longer
  stalls for seconds**: 5 of 16 presses took 1.2–8.0 s before, none of 16 after (at most
  0.75 s); the median stays about 0.3–0.4 s. A new tab settles in a median 1.9 s instead of
  3.6 s; its first change and page loads (example.com, two Wikipedia articles) did not change
  measurably (one sample of each varies more than the difference).
- The visible tests of every entry that uses these locks pass (SRW, condition variables,
  run-once, critical sections through `RtlSleepConditionVariableCS`, thread pools).
- The 32-bit modules (ADR 0186) are built from the same `poly-rtl` and change with it.
