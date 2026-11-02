# 0131: The first Windows GUI slice: win32k's window manager behind one system call, the message sequences in `user32`, blocking on a per-thread event, GDI in `poly-gdi`, Windows' non-client metrics, the desktop link through `/dev/win32k`

**Status:** Accepted. Implements the first slice handed over by
`spec-win-gui-a` (45 entries in `specs/windows/user32.dll/` and
`specs/windows/gdi32.dll/`) inside the architecture of
[ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md),
and records where the implementation made choices ADR 0128 left open or
changed its starting points. Roadmap Phase 7's first three items stay
unticked (painting through update regions, input, dialogs and menus are
later slices).

## Context

The slice covers window classes, windows (creation, destruction, styles,
positions, z order, text, window longs, parents and owners), the message
queue (post, send within and across threads, get/peek/dispatch/translate,
quit, queue status, `MsgWaitForMultipleObjects`), timers, and drawing
through memory DCs and DIB sections (the oracle's desktop is not
visible, so nothing is painted through windows). Sources: the slice's
sheets and visible tests, Microsoft Learn, `winuser.h`, `wingdi.h`,
`windef.h`, `winerror.h`.

## Decisions

### 1. One system call carries win32k's requests

ADR 0128 listed a starting set of `PolyUser*` services, one per function.
The slice replaced it (no number had shipped with an implementation)
with **`PolyUserCall`** (`0x1000`): `user32` encodes a
`poly_win32k::proto::Request`, the NT personality decodes it, runs it
against the session's window manager and encodes the `Reply` into the
caller's buffer (`STATUS_BUFFER_TOO_SMALL` with the length needed). The
records are one Rust enum both sides are built from (`poly_win32k::wire`:
little-endian, tagged), so a new request touches neither the service
table nor `win32u`'s stubs. `PolyUserCommitSurface` (`0x1001`) is the
desktop link's (Decision 8).

### 2. The window manager in the kernel, the sequences in `user32`

`poly_win32k::wm::Wm` (one per session, in `NtSystem.win32k`) holds the
classes and atoms, the window tree with each window's rectangles
relative to its parent's client area, per-thread queues (posted messages
up to 10 000, the quit request, sent messages, the "new since last look"
bits), the sends between threads and the timers. It never calls user
code. The **message sequences** — `CreateWindowExW`, `DestroyWindow`,
`ShowWindow`, `SetWindowPos`/`MoveWindow`, `SetParent`, `DefWindowProcW`,
the get/peek/send/dispatch loops — are `poly_win32k::user`, generic over
an `Env` (the system call, calling a procedure, waiting): `user32.dll`
runs them for real, host tests against a `Wm` in memory with procedures
in Rust. The orders and `WINDOWPOS` flags follow the sheets; two rules
carry most of them:

- `WM_WINDOWPOSCHANGED`'s flags gain `0x800` when the client area kept
  its size and `0x1000` when it kept its position (the sheets' observed
  bits), and `DefWindowProcW` sends `WM_MOVE` / `WM_SIZE` exactly when
  those bits are clear — which is why calling it directly sends both.
- `SWP_NOREDRAW` is added when the window is not visible (`WS_VISIBLE`
  on it and every ancestor) before and after the change.

### 3. Blocking in `user32` on a per-thread event

A thread's first wait creates an auto-reset NT event and registers it
(`RegisterEvent`); win32k sets it when a message is posted or sent to
the thread or a reply to its send arrives. `GetMessageW` waits on it
with the next timer as the timeout; `MsgWaitForMultipleObjectsEx` waits
on the caller's handles plus it. A **cross-thread `SendMessageW`**
queues the message for the receiver and polls `SendWait` between waits,
running messages sent *to* it meanwhile; the receiver runs the procedure
inside its next `GetMessageW`/`PeekMessageW` and reports `SendDone`
(`ReplyMessage` answers early). So the kernel never blocks for win32k
and never re-enters user mode, as ADR 0128 wanted (callouts are these
returned `Incoming` records). A thread's or process's end frees its
windows, queue, timers and event (`exec::thread_exit`, `process_exit`).

### 4. Windows' non-client metrics

The sheets fix the frame sizes programs compute with (`SM_CYCAPTION`
23, `SM_CXFRAME` 4, `SM_CXPADDEDBORDER` 4, ...: `AdjustWindowRectEx` adds
8/31/8/8 for an overlapped window, scroll bars are 17), so
`poly_win32k::metrics` reports them and the default `WM_NCCALCSIZE` uses
them. ADR 0128's Decision 4 stands — Poly's frame is the non-client area
— but its *size* is Windows': the compositor draws Poly's frame inside
the non-client rectangle win32k computes. On the desktop (Decision 8)
the surface is the client area and Poly's frame is drawn around it, so
on the screen the frame is Poly's size while the window rectangle apps
see keeps Windows' insets; `poly_win32k::frame`'s hit testing over Poly's
frame comes with input.

### 5. Handles and system windows

Window handles are `generation << 16 | index` (a reused slot gets a new
handle; the lower 32 bits are significant). The desktop
(`GetDesktopWindow`, the screen's rectangle) and the message-only
parent (`HWND_MESSAGE`) are windows of no thread; a message sent to them
runs `DefWindowProcW` in the sender.

### 6. GDI in `poly-gdi`

`crates/poly-gdi` (host-tested) holds a process's objects (DCs, bitmaps,
brushes, pens, fonts, rectangle regions) and the drawing; `gdi32.dll`
keeps one behind a lock and maps the exports onto it, with private
`PolyGdi*` exports for `user32` (window DCs, `FillRect`, `DrawTextW`).
DIB sections (1/4/8/16/24/32 bits, top-down or bottom-up, colour tables)
are pages from `VirtualAlloc`; monochrome device-dependent bitmaps map
only white to 1; `BitBlt`/`PatBlt` apply the general ternary raster
operation to raw pixel values (so `NOTSRCCOPY` sets the high byte, as
observed); colour↔monochrome blits use the source's background and the
destination's text and background colours. The observed object
lifetimes are kept: a deleted DC answers `OBJ_DC`, a deleted unselected
solid brush stays readable (and paints where it is named) but cannot be
selected again, a selected brush or bitmap goes when it is deselected or
its DC is deleted. GDI handles carry a generation (the top byte), so a
deleted object's handle never names the object that reuses its slot.

**Fonts.** The stock fonts report the sheets' `TEXTMETRICW`. The system
font is a 16-pixel bitmap font whose advance widths were solved from
every extent the sheets measured (`Hello, world` 77, `the quick brown
fox jumps` 169, `abcdefghij` 67, `The quick brown fox` 129, ...: `f` is 4,
`j` 5, control characters 1); the rows of `A`, `g` and `1` are the
oracle's recorded pixels, every other glyph is drawn for Poly
(`poly-gdi/src/sysfont.rs`). TrueType faces through `poly-font` come with
the GDI text slice.

### 7. Visibility

`IsWindowVisible` is `WS_VISIBLE` on the window and all its ancestors.
The oracle answered 0 for every shown top-level window because its
session's desktop is not visible; on Poly's desktop they are visible.

### 8. The desktop link

ADR 0128's Decision 3, made concrete for this slice:

- **Which windows.** After every `PolyUserCall` (and when a thread or
  process ends) the NT personality brings the link's surfaces in line
  with the window manager (`poly_win32k::surfaces`): a visible, not
  minimized top-level window of an app with a non-empty client area has
  a surface — its rectangle the client area on the screen, its title the
  window text, framed when the window has a caption. A hidden or
  minimized window's surface is hidden; a destroyed window's goes.
  Nothing happens before a compositor first connects: headless programs
  (the tests) get no surfaces, and their window DCs draw nowhere as
  before.
- **Pixels.** A surface's buffer is kernel memory (32-bit `B, G, R, x`,
  client-sized, at most 2048 x 2048, white until drawn). `user32` asks
  for a window's surface size (`Request::Surface`) in `GetDC`; `gdi32`
  points the DC at a per-window top-down 32-bpp DIB section of that size
  (`PolyGdiAttachSurface`), and `ReleaseDC` commits it with
  `PolyUserCommitSurface(hwnd, pixels, width, height, stride)`, which
  copies it into the buffer and reports damage. A copy per commit is the
  simple first version; sharing the DIB's pages with the compositor is
  for when painting performance matters (WM_PAINT's slice).
- **The device.** `/dev/win32k` (char 250:0, of the local/experimental
  range of Linux's `devices.txt`; `kernel/src/nt/link.rs`) is opened by
  the compositor alone; writes are `FromCompositor` records, reads hand
  out `ToCompositor` records and never block (the compositor polls; what
  win32k produced during an app's call is collected after every NT
  system call, which wakes the poll). Records written before the NT
  state exists wait for it. The request `READ_BUFFER` (`_IOW('W', 1,
  24)`) copies rows of a buffer into the compositor's memory: the
  compositor keeps a copy of each buffer and fetches the damaged rows
  before the scene reads them, so the link carries no handles yet.
  Closing the device disconnects; reopening replays every surface.
- **The compositor.** With `--desktop` it opens the device, greets
  win32k with the output and Poly's frame, and applies the records to
  the scene through `win32::WindowsSurfaces` (`win32::Link`). Input,
  focus, the frame's buttons and interactive moves are later slices.
- **The boot test.** Boot group `desktop-win32`:
  `tests/windows/win_gui_fill.rs`, typed into the launcher, fills its
  window's client area through `GetDC` / `FillRect` / `ReleaseDC`; the
  window must be on the screen at its client rectangle, framed, in the
  colour.

## To be confirmed by the spec agents against real Windows

1. The system classes' fields besides "Button" (ours: "Static" style
   0x88, "Edit" 0x88 with 6 extra bytes), and `GetSysColor`'s values (ours:
   Windows 10's defaults as Learn names the indexes).
2. The stock grey brushes (ours: 0xC0C0C0, 0x808080, 0x404040) and the
   default palette beyond entry 2 (ours: the 20 system colours).
3. `WS_EX_STATICEDGE` with `WS_EX_CLIENTEDGE` on other frames than a
   dialog frame (ours: additive except that one observed case).
4. Thread timer ids (ours: from 0x7FEF up); `DispatchMessageW` returns
   22 after a timer callback, as observed, without a documented reason.
5. Minimize, maximize and restore in full (ours: the sheet's rectangles,
   simplified flags); `SendNotifyMessageW` to another thread (ours: posted).
6. `"The quick brown fox"`'s exact glyph pixels: only the extent and the
   recorded rows are reproducible from the sheets.

## Consequences

- 61 of the 62 visible tests pass on Poly; `to_glyphs` differs only in
  the hash and ink count of a string whose glyphs no sheet gives.
- New crate `poly-gdi`; `poly-win32k` gains `wire`, `proto`, `metrics`,
  `wm`, `user`; the NT personality's `win32k.rs` is the service and the
  event plumbing; `user32.dll`, `gdi32.dll` are real, painting through
  update regions stays stubbed (`grep -rn not_implemented`).
- The 45 handed-over entries are claimed by `impl-win-gui-a`.
- The desktop link works for drawing: `poly_win32k::surfaces`, the
  `/dev/win32k` device, `PolyUserCommitSurface`, the compositor's
  `win32::Link`, boot group `desktop-win32`.
