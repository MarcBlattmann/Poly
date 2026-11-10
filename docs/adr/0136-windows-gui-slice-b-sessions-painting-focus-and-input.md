# 0136: The second Windows GUI slice: two oracle sessions, update regions and WM_PAINT, activation and focus, raw input, the keyboard layout, GDI regions

**Status:** Accepted. Implements the second slice handed over by
`spec-win-gui-b` (47 entries: painting, focus and activation, capture,
mouse, cursor and coordinates, keyboard, metrics, monitors and DPI, gdi32
regions and clipping) inside the architecture of
[ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
and [ADR 0131](0131-windows-gui-slice-a-window-manager-messages-and-gdi.md).

## Context

Slice A's references were recorded in the oracle's SSH session (session 0:
1024x768, no visible desktop, nothing painted, no activation, no input).
Slice B's were recorded with `poly-oracle.ps1 -Interactive` in the logged-on
user's desktop session: one 1920x1080 monitor at 96 DPI, the taskbar 24
pixels high at the top, the runner's "keeper" window of another process in
the foreground before each test, the cursor parked at (1500, 600), no mouse
attached, a German keyboard layout under English (Switzerland) (HKL
0x04072000). A `windows.json` says which: `system.session` is `interactive`
for the second (docs/AGENT_PIPELINE.md). Both slices' references have to
keep passing.

## Decisions

### 1. win32k reproduces one of the two sessions per boot

`poly_win32k::metrics::Display` is the session's screen: `SERVICE`
(session 0, the default) or `INTERACTIVE`. The kernel argument
`poly.win32k.session=interactive` (`kernel/src/nt/mod.rs`) selects the
second; a compositor greeting win32k through the desktop link makes the
session interactive too, its screen the output and the work area below
Poly's 24-pixel ridge. Everything slice B observed that slice A's session
does not do (painting, activation on show, surfaces of window DCs without
a compositor, the keeper) happens only in an interactive session.
`pipeline win build` records each test's session from the `windows.json`
beside its source (`Built.session` in the manifest) and `win poly` boots
such tests with that kernel argument; held-out tests are built from
directories that hold their `windows.json` the same way.

GetSystemMetrics answers every SM_* index of the sheet from the session's
screen and work area (`metrics::system_metric`, `system_metric_for_dpi`);
the processes are DPI-unaware (96 everywhere); one monitor
(`MonitorFromPoint/Rect/Window`, `GetMonitorInfoW`, the same handle for
every point and thread).

### 2. Update regions in win32k, the paint sequences in `user32`

Each window has an update region (`poly_win32k::rgn::Rgn`, y-x banded
rectangles, the order Windows prints them), a pending erase, a pending
frame (whole or a part: WM_NCPAINT gets 1 or a region), an internal-paint
flag and the fErase it reports when no erase is pending (the last erase's
outcome). `wm::paint` implements RedrawWindow's region work (InvalidateRect,
InvalidateRgn, ValidateRect, ValidateRgn map onto it): clipped to the
client area; children of a parent without WS_CLIPCHILDREN take their part,
with frame and erase pending; hidden windows keep nothing; WM_PAINT comes
after posted and input messages and before timers, later-created top-level
windows first, a parent before its children, and stays while the window is
invalid. `user::paint` sends WM_NCPAINT and WM_ERASEBKGND (BeginPaint,
GetUpdateRect/Rgn with erase, RDW_ERASENOW), validates (BeginPaint), and
does what showing, moving, resizing and hiding windows repaint as the sheets
saw it (a shown top-level window erases at once, a child's parent erases
the area under it at once, growing windows erase the new strip).

### 3. Activation and focus

win32k keeps each input queue's active, focus and capture window and the
foreground window (`wm::input`; AttachThreadInput shares a queue's
state); `user::focus` runs the sequences: deactivation of the old window,
the new one brought to the top with its owned windows and its owner along,
WM_ACTIVATEAPP when the thread had no active window, WM_NCACTIVATE,
WM_ACTIVATE, WM_KILLFOCUS / WM_SETFOCUS. Where the activation leaves the
process's windows it goes to the keeper, a 16x16 popup of no process of
ours in the screen's bottom-right corner, which is the foreground window
at the start of an interactive session.

### 4. Raw input, the key state and the keyboard layout

SendInput's events (and, later, the desktop link's) go through `wm::raw`:
the physical key state (GetAsyncKeyState), a keyboard event to the
foreground thread's focus window (its active window as a system key when no
window has the focus), a mouse event to the capture or the window under the
cursor; input messages wait in the thread's queue, coalesced as the sheets
saw (key repeats, moves, wheel turns), and the thread's key state follows
them as they are retrieved. A retrieved mouse message first sends
WM_NCHITTEST, WM_MOUSEACTIVATE (button-down into an inactive window) and
WM_SETCURSOR (`user::input`). The German layout is Poly's own data
(`poly_win32k::kbd`): MapVirtualKey, ToUnicode with dead keys, and the
characters TranslateMessage posts.

### 5. GDI regions and clipping

`poly_gdi::region` is GDI's region algebra (RGN_* modes, canonical bands,
GetRegionData's order); a DC's clip is a region, with viewport and window
origins, a visible region for window DCs (their client or window area,
less clipped children and siblings) and a system clip BeginPaint sets to
the update region (seen by GetClipBox, not GetClipRgn). Private gdi32
exports move rectangle lists between `user32` and region objects.

### 6. Rules that finishing the visible tests settled

- **Topmost band.** The desktop's children keep the `WS_EX_TOPMOST` windows
  in front of the others: a new non-topmost top-level window, and
  `HWND_TOP`, go below the topmost band (WindowFromPoint.yaml `toplevel`).
  `WindowFromPoint` asks the candidate thread's window with
  `WM_NCHITTEST` (not for a framed window's caption, not for a disabled
  top-level window) and passes HTTRANSPARENT to the window below it
  (`Request::WindowFromPoint::below`).
- **Minimizing in the interactive session.** A window with a minimize box
  goes to the bottom-left corner like session 0's (`0, height - 28`); one
  without (a caption-only popup) is parked at -32000,-32000. A hidden
  window shown minimized keeps `SWP_SHOWWINDOW` and counts as a z-order
  change in its `WM_WINDOWPOSCHANGED`.
- **Attached input.** A thread that owns a visible window and attaches its
  input to the older, foreground thread replaces that thread's active and
  focus windows with its own (none): the old active window is deactivated.
  The older thread attaching to a newer one keeps the newer one's state
  (GetCapture.yaml `attached`, GetFocus.yaml `gf_threads`).
- **Hiding the active owned window** with `ShowWindow(SW_HIDE)` asks the
  owner (`WM_WINDOWPOSCHANGING` 0x13) inside the call and is not
  `SWP_NOACTIVATE`; the window `DestroyWindow` hides counts as a z-order
  change of its owner's activation; `SetActiveWindow` on an owned window
  reports one even at the top. `WM_ACTIVATEAPP` goes to the thread's
  windows top first.
- **Mouse.** Releasing the capture queues a move for the window under the
  cursor; a new capture before it is retrieved takes it back. A click that
  activates a window is followed by a move (after the queued button-up).
  DefWindowProc tracks the non-client area it saw a `WM_NCMOUSEMOVE` in:
  the move into the client area is followed by `WM_NCMOUSELEAVE`.

### 7. Rules of the sharpened sheets (2026-11-10)

The sheets of twelve entries were sharpened after their held-out tests
failed (BeginPaint, GetUpdateRect, ShowWindow, GetActiveWindow, GetFocus,
GetForegroundWindow, EnableWindow, SetCapture, GetSystemMetrics,
MonitorFromWindow, CreateRectRgn, DeleteObject). What that settled:

- **One rule for the flags of a position change** (`user::swp`).
  `WM_WINDOWPOSCHANGING` carries what was asked. A call without
  `SWP_NOZORDER` asks for a place in the z order (the top, unless said
  otherwise). `WM_WINDOWPOSCHANGED` has the same flags, less
  `SWP_SHOWWINDOW` for a window that was visible, plus `SWP_NOZORDER` when
  the window was already where the call puts it; `WM_NCCALCSIZE`'s flags are
  those with 0x1800. The special cases of the first version are gone.
- **The z order knows other processes.** A top-level window can lie below
  the windows of other processes (`wm::Window::below_others`): minimizing
  the active window puts it at the bottom, the keeper taking the foreground
  puts every window of ours below it, raising a window brings it back, and
  each of these is a move. `SetFocus` on a child of the active window
  leaves that window on top.
- **ShowWindow is one function for both sessions** (`user::show_window`,
  `minimize`, `maximize`, `restore`). Commands that minimize, maximize or
  restore send no `WM_SHOWWINDOW`; the `WM_SIZE` / `WM_MOVE` held back at
  creation follow the first call that sends one. A maximized window is
  fitted to the work area after `WM_WINDOWPOSCHANGING`, only if it has a
  maximize box. A minimized window is parked at -32000,-32000 if it has a
  taskbar button (`metrics::has_taskbar_button`), else in the bottom-left
  slot. A window minimized while maximized is restored to maximized.
- **Session 0 has an active window per thread** and nothing else: a window
  that takes the activation from another is asked to come to the top
  (`WM_WINDOWPOSCHANGING` 0x13), the first one is not; a hidden window stays
  active (`focus::activate_local`).
- **Who takes the activation** when the active window is hidden, destroyed
  or minimized (`focus::successor`): its owner; else the window that was
  active before it (`Request::ActivationHistory`, a history that starts
  anew when the thread loses the activation); else, only if there was such
  a window, the topmost one that can; a minimized window hands it to the
  topmost window from its own z-order band down. Otherwise the activation
  leaves the thread: its capture is cancelled, `WM_ACTIVATEAPP` goes to its
  windows in z order, and the foreground goes to a window of another thread
  of the process that has no active window, else to the other process.
- **The foreground at creation.** A window being created takes the
  foreground from another process only if its thread may
  (`wm::input::may_take_foreground`): the process has it, never had it, or
  the thread destroyed its foreground window. Otherwise it is the active
  window of its thread without the foreground. `ShowWindow` of a hidden
  window and `SetForegroundWindow` take it.
- **Painting what cannot be seen.** `rcPaint` is cut to the part of the
  client area on the screen; a window of which nothing can be seen
  (minimized, off the screen) gets no `WM_ERASEBKGND`. Moving a window
  makes invalid what was off the screen and is on it now. A window's
  consecutive paints see the same `HDC` value (`poly_gdi`: the DC a window
  released last is parked for it).
- **Window surfaces are kept in bands of rows** (`surfaces::BAND_BYTES`):
  a maximized window's client area on a 1920x1080 screen is more than one
  kernel allocation may be.
- **GDI objects.** A deleted object's handle keeps answering
  `GetObjectType` until its slot comes round (`gdi::DEAD_SLOTS`); a brush or
  pen deleted while selected draws in its own colour only if the DC drew
  with it before or saved its state with it, else as the default white
  brush / black pen.

## Consequences

- The interactive session is a kernel argument; the two oracle sessions'
  references both pass on the same kernel.
- New modules: `poly_win32k::{rgn, kbd}`, `wm::{paint, input, raw}`,
  `user::{paint, focus, input}`, `poly_gdi::region`.
- Still to come: hover tracking beyond TME_HOVER,
  double clicks, the full mouse-activation and WM_SETCURSOR chains through
  parents, input from the desktop link.
