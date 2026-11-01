# 0117: The shell's window management, notifications over a minimal D-Bus session bus, and a settings window, in the look of Alpine Shell

**Status:** Accepted. Finishes roadmap Phase 6's "Shell: launcher,
window management, notifications, settings (own non-glass design)",
which [ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md) started
with the ridge and the launcher. The item is ticked with this change.

Owner decisions this follows: the UI is the owner's **Alpine Shell**
(a design reference only: values were read, no code copied); **one
neutral frame** drawn by Poly for every window (ADR 0106); **per-window
menus**, no global menu bar, and **nothing drops down from the ridge**,
which stays a status strip (ADR 0107).

## Context

ADR 0107 left the shell with a ridge whose network and volume were
placeholders, a clock in UTC, a launcher, and windows that could only be
moved and closed. Missing: window management (maximize, minimize,
snapping, a switcher, keyboard shortcuts), notifications, and settings.

Linux desktop programs send notifications through
`org.freedesktop.Notifications` on the D-Bus session bus (libnotify's
`notify-send`, GTK's `GNotification` backend, most toolkits). Poly had no
D-Bus. The task allowed either a minimal session bus for this one service
or a small Poly protocol with a `notify-send`-like CLI.

Another agent is moving the compositor to DRM/KMS at the same time, so
changes to `poly-compositor` and `poly-wayland` are additive: new modules
(`server/wm.rs`, `desk.rs`, `session.rs`, `mixer.rs`, `status.rs`), new
public methods, and new `Hit` variants; the frame clock, the screen and
the backends are untouched.

Sources, all public: the D-Bus Specification 0.43 (type system,
marshalling, message format, authentication, the message bus and its
driver, match rules); the Desktop Notifications Specification 1.2;
xdg-shell.xml (configure sizes and states, `tiled_*` from version 2,
`wm_capabilities`, `set_minimized`); POSIX.1-2017's `TZ` format; the uapi
header `sound/asound.h`; getifaddrs(3); netdevice(7). No D-Bus
implementation, compositor, libnotify or toolkit source was read
(CLEAN_ROOM.md).

## Decision 1: window management

**Where it lives.** Mechanism in `poly-wayland`, policy in `poly-shell`,
glue in the compositor:

- `poly-wayland/src/server/wm.rs` (new): a shell's API —
  `managed_windows()` (every mapped toplevel and internal window, bottom
  to top, with title, app ID, active, minimized, maximized, tiled),
  `activate_window` (un-minimizes), `minimize_window`, `set_maximized`,
  `toggle_maximized`, `tile_window(w, outer)` (the frame's outer
  rectangle), `restore_window`, `request_close`, `active_window`,
  `moving_window`.
- `poly-shell/src/wm.rs`: snap zones, their rectangles and Super+arrow.
- `poly-shell/src/switcher.rs`: the switcher's order, state, cards and
  thumbnails.
- `poly-compositor/src/desk.rs` (the desktop moved out of `main.rs`):
  keys, drags and what they do.

**The frame** (`compose/frame.rs`) has three buttons on the right, 16
pixels, 6 apart: minimize (a line), maximize (a square; two squares,
"restore", when the window is maximized or tiled), close. A narrow window
drops minimize first, then maximize. `Hit::Maximize` and `Hit::Minimize`
are new. A double click on the title bar (two left presses within 400 ms)
maximizes or restores.

**Maximize, tile, restore for clients.** A state change is a configure
sequence: maximized sends the work area less the frame and `maximized`;
tiled sends the tile less the frame and the `tiled_left/right/top/bottom`
states of the work area edges it touches (only to version 2 and later);
restoring sends the size the window had before (remembered when it left
the floating state; xdg-shell lets the compositor suggest it). The window
moves when the client has acked and committed a buffer, as maximize did
in ADR 0106. Dragging a maximized or tiled window by its title bar
restores it once the pointer is 8 pixels away, keeping the pointer at the
same share of the title bar.

**Minimize.** xdg-shell has no minimized state event: the scene window is
hidden (`Scene::set_hidden`; hidden windows are neither drawn nor hit),
its popups get `popup_done`, and the focus goes to the topmost shown
window. `set_minimized` from the client does the same, and
`wm_capabilities` now offers minimize. There is no taskbar, as Alpine
Shell has none: minimized windows come back through the switcher. When
every window is minimized or closed, the launcher shows on the empty
desktop, as in ADR 0107.

**The focus moves on.** When the active window closes or unmaps, the
topmost shown window gets the focus (before, nothing had it).

**Snapping** follows Alpine Shell's tiling: zones `left`, `right`, the
four quarters and full; an 8-pixel gap between tiles (half of it at the
edges), no margin. Dragging with the pointer at the work area's left or
right edge (6 pixels) picks that half, near a corner (the top or bottom
quarter of the edge) that quarter, at the top (onto the ridge) maximized;
a translucent rounded rectangle (white at 12 %, outlined at 40 %, radius
10) shows the zone while the button is held, and the release applies it.
**Super+arrows** use Alpine's transitions: Left/Right to the halves (a
quarter on the other side keeps its row), Up from a half or a bottom
quarter to the top quarter and otherwise to maximized, Down from a half or
a top quarter to the bottom quarter, from maximized back to floating, and
otherwise to minimized.

**Alt+Tab** opens the switcher in Alpine's look (cards of a 124-pixel
thumbnail under a 28-pixel name row, 100 to 200 pixels wide by the
window's aspect, 8 apart, 12 pixels of padding, radius 6, the selected one
ringed white at 70 %, the others at 10 %), floating in the middle of the
screen instead of hanging from the bar. Order is most recently used; it
opens on the second window, Tab/Shift+Tab or Left/Right step, releasing
Alt switches, Escape cancels, a click on a card switches. Minimized
windows are in it (their thumbnail dimmed). **Alt+F4** asks the active
window to close. **Super+I** opens the settings, **Super+N** the
notification history. Keys the compositor takes never reach the clients
(their releases are dropped by the seat, which never saw the press).

**Internal windows.** The settings window is drawn by the shell, but it is
a real window: `Server::add_internal_window` makes a scene window with an
owner tag of client 0 (never a real client), framed, placed, focused,
moved, maximized, tiled, minimized and switched like any toplevel. Its
pointer and key input is queued as `InternalEvent`s (motion, leave,
buttons with an implicit grab, wheel, keys while it has the focus, close,
resize) and its owner sets its pixels (`set_internal_pixels`). The focus
is either a client's toplevel or an internal window, never both.

## Decision 2: notifications through a minimal D-Bus session bus

The first option: a real, small session bus, so that programs that use
the standard interface work unchanged. A Poly protocol would have needed
every toolkit patched.

**`crates/poly-dbus`** (new, `no_std` + `alloc`, no I/O, host-tested):

- `wire`: the type system (`y b n q i u x t d s o g a ( ) { } v h`),
  signature validation, marshalling little-endian, reading both byte
  orders, the specification's limits (nesting 64, signatures 255 bytes,
  arrays 64 MiB, messages 128 MiB), valid object paths; malformed input is
  an error, never a panic.
- `message`: header fields, required fields per type, a stream reader.
- `auth`: the server side of the SASL exchange: the NUL byte, `AUTH
  EXTERNAL <hex uid>` (checked against `SO_PEERCRED` when the socket tells
  it), `AUTH EXTERNAL` then `DATA`, `ANONYMOUS`, `REJECTED EXTERNAL
  ANONYMOUS` for anything else, `NEGOTIATE_UNIX_FD` answered with `ERROR`
  (no descriptor passing on this bus), `BEGIN`.
- `bus`: connections, `Hello` first (anything else cuts the connection
  off), unique names `:1.N`, well-known names (`RequestName` without
  queueing: 1 primary owner, 3 exists, 4 already owner; `ReleaseName`),
  routing by `DESTINATION` with the bus setting `SENDER`, `ServiceUnknown`
  for unknown names, signals to connections whose match rules match
  (`type`, `sender` by unique or well-known name, `interface`, `member`,
  `path`, `path_namespace`, `destination`, `argN`), `NameOwnerChanged`,
  `NameAcquired`, `NameLost`, and the driver's `GetNameOwner`,
  `NameHasOwner`, `ListNames`, `ListActivatableNames`, `StartServiceByName`
  (an owned name is "already running", others `ServiceUnknown`),
  `AddMatch`, `RemoveMatch`, `GetId`, `Peer.Ping`, `GetMachineId`,
  `Introspect`. **Local services**: names owned by the bus's own process;
  method calls to them queue up (`take_local`) and their replies and
  signals go out through `send_local`.
- `notifications`: `Notify` (with the `urgency`, `category`, `transient`
  and `resident` hints), `CloseNotification`, `GetCapabilities`,
  `GetServerInformation` (spec version 1.2), `Introspect`, the
  `NotificationClosed` and `ActionInvoked` signals, and the client's
  `Notify` call.

**The compositor hosts the bus** (`session.rs`) on
`$XDG_RUNTIME_DIR/bus` — where GLib looks without
`DBUS_SESSION_BUS_ADDRESS` — and sets `DBUS_SESSION_BUS_ADDRESS=unix:path=…`
for the apps it starts. The shell owns `org.freedesktop.Notifications` as
a local service; capabilities `body`, `actions`, `persistence`.

**The shell's side** (`poly-shell/src/notify.rs`): toasts stack in the top
right corner below the ridge (8 pixels from the edge and the ridge, 8
apart), newest on top, at most three (more wait). Each goes after the
sender's timeout, or Alpine's notices' default of 7 s for -1; never for 0
or for a critical notification asking for the default. `replaces_id`
updates in place. A click dismisses (or invokes the `default` action if
there is one); other actions are buttons. The history keeps the last 50
(not `transient` ones), newest first; what came while it was closed shows
as "N new" in a pill on the ridge, and a click on it (or on the clock, or
Super+N, or the launcher's "Notifications") opens the history as a panel
in the middle of the screen, not hanging from the ridge.

**`notify-send`** (`userland/poly-notify`, new): the command line of
libnotify's `notify-send` (`-u`, `-t`, `-a`, `-i`, `-c`, `-e`, `-r`, `-p`,
`-A`, `-w`), from its manual page, on `poly-dbus`.

Consequences: GTK and other GLib programs find a session bus with the
notification service (GDBus authenticates with EXTERNAL and copes with
`ERROR` for descriptor passing). Programs that need other services
(accessibility, portals, settings daemons) find only the names the
compositor serves, and messages with descriptors are not passed. The bus
is the compositor's: it goes when the compositor does.

## Decision 3: settings

**The window** (`poly-shell/src/settings_ui.rs`) is an internal window
(Decision 1), 760 × 520, re-creating Alpine Shell's settings app in its
dark theme: a 40-pixel header ("Settings › page"), a 224-pixel sidebar of
pages (36-pixel buttons, radius 8, 15-pixel icons, the active one on
surface 3), the page on surface 1 with a rounded top-left corner and a
hairline, a 22-pixel title, cards (white at 6 %, radius 12) of rows (48
pixels, 60 with a line under the label) with their controls on the right:
segmented choices (Alpine's filled `Segmented`), a slider (8 pixels,
`fill-solid` on `fill-track`), a pill toggle (38 × 22), lists with a tick,
wallpaper tiles. Pages: Display (resolution, scale), Keyboard (US, Swiss
German, German from `poly-input`), Wallpaper (six generated gradients),
Sound (the master volume and mute), Date & Time (local time, nine time
zones), About (system, kernel, processor, memory, display, sound card,
network, session bus). Up and Down step through the pages.

**The file** (`poly-shell/src/settings.rs`): `key=value` lines in
`$XDG_CONFIG_HOME/poly/shell.conf` (else `$HOME/.config/…`, else
`/root/.config/…`), or `--settings FILE`; written whole to a temporary
file, synced, renamed, then `sync(2)`. Reading is forgiving: a bad value
keeps its default, unknown keys are kept and written back. `--keymap` and
`--scale` on the command line win over the file.

**What applies at once**: the keyboard layout (a new keymap file, sent to
every `wl_keyboard` with `Server::keymap_changed`, and the launcher's
layout), the volume and mute (the mixer, below), the wallpaper, the time
zone (the clock), the scale (`Server::set_output`; the shell is laid out
for the new logical size). **The resolution** is stored but applies only
when the display can change modes: fbdev has one mode, and modes come with
the KMS path the other agent is building; the page says so.

## Decision 4: the ridge's status

- **Network**: `getifaddrs` every 2 s (glibc answers it with the
  `RTM_GETLINK`/`RTM_GETADDR` netlink dumps of the Linux personality):
  online through the first interface that is up, running and has an
  address (loopback aside); the bars dim with a slash when offline.
- **Volume**: ALSA's control device `/dev/snd/controlC0` (ADR 0109's
  "Master Playback Volume" and "Master Playback Switch", found by name;
  card name from `CARD_INFO`), subscribed to change events and polled, so
  a change by another program shows; a wave per third of the level, a
  cross when muted, dim without a sound card. At start the mixer gets the
  saved volume; with no settings file yet, the card's own volume stays.
- **Local time**: POSIX `TZ` rules (`std offset dst [offset],start[/time],
  end[/time]` with `Jn`, `n`, `Mm.w.d`), host-tested, and a short list of
  named zones with the rules their regions use today; the clock shows the
  local time, the settings show each zone's offset.

## Alpine Shell measurements (new parts)

Read from the owner's Alpine Shell repository (`apps/desktop/src/…`,
`electron/modules/…`, `@alpine/ui`'s tokens). Only values were taken; its
code was not copied. Tailwind units at 4 px.

| Element | Alpine Shell | Poly | Source in Alpine Shell |
|---|---|---|---|
| Notice body | 14 × 10 px padding, 10 px gap, max 440 px wide | same padding and gap, 360 px wide cards | `topbar/noticeStrip.tsx` |
| Notice dot | 8 px, white at 45 %; rose/emerald by tone | same; rose for critical | `noticeStrip.tsx` |
| Notice text | title 12 px medium white 95 %, message 12 px white 70 %, 16 px lines | same (DejaVu Sans has no medium; regular) | `noticeStrip.tsx` |
| Notice dismiss | × 13 px, white 60 % | same | `noticeStrip.tsx` |
| Notice action | 11 px on white 15 %, radius 6 | same | `noticeStrip.tsx` |
| Notice timeout | 7000 ms default | 7000 ms for -1 | `electron/modules/shellNotice.js` |
| Notice placement | hangs from the bar's centre | floating cards top right, below the ridge (nothing drops from the ridge) | `noticeStrip.tsx` |
| Ridge notice | dot 6 px + 11 px text white 75 % | "N new" pill: dot 6 px, 11 px white 75 % | `faces/noticeFace.tsx` |
| Switcher | padding 12, gap 8, cards 100–200 px by aspect, 124 px thumbnails, 6 px name row padding, 16 px icon, 12 px text, radius 6, selected ring 2 px white 70 %, others 1 px white 10 %, fill white 5 % | same, floating in the middle | `topbar/windowSwitcherPanel.tsx` |
| Switcher order | starts on the second window | same, most recently used | `electron/modules/windowSwitcherPolicy.js` |
| Tiling | zones left, right, quarters, full; gap 8, margin 0 | same | `electron/modules/tiling.js` |
| Super+arrows | Alpine's transition table | same | `tiling.js` |
| Settings layout | header 40, sidebar 224, nav rows 36 (radius 8, 15 px icons, 10 px gap), page radius 12 with hairline, gutter 20 | same | `components/settings/settingsApp.tsx` |
| Settings colours | ground #202020, surfaces = ground mixed with white 4 % / 14 %, inks 92 % / 56 % / 38 %, hairline white 8 %, ghost white 6 %, track white 18 % | #202020, #292929, #3f3f3f, #ededed, #9d9d9d, #757575 (sRGB approximations of the oklab mixes) | `@alpine/ui` `tokens.css`, `theme.css` |
| Rows and cards | rows min 48 px, 14 × 10 px padding, label 14 px, description 12 px; cards radius 12 | same | `@alpine/ui` `row`, `card` |
| Controls | segmented filled (24 px options, radius 6, 12 px text, active `fill-solid`), slider 8 px, toggle 38 × 22 with a 16 px knob | same | `@alpine/ui` `segmented`, `components.css`, `toggle` |
| Page title | 22 px medium | 22 px | `@alpine/ui` `pageTitle` |
| Quick settings | a control centre dropping from the bar (260 px) | not made: the ridge's status pill opens the settings window's Sound page instead | `topbar/status/controlCenter.tsx` |
| Taskbar | none | none: minimized windows are in the switcher | — |

## Tests

- Host: `poly-dbus` (marshalling by the specification's examples, both
  byte orders, malformed input, messages, streams, authentication,
  routing, match rules, local services, the notification messages);
  `poly-wayland` (the frame's buttons and glyphs; window management:
  maximize and restore by the button with the old size suggested, a double
  click, tiling with the tiled states, minimizing and the focus moving on,
  closing the active window, dragging a maximized window off its place,
  internal windows' events, focus and frames); `poly-shell` (snap
  geometry, drag zones, Super+arrow transitions, switcher order, stepping
  and layout, thumbnails, the notification queue — timeouts, at most
  three, replacing, critical, transient, actions — toasts and the history
  drawn and clicked, time zones, the settings file's round trip and
  forgiving reading, every settings page laid out and drawn at scale 1 and
  2 with its controls hit); `poly-compositor` (the mixer's structures, the
  session bus over a real socket, window management keys, the network and
  `/proc` parsing); `notify-send`'s options.
- Boot group **`desktop-shell`** (shard `userland`): the desktop with a
  virtio-sound card and an ext2 disk on `/mnt`, driven through QMP with a
  screenshot per step: the demo maximized by the frame's button (it draws
  1278 × 750), restored by a double click, tiled left with Super+Left,
  dragged to the right edge (the preview, then the right half), a
  notification from `notify-send` through the bus (the toast), the
  settings window setting the volume to 30 % (the mixer) and the layout to
  Swiss German (the client gets a second keymap), Alt+Tab (the switcher),
  minimized by the frame's button and back with Alt+Tab, closed with
  Alt+F4; then a second boot from the same disk reads the settings and
  starts with the Swiss German keymap.

## Consequences

- The Shell roadmap item is ticked: launcher (ADR 0107), window
  management, notifications and settings with tests.
- `wl-demo` follows configure sizes now (a buffer of each new size).
- Not done: interactive resize by the border; workspaces (optional);
  window menus; keyboard repeat for the shell's keys; hover states of the
  frame's buttons; animations; notification icons, markup, sounds and
  the "do not disturb" state; activation of services from `.service` files
  and descriptor passing on the bus; a quick settings panel (the owner's
  rule keeps it off the ridge; one could float in the middle like the
  history); display modes (with KMS); more time zones or a time zone
  database; a settings search.
