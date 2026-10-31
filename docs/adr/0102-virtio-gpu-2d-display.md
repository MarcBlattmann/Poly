# 0102: virtio-gpu (2D): a host-tested driver crate, synchronous fenced commands, one framebuffer resource mirrored on every scanout, the display's back buffer as its guest memory, and polled display-change events

**Status:** Accepted. Roadmap Phase 6's "virtio-gpu driver: modesetting
and 2D framebuffer". Builds on [ADR 0015](0015-pci-and-virtio-in-the-kernel.md)
(PCI and virtio in the kernel), [ADR 0009](0009-drivers-behind-access-traits.md)
(drivers behind access traits) and [ADR 0019](0019-display-framebuffer-and-drawing.md)
(the display, its back buffer and `poly-gfx`), which it extends with a
second output.

## Context

Phase 6 needs a display device the desktop can grow on: modesetting
without firmware, more than one display, mode changes when a window
resizes (QEMU's GTK window, a remote viewer), and later 3D (virgl /
Venus) and a Linux DRM/KMS interface for Mesa. QEMU's virtio GPU
(`virtio-gpu-pci`) is that device. The Bochs VBE device of ADR 0019
stays for machines without one.

The source is the OASIS VIRTIO 1.2 specification, §5.7 "GPU Device":
§5.7.2 virtqueues, §5.7.3 feature bits, §5.7.4 the device configuration
(`events_read`, `events_clear`, `num_scanouts`, `num_capsets`) and its
display event, §5.7.5 initialisation, §5.7.6.1-§5.7.6.2 creating,
showing and updating a framebuffer, §5.7.6.4 multihead, §5.7.6.5 fencing,
§5.7.6.7 the request header and §5.7.6.8 the 2D commands. EDID blocks
follow VESA E-EDID 1.4 (base block only). No other driver's source was
read (CLEAN_ROOM.md).

## Decision

**A crate of its own, `poly-virtio-gpu`, on `poly-virtio`.** Like
virtio-blk and virtio-net, the device class keeps its protocol to itself
and allocates nothing; the kernel gives it the queue memory, an 8 KiB
command area and a clock. It has four parts:

- `protocol`: the wire format. `Command` encodes `GET_DISPLAY_INFO`,
  `GET_EDID`, `RESOURCE_CREATE_2D`, `RESOURCE_UNREF`, `SET_SCANOUT`,
  `RESOURCE_FLUSH`, `TRANSFER_TO_HOST_2D`, `RESOURCE_ATTACH_BACKING` (up to
  126 scatter-list entries) and `RESOURCE_DETACH_BACKING`;
  `decode_response` checks a response's type, fence and length and parses
  display info and EDID blobs.
- `edid`: the vendor, product, version, monitor name and preferred
  timing of a base block, with the header and checksum checked.
- `modes`: the policy. The framebuffer's size is the command line's
  (`poly.display=<w>x<h>`), else the preferred mode of the first enabled
  scanout, else the default 1280x800, and never more than 2560x1600 (memory).
  One resource is shown on every enabled scanout ("single framebuffer"
  multihead, §5.7.6.4): the primary scanout shows all of it, the others
  mirror the top-left corner that fits their own preferred size. A new
  plan says whether the framebuffer must be made again (another size)
  and which scanouts to turn off.
- `VirtioGpu`, the driver: initialisation (VIRTIO_GPU_F_EDID, EVENT_IDX and
  ACCESS_PLATFORM accepted; VIRGL, blobs, UUIDs and context init never),
  the control queue, the cursor queue left disabled, and one method per
  command plus `show` (create, attach, set the scanouts; destroy again on
  failure), `update` (transfer + flush), `destroy` (detach + unref) and
  `take_events`.

It is tested on the host against a simulated GPU written from the spec's
device requirements (resources as host images, scatter-list backing,
per-scanout "screens" that `RESOURCE_FLUSH` updates, error responses,
display events, a hung device, wrong fences and used IDs).

**Synchronous, fenced, polled commands.** The console draws with
interrupts off (the display's spinlock, ADR 0019), so the driver cannot
sleep until an interrupt says the device answered. Every command sets
VIRTIO_GPU_FLAG_FENCE with a fresh ID and the driver polls the used ring
until the response comes, which by §5.7.6.5 means the command is
complete; the response must echo the fence. A transfer and its flush go
out together in one notification (two chains, four descriptors: the
whole queue). A device that does not answer within 2 s, or that breaks
the queue protocol, is given up on: it still owns the buffers in flight,
so the driver refuses every further command and the memory is never
reused. No interrupts are set up (no MSI-X vectors, INTx disabled).

**The display's back buffer is the resource's guest memory.** The display
keeps its interface (`with_canvas`, `flush`, `draw`, `info`), and with it
everything that draws (the boot screen, the screen terminal). Behind the
back buffer there is now an output: the Bochs VBE framebuffer, which
`flush` copies into, or a virtio-gpu resource whose backing *is* the back
buffer (physically contiguous frames, so one scatter-list entry, in
B8G8R8X8, which is `poly-gfx`'s XRGB8888), which `flush` turns into
`TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` of the damaged rectangle. No
copy in the guest at all. A virtio GPU wins over the VGA when both are
present; the VGA is the fallback, also when the virtio GPU cannot be
brought up.

**Display-change events are polled at flush time.** Each flush reads
`events_read` (one 32-bit register read) before updating. On
VIRTIO_GPU_EVENT_DISPLAY it acknowledges the event (`events_clear`),
fetches the display info again and plans again: the same size only
re-sets the scanouts (a display added or removed); another size makes a
new back buffer and resource (ID 1 and 2 alternate), copies what was
drawn into its top-left corner, shows it everywhere the plan says, then
destroys the old resource and frees its frames once the device let go
(fenced detach). The screen terminal keeps its text grid for now and
draws into the new buffer clipped; laying the terminal out again on a
resize is left to the terminal. With nothing flushing (`poly.terminal=off`
and no other drawing) events wait until the next flush.

**DRM/KMS can sit on top later.** The driver deals in resources and
scanouts and knows nothing of the console: a dumb buffer or framebuffer
object is a resource with backing, a CRTC/connector pair is a scanout
(`GET_DISPLAY_INFO` and EDID give the modes and the connector's
properties), a mode set or page flip is `SET_SCANOUT`, a damage clip or
`DIRTYFB` is `update`, and the display event is a hotplug uevent. The
next roadmap item ("kernel GPU interface compatible with Linux DRM/KMS")
should own the `VirtioGpu` and let the console display become one client
(its framebuffer one more resource), rather than the console owning the
device as now. Resource IDs, the single-scanout-per-resource
mirroring and the 2-command batch are the console's choices, not the
driver's. 3D (virgl / Venus: contexts, capsets, blob resources, the
cursor queue) adds commands to the same queue machinery.

**Boot test.** Boot group `virtio-gpu` starts QEMU with `-vga none
-device virtio-gpu-pci,xres=1152,yres=864` and no `poly.display=`: the
kernel must take 1152x864 from `GET_DISPLAY_INFO` (logged with the EDID),
pass the display tests on it (a pattern drawn and flushed as one accepted
update of its rectangle; a switch to a larger mode and back that keeps
the picture; the generic drawing tests, which check the update rectangle
where they cannot read the screen back), and then show the boot screen,
which a QMP `screendump` of the virtio GPU's scanout compares pixel for
pixel with `poly_gfx::boot_screen` at that size. `cargo xtask run
--virtio-gpu` and `cargo xtask screenshot --virtio-gpu` use a virtio GPU
instead of the VGA.

## Consequences

- The kernel has a display with modesetting from the device's preferred
  mode, several displays (mirrored), and mode changes on display events;
  the screen terminal runs on it unchanged.
- Each flush costs two commands and a notification, polled, and the
  device copies the rectangle on the host side (not measured against the
  VBE copy yet). If it shows up in profiles, updates can be batched
  further (several rectangles before one notification) without changing
  the interface.
- QEMU's headless display cannot raise a display change, so the resize
  path runs in the boot test only through a forced re-plan, and the event
  itself only in the host tests against the simulated device.
- Not done: the cursor queue (`UPDATE_CURSOR` / `MOVE_CURSOR`), per-scanout
  resources (extended desktops), page flipping, EDID extension blocks,
  interrupts for responses (once something wants to sleep while the GPU
  works, e.g. a DRM client), 3D.
