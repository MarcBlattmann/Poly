# 0069: POSIX timer overruns counted when the signal is taken; the last thread's exit status; child subreapers

**Status:** Accepted. Amends [0044](0044-time-syscalls-timers-and-the-vdso.md) (how
timers fire and count overruns), [0041](0041-processes-threads-fork-exec-wait.md)
(threads and exits; the process tree) and the behaviours of
[0064](0064-test-runs-as-a-tracer-sees-them-and-spec-a-behaviours.md).

## Context

The runner's third pass over spec-a's entries left a few red ones whose cause was in
Poly, not in the record of a run:

- `exit`: wait4 sometimes reported the main thread's code where the spec sheet says
  "the process ends when its last thread exits, and wait4 then reports the last
  thread's code". The visible test's worker thread waits until the main thread's
  `clear_child_tid` word is cleared and then exits; on Poly the main thread cleared
  the word *before* it left its process, so the worker could leave first and the main
  thread, leaving last, set the status.
- `timer_getoverrun` (new revision): the count is "the number of expiries due from the
  one that queued the signal up to T", T being when the signal is taken, "every expiry
  due at or before T on the timer's clock counts"; it is capped at `DELAYTIMER_MAX`
  and a 1 ns periodic timer pending for 2.5 s must reach the cap without using CPU
  time. Poly counted expiries when its timer thread looked (one wake-up per expiry
  while the signal was pending, so a 1 ns timer kept a CPU busy), and expiries due
  but not yet looked at when the signal was taken were missing (a count of 19 below
  the allowed range).
- `timer_settime`: an absolute time in the past "expires at once", but timer_gettime
  reported 1 ns left until the timer thread had run; a null new value must be
  `EINVAL`; a pending signal of a re-armed or disarmed timer "is not delivered
  anymore".
- `getppid`: orphans go to "the nearest ancestor marked as child subreaper (prctl
  PR_SET_CHILD_SUBREAPER 36) or to init", which Poly did not support; `getpgid` /
  `getsid` of a thread id must answer for its process (Poly said `ESRCH`);
  `renameat2(RENAME_NOREPLACE)` onto the source's own name must be `EEXIST`.

Sources: the spec sheets and visible tests of these entries in `specs/linux/x86_64/`,
the man pages timer_create(2), timer_settime(2), timer_getoverrun(2), prctl(2) and
the UAPI headers for numbers. No Linux or C library source was read and no Linux
system was run or traced.

## Decision

**Timer overruns are counted when the signal is taken** (`time/timers.rs`).

- When a POSIX timer comes due and its signal is queued, the timer is marked as
  having its signal pending and its deadline stays at the first expiry after the one
  that queued the signal, even after that is past. `Timers::wakeup` reports a
  pending timer's *recheck* time instead of its deadline: one interval later, at
  least `PENDING_RECHECK_NS` (10 ms). So the timer thread does nothing per expiry
  while the signal waits, whatever the interval.
- Taking a signal is two steps. Every path that takes signals for a thread
  (delivery on the way back to user mode, rt_sigtimedwait, a signalfd read) calls
  `timers::signal_taken` in the same hold of the process's lock as the dequeue: a
  timer's signal marks the timer as being taken (the timer thread leaves it alone).
  The path then reads the timer's clock and calls `timers::count_taken`, which
  computes the count arithmetically (`(T - deadline) / interval + 1` expiries, at
  most `DELAYTIMER_MAX`), writes it into the siginfo (`si_overrun`) and into the
  timer (timer_getoverrun returns it until the next signal is taken), moves a
  periodic timer to its first expiry after T and tells the kernel the next wake-up
  (`LinuxKernel::timers_changed`, or in the kernel `linux::timers::count_taken`).
- A signal that went without being taken (discarded by `SIG_IGN`, or a thread's
  queue that went with the thread) is noticed at the next recheck: the timer goes on
  as if it had never been queued. A plain `LinuxProcess::dequeue` that skips the
  first step loses only the count.
- `timer_settime` discards the timer's pending signal and resets the count;
  `timer_gettime` reports a one-shot timer whose time has come as 0 and a periodic
  one with expiries due as the time to the next one to come (`Timer::left`;
  `getitimer` keeps its own rule). A null new value is `EINVAL`.

**The last thread's exit status** (`process.rs`, `user::end_thread`). A Linux thread
records its status as the process's candidate status (`Process::thread_exiting`)
before its thread-exit duties (robust futexes, `clear_child_tid`); the thread that
leaves the process last still ends it, with the group exit's status if there is one,
else the candidate of the thread that *started* to exit last. A thread that sees
another's `clear_child_tid` cleared and then exits is therefore the one whose code
the parent sees, whichever of the two detaches last.

**Child subreapers** (`task/tree.rs`, `sys/subreaper.rs`). The process tree keeps a
subreaper mark per process (prctl(2) `PR_SET_CHILD_SUBREAPER` 36 sets or clears it,
`PR_GET_CHILD_SUBREAPER` 37 reads it as an `int`; not inherited by fork, kept across
execve). When a process ends, each child goes to the nearest still living ancestor
marked as subreaper, below the child's pid namespace's init (a subreaper outside the
namespace does not take its init's place), else to the reaper as before (ADR 0054).

**Smaller points.** getpgid and getsid map a thread id to its process
(`LinuxKernel::with_process(Target::Thread)`); a zombie, which has no threads, is
still found by its pid. `RENAME_NOREPLACE` is `EEXIST` for any existing target, the
source's own name and its other hard links included (`Vfs::rename`).

## Consequences

- A periodic timer whose signal stays pending costs one look per interval (at least
  10 ms apart) instead of one per expiry; its count is exact to the time the signal is
  taken, read just before the count is fixed.
- A discarded timer signal is noticed up to one recheck late, so the timer's next
  signal can come that much later than its due time in that case.
- `timer_create` still gives a failed call no id (the spec sheet leaves it open).
- Not done: a timer on another thread's CPU clock whose signal is taken by a thread
  that cannot read that clock any more counts 0.
