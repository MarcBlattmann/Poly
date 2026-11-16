# 0202: Thread locks, the thread pool and a thread's first APC

**Status:** Accepted. Track `impl-win-apps-4` (spec track `spec-win-apps-4`). Inputs: Microsoft
Learn ("Slim Reader/Writer (SRW) Locks", "Condition Variables", "One-Time Initialization",
"Thread Pools", `QueueUserAPC`, `VirtualAlloc`), the public SDK headers, and the spec sheets and
visible tests of `specs/windows/` (Windows 11 26200 observations). No Wine, ReactOS, Mozilla,
MSYS2, Cygwin or Git source was read; the applications were run as black boxes.

## What was there

`ntdll`'s `RtlAcquireSRWLockExclusive` and its relatives, `RtlWakeAllConditionVariable` and
`RtlRunOnceInitialize` were fail-only stubs that returned at once, so a program that reached them
through `ntdll` (Firefox, Node, Audacity) raced. `kernel32`'s versions were separate, polling
(spin, yield, sleep a millisecond) and wrong for the rules the sheets pin: waiting writers hold
back readers, condition variables wake the longest sleeper, `RtlSleepConditionVariableCS` leaves
a section once. The thread pool (`CreateThreadpool*`, `SetThreadpool*`, `QueueUserWorkItem`)
failed. `_get_stream_buffer_pointers` wrote nothing (the null read of Qt6 programs). Commit
requests had no limit. The MSYS2 runtime never made its signal thread.

## Decisions

### 1. One lock implementation, in `ntdll`, over a parking lot

`poly_rtl::sync` (host-tested, 15 tests with many threads) implements the SRW lock, condition
variable, run-once object and `RtlWaitOnAddress`. A waiter parks in a table of 64 buckets keyed by
the word's address (a spin lock and a first-come list of nodes on the waiters' stacks: nothing is
allocated) and blocks on an event of its own, taken from a small pool. A release hands the lock
to the first waiter (or all the readers at the head of the queue) before it wakes them, so there
is no barging and a waiting writer holds readers back. Words keep Windows' observable values (0
free; `(n << 4) | 1`). `ntdll` exports the `Rtl*` names; `kernelbase` (and so `kernel32`) calls them,
so there is one implementation per process; the old code in `poly-kernelbase::sync` is gone. No
kernel change: a keyed event or futex would be cheaper per wait but the lot needs only the events
Poly already has.

### 2. The thread pool lives in `kernelbase`, one lock for all of it

Pure rules (`poly_kernelbase::threadpool`: sizing, the environment layout, due times; tested with a
threaded simulation) and glue (`kernelbase/src/threadpool.rs`). A pool's threads sleep on a condition
variable; a push wakes a sleeper and starts a thread when more jobs wait than there are sleepers
(two callbacks that wait for each other both run; a one-thread pool runs them in turn). Timers and
waits are kept by service threads (each waits for at most 63 handles and a control event, with the
earliest due time as the timeout); I/O objects share one completion port and thread. Objects are
freed by the last finishing callback after a close. `RegisterWaitForSingleObject` keeps its thread
per registration.

### 3. A thread's queued user APCs run before it starts

Windows starts every thread with `NtContinue(.., TestAlert = TRUE)`, so "an APC queued before the
thread begins running" runs first (Learn, `QueueUserAPC`). The MSYS2 runtime queues one to its own
thread from `DllMain` and that routine creates its signal thread; Poly never delivered it, so the
parent waited for a thread that did not exist. `ntdll`'s thread start now calls `NtTestAlert`
before the start routine.

### 4. Commit limit, UCRT buffer slots, queries

`NtAllocateVirtualMemory` refuses sizes of 128 TiB and more (`STATUS_INVALID_PARAMETER`) and a
commit beyond 1 TiB or a job's process/job memory limit (`STATUS_COMMITMENT_LIMIT`; the job's
limit counts the calling process's committed private bytes). The UCRT's `FILE` keeps `_ptr`,
`_base`, `_cnt` at +0, +8, +0x10: only `ucrtbase.dll` publishes the stream's buffer there after every
call and takes back what a program's inline `getc` did (`msvcrt.dll`'s classic `_iobuf` has other
fields at those offsets). `NtQueryInformationProcess/Thread` answer the classes and access rules of
the sheets; priorities and the priority class are kept by the kernel.

## Open

The MSYS2 signal thread now exists and `sh.exe -c "/usr/bin/true; echo hi"` runs; a pipeline of
two forks (`ls / | head`) still does not finish (next stopper, see the hand-back).
