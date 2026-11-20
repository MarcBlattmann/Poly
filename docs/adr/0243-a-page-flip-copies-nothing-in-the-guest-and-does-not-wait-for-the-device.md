# 0243: A page flip copies nothing in the guest and does not wait for the device

**Status:** Accepted. Track `display-present`, after chrome-paint-3 ([ADR 0232](0232-a-frame-draws-only-what-shows.md))
made composition cheap and left the present to the display as the slowest step of a frame. Builds on
[ADR 0102](0102-virtio-gpu-2d-display.md) (the virtio-gpu driver), [ADR 0108](0108-drm-kms-device-on-virtio-gpu.md)
(the DRM device, its mirror resource and `DIRTYFB` damage) and [ADR 0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md)
(the compositor's double-buffered KMS output).

## Context

What a page flip of a plain 2D framebuffer did, measured in the kernel (new `poly.drm.stats`, below) on WHPX at
2560x1600 and 200 % with Chrome for Windows (chrome-paint-3's driver and scenarios):

| Scenario | Present (compositor) | Copy into the mirror (CPU) | Waiting for the device | Bytes per flip |
|---|---|---|---|---|
| Scroll (61 % damage) | 7.1–7.6 ms | 1.2–1.3 ms | 5.7–6.1 ms | 10 MB |
| Whole-page animation (72 %) | 8.7–9.5 ms | 1.5–1.7 ms | 7.0–7.6 ms | 12 MB |
| Typing in a text field (1–2 %) | 2.4–3.4 ms | < 0.05 ms | 1.5–2.3 ms | 0.2–0.4 MB |

The flip was already proportional to the damage (ADR 0108's `DIRTYFB` clips; one rectangle per flip in these
scenarios). The time went into two things:

- **A guest copy.** The damage was copied from the framebuffer into the mirror's own memory, then transferred
  from there.
- **Waiting.** The ioctl spun until QEMU had transferred and flushed: about 1.5 ms per flip plus about 0.45 ms per
  MB. The compositor could do nothing else meanwhile, and at 75 Hz a 3 ms compose plus a 7 ms present missed
  vblanks.

## Decision

1. **The mirror has no memory of its own.** Its backing is the framebuffer being shown. A flip to the other
   framebuffer swaps it (`RESOURCE_DETACH_BACKING` and `RESOURCE_ATTACH_BACKING` in one notification, new driver
   call `replace_backing`); the host's copy keeps the previous frame, and the rectangles that changed are
   transferred from the new backing. A flip copies nothing in the guest. When a framebuffer that backs the mirror
   is removed, the mirror lets go of it first; the host keeps showing its copy.
2. **Each rectangle is sent as it is, once.** The mirror keeps a list of pending rectangles (at most 16, then
   their bounding box), not one bounding box. Each gets a `TRANSFER_TO_HOST_2D` and a `RESOURCE_FLUSH` (new driver
   call `update_rects`, up to 8 pairs per notification). A flush of a rectangle the batch already covered sends
   nothing again. Before, the first flush sent the bounding box of every clip and each further clip was sent
   again.
3. **The flip does not wait for the device.** The transfers are posted (`post_update_rects`: queued, the device
   notified, the responses not collected; an error still counts in the driver's statistics). The next send waits
   for the previous one first (`wait_posted`), and a backing swap is answered only after everything before it
   (the control queue is answered in order). So the device is at most one frame behind, the compositor composes
   the next frame while the device works, and nothing writes a buffer the device still reads: the compositor
   draws into the other buffer, and a flip back to this one swaps the backing first.
4. **`poly.drm.stats`** (set by `cargo xtask desktop --compositor-stats`) logs every three seconds what the
   present path cost: sends, rectangles, bytes, how long a send blocked, backing swaps.

Kept as it was: the flip-complete event comes at the CRTC's next virtual vblank (ADR 0108). It does not wait for
the transfer, which is at most one frame behind. The `DIRTYFB` contract is unchanged ("everything changed since
the buffer was last shown"), and 3D framebuffers are untouched.

## Consequences

Same driver, scenarios and machine, steady state:

| Scenario | Present before → after | Compositor fps before → after | Key → frame before → after |
|---|---|---|---|
| Scroll (61 % damage) | 7.1–7.6 → 1.3–1.7 ms | 31–34 → 39–44 | 22–25 → 14–18 ms |
| Whole-page animation (72 %) | 8.7–9.5 → 2.1–2.8 ms | 31–38 → 40–49 | (no keys) |
| Omnibox typing | 4.5–6.3 → 1.4–1.8 ms | (key-bound) | 44–50 → 32–36 ms |
| Text field typing | 2.4–3.4 → 1.1–1.4 ms | (key-bound) | 20–33 → 11–13 ms |

- What is left of the present is mostly waiting for the previous frame's transfer. Allowing two frames in flight
  would hide that too, at the price of a display one more frame behind; not done.
- The host's display still copies each damaged rectangle twice: guest memory to QEMU's image, then QEMU's image to
  its window. Removing those copies needs a shared-memory display path in QEMU (blob resources with `udmabuf`),
  which the Windows build does not offer.
