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

`default_mode` is **`client`**, although Poly prefers its own frame:

- With `server`, GTK 3.24.41 takes another path to its window size, and under
  Poly that size was garbage in a quarter to a half of the starts (2 of 6, 2 of 6, 1 of 6 and 4 of 8 in four boots): `Gdk-WARNING:
  Native Windows taller than 65535 pixels`, a `wl_shm` pool of 2 GB for a
  window 1.4 million pixels tall, then a crash, before the compositor sent
  anything else. Six starts of the baseline never failed; eight of the same
  build with `client` never failed. The cause is not understood (a GTK 3 path
  reading an unset value; not found on Poly's side), so Poly does not take it.
- Told `client`, GTK 3 and GTK 4 draw their own title bar (one, as in
  GNOME or on any desktop that does not draw frames), and say it themselves
  (`request_mode(client)`, a header bar's missing object, the shadow).
  Poly draws nothing around them.

GTK's own title bar has a close button only (double click maximizes; the
shell's window list minimizes). `gtk-decoration-layout=:minimize,maximize,close`
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
- **Server-side GTK** (`default_mode(server)`): what Poly would prefer, so GTK
  windows look like Alpine Shell's frame. Revisit it when the GTK 3 crash above
  is understood; the decision code does not change (a client that asks for
  `server` is framed: GTK 4 asked for it when told `server` was the default).

X11 clients (Xwayland) are decided by their window properties as before
(`_MOTIF_WM_HINTS`, window types, override-redirect; ADR 0118): GTK 3 under X11
sets the Motif hint for its own title bar.

## Consequences

- GTK 3, GTK 4, Qt 5/6 and Chromium windows have one title bar and one set of
  buttons. GTK windows look like GTK (Adwaita title bars), not like Poly's
  frame: the price of the decision above.
- The toolkits boot group asserts who draws each app's frame
  (`poly-compositor: window "T" frame: own|poly`, new): `own` for the GTK apps,
  `poly` for Qt; Poly's title bar pixels are only looked for where Poly draws.
- `cargo xtask wayland` takes a protocol file of Poly's own (no SHA-256 of a
  release); its generated module says so.
- Not done: Poly's shadow and rounding around client-drawn windows without
  their own (Chromium); the cause of GTK 3's server-mode crash; minimize and
  maximize buttons in GTK's title bars (`gtk-decoration-layout` had no effect
  in the test apps); a dark GTK theme for the title bars
  (`gtk-application-prefer-dark-theme=1` works).
