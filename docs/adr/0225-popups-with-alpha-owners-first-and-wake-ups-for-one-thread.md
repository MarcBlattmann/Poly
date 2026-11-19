# 0225: Popups with their alpha, owners before what they own, and a wake-up for one thread

**Status:** Accepted. Track `chrome-feel`: Chrome for Windows in real use at the owner's settings
(2560x1600, 200 %, WHPX, sandbox and GPU on). Builds on ADR 0128 (each top-level window is a
surface of the compositor), ADR 0106 (the scene composes premultiplied ARGB), ADR 0211 (the software
Direct3D 11 device and DirectComposition) and ADR 0219. Inputs: Microsoft Learn ("DXGI_ALPHA_MODE",
"IDXGIFactory2::CreateSwapChainForComposition", "IDCompositionDevice::CreateTargetForHwnd",
"BLENDFUNCTION", "UpdateLayeredWindow", "Window Features": owned windows, "NtWriteFile"); Poly's own
traces and timings of Chrome running on Poly. No third-party source.

## What was there

The owner reported three things: popups with a black frame, typing and switching tabs that lag, and
artifacts when a window moves. Measured on Poly:

- **Black frames.**
  - Chrome's menus and bubbles are popups with `WS_EX_NOREDIRECTIONBITMAP`.
  - The GPU process shows each one through DirectComposition: a target for a child of the popup
    and a composition swap chain with `DXGI_ALPHA_MODE_PREMULTIPLIED`.
  - win32k gave every surface the opaque `Xrgb8888` format, so the transparent corners and the
    shadow came out black.
- **Lag.**
  - A key reached Chrome's queue in 4–8 ms, but the first frame showing it came 110–340 ms later,
    up to seconds for Ctrl+L, Ctrl+T and Ctrl+Tab, while the four virtual CPUs were 0–30 % busy.
  - Chrome's threads sat in `NtWaitForSingleObject` on events that were set, each wait lasting
    ~70–90 ms.
  - Every parked thread of every process slept on **one** kernel wait queue (`process::EVENTS`).
    Each `unpark`, and each tick that ended a timed park, woke all of them; each checked its flag
    and slept again. With Chrome's few hundred threads under WHPX, the thread meant to wake waited
    behind all the others.
- **Window move.**
  - Chrome shows its "Restore pages?" bubble before its main window. win32k made the bubble's
    surface without an owner, because the owner had no surface yet, and never fixed that.
  - So the bubble did not move with the window and lay below it. The window covered it, and a move
    uncovered it as a stale rectangle.
- **A kernel panic.** `NtWriteFile` copies overlapped and pipe writes of up to 16 MiB into one
  kernel buffer, but the kernel heap gives at most 4 MiB in one piece. A 7 MB write of Chrome's
  ended the kernel ("memory allocation of 6992584 bytes failed").

## Decisions

1. **A window's surface may carry premultiplied alpha.**
   - New win32k request `SurfaceAlpha { hwnd, premultiplied }` (118).
   - `d3d11.dll` sends it when DirectComposition binds a swap chain to a window: premultiplied for
     `DXGI_ALPHA_MODE_PREMULTIPLIED`, opaque otherwise.
   - win32k takes a child window's flag for its top-level window, since a child's pixels are part
     of its top-level window's surface, and attaches that surface's buffer as `Argb8888`.
   - The same holds for a layered window given its content by `UpdateLayeredWindow` with
     `ULW_ALPHA` and `AC_SRC_ALPHA`.
   - A buffer is attached anew when the format changes, and keeps its pixels. The compositor
     already blends `Argb8888` (ADR 0106).
   - This says nothing about how pixels travel, so a shared-buffer present (ADR 0224) keeps it.
2. **An owned window joins its owner once the owner has a surface.** When a shown window's owner
   gets its surface later, the window's surface is made again with that owner, keeping its picture.
   It then moves with its owner and is composed above it, as owned windows are on Windows.
3. **A thread parks on its own queue.**
   - `ThreadFlags` has its own wait queue. `park` sleeps on it, `unpark` wakes only that thread,
     and a timed park's deadline wakes only that thread.
   - `notify` also wakes `EVENTS`, because `wait_event` waiters look at the signal flag too.
   - A group exit and `end_other_threads` wake the parked threads of their process.
   - `wait_event` and process waits stay on `EVENTS`.
4. **A write the kernel cannot buffer fails instead of ending the kernel.** The buffer of a captured
   write (and of a captured socket send) is reserved fallibly. When it cannot be had, the call
   returns `STATUS_INSUFFICIENT_RESOURCES`.

## Consequences

- Chrome's ⋮ menu, right-click menu, Translate bubble and "Restore pages?" bubble show rounded
  corners and shadows over the page.
- The "Restore pages?" bubble is now visible at all; before, it lay under the window.
- Latency, same build and scenario, the old shared queue (kernel argument used only for that
  measurement) against the new one, at the owner's settings with KVM-accelerated WHPX:
  - a key reached Chrome in 0–1213 ms before and 0–6 ms after;
  - key → glyph on screen in the address field had a median of ≈1.2 s before and ≈130 ms after
    (29–210 ms);
  - Ctrl+Tab took 760–980 ms before and 480–560 ms after;
  - Ctrl+W took 1007 ms before and 107 ms after.
- A window moves with its bubbles and leaves nothing behind.
- What still costs time is drawing:
  - Chrome redraws its whole 2074x1499 window for most changes (5–35 presents per keystroke,
    3–27 ms each);
  - the compositor composes a full 3.1 MP frame in ~30 ms.
  - Both are the shared-buffer work of ADR 0224.
- A captured write over the kernel heap's 4 MiB now fails instead of panicking. Capturing large
  writes in pieces, or from scattered frames, is still to do. The page that ended the kernel
  (de.wikipedia.org) now loads and Chrome browses on; whether the large write came again in those
  runs was not traced.
- Before its first frame a bubble shows its window opaque white for a moment (the browser's own
  erase). On Windows a `WS_EX_NOREDIRECTIONBITMAP` window shows nothing of what GDI draws; Poly
  still shows it.
