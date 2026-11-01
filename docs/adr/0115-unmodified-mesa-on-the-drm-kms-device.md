# 0115: Unmodified Mesa on the DRM/KMS device: Mesa, modetest and kmscube from the root disk, kmscube rendering with kms_swrast through GBM and EGL, and the bus ID bound by interface version 1.1

**Status:** Accepted. Closes roadmap Phase 6's "Kernel GPU interface
compatible with Linux DRM/KMS so Mesa runs unchanged", which
[ADR 0108](0108-drm-kms-device-on-virtio-gpu.md) left open until Mesa ran
on the device. Builds on ADR 0108 (the device), [ADR 0059](0059-bash-coreutils-python-gcc-from-the-persistent-disk.md)
(host programs on the persistent root disk) and [ADR 0102](0102-virtio-gpu-2d-display.md)
(the virtio GPU).

## Context

ADR 0108 put a Linux DRM/KMS device on the virtio GPU (`/dev/dri/card0`,
`renderD128`, dumb buffers, legacy and atomic modesetting, page flips
with events, PRIME, sysfs device discovery) and tested it with raw
ioctls. No image had Mesa, libdrm or a KMS client, so nothing showed that
the stack above the kernel (libdrm's device lookup, Mesa's loader, GBM's
DRI backend, EGL on GBM, the software rasterisers) works on it.

Sources: as for ADR 0108 (the MIT uapi headers `drm/drm.h`,
`drm/drm_mode.h`, `drm/drm_fourcc.h`, `drm/virtgpu_drm.h` for request
numbers, the kernel's public DRM documentation), Mesa's public
documentation (its environment variables: `LIBGL_ALWAYS_SOFTWARE`,
`GALLIUM_DRIVER`, `MESA_SHADER_CACHE_*`; the EGL and GBM platform
pages), glvnd's documented EGL vendor JSON files, the programs' own
`--help` and messages, and `poly.strace` on Poly. No Linux, libdrm, Mesa
or kmscube source was read, and nothing was observed on a real Linux
system (CLEAN_ROOM.md: running unmodified binaries on Poly, like 7-Zip).

## Decision 1: the host's Mesa on the root disk, as for gcc

`tools/xtask/src/mesa.rs` adds to the root disk (`rootdisk::collect`
calls it, so `cargo xtask run --disk-root` has them too): `modetest`,
`kmscube` and `eglinfo` from `/usr/bin`; the DRI loader stubs Poly's GPU
can lead to (`dri/kms_swrast_dri.so`, `swrast_dri.so`, `virtio_gpu_dri.so`,
links to `libdril_dri.so`); GBM's backend `gbm/dri_gbm.so`; glvnd's
`libEGL.so.1` and `libGLESv2.so.2`; every EGL vendor library the JSON
files in `/usr/share/glvnd/egl_vendor.d` name (`libEGL_mesa.so.0`) with
the files themselves; Mesa's driver options in `/usr/share/drirc.d`. These
are what programs open by name at run time; the `DT_NEEDED` closure brings
the rest (the Gallium library with llvmpipe and softpipe, LLVM, libdrm,
libgbm, the X and Wayland client libraries `libEGL_mesa` links). Nothing
runs on the host to find them, nothing is committed; a host without them
gets a note and the boot test is skipped.

The packages come from Ubuntu 24.04 in the poly-dev image
(`libgl1-mesa-dri libgbm1 libegl-mesa0 libegl1 libgles2 libdrm-tests
kmscube mesa-utils`: Mesa 25.2.8, libdrm 2.4.125, LLVM 20), about 200 MiB
more on the disk (316 MiB of files with bash, coreutils, Python and gcc).
Taking them from the image rather than pinned `.deb`s (as `sevenzip.rs`
pins 7-Zip) keeps one way of bringing Linux programs onto the disk and lets
the loader-style lookup find dependencies; the image is the pin.

## Decision 2: what Poly lacked was one ioctl answer

Everything ADR 0108 built worked under real Mesa except libdrm's lookup
by driver name: `modetest -M virtio_gpu` (libdrm's `drmOpen(name, NULL)`)
failed with `failed to open device 'virtio_gpu' with busid '(null)'`.
`poly.strace` showed it open `card0`, read the version (the name
matched), read `GET_UNIQUE` twice (length, then string), close the file
and move on to `card1`. libdrm treats a minor whose bus ID is already set
as taken by another client, and Poly reported the bus ID
(`pci:0000:00:03.0`) to everyone.

Now `GET_UNIQUE` answers an empty string until the file binds the bus ID
by asking for interface version 1.1 or later with `SET_VERSION` (what
libdrm's `drmSetInterfaceVersion` documents itself as doing); then it
answers the PCI bus ID, per file (`poly_drm::File::bus_id`, host test
`unique_after_interface_version_1_1`). `SET_VERSION` with `di_major` -1
(only the driver's version asked) binds nothing.

What worked unchanged, as `poly.strace` shows for `kmscube`:

- **Device discovery**: Mesa's loader and libdrm's `drmGetDevices2` read
  `/sys/dev/char/226:{0,128}` (links), `device/subsystem` (`pci`),
  `device/{vendor,device,subsystem_*}` and `uevent`, so Mesa finds PCI
  1af4:1050 and picks its `virtio_gpu` driver.
- **The virgl probe fails cleanly**: that driver asks
  `DRM_IOCTL_VIRTGPU_GETPARAM` (`0xc0106443`) and `GET_CAPS`
  (`0xc0186449`), both `EINVAL` (unknown driver ioctls, ADR 0108), and
  GBM falls back to `kms_swrast` by itself; with `LIBGL_ALWAYS_SOFTWARE=1`
  it goes there directly. Either way the renderer is llvmpipe (LLVM 20,
  128 bits).
- **kms_swrast's buffers**: dumb buffers (`CREATE_DUMB`), mapped with
  `MAP_DUMB` and `mmap` every frame, `ADDFB2`, then `SETCRTC` and a
  `PAGE_FLIP` with an event per frame, the events `read` after
  `pselect6` says the card file is readable.
- **Modifiers**: GBM's buffers report `DRM_FORMAT_MOD_INVALID`, which
  kmscube passes to `ADDFB2` with `DRM_MODE_FB_MODIFIERS`; Poly refuses it
  (`EINVAL`: `drm_mode.h` says the invalid modifier must not be used
  there), kmscube prints `Modifiers failed!` and adds the framebuffer
  without one. That is kmscube's documented fallback, not an error.
- **Mesa's shader cache** in `/root/.cache/mesa_shader_cache` on the
  ext4 disk (`flock`, `rename`, `pwrite64`): the first run compiles (its
  first frame takes about 2.7 s under TCG), later runs find the shaders.
- **llvmpipe itself**: worker threads (`clone3`), `sched_{get,set}affinity`,
  `sched_setscheduler(SCHED_BATCH)`, a `memfd_create` file, read-write-execute
  anonymous mappings for LLVM's JIT, futexes; no system call is missing.

Two answers differ from Linux without stopping anything, and stay for
the Linux personality's own work rather than this change:

- `fallocate` on the ext4 disk is `EOPNOTSUPP` (ADR 0112 gave it to tmpfs
  only); glibc's `posix_fallocate` then writes a byte per block, as
  posix_fallocate(3) describes, so Mesa's 1.25 MiB cache index costs about
  320 small writes once.
- `setpriority(PRIO_PROCESS, <thread id>, 19)`, with which Mesa lowers one
  of its threads, is `ESRCH`: Poly keeps the nice value per process and
  looks `who` up as a process, while setpriority(2) notes that on Linux it
  is per thread. Mesa ignores the error; nothing in Poly's scheduler uses
  nice values yet.

## Decision 3: the boot test

Boot group `mesa` (CI shard `userland`) boots the root disk with a
virtio GPU preferring 1024x768 and a QMP socket, and types into its bash:

1. `modetest -M virtio_gpu`: encoders, connectors (`Virtual-1`,
   connected, 1024x768), CRTCs and planes with properties are listed;
   their IDs are read from the listing.
2. `modetest -s <connector>@<crtc>:1024x768` (legacy), the same with
   `-a ... -P <plane>@<crtc>:1024x768` (atomic), and with `-v` (vsynced
   page flips): a screenshot of each must show modetest's seven colour
   bars; with `-v` screenshots must catch both the bars and modetest's
   plain second buffer, and modetest reports about 75 Hz (the virtual
   vblank clock of QEMU's EDID mode, ADR 0108).
3. `LIBGL_ALWAYS_SOFTWARE=1 kmscube -D /dev/dri/card0 -c 150`, then
   `kmscube -c 75` without the variable (the fallback above): screenshots
   until one shows a cube over kmscube's grey clear colour, then another
   half a second later, which must differ in at least 100 pixels (the cube
   turns; in practice over 130 000 of the 786 432 pixels change); the
   renderer must be llvmpipe or softpipe, kmscube must report all its
   frames (about 8 to 11 frames per second under TCG) and exit 0.
4. The console must take the display back after each of the five
   programs (ADR 0108), no system call may be missing, and `poweroff`
   ends QEMU with status 0.

`POLY_MESA_STEPS=<text>` runs only the matching steps and
`POLY_MESA_APPEND` adds kernel arguments (`poly.strace=kmscube`), for
debugging.

## Consequences

- Unmodified Mesa renders on Poly's DRM device with its software
  rasteriser through the same GBM/EGL path a compositor or SDL's KMSDRM
  backend uses; the DRM/KMS roadmap item is done.
- `modetest` reports `failed to set gamma: Invalid argument`: the device
  has no gamma table (`gamma_size` 0, ADR 0108), and the virtio GPU could
  not apply one. modetest goes on.
- `kmscube -A` (atomic with fences) stops at `no
  eglDupNativeFenceFDANDROID`: Mesa's software drivers do not offer
  `EGL_ANDROID_native_fence_sync`, so that is not a kernel gap; atomic
  modesetting is covered by modetest. Explicit sync (sync objects,
  `sync_file` fences on dma-bufs) stays on ADR 0108's list.
- Still open from ADR 0108's list: the virgl/Venus driver interface (the
  next roadmap item; with it, Mesa's `virtio_gpu` driver would be used
  when QEMU offers 3D, and this test's fallback step would then need a
  device without 3D), sync objects, a cursor plane and overlays, hotplug
  uevents, real vblank, gamma, leases.
- The root disk is about 200 MiB larger with Mesa and LLVM; the boot test
  builds its own image (`poly-root-disk-mesa.img`) and takes about two
  minutes under TCG after the image is made.
