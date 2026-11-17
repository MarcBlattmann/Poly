# 0219: A window's surface takes only what a process drew

**Status:** Accepted. Track `chrome-paint`. Builds on ADR 0128 (each top-level window one surface
of the compositor, over the desktop link), ADR 0131/0136 (GDI draws in the process, `ReleaseDC`
and `EndPaint` commit), ADR 0211 (the software Direct3D 11 device, whose swap chains present
through GDI) and ADR 0179 (whole-number scales for Windows apps). Inputs: Microsoft Learn
("Desktop Window Manager": each window draws into an off-screen surface that is composed;
`SetDIBitsToDevice`, `BitBlt`, `GetDC`, `ReleaseDC`); Poly's own traces of Chrome for Windows
running on Poly. No third-party source.

## What was there

Every process that draws on a window keeps its own copy of the window's pixels: `gdi32`'s
surface DIB, made on the first `GetDC` of that window in that process. `ReleaseDC` and `EndPaint`
committed the **whole** copy (`PolyUserCommitSurface`), and win32k damaged the whole surface.

Chrome for Windows draws one window from two processes. The GPU process presents its swap chain
into a child window of the browser's top-level window (`SetDIBitsToDevice` on the child's DC,
ADR 0211). The browser process paints the same top-level window itself, for instance on
`WM_PAINT` while a page loads. Measured at the owner's settings (2560x1600, 200 %, WHPX):

- the browser committed its whole copy of the 1034x748 window about three times a second while a
  page loaded and scrolled. Its copy never held the GPU process's frames, so each commit put stale
  pixels over the last frame until the next present: Chrome **flickered**;
- each present cost **21–32 ms** in `gdi32`: `SetDIBitsToDevice` went through the general blit
  (a 9 MB vector of source cells, then a clip test, a raster operation and a store per pixel),
  then the whole surface was committed. Chrome's GPU thread spent most of a frame there and
  scrolled at 23–36 presents a second;
- the software arrow was drawn one physical pixel wider and taller than the rectangle the scene
  damages for it (`draw_cursor`'s `+ 1`). A pixel drawn there by a larger repaint stayed when the
  arrow moved on. Windows apps always get the arrow, since their cursor records are not shown yet;
  Wayland clients mostly set their own cursor image.

## Decisions

1. **A bitmap knows what was drawn on it.** `poly_gdi::Bitmap::drawn` is the bounding rectangle
   of the pixels written since it was last taken. Every pixel store goes through `set_raw` or the
   row copy below, and both add to it.
2. **A commit carries only what this process drew.** `gdi32`'s `PolyGdiTakeSurfaceDrawn(hwnd)`
   takes the union of the drawn rectangles of a window's surface DIB and its views (the rows
   below a menu bar), in surface pixels. `user32`'s commit sends that rectangle with the new
   win32k service **`PolyUserCommitSurfaceArea(hwnd, pixels, width, height, stride, area)`**
   (`0x1002`; the WOW64 signature has a 32-bit `area` pointer). Nothing drawn: no commit. win32k
   writes only those pixels into the window's buffer and damages only that rectangle. What
   another process committed elsewhere in the window stays. `PolyUserCommitSurface` stays in
   the table (numbers are never reused) for the child-window path and older callers.
3. **A resized window keeps its picture in win32k too.** Since commits no longer resend what was
   drawn before, the buffer win32k makes for a new size takes the old buffer's pixels from the
   top-left corner, as each process's DIB already did. New pixels are blank until drawn.
4. **A plain copy is a row copy.** `BitBlt`/`SetDIBitsToDevice` with `SRCCOPY`, no stretching
   and no mirroring, between 32-bit bitmaps, copy each row of each clip rectangle with one
   `memmove` (bottom-up when source and destination share memory and the destination is
   further down). The result is the general path's, pixel for pixel (host tests compare them).
5. **The arrow is drawn exactly inside its damage.** `draw_cursor` covers `Scale::rect` of the
   logical cursor rectangle, which lies inside the `rect_out` the scene damages.
6. **A scale-1 buffer at a whole scale doubles rows by copying.** The compositor's integer
   scaling (a Windows window at 200 %) steps through the buffer row without a division per pixel
   and copies an output row whose buffer row was opaque to the next output row of the same buffer
   row, instead of blending it again.

## Consequences

- Chrome for Windows: the browser process no longer overwrites the GPU process's frames (in the
  traces it commits only its own popups). A present costs **1.6–2.9 ms** instead of 21–32 ms, and
  while scrolling Chrome presents **57–59 frames a second** instead of 23–36.
- The compositor is now the limit while a whole 1034x748 window changes at 200 %: about 37
  frames a second, of which about 8 ms are composing and about 8 ms the KMS page flip with the
  virtio-gpu transfer of the damage to the host.
- Each process still keeps its own copy of a window's pixels. A process that reads the window
  back (`BitBlt` from a window DC, `ScrollDC`) reads its own copy, which may be older than what
  another process committed. A shared surface section mapped in every drawing process, which is
  what ADR 0128 describes and what a zero-copy present needs, is left for later.
- Not done here, and needing spec sheets first (listed in the track's hand-back): DPI
  awareness above 96 DPI (Chrome renders at 100 % and is scaled up, hence blurry at 200 %),
  `IDCompositionDevice`'s other interfaces, and cursor shapes over Windows windows.

## Addendum (2026-11-17): Windows apps in output pixels, so Chrome draws at 200 % itself

The owner: at 2560x1600 and 200 % Chrome for Windows looked pixelated. Sharpness is a must.

**What was there.** Every Windows app lived in logical pixels at 96 DPI: `GetDpiForSystem`,
`GetDpiForWindow`, `GetDpiForMonitor`, `GetDeviceCaps(LOGPIXELSX/Y)` and
`GetScaleFactorForMonitor` answered 96 (100 %). Chrome drew 1034x748 pixels and the compositor
enlarged them 2x by nearest neighbour. ADR 0179 already said Windows apps "are told the next whole
scale and draw at it", but win32k only stored `Hello.scale`. Observed on Poly with a temporary trace:
all of Chrome's processes call `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)` (its
`chrome.exe` manifest declares no awareness). The browser takes its scale from
`GetDpiForMonitor(MDT_EFFECTIVE_DPI)` and `GetDeviceCaps(LOGPIXELSX/Y)`, its UI font from
`SystemParametersInfo(SPI_GETNONCLIENTMETRICS)`, and calls `GetSystemMetricsForDpi` and
`EnableNonClientDpiScaling`. It never calls `GetDpiForWindow`.

**Decisions.**

7. **Units on the desktop link.** A new record `FromCompositor::Units { units }` follows `Hello`.
   It means that Windows apps live in `units` output pixels per logical pixel. The output's size and
   Poly's frame in `Hello` are then given in those pixels already, and so is every position and size
   on the link. Without the record a unit is one logical pixel, as before.
8. **The compositor converts, win32k does not.** With `--windows-hidpi` the compositor uses the
   output's whole scale as the units. It multiplies pointer positions and its own moves and resizes
   (`Configure`) by them, divides `Geometry` and `Margins` by them, and shows a Windows buffer at
   buffer scale `units`. At 200 % that is pixel for pixel on the output, with nothing enlarged.
   win32k keeps one coordinate space, so hit testing, the cursor, capture and z-order stay as they
   were.
9. **The system DPI is 96 times the units.** win32k answers it in `Reply::Desktop.dpi`. `user32`'s
   `GetDpiForSystem`, `GetDpiForWindow`, `GetSystemDpiForProcess` and `GetDpiForMonitor`
   (effective, angular), `shcore`'s `GetDpiForMonitor` and `GetScaleFactorForMonitor` (`dpi * 100 /
   96`), and `gdi32`'s `GetDeviceCaps(LOGPIXELSX/Y)` give it. `GetSystemMetrics` gives the scaling
   metrics at that DPI (`GetSystemMetricsForDpi.yaml`'s table). `SystemParametersInfo`'s
   non-client metrics, icon metrics and icon-title font come from the values of
   `SystemParametersInfoForDpi.yaml` at that DPI. At one unit every answer is the 96-DPI one the
   sheets observed.
10. **Surfaces up to 8192 pixels a side** (`MAX_SIDE`, was 2048). Chrome's default window at 200 %
    is 2068 pixels wide, and without a surface it fell back to its minimum size.
11. **`cargo xtask desktop --chrome-win` turns it on** (`poly.init.arg=--windows-hidpi`). Other
    desktops and the boot tests keep one unit.
12. **Windows that are still enlarged are smoothed.** A Windows surface whose buffer scale is below
    the output's (every one without `--windows-hidpi` at 200 %) is enlarged bilinearly, with a
    fixed-weight path for exactly 2x, instead of in 2x2 blocks. This was the owner's interim.

**Consequences.**

- Chrome for Windows at 200 % draws its UI and pages at 192 DPI: its window is 2068x1496 pixels,
  shown pixel for pixel (`chrome-paint-01-sharpness-before-after.png` in the track's hand-back).
- In this mode **every** Windows app is treated as DPI aware. A DPI-unaware app sees a 2560x1600
  screen at 192 DPI. Unless it scales its own drawing by the DPI, it comes out at half size, but
  sharp. Windows instead virtualises unaware apps: it gives them logical coordinates and 96 DPI and
  enlarges their windows. Doing that per process needs the behaviour observed at a 200 % oracle
  first (the sheets cover 96 DPI only), so it is left for a spec agent and a later track.
- Chrome renders and presents four times the pixels. While a whole window changes, the compositor
  is at about 32 frames a second (compose about 5 ms without scaling, the KMS present about 7 ms).
- `WM_DPICHANGED` is not sent: the scale is fixed for the session.
