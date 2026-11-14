# 0179: Fractional display scales: 125 %, 150 % and 175 % next to 100 % and 200 %, one conversion between logical and physical pixels, `wp_fractional_scale_v1` and `wp_viewporter` for clients that know them

**Status:** Accepted. Answers the owner's question why the display scale can
only be 100 % or 200 % ("in QEMU 100 % is sometimes too big") and delivers the
in-between scales they approved. Builds on [ADR 0106](0106-wayland-server-core-and-software-compositor.md)
and [ADR 0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md)
(which listed fractional output scales as not done: "a fractional scale is
offered as the integer one"), [ADR 0117](0117-shell-window-management-notifications-and-settings.md)
(the settings), [ADR 0127](0127-the-alpine-shell-look.md) (the look) and
[ADR 0145](0145-the-desktop-follows-the-size-of-the-window-that-shows-it.md)
(the screen follows the window).

## Context

The output scale was an integer everywhere: `OutputConfig::scale: u32`,
`Scene`'s `scale`, every `x * s` in the compositor, the shell and the native
apps, `Settings::scale` with `SCALES = [1, 2]`, `wl_output.scale`. A QEMU window
of 1280x800 at 100 % has a desktop of 1280x800 logical pixels, too big for
what the owner wants to see; 200 % is 640x400, too small. Chromium already
asked for `wp_fractional_scale_v1` (ADR 0116 answered it with the integer
scale times 120) and drew through a `wp_viewport`; a bug in that path
(`chromium-damage-fix`) was the reason to look at the coordinate spaces.

## Decision

### The scale and the one conversion

`poly_wayland::compose::Scale` (`crates/poly-wayland/src/compose/scale.rs`)
is a scale between 100 % and 800 % **in 120ths**, the unit of
`wp_fractional_scale_v1`: 125 % is 150, 150 % is 180, 175 % is 210. All the
presets are exact; nothing is stored as a float. It is the only place where
logical pixels become physical ones and back; the compositor, the shell, the
native apps and the pointer code call it, and it is host-tested
(`compose::scale::tests`).

The rounding rule:

| What | Rule | Function |
|---|---|---|
| a position or an edge | the exact product rounded to the nearest pixel, halves up | `Scale::px` |
| a rectangle | by its edges: `px(left)`, `px(right)`, so rectangles that touch in logical pixels touch in physical ones | `Scale::rect` (`Rect::scale`) |
| a length that must be the same everywhere (a border, a radius, a gap) | converted alone | `Scale::px`; `Scale::line` is at least 1 for a positive length |
| the buffer of a client for a `w` x `h` surface | `px(w)` x `px(h)` (fractional-scale-v1: the logical size times the scale, rounded) | `Scale::buffer_size` |
| physical to logical (hit-testing, pointer, an output's logical size) | floor: a physical pixel belongs to the logical one it lies in; `output_size` never has a logical pixel cut by the screen's edge | `Scale::to_logical`, `output_size` |
| a pointer or touch position, a motion | physical pixels in 1/256ths made logical `wl_fixed` | `Scale::physical_to_logical` |
| damage | edges rounded outwards (it covers what `rect` draws), plus two pixels at a fraction because frames are laid out from rounded corners | `Scale::rect_out`, `Scene::physical_extent` |

Edge rounding for shapes is what Chromium does to layout (pixel snapping), so
the shell can be compared with Alpine Shell at the same size. At a whole scale
every rule gives the old integer arithmetic, and the existing tests of pixels
at scales 1 and 2 pass unchanged.

`impl Mul<Scale> for i32` is `px`, so shell code reads `RADIUS * s`; a
hairline is `s.line(1)`; font sizes and anti-aliased shapes use `s.factor()`
(the float), so text is rendered at the scaled pixel size, not stretched.

### Settings, the dev menu, the settings file

`SCALES` is `Scale::PRESETS` (100, 125, 150, 175, 200 %). The Displays page
shows the five as one segmented row. The file keeps the percentage
(`scale=150`); the old `scale=1` and `scale=2` still read as 100 % and 200 %,
a value that is not a preset keeps the default. `--scale` of the compositor
and `POLY_DISPLAY_SCALE` (the kernel's `poly.display.scale=`) take `2`, `1.5`,
`1.25`, `150` or `150%` (`Scale::parse`); `cargo xtask desktop --scale=` and the
`cargo xtask start` menu's Scale option offer 1.25, 1.5 and 1.75 as well (the
screen of `desktop` is the base size times the scale). "auto" still picks a
whole scale from the connector's density.

### Protocols

- `wl_output.scale` and `wl_surface.preferred_buffer_scale` say the **next whole
  scale** (`ceil`: 2 at 125, 150 and 175 %). Clients that know only these draw
  at it, and the compositor **shrinks** their buffers to the output's pixels
  (bilinear, premultiplied channels, 16.16 steps; a whole ratio keeps the
  nearest neighbour or the 1:1 copy it always had).
- `wp_fractional_scale_v1` says the **exact** scale in 120ths (it already did,
  as an integer times 120) and again when it changes. A client that uses it
  with `wp_viewporter` (Chromium, GTK 4, Qt 6, and now Poly's own apps) draws a
  buffer of `round(size * scale)` pixels of buffer scale 1 and shows it at the
  logical size through `wp_viewport.set_destination`. The compositor copies such
  a buffer **pixel for pixel**: a rectangle rounded by its edges can differ from
  the buffer by one pixel, and `Content::sample` then maps one to one instead of
  skipping or repeating a column in the middle of the window.
- The `wl_surface.damage` mapping of the `chromium-damage-fix` PR
  (`View::buffer_rect`) is unchanged: it works in surface coordinates through
  the viewport, so a fractional buffer is damaged in the same way at 1.5 as at 2
  (host tests of both, `surface_damage_goes_through_a_fractional_viewport`).
- **Poly's own apps** (`poly-app`: Files, Terminal, the installer, and what is
  built on it) bind `wp_fractional_scale_manager_v1` and `wp_viewporter` when
  offered and draw at the exact scale; at a whole scale they use the buffer
  scale as before, and leave the viewport when the scale goes back to a whole
  one. `Ui::scale` and `AppEvent::Resized::scale` are `Scale`.
- **The compositor's own windows** (the settings window, drawn by the shell into
  a buffer) do the same: a buffer of the rounded size with a viewport at a
  fraction. After a scale change the settings window is fitted to the new
  logical screen (`Server::resize_internal_window`): 720 logical rows do not
  fit 533.
- **What XWayland and Windows apps get.** X11 apps are not scale-aware (the X
  screen's size is not updated either, ADR 0145): their surfaces are drawn at
  buffer scale 1 and the compositor enlarges them to the output's pixels
  (bilinear), so they look as soft as on other compositors that do not scale
  XWayland. Windows apps are told the next whole scale (`Hello.scale` of the
  win32k link is `ceil`) and logical size, draw at it, and are shrunk like any
  whole-scale Wayland client.

### Drawing

- The software compositor places a window by its visible bounds: the corner
  rounded, the size rounded ([`Window::placed`]); the surface hangs from it by
  its distance from the bounds, and the **frame is laid out around the bounds**:
  `Scale::line(BORDER)` on every side, a title bar of `px(TITLE_HEIGHT)`,
  buttons and text at their logical distances from the title bar's edges. A
  frame therefore has the same border and title bar wherever the window is
  (dragging it does not make the border thicker or thinner), and the content and
  the frame always touch.
- Window titles are drawn at the scaled font size (`TextRenderer::coverage_at`;
  `TitleFont` keeps its glyphs per scale), not stretched from the logical size;
  bitmap fonts stay stretched. The cursor and the busy spinner start at the
  hot spot's rounded pixel. Shadows scale their lengths by the float.
- The shell draws everything at the scale: the wallpaper at the output's own
  physical size (a fractional scale's logical size times the scale can fall a
  pixel short), the ridge, launcher, panels, switcher, notices, the settings
  window, and the native apps' contents.

### Pointer, touch, placement, resize

The seat works in logical `wl_fixed` coordinates. `poly-compositor`'s
`evdev::Geometry` is the output's physical size and scale: an absolute device
covers the physical pixels and its position becomes logical with
`Scale::physical_to_logical`; a relative device moves in physical pixels (three
pixels are two logical ones at 150 %, so the pointer covers the same distance of
the screen at every scale; before, one count was one logical pixel, twice the
distance at 200 %). Hit-testing, window placement, snapping and interactive
resize are logical and unchanged; `Scene::logical_size` is the floor of the
physical size over the scale (1280x800 at 150 % is 853x533). Maximized and
tiled windows follow the work area as in ADR 0145; floating windows are shifted
up or left until their frame is on it (a demo window at 304 logical rows is at
292 after the switch to 150 %).

## Verification

- Host: `compose::scale::tests` (the table above, reading and writing scales),
  `compose::tests` (a frame keeps its 2 pixel border and 36 pixel title bar
  wherever it is placed; a fractional buffer is copied pixel for pixel; a whole
  scale buffer is shrunk smoothly; damage drawn piece by piece equals a fresh
  picture at 125, 150 and 175 %; the cursor), `server::tests` (150 %: `wl_output`
  says 2, the fractional scale 180; damage through a 1.5 viewport; an internal
  window's buffer and its fitting), `poly-app` (a client at 125, 150 and 175 %
  makes a buffer of the rounded size of scale 1 with a viewport destination and
  leaves it when the scale becomes whole), `evdev` (tablet and mouse at 150 % and
  200 %), `poly-shell` settings (the five scales, the file, old files) and the
  settings window (five buttons that fit the page).
- Boot group `desktop-scale` (in the `desktop` shard): at 100 % the demo window
  and the settings are opened; the Displays page's 150 % is clicked; the
  compositor says `scale 150%`; with the settings closed the demo window's frame
  is a 2 pixel border and a 36 pixel title bar with the content (one colour) right
  inside, the right edge sharp; the ridge is 36 pixels with a sharp lower edge; a
  click at the physical place of the maximize button maximizes it and the content
  fills the work area. Screenshots: `fractional-scale-*.png` in the hand-back.
- Chromium at 150 %: `cargo xtask chromium --scale=1.5` (see the hand-back).

## Not done

- "auto" does not pick a fraction from the screen's density (it rounds to a
  whole scale as before).
- X11 apps are enlarged, not scale-aware (no `Xft.dpi`, no RandR size).
- A shrunk whole-scale client is bilinear, not a better filter; the compositor
  redraws the whole window for any damage of a viewport'd or shrunk buffer
  (unchanged).
- Windows apps are not told the exact scale (their DPI is the whole one).
- Touch screens: there is no touch device in the seat yet; it will map its
  physical position the way an absolute pointer does.
