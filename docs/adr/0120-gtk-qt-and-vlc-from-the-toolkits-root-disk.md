# 0120: GTK 3, GTK 4, Qt and VLC run unchanged on the desktop from a toolkits root disk; what they needed from Poly

**Status:** Accepted. Roadmap Phase 6, toward the milestone "GTK and Qt
apps run; VLC plays a video with sound". Builds on the persistent root
disk ([ADR 0059](0059-bash-coreutils-python-gcc-from-a-root-disk.md)),
fonts and fontconfig-compatible discovery ([ADR 0105](0105-fonts-poly-font-font-store-and-fontconfig-discovery.md)),
the Wayland server and compositor ([ADR 0106](0106-wayland-server-core-and-software-compositor.md))
with the desktop and shell ([ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md)),
and ALSA on the audio server ([ADR 0104](0104-virtio-sound-and-alsa-pcm.md),
[ADR 0109](0109-audio-server-capture-mmap-mixer-and-intel-hda.md)).

## Context

Everything a GTK or Qt app needs underneath was there: dynamic linking
with glibc's own `ld.so`, AF_UNIX sockets with descriptor passing, `memfd`
with seals, shared mappings, `poll`/`epoll`/`eventfd`, threads, inotify, a
Wayland compositor on the screen with the keyboard and pointer routed to
clients, fonts, and ALSA's kernel interface. Nothing had run a real
toolkit: the compositor's only clients were our own C programs speaking
the wire protocol.

GTK, GLib, Qt, VLC, alsa-lib and fontconfig are LGPL or GPL: they run as
unmodified test programs from the build image's Ubuntu 24.04 packages and
are never committed. What they needed was found only on Poly, from their
own messages and the kernel's log; no GTK, GLib, Qt, VLC, ALSA or Linux
source was read (CLEAN_ROOM.md). Sources: the public API documentation of
GTK 3 and GTK 4 (docs.gtk.org) and Qt 6 (doc.qt.io), which the test apps
are written from; the Desktop Entry, Base Directory and Shared MIME-info
specifications (freedesktop.org); fontconfig's user documentation
(`fonts-conf(5)`); VLC's command-line help (`vlc -H`); ALSA's
configuration documentation (`plughw`); QEMU's documentation (QMP
`input-send-event`, `screendump`, the `wav` audio backend).

## Decision

### The toolkits' root disk

`tools/xtask/src/toolkits.rs` makes it: ADR 0059's root disk
(`rootdisk::collect_with`, which now takes a callback that adds host files
before the shared objects are resolved), plus:

- **Test apps**, `tests/linux/toolkits/`: `gtk3-hello.c` (GTK 3),
  `gtk4-hello.c` (GTK 4) and `qt-hello.cpp` (Qt 6 Widgets, no moc), built
  at test time against the host's toolkits with `pkg-config`. Each is a
  `GtkApplication` / `QApplication` with one 360×240 window holding a
  button coloured by CSS or a style sheet, recoloured on a click, and
  prints what happens (drawn, clicked, keys, closed) for the boot test.
  The toolkits' own `gtk3-demo` and `gtk4-demo` come along.
- **What the toolkits load rather than link**, whose `DT_NEEDED` closure
  the collector then adds: GDK-Pixbuf's loaders and `loaders.cache`,
  GTK 3's input method modules, Qt 5's and Qt 6's Wayland platform plugin
  with its shell, decoration and buffer integration plugins
  (`QT_PLUGINS`), VLC's plugins for Ogg/Theora/Vorbis through ALSA onto a
  Wayland window or into a file, its Qt interface and the dummy interface
  `cvlc` uses (`VLC_PLUGINS`, about 110 of its 370).
- **Data**: the compiled GSettings schemas, the Adwaita and hicolor icon
  themes, the shared MIME database's compiled files (GIO guesses content
  types from them and GDK-Pixbuf picks a loader by content type: without
  them no SVG icon loads, and GTK 3 aborts on its missing-image icon),
  the `C.UTF-8` locale (glibc builds it as files like any other locale;
  without it GTK falls back to `C` with a warning), XKB data, and ALSA's
  configuration (`/usr/share/alsa`).
- **The desktop**: `poly-compositor` (`POLY_TOOLKITS_COMPOSITOR=<file>`
  takes another build of it, to try the apps against a compositor in the
  making), desktop files for the launcher (GTK 3 Hello, GTK 4 Hello, Qt
  Hello, GTK 3 Demo, GTK 4 Demo, VLC), and `/etc/poly-desktop.env`, the
  environment every app gets: the XDG Base Directory variables
  (`XDG_RUNTIME_DIR=/run/user/0`, config, data, cache and state under
  `/root`), `LANG=C.UTF-8`, `GDK_BACKEND=wayland`, `GSK_RENDERER=cairo`
  (GTK 4 draws with cairo in shared memory until Mesa runs) and
  `QT_QPA_PLATFORM=wayland`.
- **A test video**, made by the host's `ffmpeg`: three seconds of 320×240
  red, green and blue at 15 frames a second in Theora over a 440 Hz tone
  in Vorbis, in Ogg.

The packages are in the `poly-dev` image (`libgtk-3-dev`, `gtk-3-examples`,
`libgtk-4-dev`, `gtk-4-examples`, `qt6-base-dev`, `qt6-wayland`,
`qtwayland5`, `vlc-bin` and its plugin packages, `ffmpeg`, `alsa-utils`,
the schema, icon and MIME packages). A host without them gets a disk
without them, with a note, and the boot test skips what is missing.

### Booting it

BusyBox's init again (ADR 0059), with another `inittab`
(`initramfs::disk_root_image_with`): after mounting the disk it starts
`/usr/local/sbin/poly-desktop` in the chroot (the compositor with
`--desktop`, under the desktop environment) once, and the disk's bash on
the serial console, which reads the same environment from `/etc/profile`.
Apps started from the launcher or the console both print to the console.

`cargo xtask toolkits [--fresh-disk] [STEP...]` boots it and runs steps
(`run:`, `send:`, `expect:`, `shot:`, `click:X,Y`, `key:`, `type:`,
`sleep:`): the way the gaps below were found.

### What they needed

The kernel needed far less than expected: GTK 3, GTK 4 and Qt 6 ran
their first window without a single missing system call, `/proc` or
`/sys` file. What they did need:

- **Poly's `fonts.conf`** (`crates/poly-font/fonts.conf`, ADR 0105) had
  several `<family>` elements in one `<alias>`; fontconfig's
  documentation gives an alias one family, and the real fontconfig warns
  about every such alias at every program's start. One alias per name
  now, the same substitutions for `poly-font`'s matcher; a host test runs
  the host's `fc-match` on the file and fails on any warning.
- **`O_NONBLOCK` as it is now, for ioctls.** alsa-lib opens a PCM
  non-blocking and then clears the flag with `fcntl(F_SETFL)`
  (`poly.strace` of `aplay` shows the two `fcntl` calls before
  `SNDRV_PCM_IOCTL_DRAIN`). The PCM file kept the flag it was opened with,
  so on Poly `DRAIN` returned `EAGAIN` instead of waiting (`aplay` cut its
  last period short) and, after an underrun, every
  `SNDRV_PCM_IOCTL_WRITEI_FRAMES` with a full ring returned `EAGAIN`,
  which `snd_pcm_recover` passes on (its documentation: errors other than
  `EINTR`, `EPIPE` and `ESTRPIPE` come back unchanged): VLC logged
  "cannot recover playback stream: Resource temporarily unavailable" for
  the rest of the file. `poly_vfs::CharDevice` gets `ioctl_nonblock`, the
  ioctl told whether the open file is non-blocking at the time of the
  call (a default method calling `ioctl`, so other devices are
  unchanged); `OpenFile::ioctl` calls it, and the PCM's transfers and
  drain use it. Host test: a device that blocks unless non-blocking sees
  the flag change with `F_SETFL`.
- **Files a desktop has**: the `C.UTF-8` locale, the shared MIME
  database (GTK 3 aborts without an icon: `ensure_surface_for_gicon:
  assertion failed ... image-missing.svg: Unrecognized image file
  format`), the icon themes, XKB data, `/usr/share/alsa`; and VLC's
  plugins that only register options other modules read
  (`video_filter/libdeinterlace_plugin.so` registers
  `sout-deinterlace-mode`, without which every video output fails).
- **A user.** VLC refuses to run as root ("VLC is not supposed to be run
  as root"). The disk gets user `poly` (uid 1000) and
  `tests/linux/toolkits/poly-runas.c`, a small static helper (passwd(5),
  setgroups(2), setgid(2), setuid(2), the XDG Base Directory
  specification) that makes the user's home and `/run/user/<uid>`, points
  `HOME` and the XDG directories at them, gives the compositor's socket a
  second name there (a hard link: unix(7) finds a socket by its inode)
  that the user may connect to, and runs the program as the user. The
  compositor's runtime directory stays private: opening it to others made
  Qt warn "wrong permissions on runtime directory /run/user/0", and
  handing over a connection as `WAYLAND_SOCKET` does not survive VLC's Qt
  interface, which connects and disconnects once to see whether Wayland
  is there. The launcher's VLC entry goes through `poly-runas` too.
- **A thread-directed signal wakes a thread that waits for it.** `cvlc
  --play-and-exit` played to the end and never exited. `poly.strace=vlc`
  showed the last call: `tgkill(tgid, tgid, SIGTERM)`, the end of the
  playlist telling the main thread, which waits in `sigwait(3)`
  (`rt_sigtimedwait`) with `SIGTERM` blocked. `LinuxProcess::send` counts
  a signal a thread waits for as blocked (so it stays pending and kills
  nobody), and for a signal sent to one thread it woke the thread only if
  it was not blocked: the waiter was never woken. Now a thread that waits
  for the signal is woken too (`poly-personality-linux`, host test: the
  waiter alone is notified, and takes the signal). Process-directed
  signals already found the waiter, whose mask excludes what it waits
  for.
- **`poly.strace=<name>` could panic the kernel**: it held its name's
  spinlock while reading the process's `comm`, which takes a sleeping
  lock (the panic "blocks or yields while holding a spinlock" at the
  power-off after tracing VLC). The name is copied out first.

VLC's own Wayland windows need a shell Poly does not offer: its
`xdg_shell` window module speaks the old `xdg_shell` interface
(xdg-shell unstable v5, before `xdg_wm_base`) and its other one
`wl_shell`; with neither, "no vout window modules matched" and every
Wayland video output fails. So the test takes VLC's `yuv` video output,
which writes the decoded pictures to a YUV4MPEG2 file, and reads that
file back from the image after the power-off. Its Qt interface draws its
window through Qt's Wayland plugin, which works.

### D-Bus

There is no session bus. GLib reports it and goes on: GTK 4 prints
`Unable to acquire session bus` once, `GApplication` stays a local
instance (no uniqueness, no activation), GTK 3 prints nothing about it,
and with no GIO modules on the disk GSettings keeps settings in memory.
Nothing waits for a bus or fails without it; a session bus is for when the shell has settings and
notifications to share.

### The boot test

Boot group `toolkits` (shard `userland`): the disk made afresh, booted
with a virtio GPU, keyboard and tablet, virtio-sound recorded by QEMU's
`wav` backend, and QMP. For each of `gtk3-hello`, `gtk4-hello` and
`qt-hello`: typed into the launcher and started with Enter (the desktop
file path); its window appears (the compositor's log) in Poly's frame (the
title bar's grey above it) with the button's colour filling most of it; a
click on the button reaches it (its log, and the next screenshot shows the
other colour); a typed `a` reaches it; Q closes it and it exits 0. Then
`cvlc`, as `poly`, plays the test video from the console with its sound
through ALSA (`plughw:0,0`) and its pictures into
`/home/poly/poly-frames.y4m` (`--vout yuv`), and exits 0. After the
power-off the recording must hold the 440 Hz tone on both channels in at
least 20 of its 100 ms windows; the pictures, read back from the image
with `poly-ext2`, must be 320×240 and show red, green and blue in that
order (each frame's mean colour, BT.601); and no system call may be
missing. Last, VLC's Qt interface is started from the launcher: its
window must show one of the video's colours, and Ctrl+Q ends it (reported
but not counted yet, `POLY_TOOLKITS_VLC_GUI=required` counts it: see
Consequences). Screenshots go to `$POLY_TOOLKITS_SHOTS` (default
`target/toolkits/`); `POLY_TOOLKITS_ONLY=gtk4-hello,vlc` runs some of the
apps.

The GTK apps need the compositor protocols that the compositor-on-KMS
work adds (see Consequences); against that compositor the group passes
on TCG in about six and a half minutes (the disk is made in another
minute or two).

## Consequences

- GTK 3 and GTK 4 apps (software rendering), a Qt 6 Widgets app, `cvlc`
  and VLC's Qt 5 interface run unchanged on Poly's desktop, framed by
  Poly, with the keyboard, the pointer and sound. The roadmap milestone is
  ticked when the `toolkits` group passes on the merged tree: it needs the
  compositor protocols below.
- **What the compositor must offer** (from the apps on the compositor of
  ADR 0106/0107, whose globals are `wl_compositor`, `wl_shm`,
  `wl_output`, `wl_seat`, `xdg_wm_base`, `zxdg_decoration_manager_v1`,
  `wp_presentation`):
  - GTK 3 makes its `GdkSeat` only once `wl_data_device_manager` is
    announced; without it `gdk_seat_get_keyboard: assertion 'GDK_IS_SEAT
    (seat)' failed` and a `SIGSEGV` at address 0.
  - GTK 4 refuses the display: "The Wayland compositor does not provide
    one or more of the required interfaces, not using Wayland display"
    (with `wl_subcompositor` and `wl_data_device_manager` it runs).
  - VLC's Qt interface draws the video in a `wl_subsurface` of its
    window: without `wl_subcompositor` the video area stays black.
  - VLC's own Wayland windows need `xdg_shell` (xdg-shell unstable v5) or
    `wl_shell`; Poly offers neither (not needed by the test: VLC's Qt
    interface and `--vout yuv` cover it).
  - The compositor-on-KMS work (branch `p6-compositor-kms`: subsurfaces,
    the data device, viewporter, cursor shape, fractional scale,
    activation, dma-bufs) provides all but the last; every counted check
    of the group passes against a build of it
    (`POLY_TOOLKITS_COMPOSITOR`).
- Seen on that build, for its owner: a window started from the launcher
  after another one closed is shown inactive (grey title), also after a
  click on its content, though keys reach it. VLC's Qt interface showed
  the video in two of five runs; in the others VLC died early of a
  `SIGSEGV` at the same instruction each time (a garbage address), once
  right after that build cut it off with the protocol error
  `get_viewport: object 24 is a wl_subsurface, not a wl_surface` (raised
  for the surface argument of `wp_viewporter.get_viewport`, which VLC's Qt
  plugin passes a `wl_surface`; the build's object table seems to keep a
  stale entry). Whether Poly's kernel has a part in the crash is open;
  that is why the check is reported and not counted yet.
  GTK 3 and GTK 4 draw their own title bar inside Poly's frame (they do
  not use `zxdg_decoration_manager_v1`; Qt does and gets Poly's frame
  only): the price of ADR 0106's one frame for every app.
- VLC's Qt interface aborts at exit after Ctrl+Q ("free(): double free
  detected in tcache 2", status 134) on either compositor; not yet known
  whether Poly has a part in it (the test only waits for it to end).
- Not done: `/proc/<pid>/task/<tid>/stat` (only `comm` is there), a
  session bus, GL rendering for GTK 4 and Qt (with Mesa), Qt 5 apps other
  than VLC, a fontconfig cache made ahead (fontconfig scans the twelve
  fonts at every start, which is quick), input methods.
