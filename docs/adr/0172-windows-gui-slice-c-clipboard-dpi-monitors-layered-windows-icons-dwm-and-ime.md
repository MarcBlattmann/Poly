# 0172: The third Windows GUI slice: the clipboard, DPI awareness and monitors, layered windows and window regions, icons and cursors, `dwmapi`, `imm32` with no IME, `shcore`, and what a visible Chrome window calls

**Status:** Accepted. Implements the third slice handed over by `spec-win-gui-c`
(68 entries: 47 `user32`, 6 `dwmapi`, 14 `imm32`, 1 `shcore`) inside the architecture of
[ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md),
[ADR 0131](0131-windows-gui-slice-a-window-manager-messages-and-gdi.md) and
[ADR 0136](0136-windows-gui-slice-b-sessions-painting-focus-and-input.md),
plus the S3 delta of the Chrome gap map (`specs/windows/apps/chrome/gap-map.md`
sections 7, 8 and batch 7 of section 9). Its goal is stage S3 of the Chrome ladder: a
visible `Chrome_WidgetWin_1` window that renders and takes input.

## Context

Slice A and B gave windows, messages, GDI, painting, focus and input on one 96 DPI
monitor. What Chrome's window code calls next is a third family: the clipboard, the
DPI awareness contexts and per-monitor queries, layered windows and window regions,
icons and cursors, the desktop window manager's attribute calls, input contexts with no
IME, and a handful of small user32 queries. The references were recorded on the
interactive session of the slice B oracle (one 1920x1080 monitor at 96 DPI), so the
slice reproduces that session (ADR 0136, Decision 1).

## Decisions

### 1. The clipboard is a model in `poly_win32k::clip`; bytes live in the kernel

One `Clipboard` per session (in `Wm`): the open state (a thread, a window or NULL;
several threads may hold an open-for-NULL clipboard), the owner, the formats in the
order of their first `SetClipboardData`, the sequence number, the format listeners and
the registered format names (case-insensitive for ASCII, first spelling kept, ids from
`0xC000`). `SetClipboardData` copies the caller's global block into the kernel (a
`CF_BITMAP` is stored as the packed DIB `user32` made of the bitmap), so another
process can read it and the data outlives its owner. The setter's own handle comes back
unchanged from `GetClipboardData`; a reader in another process, and every synthesized
format, gets a new block. Those blocks are freed when the sequence number moves.

`CloseClipboard` synthesizes (`CF_LOCALE` and the missing text formats after text, in the
order the sheet observed; `CF_DIB`, `CF_DIBV5` and `CF_BITMAP` after an image), one
sequence number per format; the contents are made when read
(`clip::convert_text`: code page 1252 / 850, the terminating NUL; `dib::to_v5`,
`dib::from_v5`, `dib::to_bgra`). `WM_CLIPBOARDUPDATE` is posted to each listener, newest
registration first, when a close changed anything. Delayed rendering is a loop in
`user32`: the kernel answers a read of a promised format with "render this, owner", the
DLL sends `WM_RENDERFORMAT`, the owner's `SetClipboardData` is not a change, and the
read is asked again once. `WM_DESTROYCLIPBOARD` goes to the old owner inside
`EmptyClipboard`; an owner window that is destroyed gets `WM_RENDERALLFORMATS` first
(`user::send_destroy`), then its delayed formats go.

Poly has no desktop clipboard yet to bridge to; the model is the internal one. A bridge
would read and write the same format list at `CloseClipboard` and on a change of the
compositor's selection.

### 2. DPI awareness is kernel state; the monitor is one at 96 DPI

`SetProcessDpiAwarenessContext` works once per process (`Wm::proc_dpi`; later calls and
the invalid values after the first one give `ERROR_ACCESS_DENIED`), a thread's own
context beats the process's, a window keeps the awareness it was created under (a child
takes its parent's) but follows the process call, and `SetParent` across awareness kinds
fails with `ERROR_INVALID_STATE`. Contexts are the pseudo handles `-1..-5` and the real
values `0x6010`, `0x6011`, `0x12`, `0x22`, `0x40006010` (bit 31 ignored)
(`poly_win32k::guic`). `GetSystemMetricsForDpi`, `AdjustWindowRectExForDpi`,
`SystemParametersInfoForDpi` use `metrics::*_for_dpi` (the sheet's tables at 96..384 DPI
and the formulas that fit the odd DPIs: caption = round(22 dpi / 96) + 1, frame =
3 + round(dpi / 96), padded border = round(4 dpi / 96) up to 100, ...).
`EnableNonClientDpiScaling` is true during the creation of a top-level window of a
per-monitor-aware thread; in a per-monitor (v1) process it recomputes the frame once
through `SetWindowPos(SWP_FRAMECHANGED ...)`, which is the message cascade the sheet
saw. `shcore.dll` has `GetDpiForMonitor` (96 for the effective and angular types,
`0x80070032` for the raw one) and the legacy process calls.

`EnumDisplayMonitors`, `EnumDisplaySettingsW`, `EnumDisplayDevicesW` and
`GetMonitorInfoW` describe the session's screen; `EnumDisplayMonitors` with a DC cuts the
callback rectangle by the DC's visible region and gives the callback the same DC with its
clip set to it. `QueryDisplayConfig` and its two companions fail with `ERROR_NOT_SUPPORTED`
(Chrome tolerates it).

### 3. Layered windows and regions

The attributes of `SetLayeredWindowAttributes` live in the window (`guic::Layered`); the
rules of `UpdateLayeredWindow` (argument checks, ordering with `SetLayeredWindowAttributes`)
are in `user32`, and its content is blitted into the window's surface. `SetWindowRgn` keeps
the region's rectangles in `user32` (the caller's handle is deleted, the system owning it),
sends the frame messages with flags `0x1001837`, and `GetWindowRgn`/`GetWindowRgnBox` answer
from them.

What Windows itself shows of such windows is win32k's (rework after the held-out run):

- `UpdateLayeredWindow` also hands win32k the source bitmap's pixels with their alpha
  (`Request::LayeredContent`, kept per window in `Surfaces`, at most 1 MiB) and how to show
  them (`Request::Layered` op 3: the `ULW_*` flags, the colour key, the blend's constant
  alpha and `AC_SRC_ALPHA`). `guic::Layered::ulw_pixel` composes a pixel over what is below
  by the sheet's rules (premultiplied per-pixel alpha, constant alpha, colour key,
  `ULW_OPAQUE`); a pixel without alpha passes hits through. The window's surface cannot
  hold this: it is `B, G, R, x`, and a content must outlive hiding and resizing.
- `SetWindowRgn` also tells win32k the region (`Request::WindowRgn`, a `rgn::Rgn` in the
  window): `WindowFromPoint` (so mouse input too) and the screen's pixels leave out what is
  outside it.
- Both are what `GetPixel` on the screen DC and `WindowFromPoint` answer (win32k's own
  composition, `Surfaces::screen_pixel` and `hit_passes`). Poly's compositor still shows
  such windows opaque and unclipped: the desktop link carries neither alpha nor a shape.
- A resized window keeps what it showed from its top-left corner (`gdi32` copies the old
  surface DIB into the new one, and a window that only shrank commits it), as only the area
  it gained is invalid.
- `gdi32` gains `CreateRoundRectRgn` (`poly_gdi::region::Region::round_rect`), which the
  region sheets use.

### 4. Icons and cursors are `user32` objects made of pixels

An icon or cursor is an AND mask and BGRA colour (or a mask of twice the height),
a hotspot and a lifetime (`user32::icons`); `GetIconInfo` makes fresh GDI bitmaps of it
each call. System cursors keep slice B's handles, the seven system icons are shared
handles with Poly's own artwork (`icofile::system_icon`: a plain shape per icon that covers
exactly as many of the 32 x 32 pixels as the oracle's icon draws, which is all the sheet
tells about their look). `LoadImageW` reads `.ico`, `.cur` and `.bmp` files
(`poly_win32k::icofile`), picks and scales the closest image (hotspots scale with it).
`DrawIconEx` draws pixel by pixel with `GetPixel`/`SetPixelV`.

### 5. `dwmapi.dll`, `imm32.dll`, `shcore.dll`

New real PE DLLs in the Win32 workspace, registered like `gdi32` (`tools/xtask/src/win32.rs`
output list, cache key, export check; the kernel's `build.rs`, `nt/mod.rs`, `nt/layout.rs`).
`dwmapi` keeps the attribute values as window properties (`SetPropW`, a new kernel
feature that Chrome needs anyway); `DwmGetCompositionTimingInfo` always fails with
`0x88980090`. `imm32` maps the `Imm*` calls onto `poly_win32k::ime` (one default context
per thread, own contexts, the window association stored in the window); the activation
sequence of `user::focus` sends `WM_IME_SETCONTEXT` and `WM_IME_NOTIFY` as the sheet
saw; a process that has `imm32` loaded gets each thread's hidden "IME" window with its
first window, which outlives that window and goes with the thread's last one.

### 6. The S3 delta

`RegisterWindowMessageW`, `SetPropW`/`GetPropW`/`RemovePropW`, `EnumThreadWindows`,
`GetWindowPlacement`, `IsWindowArranged`, `GetWindowDisplayAffinity`, `WindowFromDC`,
the keyboard layout list (three installed layouts), `GetLastInputInfo`, `SetWinEventHook`
and friends (accepted, nothing raised), the device and power notification registrations,
`GetCaretBlinkTime`, `GetPointerDevices`. These have no sheets: they are written from the
Learn pages and the plainest answers the gap map shows Chrome accepts.

## Consequences

- New modules in `poly-win32k`: `clip`, `dib`, `guic`, `icofile`, `ime`; `metrics` grows
  the DPI tables; `Request` tags 100..106 and a few `WinInfo` fields (`dpi`, `creating`, `ime`).
- New DLLs `shcore.dll`, `dwmapi.dll`, `imm32.dll`; `user32.dll` modules `clipboard`,
  `display`, `icons`, `layered`, `misc`, `s3`.
- The rework adds `Request` tags 109 (`WindowRgn`) and 110 (`LayeredContent`) and op 3 of
  `Layered`.
- Not done here: a compositor cursor bridge for created cursors, shaped and translucent
  windows in Poly's compositor (win32k's own screen and hit test have them), the clipboard
  bridge to a desktop clipboard, display-config adapters, WinEvent delivery.
