# 0234: Chrome QA fixes: a window made maximized waits for its show

**Status:** Accepted. Track `chrome-qa-fixes`. Builds on ADR 0172 (win32k's window model and
the compositor link), ADR 0176 (activation and z order) and ADR 0216 (Chrome for Windows from the
menu). Inputs: Microsoft Learn (Window Styles: `WS_MAXIMIZE` "the window is initially maximized";
`WM_SYSCOMMAND`; `ShowWindow`), the spec sheets `ShowWindow.yaml`, `SC_MAXIMIZE.yaml`,
`IsZoomed.yaml` and `GetWindowPlacement.yaml`, and Poly's own traces of Chrome running on Poly
(`poly.nt.trail`, `poly.nt.trace` with `poly.nt.only`, `poly.nt.exceptions`). No third-party source.

## What was wrong

A QA walk of Chrome for Windows (handoff `chrome-qa.md`, bug 1) found that Chrome never started
again after it had been closed while maximized, also after a reboot: every start ended after
5 to 19 s with `exited with status 0`, and no window appeared.

The traces of a failed start show this:

1. Chrome's saved placement says *maximized*, so Chrome makes its browser window **hidden with
   `WS_MAXIMIZE`**. win32k marked the window maximized but left it at the rectangle asked for.
2. While Chrome is still building that window (one of its child windows exists, the others do
   not yet), it sends itself `WM_SYSCOMMAND` `SC_MAXIMIZE`.
3. DefWindowProc turned `SC_MAXIMIZE` into `ShowWindow(SW_SHOWMAXIMIZED)`. For a hidden window
   that is maximized already, that **shows and activates it**: `WM_ACTIVATEAPP`,
   `WM_NCACTIVATE`, `WM_ACTIVATE` reached Chrome in the middle of step 2.
4. Chrome's UI thread faulted in its `WM_ACTIVATE` handling (an access violation reading
   address `0xa0`). Its crash handling ended the process with status 0, which is why the
   launcher only saw "exited, no window".

A start that is not maximized never sends `SC_MAXIMIZE`. Chrome creates all its child windows
and popups first and then shows the window with `ShowWindow`. Chrome restores maximized windows
on Windows every day, so on Windows the command in step 2 cannot activate the window. No spec
sheet records `SC_MAXIMIZE` for a hidden window (the sheets use visible windows). The rule below
is the one that makes Chrome's observed sequence work. It agrees with ShowWindow's `same_state`
rule: a command for the state a window is already in changes nothing.

## Decisions

1. **`SC_MAXIMIZE` of a window that is maximized already** (`WS_MAXIMIZE`, not minimized)
   **and hidden does nothing.** It sends no message, does not show the window and does not
   activate it. The window keeps waiting for its show. `SC_MAXIMIZE` of any other window (a
   hidden one that is not maximized, or a visible one) is unchanged: it acts as
   `ShowWindow(SW_SHOWMAXIMIZED)`.
2. **A top-level window made hidden with `WS_MAXIMIZE` is made at its maximized rectangle.**
   That rectangle comes from the `WM_GETMINMAXINFO` answer at creation, fitted to the work area as
   `maximize` does. The rectangle asked for becomes the window's restore rectangle
   (`GetWindowPlacement`'s normal position). Before, the window kept the asked-for rectangle
   while it said it was maximized. When it was shown later ("a hidden maximized window shown
   again" does not move it), it was a maximized window at its restored size. Visible windows made
   with `WS_MAXIMIZE`, children and minimized ones are unchanged.
3. **A boot check**, `cargo xtask boot-tests chrome-try-max` (shard `chrome-win-window`). It uses
   the menu's Chrome on a disk written afresh for each run, so the profile is empty and the menu's
   profile is untouched. The steps:
   - start Chrome on a green page;
   - maximize it by a double click on its caption (the page then reaches x = 1200);
   - close it with Ctrl+Shift+W;
   - start it again from the launcher, wait for the second `launch-to-window`;
   - check that the page is there, maximized.

   The check has no `--window-size`, because that switch wins over the saved maximized state. The
   desktop runs gain a `waitn:N|TEXT` step: wait until the log has TEXT N times.
   With win32k as it was the check fails: the second window never comes.

## Consequences

- Chrome closed maximized starts again maximized (1.8 s to its window on a warm disk at 200 %),
  and its restore button returns it to the saved normal rectangle.
- Unit test: `a_window_made_hidden_and_maximized_waits_for_its_show_at_the_maximized_size`.
- Open: the maximized window leaves a 5-pixel strip at the right edge at 200 %, as before this
  change. Chrome's `WM_NCCALCSIZE` answer for a maximized window is not the whole hidden frame
  there. That is not addressed here.
