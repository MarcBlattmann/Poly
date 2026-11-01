# 0118: X11 apps through the unmodified Xwayland, an X11 window manager of our own in a host-tested `poly-x11` crate, and X windows as framed scene windows

**Status:** Accepted. Roadmap Phase 6, "XWayland for X11 apps" (ticked).
It builds on the Wayland server core and the software compositor
([ADR 0106](0106-wayland-server-core-and-software-compositor.md)), the
desktop with input routing and the shell
([ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md)), and the root
disk of host programs ([ADR 0059](0059-bash-coreutils-python-gcc-from-the-persistent-disk.md)).
The owner's decisions apply unchanged: X windows get Poly's one neutral
frame, and menus are per window.

## Context

Many Linux programs still speak only X11 (xterm, older toolkits, games,
tools). On a Wayland desktop they run through Xwayland: an X server that
is itself a Wayland client and turns each X top-level window into a
`wl_surface`. In rootless mode Xwayland draws no frames, places nothing
and does not decide focus; that is the job of an X11 window manager that
the compositor runs and that tells the compositor which surface shows
which X window.

Sources, all public: "X Window System Protocol, Version 11" (connection
setup, the requests, replies, errors and events and their encoding in
appendix B, the predefined atoms); the "Inter-Client Communication
Conventions Manual" 2.0; the freedesktop.org "Extended Window Manager
Hints" 1.5; the "Composite Extension" protocol description (version 0.4:
`QueryVersion`, `RedirectSubwindows`); Xwayland(1) and its `-help` (the
options `-rootless`, `-wm`, `-displayfd`, `-listenfd`, `-nolock`,
`-shm`); the `xwayland-shell-v1` protocol XML of wayland-protocols 1.50
(staging), which also describes `WL_SURFACE_SERIAL` and the older
`WL_SURFACE_ID`; unix(7) for the abstract namespace; and Motif's public
`Xm/MwmUtil.h` for the layout of `_MOTIF_WM_HINTS`. No Xorg, Xwayland,
Xlib, XCB, wlroots, Weston, KWin, Mutter or other window manager source
was read (CLEAN_ROOM.md). The unmodified Xwayland binary and X11 programs
were run on Poly only.

## Decision

### Xwayland and X programs on the root disk

As ADR 0059 brings host programs, `tools/xtask/src/xwayland.rs` adds to
`rootdisk::collect` (one call, so other agents' additions stay apart):

- `/usr/bin/Xwayland`, and `xkbcomp` (the server compiles its keymaps
  with it, into `/var/lib/xkb`);
- test programs: `xeyes`, `xclock`, `xdpyinfo`, `xev`, `xterm`, `xprop`,
  `xwininfo`, `xlogo`, `xmessage`;
- their data: `/usr/share/X11/xkb`, X's locale tables and `XErrorDB`,
  `rgb.txt`, `/etc/X11/app-defaults` (Xt programs' resources), the core
  fonts in `/usr/share/fonts/X11/misc` and their encodings, Xwayland's
  `protocol.txt`, terminfo for `xterm*`, and xterm's own desktop file (so
  the launcher lists XTerm);
- every shared object through the root disk's `DT_NEEDED` closure.

They come from Ubuntu's packages `xwayland x11-apps x11-utils xterm
x11-xkb-utils xkb-data libx11-data xfonts-base`, installed in the
`poly-dev` image (Dockerfile) and, for the `userland` shard, in CI. A host
without them gets a root disk without X and the boot test skips. Nothing
is committed.

### `crates/poly-x11`: the X11 wire protocol and the window manager

`no_std` with `alloc`, tested on the host with byte buffers, no I/O:

- **`wire`, `setup`**: little-endian (`'l'`) integers and padding; the
  setup request (11.0, no authorization: Xwayland's `-wm` connection and
  local clients without `-auth` need none) and the three replies
  (`Success` with vendor, resource ID base and mask, screens with root,
  size, visual, depth; `Failed`; `Authenticate`).
- **`request`**: the core requests a window manager makes, encoded as
  appendix B gives them (`CreateWindow`, `ChangeWindowAttributes`,
  `ConfigureWindow` with its 16-bit mask, `InternAtom`, `ChangeProperty`,
  `GetProperty`, `SetSelectionOwner`, `ConvertSelection`, `SendEvent`,
  `SetInputFocus`, `QueryExtension`, `KillClient`, ...; value lists by mask
  bit), any extension request (major opcode from `QueryExtension`, minor
  in the data byte), and the replies it reads.
- **`event`**: the 32-byte events a window manager handles, decoded and
  encoded (`CreateNotify`, `MapRequest`, `ConfigureRequest`,
  `ConfigureNotify`, `PropertyNotify`, `ClientMessage`, the selection
  events, ...), errors, and the split of the server's stream into
  replies (with their length), errors, events (top bit: made by
  `SendEvent`) and `GenericEvent`s (skipped whole).
- **`conn`**: requests numbered from 1, replies and errors matched by the
  16 bits the server echoes (widened by the last request sent), resource
  IDs from base and mask.
- **`atoms`**, **`icccm`**: the predefined atoms, the 39 names the window
  manager interns, and the properties it reads and writes (`WM_NAME` in
  `STRING`, `UTF8_STRING` or the Latin-1 part of `COMPOUND_TEXT`,
  `_NET_WM_NAME`, `WM_CLASS`, `WM_PROTOCOLS`, `WM_HINTS`' input model,
  `WM_NORMAL_HINTS`' size limits, `WM_TRANSIENT_FOR`,
  `_NET_WM_WINDOW_TYPE`, `_MOTIF_WM_HINTS`, `_NET_WM_PID`; `WM_STATE`).
- **`xwm`**: the window manager as a state machine. X bytes in; X requests
  and `Action`s for the compositor out (`Ready`, `Changed(window)`,
  `Destroyed`, `Associate { window, Serial | Id }`, `Activate`, `Move`,
  `Log`); the compositor tells it what the user did (`activate`, `close`,
  `move_to`) and answers placement through a `Policy`.

**What the window manager does** (ICCCM and EWMH):

- At start it interns its atoms, selects `SubstructureRedirect`,
  `SubstructureNotify` and `PropertyChange` on the root (a `BadAccess` is
  "another window manager runs"), and asks for Composite. **Rootless
  Xwayland gives a surface only to windows a compositing manager has
  redirected**: the window manager negotiates Composite 0.4 and sends
  `RedirectSubwindows(root, Manual)` (found on Poly: without it no X window
  ever got a surface). It makes its check window (`_NET_SUPPORTING_WM_CHECK`
  on the root and itself, `_NET_WM_NAME` "Poly"), sets `_NET_SUPPORTED`,
  `_NET_ACTIVE_WINDOW` and `_NET_CLIENT_LIST`, and owns `WM_S0` and
  `_NET_WM_CM_S0` (conversions of them are refused). Events that come
  before its atoms are known wait.
- A **new top-level window** has its properties read and watched.
- A **map request** makes it managed: the `Policy` places its content
  (the compositor's placement; a transient centred over its parent), its
  size is kept within `WM_NORMAL_HINTS` and the screen, its border made 0,
  `WM_STATE` set to `NormalState`, `_NET_FRAME_EXTENTS` to the frame's
  size (1, 1, 25, 1; 0 for undecorated windows), then it is mapped.
- A **configure request** is granted for the size (within the hints), and
  for the position only before the window is managed; one that changes
  nothing gets the synthetic `ConfigureNotify` of ICCCM 4.1.5.
- **Override-redirect** windows (menus, tooltips) are only followed.
- **Unmap** (or a client's synthetic one) withdraws (`WithdrawnState`);
  **destroy** forgets.
- **Focus** follows the ICCCM input models: `SetInputFocus` unless
  `WM_HINTS` says the client takes no input, `WM_TAKE_FOCUS` for clients
  in that protocol; the window is raised in X and named in
  `_NET_ACTIVE_WINDOW`. When a Wayland window takes the keyboard, X focus
  goes to `None`.
- **Close**: `WM_DELETE_WINDOW` to clients that take part in it,
  `KillClient` otherwise.
- **Client requests**: `_NET_ACTIVE_WINDOW` (activate) and
  `_NET_WM_MOVERESIZE` with `_NET_WM_MOVERESIZE_MOVE` (a move with the
  pointer).
- **Frames**: a window is framed unless it is override-redirect,
  `_MOTIF_WM_HINTS` asks for no decorations, or its first known window
  type is not normal, dialog or utility.

The window manager does not reparent: Poly draws frames in the compositor
(server side, ADR 0106), so the X window's position is its content's, in
root coordinates, which are the output's logical ones.

### The Wayland side: `xwayland_shell_v1` and X windows in the scene

`xwayland-shell-v1.xml` is committed unmodified under
`third_party/wayland-protocols/xwayland-shell/`, pinned by SHA-256 and
generated into `crates/poly-wayland/src/protocol/xwayland_shell.rs` (the
module is named after the file because its protocol and its first
interface share a name). `crates/poly-wayland/src/server/xwayland.rs`:

- `xwayland_shell_v1` is offered only to the client the compositor
  started as Xwayland (`Server::set_xwayland_client`), as the protocol
  asks: others do not see it in the registry and get an error if they
  bind it anyway. `get_xwayland_surface` gives a surface the
  `xwayland_surface` role (`role` error for a surface with another one);
  `set_serial` is double-buffered (`invalid_serial` for 0,
  `already_associated` for a second committed serial).
- **Association from both sides.** The window manager passes
  `Server::x_associate(window, Serial | Id)` and
  `Server::x_update(window, state)` (geometry, mapped, managed,
  override-redirect, framed, title, transient-for); a surface's commit
  completes what is waiting, whichever side came first. The legacy
  `WL_SURFACE_ID` names a surface by object ID; it takes the role then.
- **Shown** while associated, mapped and with a buffer: a scene window at
  the X window's position, with the content of the surface's last buffer
  (committed before the association), framed by Poly when the window
  manager says so, override-redirect windows on top and unframed,
  transients above their parent. A managed window that appears takes the
  focus.
- **The seat and the frame.** Pointer focus and buttons go to Xwayland's
  surfaces like any client's. A click on a managed X window activates it:
  the keyboard focus is its surface (the active-window slot holds it, as
  it holds an xdg toplevel), and `XRequest::Activate` asks the window
  manager for X's focus; an xdg toplevel taking the focus sends
  `Deactivate`. The close button sends `Close`, dragging the frame moves
  the scene window and sends `Move` (late `ConfigureNotify`s are ignored
  while the window is dragged). `x_start_move` serves
  `_NET_WM_MOVERESIZE`.

Hooks in the existing server are one line each (the commit, a destroyed
surface, focus on click, the close button, a moved window, the keyboard
focus target, a removed client), so the XWM stays a module of its own
beside the compositor work going on in `poly-wayland`.

### `poly-compositor --xwayland` / `--xwayland-lazy`

`userland/poly-compositor/src/xwayland.rs`:

- **The display**: the first free number `n` (a lock file
  `/tmp/.X<n>-lock` with our pid, ten characters wide, replacing stale
  ones), `/tmp/.X11-unix/X<n>` (the directory made with mode 1777) and
  `@/tmp/.X11-unix/X<n>` in the abstract namespace, made and listened on by
  the compositor. Launched apps (the launcher, `--exec`) get `DISPLAY`.
- **Starting Xwayland**, at once (`--xwayland`) or when the first X client
  connects (`--xwayland-lazy`; the connection waits in the listen queue),
  and again after it ended (at most five starts): `Xwayland :n -rootless
  -nolock -shm -displayfd <pipe> -wm <socketpair> -listenfd <path socket>
  -listenfd <abstract socket>`, its Wayland connection one end of a socket
  pair (`WAYLAND_SOCKET`). `-nolock` because the compositor holds the
  lock; `-shm` because there is no GL for glamor. The descriptors it needs
  have close-on-exec cleared in the child (`unix::spawn_keeping`). When it
  writes its display number to the pipe, it is ready.
- **The window manager** runs on the other end of the `-wm` pair, fed from
  `poll` like every client; its actions go to the server and the user's
  requests back, in one loop until both are quiet. Its `Policy` places
  windows with the scene's placement (centred on the desktop) and dialogs
  over their parent within the work area.
- Launched programs without a `/` are now found in `PATH`, as desktop
  files name them (`Exec=xterm`); before, only absolute paths ran.
- `POLY_XWM_DEBUG` prints every window manager action and request.

### What Xwayland needed from Poly

Nothing new: Xwayland, xkbcomp, xdpyinfo, xeyes, xclock, xev and xterm ran
on Poly's Linux personality unchanged, with no `not implemented` system
call (AF_UNIX path and abstract sockets with descriptor passing, socket
pairs, `memfd_create` with seals, `MAP_SHARED`, `poll`/`epoll`, pipes,
SysV shared memory for MIT-SHM, ptys for xterm were all there). The one
gap was in our own window manager (the Composite redirect above).

### The boot test

Group `xwayland` (shard `userland`, about 70 s on TCG): the root disk with
`/usr/bin/poly-compositor` and desktop files for xeyes and xclock, booted
through BusyBox's init into the disk's bash, with the desktop's virtio
GPU, keyboard and tablet and `poly.terminal=off`, driven through QMP:

1. `poly-compositor --desktop --xwayland-lazy` in the background: the
   desktop comes up and `:0` listens on its two sockets;
2. `xdpyinfo`, the first X client, starts Xwayland on demand; it exits 0
   and reports display `:0`, "The X.Org Foundation", 1280x800 and the
   extensions XKEYBOARD, RENDER, Composite and MIT-SHM; `xprop` finds the
   window manager "Poly" (`_NET_SUPPORTING_WM_CHECK`, `_NET_SUPPORTED`);
3. `xeyes` typed into the launcher starts from its desktop file; its window
   has Poly's frame in the active colours, its eyes drawn; it is
   `_NET_ACTIVE_WINDOW` and in `_NET_CLIENT_LIST`;
4. `xclock` from the shell: framed, active, its face drawn; xeyes' frame
   turns inactive;
5. `xev` gets the X focus (`FocusIn`), a key sent through QMP arrives as
   `KeyPress` keycode 38 (`a`), the pointer as `MotionNotify`;
6. dragging xev's title bar moves it, and `xwininfo` reports the new place;
7. the close buttons end xev, xclock and xeyes (status 0), their frames
   go, `_NET_CLIENT_LIST` is empty;
8. `xterm` from the launcher (its own desktop file): `touch /tmp/xt` typed
   into it runs in its shell; its close button ends it.

Host tests: the wire format (setup, every request's bytes, events both
ways, the stream split, sequence widening, resource IDs), the properties,
the window manager against a scripted X server (start, Composite, map
requests, configure requests, focus models, closing, withdraw and
destroy, client messages, selections, errors), the server's
`xwayland_shell_v1` (only for Xwayland, both association orders, the
legacy ID, role and serial errors, override-redirect, focus between X and
Wayland windows, the close button and drags, transients, unmapping and
losing Xwayland), and the compositor's placement policy and lock files.

## Consequences

- X11 apps run on Poly's desktop with Poly's frame, keyboard and pointer,
  started from the launcher by their desktop files or from a shell.
- **Not done yet:**
  - **Clipboard and drag and drop** between X and Wayland: needs
    `wl_data_device_manager` in the compositor first; then the window
    manager bridges `CLIPBOARD`/`PRIMARY` (XFIXES selection events,
    `INCR` for large transfers) to `wl_data_device`.
  - `_NET_WM_STATE` (fullscreen, maximize) and `WM_CHANGE_STATE`
    (iconify) are ignored; no interactive resize (neither for Wayland
    windows yet).
  - Shaped windows (xeyes) show their whole rectangle: Xwayland's buffers
    of depth-24 windows are opaque outside the shape.
  - HiDPI: X windows are drawn at scale 1; an output scale above 1 would
    need Xwayland's scaling and coordinate conversion.
  - Xwayland's own cursor surfaces (`wl_pointer.set_cursor`) are not shown,
    as for every client (ADR 0107).
  - GL in X clients: Xwayland runs with `-shm`; with Mesa and DRM in place,
    glamor (and `linux-dmabuf`) can be tried.
- `poly-x11`'s request and event coverage is what a window manager needs;
  the selection bridge will add XFIXES and property transfers.
