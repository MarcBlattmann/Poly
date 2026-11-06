# 0144: Apps that ask to draw their own frame get no Poly frame

**Status:** Accepted. Refines the frame decision of ADR 0106.

## Context

ADR 0106 has Poly draw one neutral frame (title bar, border, buttons)
around every toplevel. The compositor answered `server_side` to every
`zxdg_toplevel_decoration_v1.set_mode`, whatever the client asked.

Some apps draw their own frame anyway. Chromium puts its tab strip and
window buttons where a title bar would be and asks for `client_side`.
Framed by Poly too, its window showed two title bars, one above the other.

The protocol lets the compositor follow the client's mode or enforce
another. Clients that never bind the decoration manager, or ask for
`server_side`, or `unset_mode`, have no frame of their own to show.

## Decision

- A toplevel whose client asked for `client_side` (`set_mode`) gets
  `configure(client_side)` and no Poly frame. Poly keeps the window's
  place, focus, stacking, maximize, fullscreen, tiling and minimize; the
  app moves and resizes through `xdg_toplevel.move` and `resize`.
- Its maximized and tiled sizes are the whole area, not the area minus
  Poly's frame.
- Every other toplevel is framed as before: no decoration object,
  `server_side`, `unset_mode`, or a destroyed decoration object.

## Consequences

- Chromium shows one title bar: its own tab strip with its buttons.
- Apps that ask for `client_side` look like they do on other Linux
  desktops instead of all looking the same. Most apps (GTK and Qt
  apps, terminals) don't ask, so they keep Poly's frame.
