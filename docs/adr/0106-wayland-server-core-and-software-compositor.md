# 0106: A Wayland server core in a host-tested crate, protocol types generated from the XML, a software compositor with one neutral server-side frame, and `poly-compositor` on the Linux personality

**Status:** Accepted. The groundwork of roadmap Phase 6's "Compositor:
Wayland server, surface composition, HiDPI scaling, vsync" (not ticked:
this is the part that needs no GPU and no input driver). Records the
owner's decision on window frames: **one neutral frame drawn by Poly,
server-side decorations for every app** (the ⚖️ item "native window frames
per app, or one neutral frame"). The other open decision, a global menu
bar or per-window menus, is still open; nothing here depends on it.

## Context

Phase 6 wants Linux GUI apps (GTK, Qt) on a compositor. Linux apps talk
Wayland: a client connects to a Unix socket, sends requests on objects as
binary messages, and passes shared memory as file descriptors
(`SCM_RIGHTS`). Poly already has what that needs underneath: AF_UNIX
sockets with descriptor passing (ADR 0048), `MAP_SHARED` mappings of tmpfs
files (`/dev/shm`, ADR 0064), `poll`, signals, fork and exec. The
virtio-gpu framebuffer and the input stack are being built by other
agents at the same time, so the compositor must not depend on either yet.

Sources, all public: the protocol XML files (`wayland.xml` of wayland
1.26.0; `xdg-shell.xml`, `xdg-decoration-unstable-v1.xml`,
`viewporter.xml`, `presentation-time.xml`, `linux-dmabuf-v1.xml` of
wayland-protocols 1.50), which are the specification; "The Wayland
Protocol" book (wire format, object IDs, file descriptor passing, the
Wayland display socket); unix(7) and cmsg(3) for `SCM_RIGHTS`; mmap(2);
netpbm's PPM description. No libwayland, wlroots, Weston, Smithay or other
compositor source was read (CLEAN_ROOM.md); the XML files are data.

## Decision

### Layout

- **`crates/poly-wayland`** (`no_std` + `alloc`, host-tested): the wire
  format, the generated protocol types, the server (objects, dispatch,
  protocol logic) and the compositor core (scene, renderer, frame). It
  touches no socket, descriptor or mapping: the caller passes each
  client's bytes and descriptors in (`Server::receive`), sends what
  `Server::take_output` returns, and implements `server::Host`
  (`map_shm`, `close_fd`). Everything is therefore tested with byte
  buffers and test memory; the kernel could use it too.
- **`userland/poly-compositor`**: the program for the Linux personality, a
  Rust `std` binary (with the `libc` crate) linked statically against the
  host's glibc for Poly (`-C target-feature=+crt-static`, not PIE). It is
  the first Linux userland program of the repository that is ours rather
  than a test, hence the new top-level `userland/` directory (a workspace
  member, so it is linted and tested on the host like the rest).
- **`tests/linux/wayland/wl-client.c`**: a C client speaking the wire
  protocol itself (no libwayland), in a subdirectory so the diff runner,
  which compares `tests/linux/*.c` with real Linux, does not pick it up.

### Protocol types from the XML

The XML files are committed unmodified under `third_party/wayland/` and
`third_party/wayland-protocols/` (MIT, with their `COPYING`;
`THIRD_PARTY.md`), pinned by SHA-256 in `tools/xtask/src/wayland.rs`, as
the 7-Zip downloads are pinned (ADR 0087). `cargo xtask wayland` (a small
XML reader of its own; no XML crate in the tools) generates one module per
protocol into `crates/poly-wayland/src/protocol/`, checked in like the
font tables (`cargo xtask font`); `cargo xtask ci lint` runs it with
`--check`. Per interface: `NAME`, `VERSION`, an `INTERFACE` table (each
request and event by opcode with its `since`, destructor flag and typed
arguments), one `u32` newtype per `<enum>` with a constant per entry
(bitfields and enums alike, so unknown values survive), and `Request` and
`Event` enums with `opcode`, `from_args` and `into_args`, so both sides of
the protocol are typed (the tests use the request side as a client). Only
`summary` attributes become doc comments.

### Wire and dispatch

`wire::decode_args` reads a body against a `MessageDesc` (strings with
their NUL and padding, null objects and strings only where `allow-null`,
`new_id` without an interface as interface name, version and ID,
descriptors from a queue in order) and rejects malformed input without
panicking. The server handles every complete message a client sent: the
object must exist (`wl_display.error` `invalid_object`), the opcode exist
and be no newer than the object's version (`invalid_method`); every
`new_id` must be an unused client ID and every `object` exist with the
interface the XML names. A handler's protocol error goes out as
`wl_display.error` and the client is cut off after what is queued is
sent. Destructor requests remove the object and send `delete_id`; events
newer than the object's version are not sent.

### Surfaces and buffers

Double-buffered state (buffer, surface and buffer damage, frame callbacks,
scale, transform, offset, opaque and input regions, presentation
feedback) applies on `commit`. **A committed SHM buffer is copied** — the
damaged part, all of it if size or scale changed — into the window's
content in the scene, converted to premultiplied ARGB (`xrgb8888` made
opaque; `argb8888` is premultiplied already, wayland.xml), and released at
once. Clients may thus reuse one buffer, and the scene never reads client
memory while composing: only the copy at commit does (`ShmMap::read`,
bounds-checked against the mapping). A client that shrinks its file below
the pool's size could still make that read fault (`SIGBUS`); catching that
is left for when real applications run.
Frame callbacks are answered with the next frame (`Server::frame_presented`,
time in milliseconds); `wp_presentation` feedback is `presented` (with
the output's refresh) or `discarded`. Globals: `wl_compositor` 6, `wl_shm`
2, `wl_output` 4, `wl_seat` 7, `xdg_wm_base` 6,
`zxdg_decoration_manager_v1` 2, `wp_presentation` 1.

### xdg-shell

The configure sequence as xdg-shell.xml has it: the initial commit without
a buffer gets `preferred_buffer_scale`, `configure_bounds`,
`wm_capabilities` (maximize, fullscreen; once), the decoration mode if one
is due, `xdg_toplevel.configure` and `xdg_surface.configure`; a buffer
before an ack is `unconfigured_buffer`, an unknown or reused serial
`invalid_serial`. The first buffer maps the window, a null one unmaps it
and the sequence starts over. Toplevels are placed in a cascade and get
focus (`activated`); a click raises and activates (the previously active
toplevel is reconfigured), maximize and fullscreen are configured with
the output's logical size (less the frame) and applied once acked.
Popups are placed by their positioner (anchor point, gravity, offset; no
constraint adjustment yet) above their parent and dismissed with it. Role
errors (`role`, `defunct_role_object`, `not_constructed`,
`defunct_surfaces`, `invalid_positioner`, ...) are raised.

### The frame: server-side for everyone

The owner chose one neutral frame drawn by Poly for every app.
`zxdg_decoration_manager_v1` always answers `server_side`, whatever the
client asks (`set_mode`, `unset_mode`; the protocol lets the compositor
enforce a mode), and toplevels of clients that never ask are framed too.
The frame (`compose::frame`) is grey only: a 1-pixel border, a 24-pixel
title bar with the title (8×16 Spleen from `poly-gfx` behind a
`TextRenderer` trait, so `poly-font` can replace it) and a close button
whose click sends `xdg_toplevel.close`. Content in a frame is clipped to
the window geometry (xdg-shell allows clipping to it). Consequence:
clients that draw their own decorations regardless (GTK 4 does not
support xdg-decoration) show a title bar inside ours until something is
done about it; that is the price of the owner's choice, and it is
visible, not broken.

### Composition and HiDPI

The scene holds windows bottom to top with a position (logical pixels),
content (buffer pixels at a buffer scale), title, active state and
geometry, and the damage: physical rectangles to draw again (from content
damage, moves, raises, title and focus changes; collapsed to their
bounding box past 16). `Scene::render` draws only those, each as
background, then every window's frame and content clipped to it, and
calls `Framebuffer::present` with them. Logical coordinates are multiplied
by the output's integer scale; a buffer whose scale equals the output's is
copied pixel for pixel, others scaled by nearest neighbour. Blending is
premultiplied `src + dst × (1 − α)`, rounded. `Framebuffer` is rows of
`xrgb8888` (`width`, `height`, `row_mut`, `present`): what virtio-gpu's
scanout resource or a boot framebuffer can implement;
`MemoryFramebuffer` is the off-screen one, with `to_ppm`.

### The seat

`seat::InputSource` (capabilities, events) and `seat::InputEvent` (pointer
motion, buttons, axes, keys, in evdev codes) are what the input stack
plugs into. `wl_seat` announces its name and capabilities and creates
`wl_pointer`/`wl_keyboard`/`wl_touch` objects only for capabilities it
ever had (`missing_capability`); they get no events yet.
`Server::handle_input` does the frame's part: a left click raises and
activates, on a close button it closes.

### `poly-compositor`

Listens on `$XDG_RUNTIME_DIR/wayland-0` (else `/run/user/<uid>`, made
with mode 0700), polls the listener and the clients, one blocking
`recvmsg` per readable client (descriptors from `SCM_RIGHTS`), `mmap`s
pools read-only and shared, renders into a `MemoryFramebuffer` at up to
60 frames a second when something changed or a client waits for a frame,
and sends what the server queued. `SIGUSR1` writes the picture as a PPM
(renamed into place when complete) and prints the windows and `--probe`d
pixels. `--exec` starts a client with `WAYLAND_DISPLAY`,
`XDG_RUNTIME_DIR` and `POLY_COMPOSITOR_DUMP` set and ends with its status.

### The boot test

`wayland` (shard `userland`, `tools/xtask/src/compositor.rs`): the
compositor as the first process (`poly.init.arg=` words) with
`--exec /bin/wl-client`. The client binds the globals, makes a toplevel
with a server-side decoration, acks, shares a 64×48 buffer from a file in
`/dev/shm` over `SCM_RIGHTS`, commits, waits for the frame callback and
the release, asks for a picture with `SIGUSR1` and counts its pixels in
it, and checks that a request to an unknown object is an error. Skipped
without a C compiler or static C library.

## Consequences

- The compositor item's remaining parts: the virtio-gpu `Framebuffer`
  (and page flips: real vsync, frame callbacks and presentation timed by
  the display instead of a 60 Hz timer), the input stack behind
  `InputSource` and its routing (`wl_pointer` enter/motion/button with
  serials, `wl_keyboard` keymaps, focus, interactive move and resize,
  popup grabs), `wl_subcompositor`, `wl_data_device_manager` (clipboard,
  drag and drop), `wp_viewporter`, buffer transforms, popup constraint
  adjustment, `linux-dmabuf` (with the DRM/KMS work), fractional scaling,
  more than one output, and `memfd_create` (not implemented on Poly yet:
  libwayland-based clients fall back to `$XDG_RUNTIME_DIR` files, the test
  client uses `/dev/shm`). The generated types for viewporter and
  linux-dmabuf are there already.
- The server owns no I/O, so the same core can sit behind another
  transport (in the kernel, or for the NT personality's win32k handing
  surfaces to the compositor, Phase 7).
- Updating a protocol: replace the XML in `third_party/`, update its
  SHA-256 and `THIRD_PARTY.md`, run `cargo xtask wayland`.
