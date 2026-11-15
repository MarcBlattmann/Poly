# 0183: One title bar per window: who draws the frame, and how each toolkit says so

**Status:** Accepted. Refines [ADR 0144](0144-client-side-frames-for-apps-that-ask.md)
(apps that ask to draw their own frame get none) and the frame decision of
[ADR 0106](0106-wayland-server-core-and-software-compositor.md).

## Context

Poly draws a title bar, border and the three window buttons around every
toplevel ([ADR 0106](0106-wayland-server-core-and-software-compositor.md)).
Some apps draw theirs too, and a window showed two bars: in the CPU-X
AppImage (GTK 3, `linux-installer-2-09-appimage-running.png`) Poly's "CPU-X"
with its buttons sits over GTK's own header bar "CPU-X" with a second close
button. `gtk3-hello` and `gtk4-hello` showed the same (before screenshots
`one-title-bar-before-*.png`). Qt (`qt-hello`) already had one; Chromium has
one since ADR 0144.

What the toolkits do on a Wayland desktop (their public documentation, the
protocols' XML and what they send to Poly, seen with `WAYLAND_DEBUG=1`):

| Toolkit | How it decides | What it sent to Poly |
|---|---|---|
| Qt 5 and 6 | uses `xdg-decoration` when offered, and draws no bar if the compositor says `server_side` | `set_mode`, answered `server_side` |
| Chromium | `xdg-decoration`, `client_side` when it draws its tab strip | `set_mode(client_side)` |
| GTK 3.24 | draws client-side decorations (CSD) by default (`GTK_CSD`, GTK's documentation); always for a window with a custom title bar (a header bar); asks KDE's server decoration protocol (`org_kde_kwin_server_decoration_manager`) whether the compositor prefers to draw | binds the manager, makes an object per plain window and asks for the mode; a window with a header bar makes none; CSD windows give a window geometry that excludes a 26 px shadow |
| GTK 4 | the same protocol; does **not** use `xdg-decoration` | binds the manager, makes an object and asks for the mode |
| anything else | nothing | |

## Decision

One module decides, [`poly_wayland::frame_policy`](../../crates/poly-wayland/src/frame_policy.rs)
(`no_std`, host-tested); the server collects the signals and applies the
answer. Poly's frame is the default; a client that says it draws its own gets
none. **No Poly frame means no Poly shadow or rounding either: the client's
pixels as they are** (GTK draws its own shadow and corners; Chromium's are
square). Signals, strongest first:

1. **xdg-decoration**: `set_mode(client_side)` is followed, `server_side` and
   `unset_mode` get Poly's frame. The mode Poly sends back is the decision, so
   the client follows it. (As before.)
2. **KDE's server decoration protocol** (new global
   `org_kde_kwin_server_decoration_manager`, version 1; the wire format is
   [crates/poly-wayland/protocols/kde-server-decoration.xml](../../crates/poly-wayland/protocols/kde-server-decoration.xml),
   written for Poly from the protocol's public interface, names, numbers and
   argument types being ABI facts): an object for the surface gets the mode
   it asks for (`server`: Poly's frame; `client` or `none`: its own) or, if it
   asks for none, the default (below, `client`); a client that bound the
   manager but made no object for a surface that shows content draws its own
   (a GTK 3 header bar). Poly answers every mode request that changes the mode
   with a `mode` event.
3. **Shadow margins**: a toplevel whose window geometry sits inside its buffer
   on all four sides (at least 8 px on each axis) draws a shadow, which only a
   client-drawn frame has. It catches toolkits that use neither protocol and
   stays once seen (a maximized CSD window loses the shadow, not the title
   bar).
4. Otherwise Poly's frame.

A client-drawn window keeps Poly's place, focus, stacking, maximize,
fullscreen, tiling and minimize; it moves and resizes through
`xdg_toplevel.move` and `resize`. Its maximized and tiled size is the whole
work area. If the answer changes after the window was configured, a window that
is maximized, fullscreen or tiled, or that has an xdg decoration object, is
configured again (its size follows the frame, the object is told the mode); a
floating window that only uses KDE's protocol is not (below).

### What the compositor tells GTK about the default

`default_mode` is **`server`**: Poly's frame, the Alpine Shell look, for every
GTK window that does not draw its own title bar. GTK 3 and GTK 4 ask for
`server` back and draw no bar; a GTK window with a header bar makes no object
and draws its own (the toolkits group's `gtk3-header-bar`, gtk3-demo,
gtk4-demo, CPU-X). Seen with `WAYLAND_DEBUG=1`: GTK 3 hears `default_mode(2)`
right after binding, makes its toplevel, commits, then makes the object and
asks `request_mode(2)`, and gets one `mode(2)` after its first configure (0 x
0, its own size); GTK 4 makes the object and asks before its xdg_surface.

**The crash that made it `client` for a while was GTK without any font, not
this mode.** With `server`, GTK 3.24.41 sometimes computed a window about 1.4
million pixels tall (`Gdk-WARNING: Native Windows taller than 65535 pixels`, a
2 GB `wl_shm` pool, a crash). Found (hand-off `gtk-huge-window`):

- On a disk without fonts, 7 of 10 starts of gtk3-hello with `server` printed
  the warning and crashed (SIGSEGV), and the warning comes **before GTK makes
  its xdg_surface**: by then the compositor had sent only the registry,
  `wl_output`, `wl_shm`'s formats, the seat and `default_mode(2)`. No
  configure, bound, state or scale about the window can be the cause.
- That is the no-font path of ADR 0173's note (Pango with no font, font
  metrics 0): with a client-drawn frame GTK took it only after a relayout (a
  click), with the server's frame already at its start, so measured by
  starting apps the crash looked like the mode's. A `cargo xtask toolkits`
  disk made before the fonts were fetched has none (only the boot test fetched
  them); it now fetches them too and makes such a disk again.
- With the fonts: 0 failures in 45 starts of gtk3-hello with `server` (single
  boot-test runs, the whole group, 12 starts in one boot), and the toolkits
  group 10 times out of 10 with `server` as the default.

What the compositor sends was checked against the protocols on the way:
`configure_bounds` and every `configure` size are never negative (xdg-shell
gives 0 a meaning and a negative size none; an output smaller than Poly's
frame, or a tile smaller than it, made negative ones), events newer than an
object's version are not sent (GTK 3 binds `wl_compositor` 3, so it never gets
`preferred_buffer_scale`), and a `mode` event comes once when the object is
made and again only when the mode changes. Host tests in
`server/tests/frame_ownership.rs` replay GTK 3's and GTK 4's start sequences.

A GTK window that draws its own title bar has a close button only (double
click maximizes; the shell's window list minimizes). `gtk-decoration-layout=:minimize,maximize,close`
in `settings.ini` (read: `gtk-application-prefer-dark-theme` from the same file
took effect) did not add the buttons for GTK 3's default title bar, nor in the
user's `~/.config/gtk-3.0/` with `menu:` in front; why is open (below).

### Why not the alternatives

- **`GTK_CSD=0`** (documented: the window manager draws the decorations of
  windows without a custom title bar) does nothing on Wayland unless the
  compositor offers server decorations, never helps windows with a header bar
  (GTK always draws those) and is not read by every toolkit.
- **Drawing Poly's frame over GTK's** and hiding GTK's: impossible for the
  compositor; the client's pixels are the client's.
- **Client-side GTK** (`default_mode(client)`, as this ADR first decided):
  GTK 3 and GTK 4 then draw their own Adwaita title bar with a close button
  only, not Alpine Shell's frame. It was taken only because of the crash
  above, which was not the mode's.

X11 clients (Xwayland) are decided by their window properties as before
(`_MOTIF_WM_HINTS`, window types, override-redirect; ADR 0118): GTK 3 under X11
sets the Motif hint for its own title bar.

## Consequences

- GTK 3, GTK 4, Qt 5/6 and Chromium windows have one title bar and one set of
  buttons. Plain GTK windows have Poly's frame like Qt's; GTK windows with a
  header bar (and Chromium) draw their own.
- The toolkits boot group asserts who draws each app's frame
  (`poly-compositor: window "T" frame: own|poly`, new): `poly` for gtk3-hello,
  gtk4-hello and Qt, `own` for gtk3-hello with a header bar
  (`POLY_GTK_HEADER_BAR=1`, launcher entry "GTK 3 Header Bar"), whose own
  title bar the group drives (move, maximize, restore, resize, close); Poly's
  title bar pixels are only looked for where Poly draws.
- `cargo xtask wayland` takes a protocol file of Poly's own (no SHA-256 of a
  release); its generated module says so.
- Not done: Poly's shadow and rounding around client-drawn windows without
  their own (Chromium); minimize and maximize buttons in GTK's own title bars (`gtk-decoration-layout` had no effect
  in the test apps); a dark GTK theme for the title bars
  (`gtk-application-prefer-dark-theme=1` works).
