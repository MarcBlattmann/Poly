# 0176: One model for the z order of top-level Windows windows and who takes the activation

**Status:** Accepted. Builds on [ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
(win32k without the machine), [ADR 0131](0131-windows-gui-slice-a-window-manager-messages-and-gdi.md)
(the window manager) and [ADR 0136](0136-windows-gui-slice-b-sessions-painting-focus-and-input.md)
(the interactive session, activation and focus) and [ADR 0172](0172-windows-gui-slice-c-clipboard-dpi-monitors-layered-windows-icons-dwm-and-ime.md)
(the default IME window of `imm32`). It replaces sections 3 and 7 of
ADR 0136 where they differ.

## Context

Three sheets describe one thing from three sides: `GetActiveWindow.yaml`,
`GetFocus.yaml` and `GetForegroundWindow.yaml`. After two implementer
rounds each still failed one held-out test, and so did five entries next
to them. The spec agent's third round (PR #190) found why the earlier
rules could not be right: they described who takes the activation by the
z order, and what a call reports about the z order by the test's own
windows. On Windows

- the window that takes the activation when the active window is hidden,
  destroyed or minimized is chosen by the thread's **activation history**
  (the window that was active before), whatever the z order says; only a
  minimize falls back on the z order, and then on the whole desktop's, not
  the thread's;
- a `WM_WINDOWPOSCHANGED` says "moved" (no `SWP_NOZORDER`) by the order of
  **all** top-level windows: the `WS_EX_TOPMOST` band, the thread's hidden
  default IME window, and the windows of other processes (on the oracle's
  desktop the runner's topmost "keeper" and a console window);
- whether a new active window also takes the foreground depends on a
  **right** the thread keeps or loses.

Until now `user::focus` picked the successor with its own rules, win32k
kept a list of the process's windows with one stand-in for other
processes, and a flag per window (`below_others`) said that "something of
another process" lay above it. Each visible test had added a case.

## Decisions

### 1. The rules are one pure module, `poly_win32k::zorder`

`zorder` works on a snapshot of the desktop's top-level windows from the
top down (`Top`: handle, input queue, owner, and the bits topmost, visible,
disabled, minimized, no-activate, tool window, helper) and changes nothing
itself. `wm` builds the snapshot, asks, and applies the order it gets back.
Everything in it is tested on the host:

- `raise`: to the top of the window's band, the windows it owns (and what
  those own) above it;
- `sink`: to the bottom of the band (a minimize, `HWND_BOTTOM`), a helper
  the window owns directly above it;
- `new_slot`: where a new window goes;
- `helper_to_top`, `helper_above_owner`: the default IME window;
- `successor`: who takes the activation.

A call "moved" the window exactly when the order it produces differs from
the one before; `Request::ZOrder` answers that, as before.

### 2. What is in the list

- **Bands.** The `WS_EX_TOPMOST` windows lead the list; a window moves
  inside its band only.
- **The default IME window** of each thread ("helper"). `user32` makes it
  with the thread's first window, after that window's creation messages
  and before it is shown (`Env::window_created`), in every process of the
  interactive session (before: only with `imm32.dll` loaded, and after the
  first window was shown; session 0 keeps that). win32k knows it by its
  class and tells `imm32` which window it is (`Request::DefaultIme`). It
  is made directly below its owner, the thread's active window owns it
  (`GetWindow(GW_OWNER)` follows the activation), it is in its owner's
  band, and a change of the focus in the thread puts it on top of that
  band. Nothing else is special about it: because a raised window takes
  the windows it owns along above it, the first activation of a thread's
  first window is a move (the helper was below it), the active window never
  passes its own helper, and a window that lies below another window's
  helper has moved when it comes to the top. That is the whole of
  `GetFocus.yaml` `ime_window`.
- **Other processes' windows** (interactive oracle session only): the
  keeper K (topmost, in the foreground at the start), a hidden window of
  the keeper's process that is raised with K, and a visible console window
  that is not topmost. When one of them takes the foreground it comes to
  the top of its band, so our windows that are not topmost lie below
  something and raising one is a move. This replaces `below_others` for
  the interactive session (the flag stays for session 0, where a process's
  only window has nothing to be below).

### 3. Who takes the activation: `zorder::successor`

`user32` asks win32k (`Request::NextActive`) when a top-level window is
hidden, destroyed or minimized, and runs the message sequence for the
answer (`user::focus::hand_on`):

1. An owned window hands the activation to its owner.
2. The window that was active before the thread's active window (the
   queue's history, most recent first, windows destroyed since skipped), if
   it can take it: visible, enabled, not minimized, not `WS_EX_NOACTIVATE`.
3. A hide or a destroy otherwise: if the thread had an earlier active
   window that cannot take it, the top window of the thread that can; if
   the active window was the thread's first, nobody: windows that were
   never active are left alone, and the foreground returns to the window
   of another process the thread took it from (win32k remembers it).
4. A minimize otherwise: the top window of the desktop that is not topmost
   and is visible, enabled, not minimized, not `WS_EX_NOACTIVATE` and not a
   tool window, of any process. The thread's own active window or the
   foreground window: nothing changes. A window of the thread: it takes
   the activation. Another process's: the thread's windows are deactivated
   and that window gets the foreground.

A minimize of a window that is *not* the active one runs the same rules
with the thread's active window as the one that might give way.

### 4. The right to take the foreground

The right is a thread's (an input queue's). A process that never had the
foreground may take it (the foreground process started it); after that a
thread has the right once it has had the foreground (`fg_seen`,
`fg_right`), so a new thread of such a process has not (gfw_xthread: its
window becomes active in its thread only). The right is lost when the
thread hands the foreground to another process's window: by a minimize
(rule 4) or by its own `SetForegroundWindow`. It comes back when a window
of the thread takes the foreground (`SetForegroundWindow` on an own
window, a click). A hide or a destroy that lets the foreground fall back
does not lose it. Without the right an activation (a visible creation, a
show, `SetActiveWindow`, `SetFocus`) makes the window active in its thread
only: `WM_ACTIVATEAPP(1)` if the thread had no active window,
`WM_NCACTIVATE` with wParam 0, `WM_ACTIVATE`, the focus messages.

A thread without the right also cannot put a window that is not topmost
above the foreground process's windows (Microsoft Learn, SetWindowPos: "To
use SetWindowPos to bring a window to the top, the process that owns the
window must have SetForegroundWindow permission"): its top of that band is
below the *barrier*, the foreground process's upper window there. That is
why `SetForegroundWindow` on the window that already is the thread's
active one reports a move (gfw_right, scenario 3).

### 5. Session 0 has the same messages

`ShowWindow.yaml` `activation_ask` and sw_activation show that session 0
sends the activation messages too, as a thread in the background does
(`WM_NCACTIVATE` with wParam 0), with the differences the sheet lists: the
ask is `WM_WINDOWPOSCHANGING(0x13)` (0x3 for `SetActiveWindow`), the first
active window is not asked, a hidden active window stays active and only
loses the focus, and a destroy with nobody to take over sends nothing.
`focus::activate_local` does all of it; `IsWindowVisible` is 0 for every
window there.

### 6. The visible tests are replayed on the host

The scripted visible tests of the four sheets (gaw_successor,
gaw_never_active, gaw_min_inactive, gf_zorder, gf_background, gfw_right,
gfw_minimize, sw_activation), sw_minmax_styles and eight of the earlier
tests (gaw_activate_a to _c, gaw_leave_a and _b, gaw_zhide, gf_show_popup,
gf_show_overlapped) run against a `Wm` in memory in
`user_tests/replay.rs`: the scripts are read from the test programs in
`specs/windows/`, the runner follows theirs, and the output is compared
line by line with the entries' `windows.json`. A change to the model shows
in seconds what it does to 2800 lines Windows printed, without a boot.

## Consequences

- One place to change when a sheet sharpens a rule, and unit tests named
  after the rules. `user::focus` keeps the message sequences only.
- The interactive session has two more stand-in windows and every thread
  a hidden IME window. `EnumThreadWindows`, `GetWindow` walks and
  `WM_ACTIVATEAPP` see the IME window, as on Windows. Compositor surfaces
  are not affected (hidden windows have none).
- Four things are guesses that fit every observation but that no sheet
  states; they are listed as questions for the spec agent in the
  hand-back: the barrier of rule 4 (only one observation needs it), the
  helper going down with a minimized owner, a minimized window that is
  restored while its thread has no active window taking the foreground
  back (`GetFocus.yaml` `restore` against `foreground_right`), and the
  line height of the caption font at small DPIs (`GetSystemMetrics.yaml`).
- Not modelled: the second hidden window the oracle's IME window has
  above it (it never changes an answer), `SetWindowPos(HWND_BOTTOM)`
  taking a topmost window's style away, owned windows being hidden with a
  minimized owner.
