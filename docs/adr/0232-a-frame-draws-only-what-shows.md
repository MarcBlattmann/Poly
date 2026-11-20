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

## Addendum: a commit's copy in one pass (2026-11-20)

With the scene fixed, taking Chromium's buffers in was the next cost: about 3.5 ms per frame of
the whole-page animation (`take_avg_ms`). The copy was already limited to the damage, the
`wl_shm` buffer's damaged rectangles only, but it went through the client's memory twice per row:
`ShmMap::read` copied the row into a scratch buffer, and a loop then put each pixel together from
its four bytes into the window's content. `ShmMap::read_pixels` (new; its default still reads
bytes) now lets the compositor's mapping copy a row straight into the content with one `memcpy`;
`Buffer::copy_rect` then sets the alpha of `xrgb8888` in place and swaps red and blue only for the
`bgr` formats. Chrome for Windows already copied its rows this way (ADR 0224) and is unchanged.

Measured as before (same session, the commit before this one against this one):

| Linux Chromium | Before | After |
|---|---|---|
| Whole-page animation (65 %): take; fps | 3.4–3.7 ms; 70–74 | 0.67–0.76 ms; 74–75 |
| Text field: take | 0.17–0.21 ms | 0.06–0.07 ms |
| Chrome for Windows animation: take; fps | 1.8–1.9 ms; 57–60 | 1.9–2.0 ms; 53–60 |

The 0.7 ms left is the copy of 2.65 million pixels at memory speed. Composing straight from the
client's memory (no copy) was considered and not done: Poly would have to keep each buffer until the
next commit replaced it instead of releasing it after the copy, which costs a client one more buffer
in flight and is a protocol-visible change (`wl_buffer.release` comes later); the scene would read
memory another process owns and could change after a misbehaving client's early reuse, at every
redraw instead of once; and the formats would have to be converted while drawing. All that for at
most 0.7 ms of a 13 ms frame.

## Addendum: a window without a redirection surface shows only its composition (2026-11-20, track `chrome-paint-4`)

The QA round of 2026-11-20 found two white areas in Chrome for Windows: the first start showed a
plain white window before the toolbar, and resizing outward showed white in the new area until
Chrome repainted. Chrome's top-level window has `WS_EX_NOREDIRECTIONBITMAP`
(`specs/windows/apps/chrome/s3-windows.md`) and shows its pictures through DirectComposition swap
chains (ADR 0224). Learn ("Extended Window Styles"): such a window "does not render to a redirection
surface. This is for windows that do not have visible content or that use mechanisms other than
surfaces to provide their visual." Poly gave it a surface like any other window: it started white
(`BLANK`), and the one erase such a window gets (`CreateWindowExW.yaml` `noredirect`) filled it with
the class's background brush, so the window was white until its swap chain's first frame, and a
resize grew the white surface before the frame of the new size came.

5. **A window without a redirection surface shows only what composition gives it.** Its surface is
   premultiplied (`Argb8888`) from the start and starts transparent; what GDI draws into it (the
   erase, `WM_PAINT`, a child's commit) is accepted and dropped, with no damage. Its swap chains'
   layers (ADR 0224, 0225) are unchanged. Before the first frame, and where a grown window is not yet
   covered by a frame, what is below the window shows.

Measured as above (`tools/qa` on branch `chrome-qa-2`, polyd with KVM, 2560x1600 at 200 %; the
driver's `white` command: the share of pure white pixels in a region, a screendump every 30–40 ms),
`main` and this change in the same hour:

| Chrome for Windows | `main` | After |
|---|---|---|
| First start: plain white window before the first frame | 4.6 s (white from 8.1 s to 12.7 s after Enter; frame at 12.9 s) | **none** (frame at 13.6 s) |
| Outward resize, held: the new area's tab strip plain white | 363 ms, 304 ms | **0 ms** (wallpaper, then the frame) |

A flip-model swap chain of another 4-byte format than `B8G8R8A8` still reaches its window through
GDI (`d3d11` `swap.rs`, `show`), so in such a window it is no longer shown. Chrome's swap chains
are `B8G8R8A8`, and the formats of other sizes were never shown that way; none other is known.
Handing those over too (a converted copy into the section) is for when a program needs it.
