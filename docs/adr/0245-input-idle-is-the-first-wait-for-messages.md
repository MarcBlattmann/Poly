# 0245: Input idle is the first wait for messages

**Status:** Accepted. Track `impl-wait-for-input-idle`; sheets `specs/windows/user32.dll/WaitForInputIdle.yaml` and
`specs/windows/shell32.dll/ShellExecuteExW.WaitForInputIdle.yaml` (PR #380). Builds on [ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
(win32k in the NT personality) and [ADR 0223](0223-windows-downloads-streams-shell-classes-and-the-desktop.md) (`ShellExecuteExW`).

## Context

`WaitForInputIdle` lets a parent wait until a GUI child has finished starting: Microsoft Learn says it returns "as
soon as any thread of the process becomes idle" and "waits only once". The sheet pins what idle means on Windows 11:

- a thread **blocks** for messages with nothing in its queue: `GetMessage`, `WaitMessage`,
  `MsgWaitForMultipleObjects`. A `PeekMessage` that finds nothing is not idle (it does not wait), and neither is a
  `GetMessage` that returns a message already queued;
- a console-subsystem target is refused at once (`ERROR_NOT_GUI_PROCESS`, 1471), whatever it does, even if it never
  loads `user32`; a GUI child that never waits is waited for until the timeout or its end;
- once idle, every later call returns 0 at once, also while the child is busy again; calls that timed out do not count;
- a handle needs a query right (`PROCESS_QUERY_INFORMATION` or `..._LIMITED_...`), else `ERROR_INVALID_HANDLE`; a process
  that ended before the call fails without an error, one that ends during the wait satisfies it.

`ShellExecuteEx` with `SEE_MASK_WAITFORINPUTIDLE` waits the same way for a GUI program, but counts an empty `PeekMessage`
too, skips console programs, stops after about 60 s and never reports a failure of the wait.

Poly's `user32!WaitForInputIdle` was a stub that returned 0 at once, and `shell32` ignored the mask.

## Decision

1. **win32k keeps two bits per process** (`poly_win32k::idle::InputIdle`): `LOOKED` (a thread found its queue empty) and
   `BLOCKED` (a thread found it empty and is about to wait). They are set once and only forgotten when the process
   ends, which gives "waits only once" for free. `user32` reports them with a new request,
   `InputIdle { blocking }`, from the three places a thread waits for input in `poly_win32k::user`: `get_message` on an
   empty queue, `msg_wait` before it blocks with an input mask, and `peek_message` on an empty queue (not blocking).
   Each process sends each kind once (an atomic in `user32`), so an application that polls `PeekMessage` pays one
   round trip in its life. `IdleState { pid }` reads them.
2. **The kernel tells console programs from GUI ones.** `NtProcess` keeps the executable's `Subsystem` (the optional
   header field, set when the image is mapped), and Poly's own process information class `0x1003`
   (`PROCESS_POLY_SUBSYSTEM`) answers it as a `u32` to a handle with a query right. A child that never loads `user32`
   never talks to win32k, so win32k cannot know this.
3. **The waits are `poly_win32k::idle`'s**, host-tested, on a small `IdleEnv` (process id and running from
   `ProcessBasicInformation`, the subsystem, the bits, a clock, a sleep): `wait_for_input_idle` (handle, ended, console,
   then `BLOCKED` or the end, or the timeout) and `shell_wait` (GUI only, `LOOKED` or `BLOCKED` or the end, at most
   60 s). Both look every 10 ms: the sheet's tests allow up to 2 s, and a handle with only a query right cannot be
   waited on (it has no `SYNCHRONIZE`), so an event would not cover every case anyway.
4. `shell32` calls `shell_wait` on the process it started before it hands the handle back or closes it, through
   `win32u!PolyUserCall` for the bits, as `d3d11` does for its requests.

## Consequences

- Installers and launchers that start a GUI program with the mask (B0's installer path, `polyopen.exe`) now return after
  the program's start-up instead of at once; a console program is never waited for.
- A process that polls `PeekMessage` and never waits is idle for the shell but not for `WaitForInputIdle`, as on
  Windows.
- Not covered by the sheets and not done: `SendMessage` to another thread as an idle point, 32-bit (WOW64) callers of
  class `0x1003` (the thunk passes Poly's classes through unchanged; not tested), a process of another user.
