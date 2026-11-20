# 0231: Windows DPI awareness: each thread works in its own units, win32k in physical pixels

**Status:** Accepted. Track `impl-win-dpi`. Builds on ADR 0172 (the third GUI slice: DPI
awareness contexts, monitors, one monitor at 96 DPI), ADR 0179 (whole-number scales for Windows
apps) and ADR 0219 (Windows apps in output pixels on the desktop link, the system DPI 96 times the
units). Inputs: the spec sheets of PR #304 (`spec-win-dpi`: `SetProcessDpiAwarenessContext.yaml`
`at_200_*` and `manifest`, `GetWindowRect.yaml`, `GetCursorPos.yaml`, `GetMonitorInfoW.yaml`,
`GetDeviceCaps.yaml`, `DisplayConfigSetDeviceInfo.yaml`, the point conversion sheets and the
others of that PR) with their visible tests; Microsoft Learn ("High DPI Desktop Application
Development on Windows", "DPI_AWARENESS_CONTEXT", "Mixed-Mode DPI Scaling and DPI-aware APIs",
"WM_DPICHANGED", "WM_GETDPISCALEDSIZE", "Application manifests"); `winuser.h`, `windef.h`,
`shellscalingapi.h`, `shtypes.h` for constants. No third-party source.

## What was there

Since ADR 0219 every Windows app on a 200 % desktop was treated as DPI aware: it saw a
2560x1600 screen at 192 DPI. A DPI-unaware program drew at half size. The interactive test
session was always 1920x1080 at 96 DPI, so nothing above 96 DPI could be tested against the
oracle, and a window's frame was always measured at 96 DPI. The scale never changed while
programs ran, and `WM_DPICHANGED` was never sent.

## Decisions

1. **win32k keeps physical pixels; the boundary converts.** `Wm::handle` (`wm::dpi`) finds the
   calling thread's units from its awareness context: 96 DPI for unaware and GDI-scaled threads,
   the session's system DPI for system aware ones, the monitor's DPI for per-monitor (v1, v2)
   ones. It converts every coordinate of a request to physical pixels and every coordinate of the
   reply back: window rectangles, the cursor, points of mouse messages, the desktop and work area,
   update rectangles, carets, regions, the menu bar. A caller in physical pixels (every aware
   thread at a whole scale) goes through unchanged, so nothing changes at 96 DPI.
2. **Rounding** (`dpiscale::Units`): a coordinate is rounded to the nearest, halves away from
   zero; a size read by a coarser caller is rounded down, so a client area never reads larger
   than its window (`at_200_thread_contexts`: 374x229 reads 187x114).
3. **A window keeps its rectangles in its own units** (`wm::dpi::Own`) as long as they still map
   to the physical ones. At 175 % a system aware unit is smaller than a physical pixel and the
   physical rectangle alone cannot give back what the thread set; a thread of the window's own
   units reads them as it set them.
4. **A window's DPI and frame.** `WinInfo::dpi_value` is `GetDpiForWindow` (96 for unaware windows
   and the desktop, the system DPI for system aware ones, the monitor's for per-monitor ones);
   `WinInfo::frame_dpi` is the DPI its frame is measured at. `WM_NCCALCSIZE`'s default and the
   frame metrics (`metrics::client_insets_dpi`, `hidden_frame_dpi`) take it: an overlapped
   window's frame is 8 and 31 at 96 DPI, 13 and 58 at 192.
5. **The session's screen and scale** come from the kernel arguments `poly.win32k.display=WxH`
   and `poly.win32k.scale=P` that `pipeline win poly` passes for tests recorded at 200 %
   (`interactive-bottom/2560x1600/200`): the taskbar is 48 pixels at 100 %, scaled.
6. **The values of the new and changed calls.** `GetSystemDpiForProcess` is the session's system
   DPI in every mode; `GetDpiFromDpiAwarenessContext` 96, the system DPI or 0;
   `GetDpiAwarenessContextForProcess`, `GetProcessDpiAwareness` and `IsProcessDPIAware` keep
   reading the calling thread's context, as the 96 DPI sheets observed (a thread that set its own
   context reports it for the process too); `SetProcessDPIAware` succeeds again;
   `GetScaleFactorForMonitor` gives the `DEVICE_SCALE_FACTOR` of the scale, 180 at 200 % as the
   oracle's monitor (`dpiscale::device_scale_factor`) and 100 for a NULL monitor;
   `GetDpiForMonitor` the monitor's DPI in the caller's units (angular: 96 scaled the same way);
   `EnumDisplaySettingsW`, `DESKTOPHORZRES`, `HORZSIZE` and the display configuration are
   physical for every caller; `AdjustWindowRectEx` uses the caller's system DPI. The four point
   conversions follow the sheet of `LogicalToPhysicalPointForPerMonitorDPI`.
7. **The manifest.** `user32`'s `DLL_PROCESS_ATTACH` reads `dpiAware` and `dpiAwareness` of the
   executable's manifest (`dpiscale::manifest_awareness`): the first known word of the
   `dpiAwareness` list wins over `dpiAware`.
8. **A scale change** (`DisplayConfigSetDeviceInfo`'s packet -4, new service `Request::SetScale`;
   `DisplayConfigGetDeviceInfo`'s -3 reads the steps): the monitor's DPI changes at once.
   Windows the system scales (unaware, GDI-scaled, system aware) keep their size in their units;
   each top-level one is moved so that its visible frame stays where it was on the screen (the
   invisible part of the left border is the frame less one pixel), through a message from no
   thread that its thread turns into `SetWindowPos` (`WM_MOVE` only). Each per-monitor aware
   top-level window's thread gets `wm::dpi::DPI_CHANGED`: v2 sends `WM_GETDPISCALEDSIZE`,
   `WM_DPICHANGED_BEFOREPARENT` to the descendants and `WM_DPICHANGED` with the suggested
   rectangle; `DefWindowProc` measures the frame at the new DPI, sends
   `WM_DPICHANGED_AFTERPARENT` and applies the rectangle. v1 takes the suggested size with its
   old frame first, then `WM_DPICHANGED`. The suggested size is scaled from the size before the
   first change while the window kept the sizes suggested since, so a change and its reverse give
   the first size back (`dpiscale::suggest`). Top-level windows are handled topmost first.
9. **On the desktop an unaware window is enlarged by the compositor.** A window the system scales
   draws in its own units: win32k gives its surface a buffer of that size
   (`Wm::buffer_scale`), and child areas and pixel reads are converted to it. The compositor shows
   a buffer as large as its surface's logical size at buffer scale 1 and enlarges it smoothly
   (ADR 0219 decision 12), as Windows enlarges such windows.

## Consequences

- The ten visible tests of the `spec-win-dpi` entries pass, the scale change in all three modes
  among them; the scale change sequences, the units of every thread context and the rounding have
  host tests (`user_tests/dpi.rs`, `dpiscale`, `dispcfg`).
- On the 200 % desktop (`--windows-hidpi`) a DPI-unaware Windows program now sees 1280x800 at
  96 DPI and is shown at its normal size, enlarged (soft, as on Windows) instead of sharp at half
  size. Aware programs (Chrome: per-monitor v2) are unchanged, except that their frames are now
  measured at their DPI.
- Not done: the desktop's own scale cannot change while Windows programs run (the link's units
  are fixed for a session), so only `DisplayConfigSetDeviceInfo` changes it; messages sent from
  one thread to another's window are not converted (a `WM_MOVE` that thread A causes for thread
  B's window is in A's units); a swap chain of an unaware window (a layer, ADR 0224) stays in
  physical pixels; `CW_USEDEFAULT` sizes at 200 % are not the oracle's; several monitors.

## Addendum (2026-11-19): default window size and other processes' classes

The integrator's held-out run of `GetWindowRect` found two gaps.

10. **`CW_USEDEFAULT` in the caller's units, from the work area.** An overlapped window with a
    default position and size is 3/4 of the screen's width and 3/4 of the work area's height less
    21, both as the calling thread reads them (`wm::default_height`): 768x555 on the 1024x768
    session (where the work area is the screen, which is why the old formula used the screen),
    960x543 for an unaware thread and 1920x1107 for an aware one at 200 %. The cascade step and the
    size with an explicit position are taken in the caller's units too.
11. **A search finds another process's window by its class.** `FindWindowW`/`FindWindowExW` looked
    the class name up only among the caller's own and the system classes, so a process could not
    find a window whose class only the other process registered (a test that starts a copy of
    itself waited in vain for the copy's window). A class the caller cannot see is now looked up
    among every process's classes (Learn, "FindWindowW"); a name that no process registered still
    finds nothing (`FindWindowExW.yaml` `match_class`).
