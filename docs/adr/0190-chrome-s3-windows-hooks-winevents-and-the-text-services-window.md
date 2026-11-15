# 0190: Chrome S3: windows hooks and WinEvents in `user32`, the text services window, a taskbar at the bottom

**Status:** Accepted. Implements the spec round `spec-chrome-win-s3` (48 entries, `specs/windows/apps/chrome/s3-windows.md`)
for [0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)'s stage S3 (a visible window,
`--disable-gpu`), on [0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md),
[0172](0172-windows-gui-slice-c-clipboard-dpi-monitors-layered-windows-icons-dwm-and-ime.md) and
[0176](0176-one-model-for-the-z-order-of-top-level-windows-and-the-activation-hand-over.md).

## Decisions

1. **Hooks live in `user32`, in the process.** `SetWindowsHookExW`, `UnhookWindowsHookEx` and `CallNextHookEx` keep
   one table per process (`poly_win32k::hooks`, host-tested); the procedures run on the thread whose message they
   watch. `WH_CALLWNDPROC` runs before a window procedure that a message is *sent* to (the `Env::sent_hook` of
   `send`, also for a message another thread sent), `WH_GETMESSAGE` after `GetMessage`/`PeekMessage` took one
   (`Env::message_hook`, which may change the `MSG`). Hooks of other processes are not injected: a hook for every
   thread reaches this process's threads only. Nothing goes through the kernel.
2. **WinEvent hooks are out-of-context only** (`poly_win32k::winevent`). An event is queued, newest hook first, for every
   hook whose range, process, thread and skip flags match; the installing thread runs the callbacks when it looks
   into its queue (`Env::pump_events` in `PeekMessage`/`GetMessage`). Removal drops what is queued for the hook, only
   the installing thread removes it. The window functions raise `OBJECT_CREATE/DESTROY/SHOW/HIDE/FOCUS/REORDER/
   LOCATIONCHANGE/PARENTCHANGE/STATECHANGE/NAMECHANGE` (`Env::win_event`).
3. **The text services window.** The first activation of a thread makes `MSCTFIME UI`, a hidden popup owned by the
   thread's default IME window. `DefWindowProc` hands `WM_IME_SETCONTEXT`/`WM_IME_NOTIFY` on to the default IME
   window, which hands them to its UI window (`forward_ime`). The UI window lies directly above its owner and is
   outside the z-order rules of the other windows (`Wm::tops` leaves it out, `with_ui` puts it back); it goes with the
   thread's last window. The IME windows are asked first for `WM_WINDOWPOSCHANGING` when their owner hides, minimizes
   or restores.
4. **`WINDOWPOS.hwndInsertAfter` names the window the target position follows** (`Request::ZReport`): `HWND_TOP` of a
   window that is not topmost comes after the topmost ones (the oracle's keeper and taskbar), `HWND_BOTTOM` after the
   last window.
5. **A second interactive session: `poly.win32k.session=interactive-bottom`**: the same 1920x1080 desktop with the
   oracle's taskbar of build 26200.9974 (48 px at the bottom, work area 0,0,1920,1032), where `interactive` keeps the
   older 24 px bar at the top. `win build` takes the session of a test from its own `session` in `windows.json`, else
   from the file's `system.session`, and turns `interactive` into `interactive-bottom` for a reference recorded at build
   26200.9974 or later. (The older green entries keep passing under the old layout.)
6. **Smaller rules** the sheets fixed: carets per thread (`GetGUIThreadInfo` shows them), `SetMessageExtraInfo`,
   `CloseWindow`, the global atom table (`poly_kernelbase::atoms`, in `kernel32`; a property named by an atom and by its
   string is one property), display DCs for `CreateDCW`/`CreateICW`, the last deleted region still answering once
   (a ghost until the next region is made), `WS_EX_NOREDIRECTIONBITMAP` windows erased once and not again by
   `InvalidateRect`, no `WM_NCCALCSIZE` from `SetWindowLong(GWL_STYLE)` on such windows.

## Not done

Painting order of children while a window is resized (`swp_chrome_resize`), the oracle's IME windows sinking below
the thread's windows a moment after a focus change (`dw_chrome_close`, `ew_chrome`, `etw_basic`), the extra system
events of a show (`swe_delivery` counts), `GetObject` of stock fonts and palettes (`gso_objects`).
