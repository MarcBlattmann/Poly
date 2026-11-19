# 0232: A frame draws only what shows

**Status:** Accepted. Track `chrome-paint-3`. Builds on ADR 0106 (the software compositor), ADR
0116 (KMS with two dumb buffers), ADR 0224 (swap chains hand their frames to the compositor as
sections) and ADR 0225 (popups with their alpha: a DirectComposition swap chain's premultiplied
pixels make its window `Argb8888`). Inputs: Microsoft Learn (`IDCompositionDevice::Commit`,
`IDXGISwapChain1::Present1`); Poly's own traces of Chrome for Windows and Debian's Chromium
running on Poly. No third-party source.

## What was there

The `chrome-feel` track measured that Chrome for Windows redrew its whole window 5–35 times per
keystroke, and that the compositor needed about 30 ms per full frame at 2560x1600, 200 %. Measured
again on `main` with ADR 0224 and 0225 in, using temporary counters in `d3d11` and the compositor:

- Chrome presents what changed: about 2.3 presents per keystroke in the omnibox (the suggestion
  list, about 0.8 Mpx), one small rectangle per keystroke in a text field. **No** present was a
  whole window, and no DirectComposition commit re-presented one in these runs.
- One path could still do it: `IDCompositionDevice::Commit` binds every swap chain of the device's
  visual trees again, and every bind presented the whole back buffer (`deliver(s, &[])`). A
  commit for a popup re-presented the main window in full.
- A whole-window frame (a CSS animation, scrolling) cost the compositor 4.4–4.8 ms composing, 6–7
  ms for the KMS present and 1.6–1.9 ms taking the pixels. The composing did three full copies of
  the damaged area, and one of them blended:
  1. KMS `begin_frame` copied last frame's damage from the front buffer into the back buffer,
     which this frame then drew over;
  2. the wallpaper was drawn under the window;
  3. the window, made `Argb8888` by ADR 0225, was blended pixel by pixel with `over()`, although
     it is opaque.

## Decisions

1. **A commit shows a swap chain's frame only where it was not shown.** A swap chain remembers
   that its last frame is shown in its window. A bind to the same window does nothing; a bind to
   another window, a resize and the first bind show it whole, as before. Presents show their own
   frames with their dirty rectangles (ADR 0224).
2. **Opaque runs are copied.** Drawing a window pixel for pixel copies each run of opaque source
   pixels with one `memcpy` and blends only the others (`render::over_row`). The result is
   `over()`'s, pixel for pixel (host test).
3. **A window that hides the damage hides what is below it.** `Content::opaque` says that every
   pixel of a window's content is opaque. Where the topmost such window, drawn pixel for pixel,
   covers a damaged rectangle, neither the wallpaper nor any window below it is drawn there. The
   desktop link counts the translucent pixels of each Windows window as it copies them in:
   - a new content is all transparent;
   - a copy subtracts the translucent pixels it overwrites, counted only while some are known;
   - it adds those it writes, counted only when an AND of their alpha says one is not opaque.

   `Xrgb8888` surfaces become opaque once all of them was copied. Chrome's premultiplied main
   window does too: its pixels are opaque. Its menus and bubbles stay translucent and are blended
   as before. Wayland clients leave the flag false: their path is unchanged.
4. **The back buffer is not brought up to date where the frame draws anyway.** `Kms::begin_frame`
   takes the scene's pending damage. A stale rectangle inside one of those rectangles is not
   copied from the front buffer.

## Consequences

Measured at 2560x1600 and 200 % on WHPX, steady state, with the same scripted runs. The full
numbers are in the track's hand-back.

| | Before | After |
|---|---|---|
| Scrolling `chrome://credits` (61 % of the screen per frame): compose / compositor fps | 3.7–4.4 ms / 37–42 | **2.6 ms / 47–49** |
| Scrolling, key press → frame | 19–22 ms | **16–17 ms** |
| Whole-page CSS animation (73 %): compose | 4.4–4.8 ms | **3.2–3.6 ms** |
| Omnibox typing: key press → frame | 36–48 ms | **29–37 ms** |
| Text field typing: key press → frame | 10–15 ms | 10–13 ms |

- The KMS present (the virtio-gpu transfer of the damage to the host, 5–6.5 ms for a whole
  window) is now the largest stage of a whole-window frame. It is not changed here.
- Debian's Chromium on Poly (Wayland) in the same runs: every frame damages 94 % of the screen and
  composes in 20–29 ms. Key → frame is 34–40 ms in a text field and 37–57 ms in the omnibox, and
  the animation runs at about 24 fps. That path (full-buffer commits, scaling) is for a later
  track.
- A window that claims opacity it does not have would show stale pixels. The count is exact for
  what the link copies, and a window starts as not opaque.

## Addendum (2026-11-20): Linux Chromium's frames, track `chromium-damage`

Chromium on Poly is native Wayland (`--ozone-platform=wayland`, software `wl_shm` buffers). Observed
on Poly with temporary counters (removed): at 200 % it attaches a buffer of scale 1 twice its
surface's size and shows it through **`wp_viewporter`** (2100x1560 shown 1050x780), on every commit,
and it damages only what changed (`wl_surface.damage_buffer`: the text cursor, a 2x34 rectangle per
key). Poly already kept that damage and copied only it, and released each buffer right after the
copy (`wl_buffer.release`), so Chromium was never short of buffers. The cost was in the scene:

1. **Damage behind a viewport was taken as the whole surface** (`Scene::damage_content`: "through a
   transform or a viewport: all of it"): every keystroke redrew Chromium's whole window, 77 % of a
   2560x1600 screen. Now `View::logical_rect`, the inverse of `View::buffer_rect`, maps the
   buffer rectangle back through the transform, the buffer scale and the viewport's crop and scale
   (rounded outwards; one pixel more where the scaling is not whole pixels, for the filter).
2. **A viewport was always drawn by sampling each pixel and blending it**, even when it maps one
   buffer pixel to one output pixel. `render::pixel_for_pixel` now also accepts a view with no
   transform, no source and a destination that is the buffer's size in output pixels; it is drawn
   with the row copy (`over_row`) and counts for the occlusion test.
3. **The opaque region was kept but not used.** A Wayland window's content is now opaque when its
   buffer has no alpha (`xrgb8888`, `xbgr8888`) or its opaque region covers the whole surface
   (`Region::covers`; wayland.xml lets the compositor treat that region's pixels as opaque), so
   the wallpaper and the windows under Chromium are not drawn under it.

Measured as above (WHPX on the owner's PC, 2560x1600 at 200 %, `desktop --release --chromium
--compositor-stats`, the same driver; one key every 0.25 s while typing), `main` and this change in
the same session:

| Linux Chromium | `main` | After |
|---|---|---|
| Text field: damage per frame | 77 % | 1–2.5 % |
| Text field: compose; key → frame | 22–28 ms; 33–42 ms | 0.05–0.15 ms; 6–8 ms |
| Omnibox: damage; key → frame | 35–41 %; 35–43 ms | 6–9 %; 13–15 ms |
| Whole-page animation (65 %): compose; fps | 21–23 ms; ~25 | 2.6–2.8 ms; 64–72 |

Presentation feedback (`wp_presentation`) and frame callbacks were already in place and were not
the cause; Xwayland is not involved.
