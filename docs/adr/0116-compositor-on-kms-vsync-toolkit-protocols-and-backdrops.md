# 0116: The compositor on DRM/KMS with page-flip vsync, the protocols toolkits need, and the picture behind each window

**Status:** Accepted. Completes roadmap Phase 6's "Compositor: Wayland
server, surface composition, HiDPI scaling, vsync" and "Compositor exposes
the content behind each window (needed later for Liquid Glass)", both
ticked. Builds on [ADR 0106](0106-wayland-server-core-and-software-compositor.md)
(the server core and software compositor),
[ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md) (the desktop on
`/dev/fb0`, input routing, the shell) and
[ADR 0108](0108-drm-kms-device-on-virtio-gpu.md) (the DRM/KMS device).
Owner decisions kept: one neutral frame drawn by Poly; per-window menus
(the ridge is a status strip).

## Context

ADR 0107 put the compositor on the screen through fbdev with a 60 Hz
timer: no page flips, so no vsync, and frame callbacks timed by the
compositor rather than the display. ADR 0108 then gave Poly a DRM/KMS
device with dumb buffers, page flips, flip-complete events at a virtual
vblank, and PRIME dma-bufs. Toolkits (GTK, Qt; another agent brings them)
need more of Wayland than ADR 0106 had: subsurfaces, the clipboard and drag
and drop, viewports, buffer transforms, popups that stay on the screen and
grab, interactive resize, cursor surfaces, and Mesa's EGL needs
`zwp_linux_dmabuf_v1`. Phase 10's Liquid Glass needs each window's
backdrop.

Sources, all public: the protocol XML files as data (wayland 1.26.0;
wayland-protocols 1.50: xdg-shell, viewporter, presentation-time,
linux-dmabuf, and newly fractional-scale-v1, xdg-activation-v1 and
cursor-shape-v1, unmodified under `third_party/`, pinned by SHA-256 as ADR
0106 describes), the MIT DRM uapi headers through `poly-drm`'s `uapi`, the
kernel's public DRM documentation (KMS, dumb buffers, page-flip events,
PRIME), presentation-time.xml's clock and flag semantics. No wlroots,
Weston, Mutter, KWin, Smithay, libwayland, libdrm, Mesa or Linux source was
read (CLEAN_ROOM.md).

## Decision

### KMS backend (`userland/poly-compositor/src/kms.rs`)

- **Taking the screen.** The compositor opens `/dev/dri/card0` (`--drm`);
  the first opener is DRM master (ADR 0108). `probe` lists the resources
  (two-pass queries), takes the first connected connector with a mode, its
  preferred mode (else the first), and a CRTC its encoders can drive.
- **Two dumb buffers** (`CREATE_DUMB` 32 bpp, `MAP_DUMB`, `mmap`), each an
  `ADDFB2` XRGB8888 framebuffer; `SETCRTC` shows the first. The scene draws
  into the back buffer. Before a frame, the back buffer gets what it missed
  — the rectangles the previous frame drew into the other buffer — copied
  from the front buffer (cheaper than drawing them again); then the frame's
  damage is drawn and `PAGE_FLIP` with `DRM_MODE_PAGE_FLIP_EVENT` queues it.
- **Real vsync.** No frame is drawn while a flip is pending; the main loop
  polls the card's file, reads the flip-complete event (`struct
  drm_event_vblank`) and only then draws the next frame. The frame clock is
  split in `poly-wayland`: `Server::frame_submitted` takes the frame
  callbacks and presentation feedback committed for the frame that went to
  the display, `Server::frame_completed` answers them with the event's time
  (CLOCK_MONOTONIC) and vblank sequence and the feedback flags `vsync`,
  `hw_clock` and `hw_completion`. Callbacks committed while a flip is
  pending wait for the next frame. Clients waiting with nothing new to draw
  are paced by a flip of an unchanged buffer. `frame_presented` (both at
  once) stays for fbdev and off-screen.
- **Fallback.** Without a card, a connected connector or a mode,
  `--desktop` falls back to `/dev/fb0` as before (`--no-kms` forces it).
  `--kms` puts the compositor on the screen without the shell (what the
  protocol boot test uses).
- **HiDPI.** The output's integer scale is `--scale N`, else
  `POLY_DISPLAY_SCALE`, else, on KMS, the screen's density from the
  connector's physical size: pixels per inch over 96, rounded, 1 to 4 (QEMU's
  EDID says 100 dpi: scale 1). The kernel passes its parameter
  `poly.display.scale=` to the first program as `POLY_DISPLAY_SCALE`
  (`kernel/src/program.rs`: the only kernel change). The output's refresh
  (from the mode's timings) and physical size go to `wl_output` and
  presentation feedback.
- **Testing.** All of the backend goes through a `Drm` trait (`ioctl`,
  `mmap`, `read`); the host tests run it against `poly-drm`'s `Device` —
  the code Poly's kernel runs — in the test process, with the device's
  vblank clock moved by the test: the mode set, master, the console
  yielding, one flip at a time, events only at the vblank, consecutive
  flips one refresh period and one sequence number apart, the back buffer
  brought up to date.

### Protocols (`crates/poly-wayland`, all host-tested)

- **`wl_subcompositor` / `wl_subsurface`.** A subsurface is shown when it
  has a buffer and its parent is shown, recursively, as a scene window that
  is part of its parent's window: the scene now has window families
  (`Scene::set_parent`): a family moves and is raised together, and members
  can be restacked directly above or below a sibling or the parent itself.
  Position, stacking and adding a subsurface are double-buffered state of
  the parent. Synchronized subsurfaces (the default, and anything inside
  one) cache their content updates until the parent's state is applied;
  turning desynchronized applies the cache. Destroying the role object or
  the parent unmaps at once. A surface's content lives in its window when
  it has one, else in the surface itself, so a buffer is still copied once.
- **`wp_viewporter`** and **buffer transforms.** Content carries a `View`
  (transform, source rectangle in 1/256 pixels, destination size); the
  renderer samples through it (nearest neighbour), keeping the pixel-for-
  pixel path when there is none. Transforms follow wayland.xml: the client
  applied the transform, flip around the vertical axis first, then 90°
  steps counter-clockwise; the compositor undoes it. `out_of_buffer`,
  `bad_size`, `bad_value`, `viewport_exists`, `no_surface` are raised.
- **`wl_data_device_manager`** (version 3). The selection goes to the
  client with keyboard focus as a server-made `wl_data_offer`, when it is
  set and whenever a client gets the focus, before `wl_keyboard.enter`;
  `receive` passes the client's pipe to the source's client as `send` (the
  bytes go between the clients). Drag and drop: `start_drag` during an
  implicit grab on the origin, `enter`/`motion`/`leave` with offers and
  `source_actions`, `accept` → `target`, `set_actions` negotiated (the
  destination's preference if both allow it, else copy, move, ask), drop or
  cancel on release, `finish` → `dnd_finished`, the icon drawn at the
  pointer; a drag without a source stays in its client.
- **`zwp_linux_dmabuf_v1`** (version 4). Offered when the host can map
  dma-bufs (on KMS). One plane, the 32-bit RGB formats, `MOD_LINEAR` and
  `MOD_INVALID`; version 4's feedback names the render node's `dev_t` and
  one tranche through a format table file (a sealed memfd), versions up to
  3 get `format`/`modifier`. A dma-buf is mapped (`lseek(SEEK_END)` gives
  its size, ADR 0108) and copied at commit like shared memory, `y_invert`
  included. `create` and `create_immed`, and the params errors.
- **xdg-shell.** Popups: constraint adjustment against the work area
  (flip kept only if it helps, slide, resize, per axis), recomputed at each
  configure and on `reposition`; popups are part of their parent's window
  family; `xdg_popup.grab` gives the topmost grabbing popup the keyboard and
  pointer focus within its client, and a press outside the client
  dismisses every grabbing popup (`popup_done`, top down); destroying one
  under another is `not_the_topmost_popup`. Interactive resize
  (`xdg_toplevel.resize`, or a press on the frame's border): configures
  with the size and the `resizing` state while the button is held, keeps
  the opposite edge in place on commit, a last configure on release.
- **Cursors.** `wl_pointer.set_cursor` surfaces (role `cursor`) replace
  Poly's arrow while the pointer is over their client (an offset moves the
  hot spot; no surface hides the cursor); `wp_cursor_shape_device_v1`
  shapes show the arrow (Poly has one cursor image yet); the arrow comes
  back when the pointer goes to another client.
- **Input regions** decide which surface the pointer is over.
- **`wp_fractional_scale_v1`** (the output's integer scale in 120ths) and
  **`xdg_activation_v1`** (tokens used once, the newest 32 kept,
  `Server::activation_token` for apps the shell starts).
- **Formats.** `wl_shm` adds `abgr8888` and `xbgr8888`.

### The picture behind each window

`compose::Backdrop`: for a window that asks (`Scene::set_backdrop`; for a
surface, `Server::set_surface_backdrop`), the composed picture of what lies
behind it — the layers below and the windows under it, not the window
itself — in physical pixels over the window's extent (frame included),
kept up to date as frames are drawn. It costs no extra drawing: the
renderer draws each damaged rectangle bottom to top, so just before it
draws the window the framebuffer holds exactly its backdrop there, and that
part is copied. `Scene::take_backdrop_damage` / `Server::surface_backdrop`
say which parts changed since last asked (what a blur must redo): nothing
when changes are elsewhere; when the window moves or changes size the
picture follows it whole. A material like Liquid Glass (Phase 10) samples
and blurs it; whether it is handed to clients through a private protocol
is left for then.

### An extension point for XWayland

`Server::manage_surface` (and `unmanage_surface`, `set_managed_title`,
`move_managed`, `managed_window`, `take_wm_events`) lets a window manager
outside xdg-shell — XWayland's X11 window manager — make surfaces windows
of the scene (role `external`): shown with or without Poly's frame once
they have content, placed like toplevels unless positioned, focused and
given the keyboard like them; the frame's close button becomes
`WmEvent::Close`. Associating X11 windows with surfaces
(`xwayland_shell_v1`) and the X11 side are the XWayland work's. The
compositor's interface to `poly-shell` is unchanged.

### Boot tests

- **`wayland-kms`** (new, shard `userland`): `poly-compositor --kms` as
  init on a 1280×800 virtio GPU starts `tests/linux/wayland/wl-proto.c`
  (the wire protocol and DRM ioctls by hand, no libwayland or libdrm),
  which goes through five scenarios, each checked by the client against the
  compositor's picture and by the test against a QMP screenshot (colour
  pixel counts), the client waiting for the test's answer on the serial
  console:
  1. **vsync**: 60 frames, each with a frame callback and presentation
     feedback: all presented with `vsync|hw_clock|hw_completion`, after
     their commit and before the client heard of them, a whole number of
     refresh periods apart with vblank sequences counting them, never
     faster than the display (on TCG: 59 of 60 at the very next vblank of
     the 75 Hz mode);
  2. **subsurface**: a synchronized subsurface shown with its parent's
     commit, whole, above it;
  3. **popup**: a positioner that would put the popup past the right edge,
     kept on the screen by the constraint adjustment, shown whole;
  4. **dmabuf**: a dumb buffer of `/dev/dri/card0` (the client is not
     master) filled, exported with `PRIME_HANDLE_TO_FD`, imported with
     `create_immed`, composed;
  5. **clipboard**: a second client (forked) maps a window, gets the focus
     and with it the selection offer, pastes through a pipe; the first
     client's source writes the text.
  The compositor's report (`N flips in T ms (R per second)`) must show at
  least 60 flips at no more than the mode's refresh.
- **`desktop`** now runs on KMS (the log says `desktop on /dev/dri/card0
  (KMS)`); `wayland` (off-screen), `drm` and `virtio-gpu` are unchanged and
  pass.

## Consequences

- Linux GUI programs get a compositor paced by the display, with the
  protocol surface GTK 3/4 and Qt 5/6 use on Wayland (they still draw their
  own decorations inside Poly's frame where they ignore xdg-decoration, ADR
  0106).
- Every flip uploads the whole visible rectangle to the virtio GPU (ADR
  0108): fine at 1280×800 on TCG (about 30 flips a second while redrawing
  every frame); `FB_DAMAGE_CLIPS` would cut it.
- The fbdev path is no longer exercised by a boot test (the desktop takes
  KMS); `fbdev::*` kernel tests still cover `/dev/fb0`, and `--no-kms`
  selects it.
- Not done: fractional output scales (the scene's scale is an integer; a
  fractional scale is offered as the integer one), multiple outputs,
  hotplug, a cursor plane (the cursor is drawn into the frame), explicit
  sync, more cursor images than the arrow, keyboard grabs for drags, touch,
  primary selection, `wl_surface.get_release`, a private backdrop protocol.
