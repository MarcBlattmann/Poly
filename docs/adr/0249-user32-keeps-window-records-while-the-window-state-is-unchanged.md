# 0249: user32 keeps window records while the window state is unchanged

**Status:** Accepted. Track `chrome-timeline-2`. Builds on ADR 0128 (win32k in the NT personality),
ADR 0219 and 0231 (DPI units), ADR 0247 (the timeline). Inputs: Poly's own timeline of Chrome for
Windows. No third-party source.

## Context

The timeline (ADR 0247, now naming each `PolyUserCall` by its request) showed Chrome's browser UI
thread making about 1,400 `PolyUserCall`s in the 150 ms after a Ctrl+Tab, 35 ms inside them. Most
were `Window` (a window's whole record: about 730 a second while switching) and `Desktop` (190 a
second). They come from `IsZoomed`, `MonitorFromWindow`, `GetClientRect`, `ClientToScreen` /
`ScreenToClient`, `IsIconic`, `GetWindowThreadProcessId`, `IsWindow`, `IsWindowVisible`,
`GetWindowRect`, `NotifyWinEvent` and `EnumThreadWindows`, each of which asked the kernel: encode
the request, a system call, the NT state's lock, decode the reply. Windows answers these in user
mode from the window manager's data its `user32` can read.

## Decisions

1. **A window-state generation.** The kernel keeps one page (`kernel/src/nt/winstate.rs`; shared
   like the vDSO's data page: a page-cache page, a shared read-only file mapping, read-only for
   good). Its first eight bytes are a generation that goes up before every change of the window
   manager. A process gets the page on its first `Request::SharedPage`; its address is the reply.
2. **Every change goes through the generation.** The NT personality reaches the window manager
   only through `wmgen::Tracked`: reads through `Deref`, changes through `mutate()`, which calls the
   kernel's hook before it hands out `&mut Wm`. A change is never visible before the generation
   that announces it, and the compiler finds any path that would change it otherwise.
3. **Reads do not count as changes.** `Wm::handle_read(&self)` answers `Window`, `Desktop`,
   `GetLong` and `KeyboardState` exactly as `handle` does (in the caller's DPI units), for a thread
   that has a queue; the personality tries it before the full path. Host tests compare both for
   unaware, system-aware and per-monitor callers.
4. **user32 keeps `Window` and `Desktop` replies** (`wincache.rs`), per window and thread (replies
   are in the thread's units), with the generation read *before* the request was sent, and uses
   one again only while the page shows that generation. A change made while the request was on its
   way leaves the reply under the older generation, so it is asked again. A slot another thread
   holds is passed by, never waited for. A 32-bit process whose page lies above 4 GiB, or a kernel
   without the page, asks every time, as before.

## Consequences

Measured with the timeline, Chrome for Windows at 2560x1600, 200 %, one session, the cache switched
off (every `PolyUserCall` raising the generation, a temporary switch, removed) and on in blocks of 8
Ctrl+Tab switches:

| Per Ctrl+Tab (150 ms after the key), browser UI thread | Cache off | On |
|---|---|---|
| `PolyUserCall`s | 1,394 | **617** |
| Time inside them | 35.4 ms | **17.9 ms** |
| `Window` requests reaching the kernel, per second | 726 | 139 |
| `Desktop` requests reaching the kernel, per second | 188 | 60 |

The time from the key to the first frame did not change measurably (the host's load moved it by
more than the difference in that session); the browser UI thread is not the slowest step of a
switch, Chrome's own code in the GPU and renderer processes is (ADR 0247).

What `NtAllocateVirtualMemory` cost, measured with temporary counters (removed) on the same runs:
the call itself is about 10 µs; its slow cases (up to 35 ms, and those of `NtFreeVirtualMemory`
and `NtUnmapViewOfSection`) are TLB shootdowns. While Chrome switches tabs a shootdown waits about
1.3 ms on average for the CPUs that run the same process to acknowledge (30 µs when the machine is
quiet), and shootdowns are serialised system-wide. That is the next step for these calls (sending
without waiting, freeing frames once acknowledged), not part of this decision.
