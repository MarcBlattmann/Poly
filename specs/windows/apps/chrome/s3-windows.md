# What windows Chrome's S3 run creates, and the order they appear in (spec-chrome-win-s3)

Observed from outside on the oracle (homeserver2, interactive session, build 26200.9974) with
`drivers/s3_windows.ps1`: Chrome for Testing 157.0.8103.38 is started as S3 does
(`--disable-gpu --no-first-run --window-position=40,40 --window-size=900,500 <page>`), and every 10 ms the
script lists the windows of every `chrome.exe` process (`EnumWindows` + `EnumChildWindows`, class, styles, owner,
parent, rectangles, state), then drives the same steps as `s3_driver.ps1` (a click, the keys p o l y) and a
resize (`SetWindowPos`), a minimize and a restore (`ShowWindow`), and `WM_CLOSE`. Nothing is injected into Chrome: an
out-of-context WinEvent hook was tried first and made `chrome.exe` crash with a breakpoint (`0x80000003`) before
its first window was shown, so the order of events below comes from polling. The full recording of the last run
(handles as `w1`, `w2`, ...; times in ms from the start of the run) is `traces/s3-windows.recording.txt`; two more runs
gave the same windows, styles and order (only the cascade position of the hidden `Chrome_WidgetWin_0` window and
the numbering of the handles differ).

## The windows of the browser process (S3, the window shown)

| Class | Style | Ex style | Class style | Parent / owner | Notes |
|---|---|---|---|---|---|
| `Chrome_WidgetWin_1` | `0x16CF0000` (WS_OVERLAPPEDWINDOW, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_VISIBLE) | `0x00200100` (WS_EX_NOREDIRECTIONBITMAP, WS_EX_WINDOWEDGE) | CS_DBLCLKS | top-level, no owner | created at the given rectangle (40,40,940,540 for `--window-position/size`) with style `0x06CF0000`, shown with `WS_VISIBLE`; class: background brush set, no cursor, small and large icon set; client area 884x492 for a 900x500 window: it starts at the top edge (no caption area) and keeps the 8 px side and bottom borders |
| `Chrome_RenderWidgetHostHWND` ("Chrome Legacy Window") | `0x46300000` hidden, `0x56300000` shown (WS_CHILD, WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_HSCROLL, WS_VSCROLL) | `0x00000020` (WS_EX_TRANSPARENT) | CS_DBLCLKS | child of the top-level window | created with an empty rectangle and shown after its first placement; its client area equals its window rectangle (the scroll bar styles reserve nothing); ids 656, 195 (two of them at the start: page and top-chrome), more are created and destroyed as pages come and go |
| `Chrome_WidgetWin_0` (the holder) | `0x8C000000` (WS_POPUP, WS_DISABLED, WS_CLIPSIBLINGS) | `0x00000080` (WS_EX_TOOLWINDOW) | CS_DBLCLKS | top-level, hidden, 0x0 | created in the browser when the first page child is made, and in the GPU process; the render widget host children are made children of it while the top-level window is minimized and when it closes |
| `Chrome_WidgetWin_0` (first) | `0x06CF0000` | `0x00000100` | CS_DBLCLKS | top-level, hidden | a full overlapped window at a cascade position, created before the top-level window (before `Chrome_StatusTrayWindow`) |
| `Chrome_StatusTrayWindow` | `0x84000000` (WS_POPUP, WS_CLIPSIBLINGS) | 0 | none | message-style top-level, hidden | |
| `Base_PowerMessageWindow` | `0x84000000` | `0x08000000` (WS_EX_NOACTIVATE) | none | top-level, hidden, 0x0 | the first window the browser creates |
| `Intermediate D3D Window` (GPU process) | `0x58000000` (WS_CHILD, WS_VISIBLE, WS_DISABLED) | `0x00280024` (WS_EX_NOREDIRECTIONBITMAP, WS_EX_LAYERED, WS_EX_TRANSPARENT, WS_EX_NOPARENTNOTIFY) | CS_OWNDC | child of the browser's top-level window (a cross-process child), same rectangle as its client area | follows the top-level window's size changes (one step later) |
| `IME` ("Default IME") | `0x8C000000` | 0 | CS_IME | owner and parent = a window of the thread; **follows the first visible window**: owned by `Base_PowerMessageWindow` until the top-level window is shown, then by the top-level window, and by the holder after the top-level window is hidden | one per GUI thread, hidden, 0x0; its child `MSCTFIME UI` (CS_IME) appears at the first activation |
| `crashpad_SessionEndWatcher` (crashpad handler) | `0x04C00000` | `0x00000100` | none | top-level, hidden, 136x39 | |

## The order (polled, ms from the start of the run; the S3 page and 900x500 at 40,40)

1. 0-60: the crashpad handlers' `crashpad_SessionEndWatcher` and their `IME` windows.
2. about 600: `Base_PowerMessageWindow` (+ its `IME`), then the first `Chrome_WidgetWin_0` (overlapped, hidden), `Chrome_StatusTrayWindow`.
3. about 880: `Chrome_WidgetWin_1` hidden (`0x06CF0000`) at its final rectangle and the first `Chrome_RenderWidgetHostHWND` child (empty rectangle, hidden); 60 ms later the window text is set ("Untitled - Google Chrome for Testing"), the child is placed (48,86 112x34 in screen coordinates: the top-chrome child), a second child is created and placed (48,127..932,532: the page), and then **in the same poll** the top-level window and both children get `WS_VISIBLE` (shown, 1085 ms) and the thread's IME window is given to the top-level window (owner change `Base_PowerMessageWindow` -> `Chrome_WidgetWin_1`), the `MSCTFIME UI` child is created, and the foreground goes to `Chrome_WidgetWin_1` (the shell's console window had it).
4. about 1270: the GPU process creates its `Chrome_WidgetWin_0` and the `Intermediate D3D Window` (child of the top-level window, 1x1 and then the client size), and its IME window.
5. after the page loads (about 1600 ms): the title becomes the page's ("poly-s3 clicks=0 keys= moved=0 - ...", the dash is U+2013).
6. when the page is clicked or typed into the title changes (6.4 s mouse move, 7.0 s click, then the keys); two more `Chrome_RenderWidgetHostHWND` children are created and one destroyed, a new `Chrome_WidgetWin_0` holder (`0x8C000000`) appears at the first click.
7. resize (`SetWindowPos 60,60 700x400`): the top-level window, the page child, the top-chrome child and the GPU child change in the same poll; the GPU child follows 17 ms later with its final size.
8. minimize (`SW_MINIMIZE`): **the two shown children become children of the holder** (`SetParent`), lose `WS_VISIBLE` (`0x46300000`) and keep their client coordinates as window coordinates (0,46,112,80; 0,143,684,392); then the top-level window gets `WS_MINIMIZE` (`0x36CF0000`) and the iconic rectangle -32000,-32000,-31840,-31972 (160x28), the foreground goes to the console window. Restore: everything reverts in one poll and the foreground returns.
9. close (`WM_CLOSE`): the top-level window is hidden (`WS_VISIBLE` removed; its children and the GPU child become invisible with it), the children are moved under the holder, then **destroyed** (about 50 ms), the thread's IME window moves to the holder, the top-level window is destroyed (the GPU process destroys its windows in the same poll), then the status tray window, `Base_PowerMessageWindow`, the first holder and the `MSCTFIME UI` window go (about 600 ms later), the crashpad handler's windows last; the process exits 0.

## What the scenario tests of `spec-chrome-win-s3` take from this

`cw_chrome_widget`, `cw_chrome_noredirect`, `sw_chrome_show`, `bp_chrome_first_paint`, `si_chrome_click`, `si_chrome_keys`,
`swp_chrome_resize` and `dw_chrome_close` (in `specs/windows/user32.dll/<Entry>.tests/`) create these windows with these
styles and drive them in this order with our own code (the window procedure shape is in the tests' doc comments);
the per-function tests (`iwv_*`, `ii_*`, `iz_*`, `gw_*`, `ga_*`, `gcn_*`, `gwtp_*`, `sp_*`, ...) use the same three windows
(`chrome_build`: holder, top-level, child) for their "Chrome" test.
