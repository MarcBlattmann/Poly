# 0145: The desktop follows the size of the window that shows it: the display event while DRM has the screen, a DRM hotplug event, KMS mode changes in the compositor, and windows laid out for the new work area

**Status:** Accepted. Fixes what the owner saw: resizing QEMU's window
(`cargo xtask desktop`, `cargo xtask start`, `cargo xtask chromium run`;
Windows host, WHPX, SDL, virtio-gpu) left the desktop at its boot size.
Builds on [ADR 0102](0102-virtio-gpu-2d-display.md) (the virtio-gpu driver and
its display event), [ADR 0108](0108-drm-kms-device-on-virtio-gpu.md) (the DRM
device, which listed hotplug as not done), [ADR 0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md)
(the compositor on KMS, which listed hotplug as not done),
[ADR 0117](0117-window-management-notifications-and-settings.md) (window
management) and [ADR 0140](0140-damage-limited-page-flips-and-a-single-scanout-resource.md)
(the scanout mirror).

## Context

A Linux guest on virtio-gpu follows its window like this. The host's display
(QEMU's SDL or GTK window, a VNC client) tells the GPU device the size it
shows; the device raises a configuration-change interrupt with
`VIRTIO_GPU_EVENT_DISPLAY` set in `events_read` (VIRTIO 1.2 §5.7.4); the
driver acknowledges it by writing the bits to `events_clear` and asks
`VIRTIO_GPU_CMD_GET_DISPLAY_INFO` (and `VIRTIO_GPU_CMD_GET_EDID`) again
(§5.7.6.8); the DRM device reports a hotplug and a compositor reads the
connector again, sets the new mode and lays itself out.

Checked link by link, on a Windows host with QEMU 11.1, WHPX and `-display sdl`:

1. **QEMU offers resizes with no option.** The SDL window is resizable, and
   when it is, QEMU's virtio GPU reports the new size in `GET_DISPLAY_INFO`
   and raises the event. Nothing in `desktop::devices()` or
   `window_display()` had to change (`virtio-gpu-pci,xres=..,yres=..` gives
   the size the device starts with and prefers until a window says otherwise).
2. **The kernel's driver already read the event**, but only when the
   *console* flushed (`Display::poll_gpu_events`, ADR 0102). With the
   compositor on DRM the console yields the display and never flushes, so
   nobody read `events_read`. **Missing.**
3. **The DRM device re-probes the hardware when a client asks about a
   connector again** (`GETCONNECTOR` with no mode space), but nothing made a
   client ask, and Linux's hotplug signal (a udev event over netlink) does not
   exist here. **Missing.**
4. **The compositor opened KMS once**: one connector, one mode, two dumb
   buffers of the boot size, forever (`Kms::open`). **Missing.**
5. **The Wayland server and the shell took a new size only for a scale
   change** (`Server::set_output`, `Shell::resize`), and then left windows
   alone: a maximized or tiled window kept its old size, a floating one could
   end up off the screen. **Missing**; `wl_output` and `Scene::set_output`
   already described and drew a new size.

## Decision

- **The kernel reads the display event while DRM has the screen.**
  `display::take_display_event_for_drm` takes and acknowledges the device's
  event when the console has yielded (the console still follows the change
  itself otherwise). A `drm-display` thread asks every 100 ms. The event is
  polled, as ADR 0102 chose for every other one (the device's configuration
  interrupt, MSI-X vector for `msix_config`, would save the wake-ups; left for
  a time when something else needs it).
- **A DRM hotplug event.** `Device::display_changed` probes the hardware again
  (connected, size, EDID: the connector's modes and preferred mode change) and
  queues a 32-byte event of type `0x8000_0001` (`event::HOTPLUG`, number,
  length, sequence) on every file of the primary node; `read`, `poll` and the
  wait queues work as for flip events. Linux has no event of that sort on the
  card's file (a udev event announces a hotplug); the DRM documentation leaves
  types from `0x8000_0000` up to drivers, and a client that does not know one
  skips it by its length, as libdrm's event loop does. The hardware's report
  is what queues it, not a comparison with what the device knew (another client
  may have probed in between and must not make the compositor miss it), and a
  file that has not read the last one gets no second, since it probes afresh
  when it reads.
- **The compositor follows.** `Kms::dispatch` notes the hotplug event;
  `Kms::follow_display` reads the connector again, and if its preferred mode is
  another size, makes two dumb buffers of the new size, sets the mode on the
  first (`SETCRTC`) and releases the old ones; on any error the screen stays
  as it was. The main loop reads the card's file whenever it is readable (not
  only while a flip is pending) and follows 100 ms after the first event, and
  not while a flip is pending, so a drag over many sizes costs a mode set
  about ten times a second, not one per step. It then calls
  `Server::set_output` (`wl_output` `geometry`, `mode`, `scale`, `done` to
  every bound output; `preferred_buffer_scale` and fractional scale to the
  surfaces), `Shell::resize` (the ridge, the launcher, the wallpaper at the new
  size) and redraws everything. It logs `screen resized to WxH at R Hz`.
  The scale does not change (it is the user's choice, and a window made
  bigger on the same screen has the same density).
- **Sane sizes.** The screen the compositor follows to is kept between 640x400
  and 5120x5120 (`kms::clamped`): a size outside gets a generated CVT
  reduced-blanking mode of the nearest size in range at the same refresh rate,
  so a window dragged to a sliver still has a desktop that lays out, and a huge
  one does not ask for gigabytes of dumb buffers (QEMU shows the picture scaled
  if its window is another size). A mode set or buffer allocation that fails
  leaves the screen as it was. Rapid resizing costs a mode set about ten times a
  second at most (the 100 ms settle), never one per step, and never while a flip
  is in flight.
- **Without virtio-gpu nothing changes.** The bochs/dispi fallback has no DRM
  device, so the compositor runs on the framebuffer at the size it booted with
  (`take_display_event_for_drm` ignores any other output; the kernel's console
  follows a change itself while it owns the display). A fixed size there is
  fine and nothing in this change touches that path.
- **Windows follow the work area.** `Server::set_output` lays the managed
  windows out again when the work area changes (also for a scale change, which
  did not before): maximized and fullscreen toplevels get a configure for the
  new area; a tiled toplevel's rectangle keeps its share of the work area (a
  right half stays a right half) and is configured the same way; the
  compositor's own windows are placed at once and their owner told; floating
  windows keep their size and place and are shifted only as far as it takes to
  have their frame on the work area (a window larger than the area keeps its
  top left); the place a snapped window returns to is kept reachable; the
  pointer is clamped to the new output.
- **A QEMU VNC client as the automated resize.** QEMU's VNC server hands a
  client's `SetDesktopSize` (RFB, the ExtendedDesktopSize extension) to the
  display device as the size to take, which is the same thing a window does,
  so `tools/xtask/src/rfb.rs` (a handshake with no authentication,
  `SetEncodings`, `SetDesktopSize`; RFC 6143) lets a boot test resize the
  screen on a Unix host without a window. Boot group `desktop-resize`.

## Verification

- Host tests: `poly-drm` (the event, coalescing, render nodes get none, a probe
  by another client does not lose it, a client following with new buffers and
  a mode, events in one queue with flips), `poly-wayland` (maximized, tiled,
  fullscreen, internal, floating windows and the pointer on a resized output;
  `wl_output` mode and geometry; an unchanged work area configures nothing),
  the compositor's KMS backend against `poly-drm`'s device (follows to a
  larger and a smaller size, buffers and framebuffers released, waits for a
  flip, out of memory changes nothing, the picture draws the new size, the
  desktop lays out again; sizes outside the limits are clamped, also through a
  follow) and `rfb.rs` (message bytes, handshake errors).
- Boot group `desktop-resize` (Unix hosts): a maximized demo window fills the
  work area of 1100x700, then of 1500x900, the ridge across the width each
  time; restored by a double click routed by the new size; on a 700x500 screen
  the floating window is shifted onto it; then a burst of six sizes in a row
  ends on the last (1280x800, taken in fewer mode sets than sizes), and a
  320x240 request gets the smallest screen, 640x400, with the ridge across it.
  The group is in the `userland` shard. The VNC client waits for QEMU's answer
  to its `SetDesktopSize` before it goes away: on a loaded host (TCG) a request
  from a client that left at once was sometimes never carried out (one run in
  three failed before, none in eight after). QEMU's result code in that answer
  is not a yes or no (a request it carried out came back with 4), so only that
  the answer came counts.
- By hand, on the owner's setup (Windows 11, QEMU 11.1, WHPX, `-display sdl`,
  `cargo xtask desktop --release`, a main screen of 2880x1800): the guest
  followed the real window. A window resize by `SetWindowPos` to 1500x900
  client pixels gave `drm: the display configuration changed` and
  `poly-compositor: screen resized to 1500x900`, and a maximized window (by
  the user: client area equal to the work area) gave 2880x1707. The size
  settled there and stayed (the window and the guest do not chase each other).
  VNC resizes to 1024x640, 1600x900, 900x600 and 1313x877 all took. The
  screenshots show the ridge, launcher and wallpaper laid out for the new size.

## Not done

- **GTK on Linux hosts.** `desktop::window_display` leaves the display to QEMU
  there (GTK); whether QEMU's GTK window reports its size to the guest depends
  on its `zoom-to-fit` setting and was not tried (only SDL on Windows, VNC).
- **The configuration-change interrupt** (a polled event costs a wake-up every
  100 ms and up to that in latency).
- **Windows apps** (`/dev/win32k`'s desktop link) are told the output once, at
  start; they do not hear of a new size.
- **Xwayland**'s screen size (RandR) is not updated.
- A drag that fires many sizes is sampled (the first change, 100 ms later);
  the screen is black from the mode set until the next frame, a frame or two.
- More than one output, and a connector that goes away (the compositor keeps
  the old mode and logs the error).
