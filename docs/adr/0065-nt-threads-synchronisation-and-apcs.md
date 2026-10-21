# 0065: NT processes grow threads, synchronisation and APCs

**Status:** Accepted. Builds on [ADR 0063](0063-nt-processes-object-manager-and-memory.md)
(NT processes, the object manager, memory and sections in the kernel),
[ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md) (`poly-ntobj`:
the dispatcher as host-tested logic), [ADR 0053](0053-smp-per-cpu-data-scheduling-and-tlb-shootdowns.md)
(SMP-correct park/unpark and wait queues) and [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(our side is `ntdll`/`kernelbase`, not the apps' runtimes).

## Context

[ADR 0063](0063-nt-processes-object-manager-and-memory.md) left a PE running
as an NT process with one thread, a minimal TEB/PEB, the object manager,
memory and section services, and a dispatcher (`poly-ntobj`) that already
decided waits, mutant ownership and the like but was *not wired into the
kernel*. The next two Phase 5 roadmap items are:

- NT processes and threads, TEB/PEB, thread contexts, APCs;
- NT synchronisation: events, mutants, semaphores, timers,
  `NtWaitForMultipleObjects`.

As in ADR 0063 there is no Windows oracle VM, so the clean-room rules
([CLEAN_ROOM.md](../../CLEAN_ROOM.md)) allow only public Microsoft
documentation (learn.microsoft.com, the WDK `Nt*`/`Zw*` reference), public
SDK/WDK header *names, layouts and constants* (`winnt.h`, `winternl.h`,
`ntdef.h`, `wdm.h`, `ntpsapi.h`), and the documented `CONTEXT`, `TEB`,
`CLIENT_ID`, `PROCESS_BASIC_INFORMATION` and `THREAD_BASIC_INFORMATION`
layouts. Nothing is disassembled, decompiled or debugged. Where documentation
does not settle a behaviour the code picks one and says "to be confirmed by
the spec agent against real Windows".

## Decision

**The dispatcher decides; the kernel blocks.** Following ADR 0030, every
decision — which wait a signal satisfies, mutant ownership and abandonment,
semaphore limits, the `NTSTATUS` each wait returns, timer signal state — stays
in `poly-ntobj`, host-tested. The service layer (`personalities/nt`) decodes
the ABI and asks the dispatcher; when a wait must block, the kernel parks the
thread and wakes it. The seam is the `NtKernel` trait, which grew methods for
the acts that need threads, the scheduler, another thread's registers, the
clock or user memory beyond argument copying.

**Services (Poly's own numbering, `services.rs`).** New executive services,
in ADR 0063's `table << 12 | index` layout. Object-manager table, dispatcher
group: `NtPulseEvent` (0x00E), `NtClearEvent` (0x00F), `NtQueryEvent` (0x010);
`NtCreate`/`Open`/`Release`/`QueryMutant` (0x020–0x023); the semaphore four
(0x024–0x027); `NtCreate`/`Open`/`Set`/`Cancel`/`QueryTimer` (0x028–0x02C);
`NtWaitForSingleObject` (0x02D), `NtWaitForMultipleObjects` (0x02E),
`NtSignalAndWaitForSingleObject` (0x02F); `NtQueueApcThread` (0x030),
`NtTestAlert` (0x031), `NtDelayExecution` (0x032), `NtYieldExecution` (0x033).
Process/thread table: `NtTerminateThread` (0x081), `NtCreateThreadEx` (0x082),
`NtOpenProcess` (0x083), `NtOpenThread` (0x084),
`NtQueryInformationProcess` (0x085), `NtQueryInformationThread` (0x086),
`NtSuspendThread` (0x087), `NtResumeThread` (0x088), `NtGetContextThread`
(0x089), `NtSetContextThread` (0x08A). One Poly service joins table 3:
`PolyResumeApc` (0x3001). Numbers are never reused.

**Threads start through a per-process trampoline.** Until our `ntdll`
provides `RtlUserThreadStart`, each process has one executable page with a
thread-start trampoline and a user-APC trampoline (hand-assembled, no
Microsoft toolchain). A new thread starts there with its start routine and
argument in registers; the trampoline calls `routine(argument)` and, when it
returns, ends the thread with `NtTerminateThread(NtCurrentThread,
return_value)`. The first thread goes through it too (its routine is the image
entry, its argument the PEB), so **the process exits with the right code when
its last thread's entry returns**. Each thread has its own TEB (`NT_TIB`
stack base/limit and self, `ClientId`, the PEB pointer) and its own stack with
a guard page; the thread layout is host-tested like `exec::start_process`.

**`NtCreateThreadEx`** creates a thread in the *calling* process (the kernel
lays out its stack, TEB and thread object, then starts it in ring 3, deferring
the start if `CREATE_SUSPENDED`). Cross-process creation and
`NtCreateUserProcess` (a new process from an image path) are **left for the
`ntdll` milestone**: they need a second address space built from the VFS and a
new process record, which is more than this milestone's "keep the kernel edits
small and local". `NtOpenProcess`/`NtOpenThread` open by `CLIENT_ID`;
`NtQueryInformationProcess`/`Thread` answer the basic information class
(a running process/thread reads `STATUS_PENDING`; an exited thread's object
keeps its exit status while a handle to it is open).

**Blocking reuses park/unpark (ADR 0053).** A wait that cannot be satisfied at
once returns `Blocked(WaitId)` from the dispatcher; the kernel registers the
thread's wait and parks it on the generic process wait queue. Every signalling
call drains the dispatcher's `Wake`s after it runs (`drain_wakes`), records
each blocked wait's result and unparks its thread. Timeouts (relative or
absolute 100 ns `LARGE_INTEGER`), alerts and user APCs end a wait through the
dispatcher's `cancel_wait`. This is SMP-correct because park/unpark already is.

**Suspension saves the context.** `NtSuspendThread` raises a per-thread count
and kicks the thread; before it next runs user code it parks (a hook in
`return_to_user`), saving its user registers so another thread can read them
with `NtGetContextThread` and change them with `NtSetContextThread`, applied
when it resumes. `NtGet`/`SetContextThread` fill in the `CONTEXT` control and
integer registers (x64, `winnt.h` offsets); the floating-point/XMM save area
and debug registers are left for a later milestone (not asked for here).

**User APCs** (`NtQueueApcThread`) queue on a thread and are delivered at an
alertable wait (`NtWaitFor*`/`NtDelayExecution` with `Alertable`) or
`NtTestAlert`, per the documented `QueueUserAPC`/`SleepEx` semantics. Delivery
redirects the thread to the APC trampoline, which calls the routine and then
re-enters the kernel with `PolyResumeApc` to restore the interrupted context
(our stand-in for `KiUserApcDispatcher`), so the wait returns
`STATUS_USER_APC`. Kernel-mode and special APCs are not modelled.

**Waitable timers** run on the kernel timer service (the one timer files use):
`NtSetTimer` arms it with an absolute or relative due time and an optional
period and timer APC; on expiry the kernel signals the timer object (waking
waiters) and, for a periodic timer, re-arms it and queues the timer APC to the
setting thread.

## To be confirmed by the spec agent against real Windows

In addition to ADR 0030's and ADR 0063's lists:

1. The exact `PROCESS_BASIC_INFORMATION` / `THREAD_BASIC_INFORMATION` fields
   beyond `ExitStatus`, `PebBaseAddress`/`TebBaseAddress`, the ids and the
   `CLIENT_ID` (affinity, priority).
2. `MUTANT_BASIC_INFORMATION.CurrentCount` (ours: `1 - recursion`) and
   `OwnedByCaller`/`AbandonedState`.
3. Whether `NtSignalAndWaitForSingleObject` signals and begins the wait
   atomically (ours does them in order).
4. The access `NtQueueApcThread` needs (ours: `THREAD_SET_CONTEXT`), the APC
   queueing/draining order, and whether a non-alertable wait is disturbed by a
   queued APC (ours re-parks).
5. `NtCreateThreadEx`'s `CreateFlags` beyond `CREATE_SUSPENDED`, its attribute
   list, and the exact stack rounding when the request gives sizes.
6. `NtYieldExecution` returning `STATUS_NO_YIELD_PERFORMED` when nothing else
   was runnable.
7. The precise register groups `NtGetContextThread` reports for a thread
   suspended in user vs. kernel mode, and `EFlags` sanitisation on
   `NtSetContextThread`.
8. Absolute-timeout conversion (we use the `FILETIME` epoch difference against
   our real-time clock) and the `STATUS_TIMEOUT` vs. `STATUS_SUCCESS` boundary.

## Consequences

- `poly-ntobj`'s dispatcher (events/mutants/semaphores/timers, wait-any/all,
  abandonment, limits, fairness against a randomized reference model) and the
  service decoders are host-tested with no machine. The kernel side proves the
  threading, blocking, suspension, APC delivery and timers with a ring-3
  `kernel_test!` driving a self-assembled multi-threaded PE
  (`kernel/src/nt/test_threads.rs`).
- The kernel keeps one lock for the whole NT state (ADR 0030); per-thread
  control blocks (wait, suspend, APC, context) live beside it keyed by thread
  id. Blocking happens outside the lock, on the generic park/unpark.
- What is left for later Phase 5 milestones: cross-process creation and
  `NtCreateUserProcess` (so a parent can wait on and query a *child* process's
  exit code), structured exception handling (a fault still kills the thread),
  file I/O, the registry, and our `ntdll`/`kernelbase` (which will own
  `RtlUserThreadStart`, `KiUserApcDispatcher` and the loader, replacing the
  trampolines).
- The coverage map is unchanged: these services cannot go green without the
  Windows oracle, which does not exist yet.
