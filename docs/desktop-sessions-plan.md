# Desktop choice on Poly: GNOME, KDE Plasma or Poly's own desktop (survey and plan)

Track `survey-desktop-sessions` (survey agent, 2026-11-20). The owner asked: can the user choose the
desktop (Poly's own, GNOME or KDE Plasma) when installing and at login? The owner then narrowed the
question for now: **does it work, and what goes in the roadmap?** Choosing a desktop and the lean OS
installer come later (roadmap Phase 12). This document holds the findings and the batches behind
that phase.

**Inputs:** Debian 13's own packages (weston 14.0.2, seatd, KWin 6.3.6, Mutter 48.7, strace, udev,
dbus, systemd-logind), installed with `apt` inside Poly's guest. Each one was run on Poly in QEMU from
the console of Chromium's Debian disk (`cargo xtask chromium --release --disk-size=8G
follow:FILE`). The evidence is the programs' own logs, `strace` of them on Poly, and the public
documentation (weston(1)/weston-drm(7), seatd(1), the DRM/KMS uAPI and udev man pages,
freedesktop.org's login1 D-Bus API). No compositor, Mesa, systemd or kernel source was read.

## 1. Answer in short

- **Yes, it can work, and the hard part is already there.** Poly's DRM/KMS device carries a foreign
  compositor through mode setting. weston's DRM backend opened `card0` through seatd and
  reported "supports atomic modesetting", "supports GBM modifiers" and GL ES 3.2 on Mesa's llvmpipe.
  After that it stopped at input. Mutter 48 runs today as a window on Poly's desktop, and a Wayland
  app runs inside it (screenshot below).
- **What is missing is the plumbing around the device.** That means the kernel's udev uevent socket,
  a udev database, `/dev/tty0` with VT switching, and a login manager (logind) on a system bus. GNOME
  and Plasma also need their usual session services (polkit, PipeWire, the portals, NetworkManager or
  a stand-in).
- **The first stop was the same for every desktop:**
  `socket(AF_NETLINK, SOCK_RAW, NETLINK_KOBJECT_UEVENT) = -1 EPROTONOSUPPORT`. weston needs it for
  input, KWin needs it to accept the GPU, and systemd-logind won't start without it.

## 2. What Poly already has

| Piece | State on Poly today | Where |
|---|---|---|
| DRM/KMS `card0` and `renderD128` | **Present.** Legacy and **atomic** (TEST_ONLY, NONBLOCK, ALLOW_MODESET), dumb buffers, ADDFB2 with modifiers, PRIME, page-flip and vblank events, GET_CAP/SET_CLIENT_CAP. Not supported: the cursor ioctls, gamma, syncobj, overlay and cursor planes, leases | ADR 0108, 0115, 0116, 0119, 0243 |
| DRM master | The first opener becomes master. SET_MASTER returns EBUSY while another file holds it. poly-compositor holds master for the whole session and never drops it | ADR 0108 |
| evdev `/dev/input/event*` | **Present** (EVIOCGBIT/GABS/GRAB/REVOKE …). poly-compositor GRABs every device | ADR 0103 |
| udev | **Missing:** no `NETLINK_KOBJECT_UEVENT`, no `/sys/class/input`, no `/run/udev/data`. sysfs has the GPU (`/sys/class/drm`, uevent files) | ADR 0103, 0108 |
| VT / seat | Only `/dev/tty1`. **No `/dev/tty0`**, and no VT_SETMODE/VT_ACTIVATE/KDSETMODE/KDSKBMODE | `crates/poly-tty` |
| D-Bus | Session bus present (`dbus-daemon --session` as user `poly`; poly-compositor's own bus for notifications). `dbus-daemon --system` **starts fine when run by hand**, but nothing starts it | ADR 0184 |
| logind, polkit, PAM session | Missing. systemd is on the disk only as a dependency; PID 1 is BusyBox init | ADR 0130 |
| cgroups | Not mounted (logind and `systemd --user` use cgroup v2) | — |
| Mesa | Present: Debian's Mesa 25.0.7 (GBM, EGL, llvmpipe) on the disk; virgl and Venus on the dev root disk | ADR 0115, 0119 |
| Xwayland | Present (started lazily by poly-compositor). Mutter started its own Xwayland on `:1` | ADR 0118, 0184 |
| PipeWire, NetworkManager, xdg-desktop-portal | Missing (ALSA `/dev/snd` is present; the network is set by the kernel) | — |
| Windows apps' windows | Poly's private desktop link `/dev/win32k`, with frames handed over as NT sections. **Not Wayland** | ADR 0128, 0136, 0224 |
| Login screen | None ("Log Out ends without a login screen") | ADR 0117 |

## 3. Experiments: where each desktop stopped

All runs used one boot of the Debian disk: `dpkg --configure -a` (54 s), then `apt-get install
--no-install-recommends weston seatd` (1 min) and `kwin-wayland mutter` (193 packages, 2.5 min),
from the console. Poly's compositor was stopped first (`pkill poly-compositor`), so DRM master
and the input grabs were free and the console had the screen back. The console log of every step is
in the hand-back folder (`console-steps.txt`).

| Run | First stop (evidence) |
|---|---|
| weston 14 `--backend=drm`, seatd (default) | seatd's VT-bound seat: `Could not open tty0 to update VT: No such file or directory`, so the client is never active and opening `/dev/dri/card0` fails with `Operation not permitted`. weston: "no drm device found" |
| the same, `SEATD_VTBOUND=0` (seatd(1)) | **The DRM path works:** "using /dev/dri/card0", "supports atomic modesetting", "supports GBM modifiers", GL ES 3.2 llvmpipe, "Using GL renderer". Then `udev: failed to create the udev monitor` (`socket(AF_NETLINK, …, NETLINK_KOBJECT_UEVENT) = -1 EPROTONOSUPPORT`). "failed to create input devices" is fatal, even with `--continue-without-input` |
| KWin 6.3.6 `kwin_wayland --drm` (root, no logind) | Never opens the device. It enumerates `card0` through libudev, but the uevent socket fails with EPROTONOSUPPORT and `/run/udev/data/c226:0` is ENOENT, so: "failed to open drm device at /dev/dri/card0", "No suitable DRM devices have been found". It made no D-Bus or seat connection in this run |
| Mutter 48.7 `mutter --wayland --display-server` | "Failed to find any matching session": it reads `/run/systemd/users/0` (ENOENT, no logind) and tries `dbus-launch` for a session bus (not installed) |
| systemd-logind by hand (system bus started with `dbus-daemon --system`) | "Failed to create udev watchers: Protocol not supported" (the same uevent socket) |
| KWin nested in Poly's desktop (`kwin_wayland --socket …`) | No window: `DRM_IOCTL_MODE_CREATE_DUMB failed: Permission denied` (its Wayland backend allocates on the render node, where Poly refuses dumb buffers, as Linux does), then "Could not find a suitable render format" |
| **Mutter nested in Poly's desktop** (`dbus-run-session -- mutter --wayland --nested`) | **Runs.** It is a window on Poly's desktop with its own Xwayland on `:1`, and `weston-terminal` runs inside it. Warnings: no `locale1`, no colord, Xwayland glamor falls back to software |

![Mutter nested on Poly's desktop with a Wayland terminal inside](img/desktop-sessions-mutter-nested.png)

Not tried, to keep within the owner's "light" scope: sway, a full Plasma session
(`startplasma-wayland`) and a full GNOME session (`gnome-session`, `gnome-shell`). Each of them stops
at the same plumbing first.

## 4. Recommendations

### 4.1 How the choice works

- **Session files are the menu.** Each desktop's package installs its entry in
  `/usr/share/wayland-sessions/*.desktop` (freedesktop.org's convention). Poly's own session gets one
  too and stays the default.
- **A login screen of Poly's own** (native, in the Alpine look, part of poly-compositor or a small
  native greeter beside it) lists those sessions and the users, and remembers the last choice per user.
  Logging out comes back to it, so switching desktops needs only a log-out, never a reinstall.
- **Adding a desktop is a package install into the Debian base root** (ADR 0132/0173):
  `apt-get install` of a small task package per desktop (Plasma's, GNOME's). Settings → Desktop offers
  it, and so does the installer's "desktop" step (Phase 12). Removing it is `apt-get remove`
  plus `autoremove`, as for apps (ADR 0180). Poly's own session never depends on it.
- **The greeter hands the screen over.** For a foreign session it gives up DRM master and its input
  grabs, then starts the session under a logind session (PAM, `XDG_SESSION_TYPE=wayland`). When the
  session ends it takes the screen back. That needs a "drop master on hand-over" path in
  poly-compositor and VT semantics in the kernel (batch B1).
- **The login manager: use Debian's own, not a rewrite.** Run Debian's `systemd-udevd`, the system
  `dbus-daemon` and a logind unchanged in the base root, started by Poly's init for a foreign session
  only. Whether that logind is systemd-logind (it expects systemd as PID 1 for session scopes) or
  **elogind** (made for systems without systemd as PID 1) is batch B2's first question. GNOME's
  `gnome-session` also expects a `systemd --user` manager: B4 decides whether Poly runs one for GNOME
  sessions. A clean-room `login1` service of Poly's own (from the public D-Bus API documentation) is
  the fallback if neither runs, and it would be larger.

### 4.2 Windows (and later macOS) apps under GNOME or Plasma

| Option | What it means | Cost | Verdict |
|---|---|---|---|
| **A. Wayland bridge** | A native Poly process (`poly-wl-bridge`) speaks the desktop link (`/dev/win32k`) the way poly-compositor does, and shows each NT top-level as an `xdg_toplevel` (popups as `xdg_popup`) of the foreign compositor. Frames go as `wl_shm` buffers from the NT sections. Input, focus, resize and close go back as link records | M–L: the link's client side moves into a shared crate; sections must be passable as fds; the frame is drawn client-side where the compositor offers no server-side decorations (GNOME offers none, KWin does through `xdg-decoration`) | **Recommended** |
| B. Xwayland | NT windows become X11 clients | Two translations, X11's weaker model (scaling, input); duplicate work | No |
| C. Nested | poly-compositor runs as one window of the foreign desktop and draws Windows apps inside it | Small (poly-compositor needs a Wayland-client output), but the apps live in a box, not on the desktop | Only as an early stopgap |

The bridge is per window, which is what the link already is. Mac apps reach the same link later
(ADR 0107), so they come with it. Under Poly's own session nothing changes.

### 4.3 Poly's own desktop features under a foreign desktop

- **Launcher.** GNOME and Plasma read `/usr/share/applications` and `~/.local/share/applications`.
  Poly writes XDG desktop entries for Linux apps already. Windows (and later Mac) apps need entries
  too, whose `Exec` starts them through the NT personality (ADR 0230's shortcuts → launcher entries).
- **The installers' front door.** Poly's installer is an ordinary app with a desktop file. It
  registers itself as the MIME handler for `.deb`, `.rpm`, AppImage, `.exe`/`.msi` (and later
  `.dmg`/`.pkg`), so a double click in Nautilus or Dolphin opens it as it does in Files.
- **Notifications.** The foreign desktop owns `org.freedesktop.Notifications`. The bridge turns
  Windows notifications (tray balloons, toasts, ADR 0200) into D-Bus notifications, and tray icons into
  StatusNotifierItem (Plasma shows them; GNOME needs an extension).
- **Settings.** Poly's Settings stays an app. Shared settings (scale, keyboard layout, theme) are
  read from the foreign desktop (GSettings or KConfig) for the Windows side's DPI and layout (ADR
  0209, 0231). That is a later polish item.

## 5. Batches

Sizes: S ≈ one agent-day, M ≈ 2–4, L ≈ a week or more. "Spec" marks batches that need a spec agent to
record Linux's behaviour on a real Linux VM (uevent messages, sysfs attributes, VT and DRM-master
ioctl semantics) before an implementer starts.

| Batch | What | Size | Spec | What the user sees |
|---|---|---|---|---|
| **B0 udev in the kernel** | `NETLINK_KOBJECT_UEVENT` (multicast group, the "add"/"remove"/"change" messages, coldplug by writing `add` to a `uevent` file), `/sys/class/input/{eventN,inputN}` with their attributes (name, id, capabilities, `dev`), `/sys/dev/char/13:*`. Then Debian's `systemd-udevd` runs and fills `/run/udev/data` | M | yes | Nothing yet. weston on DRM with keyboard and mouse from the console (a boot test) |
| **B1 VT and seat semantics** | `/dev/tty0`, `/dev/tty2..6`, VT_SETMODE/VT_ACTIVATE/VT_WAITACTIVE/KDSETMODE/KDSKBMODE with the switch signals; DRM master dropped and taken on a VT switch; poly-compositor releases master and its grabs on request. The cursor ioctls/plane (all three compositors use a hardware cursor where one exists) | M | yes | weston and sway on stock seatd (VT-bound); Ctrl+Alt+F-keys between Poly's desktop and a console |
| **B2 Login plumbing** | The system `dbus-daemon`, `systemd-udevd` and elogind or systemd-logind started by Poly's init; cgroup v2 mounted (kernel: the cgroup2 file system, M on its own); PAM session; polkit | M–L | yes (cgroup2) | `kwin_wayland --drm` and `mutter --display-server` start from a logind session |
| **B3 KDE Plasma session** | Plasma's minimal task package in the base root; `startplasma-wayland` from a logind session; fix what stops (expect: PipeWire, `kded`, portals, a network stand-in) | M–L (unknown) | some | Plasma's desktop, started by hand from the console |
| **B4 GNOME session** | `gnome-session`/`gnome-shell`; decide on `systemd --user` for GNOME sessions; the same services as B3 | L (unknown) | some | GNOME Shell, started by hand |
| **B5 Session choice** | Poly's login screen (users, session list from `wayland-sessions`, last choice remembered); hand-over and take-back of the screen; Settings → Desktop installs or removes a desktop | M | no | **The feature:** pick Poly, Plasma or GNOME at login; switch by logging out |
| **B6 Windows apps bridge** | `poly-wl-bridge` (§4.2 A): top-levels, popups, input, focus, client-side frame under GNOME, DPI from the session's scale | M–L | no | Notepad++/Chrome for Windows as normal windows in Plasma and GNOME |
| **B7 Desktop features** | XDG entries for Windows apps, the installer as MIME handler, notifications and tray through D-Bus | S–M | no | Double-click a `.exe` in Dolphin or Nautilus; Windows apps in GNOME's and Plasma's app grids |
| **B8 Lean OS installer** | Six steps: language and keyboard, disk, user, desktop (Poly / GNOME / Plasma), default browser, done. Needs Phase 11's UEFI boot and disk installer | M | no | The installer picks the desktop; Poly's stays the default |

B0 and B1 are kernel work and come first. They also help Poly's own desktop: hotplug, and libinput
for Linux apps that open input devices. B3 and B4 are open-ended. They are probes first, with a stop
rule: if one needs more than its batch, record the stops and ask the owner. B6 does not depend on
B3/B4: it can be built and tested under weston (B1).

## 6. Risks

- **GNOME and systemd.** Recent GNOME sessions are started by `systemd --user`. Running a user
  manager without systemd as PID 1 is unproven on Poly, and is the biggest unknown here.
- **Disk.** KWin and Mutter alone pulled 193 packages. Full desktops are larger, so the base root
  needs room. The interactive disk's default 4 GB spare (`INTERACTIVE_SPARE_MB`) is too small for
  both; these runs used `--disk-size=8G`.
- **Software rendering.** Without a host GPU, Mutter and KWin run on llvmpipe, which is slow (as
  Chrome is today, ADR 0218). virgl/Venus (ADR 0119) are the way out where the host has a GPU.
- **Explicit sync.** No DRM syncobj: weston disabled it ("missing EGL_ANDROID_native_fence_sync").
  Newer KWin and Mutter prefer it, and B2–B4 will show whether they insist on it.
