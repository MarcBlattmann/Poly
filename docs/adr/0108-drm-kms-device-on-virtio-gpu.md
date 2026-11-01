# 0108: A Linux DRM/KMS device on the virtio GPU: a host-tested core with one atomic state behind legacy and atomic modesetting, dumb buffers mapped as device frames, PRIME dma-bufs, a virtual vblank clock per CRTC, and the console yielding to the DRM master

**Status:** Accepted (the 2D/KMS part of roadmap Phase 6's "Kernel GPU
interface compatible with Linux DRM/KMS so Mesa runs unchanged"; the item
stays open, see "What Mesa still needs"). Builds on
[ADR 0102](0102-virtio-gpu-2d-display.md) (the virtio-gpu driver and its
"DRM/KMS can sit on top later" plan), [ADR 0041](0041-processes-threads-fork-exec-wait.md)
(shared frames) and [ADR 0042](0042-readiness-pipes-poll-epoll-and-event-files.md) (file
objects, readiness).

## Context

Everything graphical on Linux above the kernel (Mesa's GBM and EGL
platforms, Wayland compositors, Xorg's modesetting driver, `modetest`,
SDL's KMSDRM backend) talks to the GPU through DRM: ioctls on
`/dev/dri/card<n>` and `/dev/dri/renderD<n>`, `read(2)` of events and
`mmap(2)` of buffers. Phase 6 needs that interface so the desktop's
userland runs unchanged, first for 2D (KMS and dumb buffers), then 3D
(virgl / Venus, the next roadmap item).

Sources: the MIT-licensed public uapi headers `drm/drm.h`,
`drm/drm_mode.h` and `drm/drm_fourcc.h` (numbers, layouts), the kernel's
public DRM documentation ("Userland interfaces": primary and render nodes,
DRM master, the ioctl argument size rules, "KMS Properties", "Dumb Buffer
Objects", "PRIME Buffer Sharing"), the dma-buf documentation (`SEEK_END`,
`DMA_BUF_IOCTL_SYNC`), the sysfs ABI documentation, libdrm's documented
behaviour (two-pass array queries), VESA E-EDID 1.4 and CVT 1.2, and VIRTIO
1.2 §5.7. No Linux, libdrm or Mesa source was read (CLEAN_ROOM.md).

## Decision

**A host-tested core, `crates/poly-drm`, behind a `Hardware` trait.**
Like the virtio crates it reads no clock and owns no memory: every entry
point takes "now", the kernel's `Hardware` implementation provides buffers
and device commands, and the tests drive the whole ioctl interface the way
libdrm does against a fake hardware that records what it is asked:

- `uapi`: the request numbers (`_IOC(dir, 'd', nr, size)`), every
  structure as a field list at the header's offsets (`ModeInfo`,
  `CardRes`, `GetConnector`, `FbCmd2`, `Atomic`, ...), the constants
  (caps, client caps, object types, property flags, page-flip and atomic
  flags, events) and the fourccs. Arguments follow the documented size
  rule: the kernel copies `min(user size, kernel size)` in (zero-extended)
  and out, so older and newer callers both work; unknown DRM requests are
  `EINVAL`, another ioctl type `ENOTTY`.
- `modes`: modes from the EDID's detailed timing descriptors (the first
  one preferred), else a CVT reduced-blanking mode of the device's
  preferred size, plus common sizes that fit (each size once); the device's
  preferred size wins over an EDID that disagrees (a resized window
  changes the display info first). Validation: a clock, a size up to
  8192, timings in order, no interlace.
- `Device`: the objects and the ioctls.

**Objects.** Output (virtio scanout) `i` is CRTC `i`, encoder `i`
(VIRTUAL, `possible_crtcs` = bit `i`), connector `i` (VIRTUAL,
`Virtual-<i+1>`, connected if the display info says enabled) and primary
plane `i` (XRGB8888, ARGB8888, XBGR8888, ABGR8888; linear only). All
objects, properties and blobs share one ID space. Properties are the
documented standard ones (`EDID`, `DPMS`, `link-status`, `non-desktop`,
`CRTC_ID`, `ACTIVE`, `MODE_ID`, `type`, `FB_ID`, `SRC_*`, `CRTC_*`,
`IN_FORMATS`), atomic ones listed only to clients with
`DRM_CLIENT_CAP_ATOMIC`; planes only with universal planes. A probe
(`GETCONNECTOR` with `count_modes` 0) asks the hardware again.

**One state, one commit.** `SETCRTC`, `SETPLANE`, `PAGE_FLIP`,
`SETPROPERTY` (DPMS turns the CRTC off and on), `OBJ_SETPROPERTY` and
`MODE_ATOMIC` all build a new state (per CRTC `ACTIVE` and the mode, per
plane framebuffer, CRTC, source and destination rectangles, per connector
its CRTC) and go through one commit: check it as a whole (an active CRTC
has a mode, a connector and its primary plane showing a framebuffer over
the whole mode, inside the framebuffer, unscaled: `ENOSPC` outside the
framebuffer, `ERANGE` for scaling), require `ALLOW_MODESET` when a CRTC's
activity, mode or connectors change, honour `TEST_ONLY`, refuse a
non-blocking commit (and a legacy flip) on a CRTC with one pending
(`EBUSY`; a blocking one waits), then program the hardware only for
outputs whose picture changed or that the commit named. A mode set by
legacy `SETCRTC` gets a blob the device owns, so `MODE_ID` always reads
something; a user's mode blob that the state uses survives its
`DESTROYPROPBLOB` until the state lets go. Removing a framebuffer that is
shown turns its CRTC off (legacy `RMFB` semantics).

**Vblank: a virtual clock per CRTC.** The virtio GPU has no vblank
interrupt. Each active CRTC counts vblanks on CLOCK_MONOTONIC, one per
frame of its mode (`clock * 1000 / (htotal * vtotal)` Hz; 75 Hz for
QEMU's EDID mode, 60 Hz for CVT ones), starting when it is enabled. A flip
or commit takes effect on the device at once (it is synchronous), and its
flip-complete event (`struct drm_event_vblank` with the CRTC ID,
`DRM_CAP_CRTC_IN_VBLANK_EVENT`) becomes readable at the next vblank, which
is also when the CRTC stops being busy; `PAGE_FLIP_TARGET_*` aims at a
later one. `WAIT_VBLANK` with `_DRM_VBLANK_EVENT` queues a vblank event;
without, it returns `Reply::Sleep` and the kernel sleeps and asks again
with the request rewritten to an absolute one (what Linux does for
restarted waits). `CRTC_GET_SEQUENCE` / `CRTC_QUEUE_SEQUENCE` work the same
way. Each file holds at most 4 KiB of events (`ENOMEM` beyond); `read`
returns whole events (`EAGAIN` when none is readable, `EINVAL` if the
buffer cannot hold one). A `drm-vblank` kernel thread sleeps until the
next event is due and wakes its file's readers (`poll`, blocking `read`).

**Files, master, render nodes.** The first file opened on the card node
without a master is master; `DROP_MASTER` gives it up, `SET_MASTER` takes
it from nobody for a former master or a `CAP_SYS_ADMIN` caller (`EBUSY`
while someone has it). Modesetting ioctls need master (`EACCES`). A
render-node file may use only `VERSION`, `GET_CAP`, `GEM_CLOSE` and PRIME
(`EACCES` for the rest, dumb buffers included). `GET_MAGIC` /
`AUTH_MAGIC` are recorded but nothing requires authentication.

**GEM objects and dumb buffers.** A dumb buffer is physically contiguous
frames, zeroed, its pitch `width * bpp / 8` rounded to 4 (the stride the
device's copy of a framebuffer uses), whole pages, at most 256 MiB. Each
object counts its references (handles in any file, framebuffers,
dma-bufs); handles are per file and unique per object (importing the same
buffer again returns the same handle, as the PRIME documentation requires).
`MAP_DUMB` returns a fake offset (object ID << 32); `mmap` of the card file
at that offset maps the object if the file has a handle to it (`EACCES`
otherwise). Global names (`GEM_FLINK` / `GEM_OPEN`) exist for old clients.

**Mappings map the device frames themselves.** `SharedFrame` (ADR 0041)
gains a second kind: a page of device memory that an `Arc` of its owner
keeps allocated. A mapping of a dumb buffer or a dma-buf faults in those
frames (`PageSource::shared_frame`), never a copy, so what the client
draws is what the device copies; such a frame is never taken over as a
memory's own (a private mapping copies it on a write, and a shared region
writes it in place). The buffer's memory outlives the GEM object while
mapped. The services' `mmap` asks `drm::mapping` before refusing device
files and objects; `CharDevice::as_any` lets it find the DRM file behind
an open file.

**PRIME.** `PRIME_HANDLE_TO_FD` makes a dma-buf file object holding the
object and its memory (`O_CLOEXEC`, `O_RDWR` honoured); `lseek(fd, 0,
SEEK_END)` gives its size (`FileObject::seek_size`, new), `mmap` maps the
frames, `poll` is always ready (no fences), `DMA_BUF_IOCTL_SYNC` checks its
flags and succeeds (the memory is coherent). `PRIME_FD_TO_HANDLE` accepts
this device's dma-bufs. Closing a file or a dma-buf where the device's lock
is held (or sleeping is not allowed) defers the work to the next locker or
the vblank thread.

**The hardware: virtio-gpu resources per framebuffer.** `ADDFB`/`ADDFB2`
makes virtio resource `0x1000 + fb ID` of `pitch / 4` x `height` pixels in
the matching virtio format (XRGB8888 is `B8G8R8X8`, ...), its backing one
scatter-list entry at the buffer plus the framebuffer's offset. Showing it
is `TRANSFER_TO_HOST_2D` of the visible rectangle, `SET_SCANOUT` (the
rectangle is the CRTC's viewport: `SETCRTC`'s x, y work), `RESOURCE_FLUSH`;
`DIRTYFB` uploads and flushes each clip on the outputs that show the
framebuffer; `RMFB` detaches and unrefs (if that fails the memory is never
freed). The commands go through `display::with_gpu`, under the display's
lock, with the driver ADR 0102 made: the console display keeps owning the
`VirtioGpu`, and DRM borrows it (resource IDs above the console's 1 and 2).
That is simpler than ADR 0102's suggestion to move the driver into DRM and
keeps the console independent of `/dev/dri`.

**The console yields to the DRM master.** When the device first shows a
framebuffer it calls `display::console_yield`: the console keeps drawing
into its back buffer, but `display::flush` sends nothing to the device and
display-change events wait. When the master's file closes (or the last
file), the device turns off every output it drove and calls
`display::console_restore`, which plans the console's scanouts again
(following any display change), shows its resource and flushes all of it.
`DROP_MASTER` leaves the display as it is (the next master sets its own
modes). Anything else that draws through `display::flush` (the screen
terminal, the fbdev `/dev/fb0` the UI work adds) yields the same way with
no code of its own; a compositor that wants the screen opens card0 and
becomes master.

**sysfs.** `/sys/dev/char/226:{0,128}`, `/sys/class/drm/{card0,renderD128}`
and the PCI function under `/sys/devices/pci0000:00/` (`vendor`,
`device`, `subsystem_*`, `class`, `revision`, `uevent` with
`PCI_SLOT_NAME`, `subsystem` -> `bus/pci`), as the sysfs ABI documents
them, so that device enumeration (libdrm's `drmGetDevices2`, Mesa's
loader) finds the PCI ID 1af4:1050.

**Tests.** 23 host tests (request numbers against the header values,
structure offsets, CVT against VESA's published timings, EDID parsing, two-
pass enumeration, properties, dumb buffers and their mappings,
framebuffer checks, `SETCRTC`, page flips and their events and `EBUSY`,
vblank waits, the 4 KiB event limit, `DIRTYFB` clips, `RMFB` of a shown
buffer, atomic commits with `TEST_ONLY`, `ALLOW_MODESET`, blobs, events and
blocking, DPMS, master rules, the console's return on close, render nodes,
PRIME reference counting, flink, argument size rules, `EIO`). Kernel
tests: the nodes and their sysfs, and the hardware path on the real device
(a buffer shown instead of the console, which comes back). Boot group
`drm`: those kernel tests, then `tests/linux/drm/drm_kms.c` as init (raw
ioctls through the uapi headers, no libdrm: enumerate, two dumb buffers,
`ADDFB2`, `SETCRTC`, a page flip with an event read after `poll`, a
`DIRTYFB` of front-buffer drawing, blocking and event vblank waits, an
atomic flip with a blocking `read`, PRIME with `SEEK_END` and a dma-buf
mapping, a render node, a second file that is not master, clean-up); at
each step QMP screenshots of the scanout are compared pixel for pixel with
the same pattern drawn by the test, and after `close` with the boot screen
the console shows again.

## What Mesa still needs

The KMS / dumb-buffer interface Mesa's GBM and EGL platforms use (caps,
enumeration, dumb buffers with mmap, `ADDFB2` with modifiers and
`GETFB2`, atomic and legacy flips with events, PRIME export and import,
sysfs device discovery) is in place, but Mesa has not been run on it yet:
no image has Mesa, libdrm or `modetest`. The roadmap item stays open
until a boot test runs Mesa (e.g. `kmscube` or an EGL-on-GBM client with
its software rasteriser, `kms_swrast` / `LIBGL_ALWAYS_SOFTWARE=1`). For
that and beyond, Mesa still needs:

- **The virgl/Venus driver interface** (next roadmap item): Mesa picks its
  driver by the DRM name `virtio_gpu` and PCI ID; its virgl and Venus
  drivers call the virtio-gpu driver ioctls (`DRM_IOCTL_VIRTGPU_GETPARAM`,
  `GET_CAPS`, `RESOURCE_CREATE(_BLOB)`, `MAP`, `EXECBUFFER`, `WAIT`,
  `TRANSFER_TO/FROM_HOST`, `CONTEXT_INIT`), which answer `EINVAL` now, so
  hardware GL/Vulkan fails and only the software path can work.
- **Sync objects** (`DRM_CAP_SYNCOBJ` is 0): Vulkan WSI and explicit-sync
  Wayland need `SYNCOBJ_*`; fences on dma-bufs (`poll` is always ready).
- **A cursor plane** (`MODE_CURSOR` answers `ENXIO`, so compositors draw
  the cursor themselves) and **overlay planes / scaling**.
- **Hotplug uevents** (no netlink `KOBJECT_UEVENT` yet): a display change
  is seen only when a client probes a connector again.
- **Real vblank** on the device's own timing, and flips taking effect at
  the vblank instead of at once; gamma tables (`gamma_size` 0); leases.

## Consequences

- `/dev/dri/card0` and `renderD128` exist whenever the display is on a
  virtio GPU; a KMS client takes the screen from the console and gives it
  back when it closes.
- Every flip uploads its whole visible rectangle (no damage tracking
  across flips): a few milliseconds per frame under TCG, acceptable for a
  compositor's first light; `FB_DAMAGE_CLIPS` would cut it.
- The console display still owns the virtio driver; a future 3D driver
  adds commands to the same queue machinery through `with_gpu` or takes
  the driver over, which ADR 0102 left open.
- Not done (besides the list above): multiple framebuffer planes (YUV),
  formats other than 32-bit RGB, `CLOSEFB`, `GETFB` handles for
  non-masters (they get 0), interruptible blocking vblank waits.
