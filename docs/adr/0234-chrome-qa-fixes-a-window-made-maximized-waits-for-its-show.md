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

## Addendum: opening a downloaded file

QA bug 7. "Open" in Chrome's download bubble reaches Poly as `ShellExecuteExW` → the desktop's
shell open (ADR 0223) → `Files --open PATH`. Files had no app for text. It ran `vi` in a Terminal
and then showed its own window over it. Chrome's second start for the file also failed, as
described below.

1. **The browser is the viewer of what it can show.** Chrome for Windows' launcher entry lists
   text, HTML, PDF and the common pictures in `MimeType=`, and its `Exec` ends in `%U` (the
   launcher drops it, Files fills in the files). Linux Chromium's entry lists the same types. They
   are not defaults in `mimeapps.list`: an app installed for one of these types is not overruled.
   `.exe` and `.deb` keep going to Poly's installers (ADR 0173, ADR 0230).
2. **A path argument naming a file reaches a Windows program as its DOS path.** This applies to
   a Windows program started from Poly's side (a launcher entry, Files, `xdg-open`): an argument
   that is an absolute path of an existing file or folder becomes its DOS path.
   `/nt/c/Users/poly/Downloads/a.txt` is `C:\Users\poly\Downloads\a.txt`, other paths are on
   `Z:`. A switch such as `/S` names no file and stays as it is (`layout::dos_argument`).
3. **`WM_COPYDATA` sent to another thread carries its bytes.** Learn (`WM_COPYDATA`) says the
   data is copied to the receiving process. `SendMessage` of `WM_COPYDATA` sends `dwData` and
   the bytes (`Request::SendCopy`). The receiver's `user32` gives its procedure a
   `COPYDATASTRUCT` of its own, valid while the procedure runs. Before, the receiver got the
   sender's pointer. Chrome's second start hands its command line to the running Chrome this way,
   so the running Chrome read memory that was not there, exited, and the second start opened a
   new browser ("Restore pages?"). Now the running Chrome opens the file in a new tab, in front.
4. **Files leaves the front to the app that opened the file**, also to the terminal's editor
   for a text that no app claims. It shows its own window only when nothing opened the file.

Open: Files starts the app, so the compositor's launch feedback cannot see the second
`chrome.exe` exit after its hand-over. The busy pointer then waits for its 30 s timeout over the
desktop.

## Addendum: pointer shapes

QA bug 5. The pointer was always the arrow, in Chrome for Windows and in Linux Chromium: no hand
over links, no I-beam over text, no resize arrows at window edges. Two things were missing:

- The compositor drew only its arrow. It showed every `wp_cursor_shape_device_v1` shape as the
  arrow.
- win32k already sent a `Cursor` record for a Windows program's `SetCursor` of a standard cursor
  (ADR 0172), but the compositor dropped it ("cursors come with a later slice").

Decisions:

1. **Named looks of Poly's own** (`compose::NamedCursor`): hand, I-beam, hourglass, crosshair,
   move, not-allowed, and the four resize arrows. Each is a picture of black, white and clear
   cells in logical pixels with a hot spot, drawn like the arrow: each cell is as large as the
   output's scale makes a logical pixel, so it is sharp at 200 %. The hand and the hourglass are
   drawn cell by cell. The arrows, the I-beam, the crosshair and the not-allowed sign come from
   their geometry, so they are symmetric. "Progress" is the existing busy arrow.
2. **Names to pictures.**
   - `wp_cursor_shape_device_v1`'s names map to these pictures (cursor-shape-v1.xml). Names
     without a picture show the arrow.
   - Windows' standard `IDC_*` cursors (winuser.h) map the same way.
   - xdg's resize edges map to the resize arrows.
3. **The server's own look wins over the client's.** It is the resize arrows over a frame's
   border and during a resize, and an internal window's cursor while the pointer is over it.
   The compositor sets a Windows window's cursor from win32k's `Cursor` record
   (`Server::set_internal_cursor`).
4. **user32 / win32k.**
   - DefWindowProc's class cursor (`WM_SETCURSOR`, `HTCLIENT`) is told to the compositor like a
     program's own `SetCursor`.
   - On a window's own sizing border, DefWindowProc sets the sizing cursor of that direction
     (Learn, "About Cursors"; hit codes from winuser.h).
   - user32 tells the compositor a cursor only when the window or the cursor changed, since
     programs set the cursor at every mouse move.
5. **Not Alpine Shell's cursor theme (yet).** Alpine Shell bundles the Bibata cursors, which are
   GPL-3.0. Shipping them as separate data files would be "mere aggregation", as Alpine Shell
   argues, but it is a licence decision for the owner. Until then Poly draws its own pictures.
   A Windows program's own cursor pictures (`CreateIconIndirect`, resources) show the arrow.

Checks: unit tests for the pictures, the names, the drawing at scale 2, the server's choice
(shape requests, frame borders, internal windows, resize grabs), and DefWindowProc's sizing
cursors. Live at 200 %: the hand over a link, the I-beam over text and an input field, and the
left-right arrows at the window's edge, both in Chrome for Windows and in Linux Chromium.
