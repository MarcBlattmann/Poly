# 0210: NT ids are multiples of 4, a main thread has its own id, and an exception out of a guarded call unwinds first

**Status:** Accepted. Builds on [0089](0089-process-creation-handles-and-inheritance.md) (process creation),
[0099](0099-cpp-exception-handling-in-msvcrt.md) (C++ exceptions) and [0203](0203-job-objects-nest-and-the-job-information-classes-answer-as-observed.md)
(the previous batch of "Poly differs from the sheet" fixes). The inputs are the spec sheets of
`GetThreadId`, `NtMapViewOfSection`, `__CxxFrameHandler3`, `SetThreadpoolTimer`,
`RtlSleepConditionVariableSRW`, `_get_stream_buffer_pointers`, `recv`/`connect` (UDP) and `setsockopt`, and
their visible tests; nothing else.

## Context

Integrators found entries where Poly's behaviour differs from what a sheet states. Three of the
fixes are more than a local change: they decide how ids look, what `VirtualQuery` says about pages a
program wrote, and when the C++ runtime ends an unwind. The others (listed at the end) are rules of
a sheet applied in the place that owned them.

## Decision

- **Ids.** A process or thread of the NT personality gets an id that is a non-zero multiple of 4
  (`GetThreadId.yaml`, `created_thread`), from the kernel's one counter (`process::alloc_nt_id`
  takes the next multiple of 4, not below 0x400 (a program may use a small number such as 4 as "no such thread"), and moves the counter past it, so Linux and NT ids never repeat).
  The first thread of a process made by `NtCreateUserProcess` (or started by `spawn_pe_with`) has an
  id of its own, not its process id (`child_main_thread`). The id is chosen before the TEB, the
  thread object and the `ClientId` are made (`Start::tid`), and a suspended first thread is resumed
  under the same id. Processes started by the Linux side keep their pid as process id.
- **Copy-on-write pages.** A write-copy view is mapped as a private file mapping, so the first write
  makes the kernel copy the page. `NtQueryVirtualMemory` asks the backend (`Backend::page_written`:
  a frame the space owns, with its dirty bit set) and reports `PAGE_READWRITE` (or
  `PAGE_EXECUTE_READWRITE`) for such pages, as its own run, instead of `PAGE_WRITECOPY`
  (`NtMapViewOfSection.yaml`, `copy_on_write`). A page the kernel itself wrote into (a `ReadFile`
  into the view) still shows `PAGE_WRITECOPY`; Poly cannot see that write.
- **A guarded call that is left by an exception unwinds first.** Destructors, unwind actions and copy
  constructors run through `poly_cxx_call_guarded`; an exception leaving one is `terminate()`
  (`__CxxFrameHandler3.yaml`, `terminate_destructor`). The runtime used to call `terminate()` as soon
  as the search reached the guard. It now unwinds the frames below the guard first
  (`STATUS_UNWIND_CONSOLIDATE` to the guard's own frame, callback `terminate_callback`), as Windows'
  frame handler does while the exception goes out of the call: the terminate scope the compiler
  gives a `noexcept` destructor runs its action (`__std_terminate`, the program's own), so
  `terminate()` then comes from the program (`clang_noexcept`); what is above the call is not
  unwound and no clause is entered.

## Also in this batch (rules of sheets, no design choice)

- `RtlSleepConditionVariableSRW` with a mode that does not match the hold gives the lock up as
  its hold is and takes it back so (`Lot::sleep_srw`); the status is the timeout.
- `SetThreadpoolTimerEx` returns whether a due time was still to come (`timer_was_pending`): a
  one-shot timer that fired is still "set" for `IsThreadpoolTimerSet`, but not pending.
- `_get_stream_buffer_pointers` on an unbuffered stream: after the first read the record's buffer
  holds the byte returned and the next byte (`poly_crt::slots::unbuffered_window`), and the
  count is 1 while that byte is unread.
- A receive on a datagram socket that fails with the stack's `ECONNREFUSED` (the ICMP "port
  unreachable" of an earlier send) is `WSAECONNRESET` (`datagram_receive_status`).
- A refused non-blocking connect: the error the event helper takes from the stack and stores while
  the `connect` call is still on its way is not wiped by that call (`Endpoint::connect_under_way`,
  `conn_error_stores`); `SO_ERROR` looks again at the stored error when the stack has none.

## Consequences

- Thread and process ids of NT programs jump by 4 (and by more when Linux programs also run).
- An id table indexed by id must not assume small ids; none in the tree does.
- A `terminate()` out of a guarded call is later than it was (after the frames below ran), so a
  destructor's own locals are destroyed first; programs cannot tell it from Windows.
