# 0119: 3D on the virtio GPU: a ticketed control queue with MSI-X fences, the `DRM_IOCTL_VIRTGPU_*` interface in `poly-drm`, sync files and host-visible blobs in the kernel, and unmodified Mesa's virgl and Venus drivers checked on QEMU's OpenGL display

**Status:** Accepted. Roadmap Phase 6's "3D acceleration: virtio-gpu with
Venus (Vulkan) / virgl (OpenGL) through Mesa" (ticked). Builds on
[ADR 0102](0102-virtio-gpu-2d-display.md) (the virtio-gpu driver, which
said 3D "adds commands to the same queue machinery") and
[ADR 0108](0108-drm-kms-device-on-virtio-gpu.md) (the DRM device, whose
"What Mesa still needs" named the virgl/Venus ioctls first).

## Context

Mesa drives a virtio GPU with two drivers of its own: **virgl** (OpenGL and
GLES; Gallium command streams that the host's virglrenderer replays on its
OpenGL) and **Venus** (Vulkan; Vulkan command streams that virglrenderer
replays on the host's Vulkan). Mesa picks them by the DRM driver name
`virtio_gpu`, and both talk to the kernel through the virtio-gpu driver
ioctls of `drm/virtgpu_drm.h`: parameters, capsets, contexts, resources
(plain and blob), mapping, command submission with fences, waits and
transfers. Underneath, the kernel speaks the 3D part of the virtio GPU
protocol: 3D contexts of a capset, 3D resources, 3D transfers,
`SUBMIT_3D`, blob resources and the host-visible shared memory region they
are mapped into, and fences that complete when the host's GPU work does.

Sources: OASIS VIRTIO 1.2/1.3 §5.7 (feature bits VIRGL, RESOURCE_BLOB,
CONTEXT_INIT; §5.7.4 the configuration's `num_capsets` and the
host-visible shared memory region; §5.7.6.5 fences; §5.7.6.7-§5.7.6.10 the
capset, blob and 3D commands and their structures; §2.10 / §4.1.4.7 shared
memory regions and `virtio_pci_cap64`), the MIT-licensed uapi header
`drm/virtgpu_drm.h` (request numbers, structures, flags, parameters),
`linux/sync_file.h` (the sync-file ioctls' numbers and layouts), the
MIT-licensed `virgl_hw.h` and Gallium `p_defines.h` for the three constants
the kernel's own 3D test uses, the Khronos EGL, GLES and Vulkan API
documentation for the test programs, RFC 6143 for the VNC client, ACPI 6.x
§5.2.5.1 for finding the RSDP, and QEMU's documented device options. No
Linux, Mesa, virglrenderer or QEMU source was read (CLEAN_ROOM.md); where
QEMU's behaviour mattered it was observed (below).

## Decision

### The driver: tickets, external buffers, an interrupt (`crates/poly-virtio-gpu`)

- **Features.** VIRGL, RESOURCE_BLOB and CONTEXT_INIT are accepted when
  offered (the 2D commands work the same with them); resource UUIDs still
  are not.
- **Slots and tickets.** The command memory is 16 slots of 4 KiB (request,
  then room for the response); the control queue up to 64 entries. Every
  request is fenced as before and lives in a slot until collected. A
  request in flight is a `Ticket`; `reap` files each used buffer under its
  slot, whoever waits, so responses may come back in any order (a 3D fence
  completes long after the commands behind it were answered). Synchronous
  commands (2D, contexts, resources, capsets, blobs) are sent and polled for
  as in ADR 0102, waiting for a free slot within the timeout; asynchronous
  ones (`submit_3d`, `send_transfer_3d`, `send`) return the ticket, and
  `poll` collects it (`forget` leaves the slot to be freed when the device
  is done). A request may carry up to four more device-readable buffers (a
  command stream in pieces) and answer into caller memory (a capset).
- **Commands.** `GET_CAPSET_INFO`, `GET_CAPSET`, `CTX_CREATE` (with the
  capset in `context_init`), `CTX_DESTROY`, `CTX_ATTACH/DETACH_RESOURCE`,
  `RESOURCE_CREATE_3D`, `TRANSFER_TO/FROM_HOST_3D`, `SUBMIT_3D` (the header
  carries the context and, with context init, the fence's ring:
  `INFO_RING_IDX`), `RESOURCE_CREATE_BLOB` (guest memory as a scatter
  list), `RESOURCE_MAP_BLOB` (returns `map_info`), `RESOURCE_UNMAP_BLOB`.
- **Interrupts.** `Setup::queue_vector` puts the control queue on an
  MSI-X entry; the driver then asks for used-buffer interrupts (re-armed
  with EVENT_IDX as buffers come back).
- **The host-visible region.** `poly_virtio::find_shared_memory` reads
  `VIRTIO_PCI_CAP_SHARED_MEMORY_CFG` capabilities (64-bit offset and
  length, the `shmid` in the `id` byte); `hostmem::Ranges` hands out
  page-aligned, first-fit offsets in it for mapped blobs.
- **Tests.** The simulated device grew capsets, contexts (and what each
  may use), 3D resources copied to and from their backing by box and
  stride, a toy command stream (`FILL resource byte`) so a submission has a
  visible effect, blobs and the host-visible region, fenced 3D requests
  held back and answered out of order (`Sim::retire`), and MSI-X vectors.
  Tests cover the wire format at the spec's offsets, capsets, a context
  round trip (upload, submit, partial download), out-of-order fences while
  synchronous commands go on, every slot busy, blobs, missing features and
  the interrupt vector.

### The virtio-gpu DRM interface (`crates/poly-drm`, `virtgpu.rs`)

`DRM_COMMAND_BASE` + 0x01..0x0b, all render-node ioctls, with the size
rule of ADR 0108 (so the 40-byte `EXECBUFFER` of older headers works):

- `GETPARAM` writes an `int` (3D features, `CAPSET_QUERY_FIX` 1, blobs,
  host-visible, cross-device 0, context init, a bit per capset ID, explicit
  debug names). `GET_CAPS` copies a capset (fetched once per ID and
  version; unknown IDs and versions above the newest are `EINVAL`, no 3D is
  `ENOSYS`).
- **One context per file**, ID = the file's: `CONTEXT_INIT` makes it with a
  supported capset, up to 64 rings, a poll mask within the rings and a
  debug name (`EEXIST` once it exists); otherwise the first ioctl that
  needs one makes a default (virgl) context. It is destroyed when the file
  closes.
- **Resources are GEM objects** with hardware resource `0x100000` + the
  object ID. `RESOURCE_CREATE` gives a 3D resource zeroed guest memory of
  the requested size (a 2D resource on hardware without 3D);
  `RESOURCE_CREATE_BLOB` checks the kind and flags, runs its command stream
  first if it has one (Venus allocates host memory that way), gives the
  guest kinds guest memory and a mappable host blob a window of the
  host-visible region. An object is attached to a file's context when the
  file gets a handle while it has a context, and before a submission or
  transfer that uses it; with its last reference it is detached from every
  context and destroyed. `MAP` gives the fake offset dumb buffers use;
  `RESOURCE_INFO` reports resource, size and blob kind.
- **Fences.** `EXECBUFFER` and the 3D transfers return at once with a
  hardware fence recorded on every object they use. `WAIT` sleeps until
  they signal (`EBUSY` with `NOWAIT`) by returning the new
  `Reply::WaitFence`, after which the kernel asks again; `EXECBUFFER` hands
  out a sync file for its fence (`FENCE_FD_OUT`) and first waits for the
  fences of `FENCE_FD_IN` the same way (nothing is submitted until then).
  Rings beyond the context's, unknown flags and sync objects are `EINVAL`
  (`DRM_CAP_SYNCOBJ` stays 0).
- The `Hardware` trait gained 3D methods (`gpu_caps`, `capset`,
  `ctx_*`, `create_resource`, `destroy_resource`, `submit`, `transfer`,
  `fence_done`) and `Fds` gained `export_fence` / `import_fence`, all with
  defaults (no 3D), so hardware without 3D is unchanged. A GEM object's
  memory became optional (a host resource the CPU cannot reach): mapping,
  PRIME export and `ADDFB` of one are `EINVAL`.
- Seven new host tests drive all of it against a fake 3D hardware.

### The kernel (`kernel/src/gpu3d.rs`, `drm.rs`, `virtio_gpu.rs`)

- `virtio_gpu.rs` enables MSI-X (masked) and sets the control queue up on
  entry 0, with a 64-entry queue, and finds and records the host-visible
  region's bus address.
- `gpu3d.rs`: submissions copy the command stream into DMA memory (4 MiB
  pieces) that lives, with the ticket, in a fence table until the device
  answers; the driver's fence ID is the DRM fence. A `drm-fence` thread
  collects answers when the queue interrupts (or every millisecond without
  MSI-X) and wakes waiters (`wait_fence`, a minute at most) and sync-file
  pollers. Sync files (`SyncFile`) are readable once every fence in them
  is, and answer `SYNC_IOC_MERGE` and `SYNC_IOC_FILE_INFO`. Mappable host
  blobs are mapped with `RESOURCE_MAP_BLOB` at a window of the region, and
  a mapping of the object maps those device pages (`SharedFrame::device`,
  the window going back to the region when its memory is freed).
- `drm.rs`: `KernelHw` implements the 3D methods on the driver (3D
  resources and blobs, their guest memory physically contiguous, one
  scatter-list entry). A framebuffer on a buffer with a 3D resource
  (`DumbMemory::resource`, set when the resource is made) scans that
  resource out directly: no 2D copy, no upload (the host rendered it), as
  GBM's scanout buffers under kmscube need. Blobs are destroyed
  asynchronously (`retire_blob`: unmap, unref, their memory kept until the
  device answered).
- **Observed QEMU behaviour.** QEMU 9.2 never answers a *fenced*
  `RESOURCE_UNMAP_BLOB` (it finishes the unmap after its memory map changes
  and does not signal the fence); unfenced it answers. The driver sends it
  unfenced (`send_unfenced`, used by `unmap_blob` too).
- `acpi.rs`: when the boot loader passes no RSDP (QEMU 9.2's PVH boot does
  not), the kernel searches the EBDA's first KiB and 0xE0000-0xFFFFF for it
  as ACPI §5.2.5.1 says (`Rsdp::search` existed in `poly-acpi`).
- `DRM_IOCTL_VERSION` stays `virtio_gpu` 0.1.0 (its description says 3D
  when the device has it): Mesa loads `virtio_gpu_dri.so` (virgl); on a
  device without 3D virgl fails and Mesa falls back to its software
  drivers, as on Linux.

### Running it: the host side

QEMU's `virtio-gpu-gl-pci` runs virgl on the host's OpenGL; without a GPU
that is Mesa's llvmpipe behind the `egl-headless` display, which needs a
DRM render node. The poly-dev image installs QEMU's OpenGL modules
(`qemu-system-modules-opengl`), Mesa's drivers and Vulkan pieces
(`mesa-vulkan-drivers`, `libvulkan1`, `vulkan-tools`), and `polyd` passes
`/dev/dri` into the container when the host has one (WSL's kernel has
`vgem`). Two things Ubuntu 24.04's QEMU 8.2 cannot do:

- **No Venus**: 8.2's device has neither `venus=on` nor blobs with virgl.
  The image therefore builds QEMU 9.2.4 on virglrenderer 1.1.1 (with Venus
  and its render server; Ubuntu's 1.0 has no render server) into
  `/opt/qemu-venus`, used only for the Venus boot. With it the device
  offers VIRGL, RESOURCE_BLOB and CONTEXT_INIT, capsets virgl, virgl2 and
  venus, and a 1 GiB host-visible region.
- **No `screendump` of a GL scanout** ("no surface"): the boot test takes
  its screenshots through QEMU's VNC server instead (a small RFB 3.8 client
  in `tools/xtask/src/virgl.rs`), whose surface egl-headless fills by
  reading the scanout back.

### Tests: boot group `virgl` (`tools/xtask/src/virgl.rs`)

Skipped, with the reason, where the host has no render node or no QEMU
OpenGL modules (GitHub's runners), or no Mesa. Otherwise:

1. The kernel test `gpu3d::virgl_round_trip_through_the_host` on
   `virtio-gpu-gl-pci`: a context, a 64x64 texture uploaded and read back
   unchanged through the host by fenced 3D transfers completed through
   MSI-X, an empty fenced submission, everything destroyed, no device
   error.
2. The root disk (the host's unmodified Mesa, `eglinfo`, `kmscube`,
   `vulkaninfo`, added by `virgl::add_to_root`, plus two programs built
   from `tests/linux/virgl/`): `eglinfo -B` on the GBM and surfaceless
   platforms reports `virgl (LLVMPIPE ...)` for OpenGL 4.3 core and
   GLES 3.2; `gl-readback` draws clears, a scissored clear and a shader's
   rectangle into a framebuffer object and `glReadPixels` must return
   exactly those pixels; `kmscube` on `/dev/dri/card0` renders with virgl
   (about 75 frames a second under TCG) through GBM buffers that are 3D
   resources shown by legacy page flips, and two VNC screenshots must show
   a cube that moved; the console has the screen back afterwards.
3. With `/opt/qemu-venus`: the disk again on `venus=on,blob=on,hostmem=1G`;
   the kernel must list the venus capset, `vulkaninfo --summary` must list
   `Virtio-GPU Venus (llvmpipe ...)`, and `vk-clear Venus` clears an image
   on that device, copies it into host-visible memory and checks all 4096
   pixels.

No kernel line may report a missing system call. `POLY_VIRGL_ONLY=venus`
runs the Venus boot alone; `POLY_VIRGL_RUN=<command>` runs a command first.

## Consequences

- Unmodified Mesa renders OpenGL/GLES (virgl) and Vulkan (Venus) on Poly
  through the host's GPU stack; on this machine the host renderer is
  llvmpipe/lavapipe, so the frames are correct but not fast.
- The console, DRM's KMS path and 3D share one control queue; synchronous
  commands still poll with the display's lock held, so a long synchronous
  command stalls the CPU that sent it (the 3D work itself is asynchronous).
- The `virgl` boot group is skipped on CI (no render node there); it runs
  where `polyd` passes `/dev/dri` in. The QEMU 9.2 build adds a few minutes
  to building the image once.
- `Venus` needs QEMU 9.2+ with virglrenderer built with its render server;
  the system QEMU stays 8.2 for every other test.

## Not done

- Sync objects (`DRM_CAP_SYNCOBJ`, `SYNCOBJ_*`, the `EXECBUFFER` syncobj
  arrays) and fences on dma-bufs (`poll` of a dma-buf is still always
  ready): Vulkan WSI and explicit-sync Wayland want them.
- `POLL_RINGS_MASK` fence events (`VIRTGPU_EVENT_FENCE_SIGNALED`) on the
  DRM file: the mask is kept, nothing is queued.
- Resource UUIDs and cross-device sharing; `SET_SCANOUT_BLOB` (a
  framebuffer on a host blob without guest memory is `EINVAL`); framebuffers
  on 3D resources take the whole resource as the scanout's source.
- The cursor queue; guest memory for resources is physically contiguous
  (one scatter-list entry), so very large resources may not find memory.
- `map_info`'s caching hint is not applied: host-visible pages are mapped
  write-back, which QEMU's RAM-backed region tolerates.
