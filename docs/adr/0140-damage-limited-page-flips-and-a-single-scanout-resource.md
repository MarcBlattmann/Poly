# 0140: Damage-limited page flips: `DIRTYFB` on the back buffer, one scanout resource on the virtio GPU, and cached window shadows

**Status:** Accepted. Fixes the lag the owner saw moving windows on the
desktop (`cargo xtask desktop`, QEMU with WHPX, SDL, 2560x1600 at scale 2).
Builds on [ADR 0108](0108-drm-kms-device-on-virtio-gpu.md) (the DRM device),
[ADR 0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md) (the compositor's KMS backend and
its double buffer) and [ADR 0127](0127-the-alpine-shell-look.md) (the look,
which this does not change).

## Context

Dragging a window ran at 33 frames per second on average at 1280x800 and 14 at
2560x1600 scale 2 (the compositor's own `--stats`, release build, WHPX). A
frame had two costs, both proportional to the whole screen although only the
window and its shadow changed (about 16 % of the pixels):

1. **Composing.** Every damaged rectangle drew the window's soft shadow from
   scratch: a coverage mask of the rectangle plus the blur's reach, then
   three box blurs both ways in `f32`. 10 ms at scale 1, 39 ms at
   scale 2.
2. **Presenting.** A `PAGE_FLIP` uploaded and flushed the whole screen
   (`TRANSFER_TO_HOST_2D` of 4 or 16 MB), and, because the two buffers are two
   virtio resources, switched the scanout from one to the other every frame.
   Measured through `DIRTYFB` on the shown buffer at scale 2: a full upload
   and flush took about 8 ms and a 16x16 one about 5 ms (a round trip to
   the host), but the flip took 20 to 30 ms; the difference is the scanout
   switch (QEMU replaces its display surface for another resource; that is the
   likely cause, not something that was traced into QEMU). 15 ms at scale 1,
   24 ms at scale 2.

A flip completes at the next (virtual) vblank, so a frame that takes more than
one vblank period (13.3 ms at the mode's 75 Hz) waits for the next one: a 25 ms
frame is two periods, a 62 ms frame five.

## Decision

- **Shadows are blurred once per size.** `Shadow::draw_cached` keeps the
  blurred opacity of a box of a given size (and the shadow's numbers and the
  corners) in a small least-recently-used cache in the scene (six entries,
  none past 4096 x 4096 pixels). A shadow does not depend on where its box is,
  so moving, redrawing in pieces or damaging part of it only blends from the
  cache. The picture is the one `Shadow::draw` makes (a test compares them,
  within one level of a channel from float rounding).
- **The client says what it drew.** `DRM_IOCTL_MODE_DIRTYFB` on a framebuffer
  that no output shows is remembered (`poly_drm::damage`). A flip to that
  framebuffer uploads only those rectangles, and flushes only those where the
  hardware keeps one picture across scanout switches
  (`Hardware::scanout_keeps_picture`). The contract is `DIRTYFB`'s: the clips
  cover everything changed in the buffer since it was last shown. A client that
  tells nothing, and a buffer never shown, get all of it copied, as before.
  The compositor tells the device what `begin_frame` copied into the back
  buffer plus what the frame drew, merged to at most three rectangles (each is a
  round trip to the host, which costs more than a few more pixels).
- **One scanout resource on the virtio GPU.** The kernel shows plain 2D
  framebuffers through a `Mirror`: a resource and guest memory of the screen's
  size, scanned out for good. The damage of the framebuffer flipped to is copied
  into it (a `memcpy` of the rectangles) and sent with one `TRANSFER_TO_HOST_2D`
  plus `RESOURCE_FLUSH` notification. Framebuffers on 3D resources (Mesa) are
  scanned out as before.

## Consequences

- Drag, at 2560x1600 scale 2: 14 to 64 frames per second; compose 39 to 3 ms,
  present 24 to 6 ms. At 1280x800: 33 to 60; compose 10 to 0.8 ms, present 15 to
  3.7 ms. After, a frame is shorter than a vblank period and the rate is what the
  drag's input gives (about 60 to 70 pointer events a second in the measurement),
  not what drawing costs.
- A 2D flip of a client that does not use `DIRTYFB` costs a guest copy of the
  whole framebuffer into the mirror plus the transfer, instead of the transfer
  alone; it no longer switches the host's display surface.
- The mirror is one more screen of kernel memory (16 MB at 2560x1600).
- The host's `screendump` and its window show the same pixels (compared on the
  Windows host with SDL).
- A flip's vblank is now the first one after the hardware finished
  programming it (`Hardware::now_ns`): under TCG a copy of a screen takes
  longer than a vblank, which made the `drm` boot test's "a second flip while
  one is pending" see the first flip already done.
