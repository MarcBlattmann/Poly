# 0184: X11 apps, a session bus and room for big apps on the desktop disk

**Status:** Accepted. Track `linux-app-gaps`: make the Linux apps that failed
in the app survey (`tools/app-survey/`, hand-off `linux-app-survey.md`: 25
apps, 18 working, 2 partly, 5 failing) work. This ADR decides the three
changes that live on the desktop's Debian disk and in its scripts
(Xwayland, a session bus for user `poly`, room on the disk and
non-interactive `apt`). The other three fixes of the track are kernel
behaviour per man pages and need no decision: `setsockopt` options
(`ip(7)`, `ipv6(7)`, `tcp(7)`), `ioctl(TIOCPKT)` on a pty master
(`ioctl_tty(2)`, `pty(7)`) and `mmap` address hints (`mmap(2)`); they are
listed at the end. It builds on
[ADR 0118](0118-xwayland-and-polys-x11-window-manager.md) (Xwayland and
Poly's X window manager),
[ADR 0117](0117-shell-window-management-notifications-and-settings.md) (the
compositor's own session bus),
[ADR 0147](0147-the-desktop-from-chromiums-disk-with-chromium-as-the-default-browser.md)
(the desktop from Chromium's disk, `poly-as-user`) and
[ADR 0173](0173-installing-a-deb-polys-installer-over-debians-own-apt-and-dpkg.md)
(the installer and Debian's `apt`).

## What was missing (the survey's evidence)

1. **No X display.** Blender, Krita, VLC's Qt interface and LibreOffice's
   default plugin build only an X11 back end. The disk had no Xwayland and
   the compositor of the disk was started without `--xwayland-lazy`, because
   a display offered with no server behind it made `xdg-open` (which runs
   `xprop` whenever `DISPLAY` is set) spin (ADR 0147).
2. **No session bus for `poly`.** The compositor's bus
   (`$XDG_RUNTIME_DIR/bus`, ADR 0117) is in root's private runtime directory,
   so an app run as `poly` found no `DBUS_SESSION_BUS_ADDRESS` and tried
   `dbus-launch`, which needs an X display: 13 apps printed "Cannot autolaunch
   D-Bus without X11 $DISPLAY" (dconf settings not saved, Thunar's
   thumbnailer proxy retried every second, no portals).
3. **The disk was too small and `apt` could ask questions.** The stock disk
   has about 160 MB free (`image_size`): Blender alone is 1 GB, LibreOffice and
   GIMP several hundred MB. `tzdata`'s `config` script asked for a time zone
   on the console and waited for ever.

## Decision 1: Xwayland on the disk, started by the first X client

- `xwayland` and `x11-xkb-utils` (`xkbcomp`) are seeds of the disk set
  (`DISK_SEEDS`); `cargo xtask chromium lock` added them and their ten
  dependencies (`xserver-common`, `libxfont2`, `libfontenc1`, `libxcvt0`,
  `libtirpc3t64`, ...; 4 MB of packages). `xkb-data` was there already.
- `poly-desktop` starts the compositor with `--xwayland-lazy` **where
  `/usr/bin/Xwayland` exists**, so a disk made before this change keeps
  today's behaviour (no display offered with nothing behind it). The
  compositor owns display `:0` (ADR 0118) and starts Xwayland when the first
  X client connects: nothing is paid until an X11 app runs.
- **`DISPLAY=:0`** is in `/etc/poly-desktop.env` (the console's bash reads it
  from `/etc/profile`, `poly-as-user` sources it) where Xwayland exists;
  apps started by the launcher get it from the compositor as before.
- **Access for `poly`.** Xwayland is started by the compositor as root and
  its default access list holds the server's own user only. Poly's X window
  manager (a privileged client on the `-wm` socket pair) sends one
  `ChangeHosts` request when it connects: insert the server-interpreted
  address `localuser:poly` (X Window System Protocol, ChangeHosts;
  Xsecurity(7)). `crates/poly-x11` has the request (`Request::ChangeHosts`,
  host-tested bytes). No `xhost` or `xauth` on the disk, and no
  `-ac` (open to everybody).
- VLC, Krita, Blender and LibreOffice then run with no further change; the
  apps' own choice of back end is respected (the environment still says
  Wayland first, `QT_QPA_PLATFORM=wayland`, `GDK_BACKEND=wayland`).

## Decision 2: a real session bus for `poly`

- `/usr/local/bin/poly-session-bus` (a script, `SESSION_BUS_SCRIPT` in
  `tools/xtask/src/chromium.rs`) makes sure Debian's `dbus-daemon --session`
  runs as `poly` on `unix:path=/run/user/1000/bus`: it starts it if the pid
  file does not name a live `dbus-daemon` (the disk outlives a boot, so a pid
  file of an earlier boot is told from a live one by the process's name in
  `/proc/<pid>/comm`) and waits up to 30 s for the socket.
- `poly-desktop` runs it **in the background** before the compositor, so the
  boot does not wait for it; `poly-as-user` runs it too (idempotent) and
  passes `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus` to what it
  starts. `/etc/machine-id` was already on the disk (D-Bus and Chromium read
  it).
- The compositor's own bus stays as it is for apps the launcher starts as
  root. **Not decided here:** notifications from `poly`'s apps. They go to
  `poly`'s bus, where nothing owns `org.freedesktop.Notifications`; bridging
  that name to the compositor's bus is a follow-up (the apps print nothing and
  carry on, as they did without a bus).

## Decision 3: room on the disk, and `apt` asks nothing

- **Free room.** A disk made for interactive use (`cargo xtask desktop
  --chromium`, `cargo xtask chromium run|disk|STEPS`) has
  `INTERACTIVE_SPARE_MB` = 4096 MiB free (`--disk-size=SIZE`: `small` for the
  boot tests' 160 MiB, `512M`, `8G`; the dev menu's **Disk free space**
  entry, with **Fresh disk**). The boot tests' disks keep 160 MiB: CI makes
  many images and none of them installs an app. `POLY_ROOTDISK_SPARE_MB` still
  wins (the survey driver). A kept disk is not changed: it holds Chromium's
  profile and the installed apps, and growing an ext4 image in place needs a
  resizer that is not on the disk; **Fresh disk** makes it again with the room
  asked for. (The image file is allocated but not written, so a Windows host
  pays the 4 GB of disk space, not the time.)
- **`apt` asks nothing.** The installer already ran `apt-get` with
  `DEBIAN_FRONTEND=noninteractive` (`poly_pkg::apt::ENVIRONMENT`); the
  session now exports it for the console and for `poly-as-user`, so a
  package installed by hand (`apt install inkscape` pulls in `tzdata`) does not
  wait for an answer on the console.

## Checks

- Host: `poly-x11` (the `ChangeHosts` bytes), `xtask` (the scripts' text, the
  lock, the menu entry and its arguments, `parse_spare`).
- Boot group `chromium-desktop` (`desktop_boot_test`): as `poly`,
  `DISPLAY=:0` and the bus address are set, `xdpyinfo` starts Xwayland and
  names display `:0` and "The X.Org Foundation", and `dbus-send` gets an
  answer from the session bus.
- The survey driver on the apps that failed or were partial; before/after
  per app is in the pull request and in `linux-app-gaps.md`.

## Results (survey driver, Poly's desktop disk, KVM, 4 GiB, 4 CPUs)

| app | before | after |
|---|---|---|
| VLC 3.0.24 | no window ("no suitable interface module") | window, Qt interface through Xwayland, privacy dialog then the player |
| LibreOffice Writer 25.2.3 (default plugin) | no window ("Failed to open display") | Writer 1280x800, `hello.txt` loaded, typing works |
| Krita 5.2.9 | no window (xcb: could not connect) | window, start page, Open dialog |
| Blender 4.3.2 | no window (GHOST: unable to open a display) | window, the default scene (the first start takes 3 to 5 minutes: software GL under the emulated disk) |
| Xfce Terminal 1.1.4 | dialog "Failed to open PTY: Inappropriate ioctl for device", no shell | a shell: the prompt takes typed text (`TIOCPKT`) |
| Transmission 4.1 | `setsockopt(IP_MULTICAST_*)`, `IPV6_TCLASS`, `TCP_FASTOPEN`: "Protocol not available"; "Cannot autolaunch D-Bus" | none of them printed; the dconf service of the session bus runs |
| Firefox ESR 153 | `neqo_glue ... os error 92` per UDP socket; no session bus | no socket error; window, welcome page |
| Thunderbird 153 | the same | no socket error; window, account setup |
| VS Code 1.146 | `mmap(0x1a000, ..)` answered with a high address, loop, abort | V8 reserves its address space at the hint (`mmap(0xec800000000, ..)` is honoured); the CLI still aborts later, in a thread that starts a libuv loop (open, see the hand-back) |
| mpv 0.40 | partly (software output) | unchanged (EGL/GL is not part of this track) |

## Also in this track (no decision needed; each follows a man page)

- `setsockopt`/`getsockopt` options that returned `ENOPROTOOPT`
  (`IP_PKTINFO`, `IP_RECVTOS`, `IP_MTU_DISCOVER`, the multicast options,
  `IPV6_TCLASS`, `TCP_FASTOPEN`, ...), with the ancillary data
  `recvmsg` returns for them: Firefox, Thunderbird, Transmission.
- `ioctl(TIOCPKT)` and `TIOCGPKT` on a pty master (packet mode): VTE
  terminals such as Xfce Terminal start a shell.
- `mmap` honours an address hint below 4 GiB when the range is free: V8's
  reservation loop (VS Code) ends.

## Not in this ADR

- Audio (no PulseAudio server on the disk), EGL/GL for mpv's `gpu` output,
  Wayland `xdg-foreign` and `ext_data_control`: unchanged from the survey.
- Applications installed from the launcher run as root with the compositor's
  environment (ADR 0173): they have `DISPLAY` and the compositor's bus.
- Notifications from `poly`'s apps (see decision 2).
