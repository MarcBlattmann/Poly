# Running Poly

The short version is in the [README](../README.md): `cargo xtask start`.
This page has the other commands, the repository layout and the known limits.

## Commands

Needs Rust (the pinned version installs automatically through
`rust-toolchain.toml`) and `qemu-system-x86_64`; `--busybox` takes a
statically linked BusyBox from the host (`busybox-static` on Debian and
Ubuntu) or builds the pinned release.

```sh
cargo xtask start   # a menu of everything that boots: the desktop (with Chromium, on by default), the shells, boot tests, CI
                    # (it remembers your choice: next time Enter runs it again; --last skips the menu)
cargo xtask desktop --chromium # Poly's desktop with Chromium, the default browser (the menu's default; the first
                    # start downloads about 230 MB of Debian packages and makes a 1.3 GB disk image, kept after that)
cargo xtask desktop --chrome-win # Poly's desktop with Chrome for Windows in the launcher (the menu's
                    # "Desktop with Chrome for Windows"; see below)
cargo xtask desktop # the desktop without Chromium, from a small initramfs (no test apps in the image or the launcher)
cargo xtask desktop --test-apps # add the test apps: Poly Demo, Wayland Test Client, Demo Window, Poly Fill, Send Test Notification
cargo xtask run     # boot the kernel in QEMU (Ctrl-A X to quit)
cargo xtask run --busybox   # boot into BusyBox's init and shell (`poweroff` quits; add console=tty1 for the screen)
cargo xtask test    # host unit tests + boot tests in QEMU
cargo xtask boot-tests --list   # the boot test groups; `boot-tests <group>` runs some
cargo xtask ci      # everything CI checks (`ci lint`, `ci host-tests`, ...: one part)
```

**Chrome for Windows** (ADR 0216): in the menu, *Desktop with Chrome for Windows* (or
`cargo xtask desktop --chrome-win`; add `--release` for the fast kernel, which the menu
does). It works the same from Git Bash, PowerShell and a Linux shell. The first start
downloads Chrome for Testing (the pinned version, about 210 MB, shared by all checkouts of
the repository) and writes a disk with Chrome under `C:\Program Files\Chrome`
(`target/poly-chrome-try-disk.img`, about 3 minutes); then Poly's desktop opens in QEMU's
window with network (QEMU's user networking), 4 GB and 4 CPUs (`--memory=`, `--cpus=`), on
WHPX (Windows) or KVM (Linux) when the host has it. Press the Windows key or Ctrl+Space, pick
**Chrome (Windows)** and wait about a minute for its window. Chrome's profile (bookmarks,
history, logins) is on the disk, so it survives a reboot; only `--fresh-disk` (or a new Chrome
version, which makes the disk again) starts it empty. The keyboard layout is the desktop's own
(Settings, or the menu's; ADR 0209). Chrome starts with its **default settings**, as on
Windows: its sandbox on and its GPU process on Poly's software Direct3D 11 device, with no
"unsupported command-line flag" bar (only Chrome for Testing's own "for automated testing"
bar). Its switches are in `chrometry::FLAGS`, which is empty (ADR 0216 addendum). How long it
takes: the first start on a checkout that has never built anything spends about 10 minutes on
the download and the builds (Win32 modules about 4 minutes, kernel about 2); later starts boot
in seconds, and Chrome's window shows its new-tab page less than a minute after Enter in the
launcher. `cargo xtask boot-tests chrome-try` (after `cargo xtask chrome-win fetch`; Linux or
`polyd`) also starts it from the launcher. It checks that the page paints, that a typed URL
loads and that the GPU process ran once, in its sandbox.

**Installing a Linux app** (ADR 0173): on the desktop with Chromium, download a `.deb`
from Debian 13's archive (for example from packages.debian.org) and click it in
Chromium's downloads, or double-click it in Files: Poly's installer shows what it
is, and Install has Debian's own `apt` and `dpkg` install it with what it needs (the
first time on a disk it configures the disk's own packages first, about half a
minute). The app is then in the launcher and in Files' Applications. A disk made
before this needs **Fresh disk** once (`--fresh-disk`). `cargo xtask boot-tests
linux-installer` runs the whole thing with galculator.

**Install Google Chrome for Linux** (ADR 0218): on the desktop with Chromium (`cargo xtask
desktop --chromium`, or the menu's default; a disk made before this needs **Fresh disk** once):

1. In Chromium open `https://www.google.com/chrome/`, click **Download Chrome**, keep *64 bit
   .deb (For Debian/Ubuntu)* and click **Accept and Install** (or open
   `https://dl.google.com/linux/direct/google-chrome-stable_current_amd64.deb`). The 141 MB file
   arrives in about half a minute.
2. Click it in Chromium's downloads (the arrow at the top right, then the file; the installer can
   take a minute to come up the first time, and may open behind Chromium). Poly's installer shows
   *google-chrome-stable, The web browser from Google*: click **Install**. On a new disk it first
   configures the disk's own packages (one to six minutes, longer while Chromium is busy), then
   fetches the package lists and installs Chrome with 13 packages it needs from Debian's archive
   (about a minute and a half).
3. **Open**, or later the launcher (Super, type *google*, Enter): Chrome starts as user `poly` with
   its sandbox. The first start shows Google's Terms (**Accept**), Welcome (**OK**) and the sign-in
   page (**Stay signed out**), then its new tab page.
4. Downloads land in `/home/poly/Downloads`: **Downloads** in Files' sidebar. A downloaded `.deb`
   opens in the installer from there.

Chrome draws in software (its GPU process finds no EGL on the disk and exits), so scrolling shows
unpainted tiles for a moment. `cargo xtask chromium fetch-chrome` then `cargo xtask boot-tests
linux-chrome` installs and starts it in a boot (Linux or `polyd`; the guest needs network).

**Uninstalling and AppImages** (ADR 0180): in Files' Applications folder, an app the
installer owns (a package installed after the disk was made, or an AppImage) has
*Uninstall* in its right-click menu (and on the Delete key); the installer's window
says what goes and asks, then Debian's own `apt-get remove` and `autoremove` or the
AppImage's files are taken away, and the launcher entry is gone. Chromium and Poly's
own apps are not uninstalled. Double-clicking a downloaded `.AppImage` asks *Run* or
*Install* (a copy in your user folder, listed in the launcher and in Applications).
`cargo xtask boot-tests linux-installer-2` runs both, with galculator and CPU-X.

On GitHub, CI runs the same parts as parallel jobs: lint, host tests, the boot
tests in 11 shards (`cargo xtask boot-tests --list` shows the groups of each),
the coverage checks and the pipeline checks. It takes about 4 minutes; the one
required check is `ci`, which passes when every other job did.

QEMU runs on the host's hypervisor when it can (much faster): KVM on Linux,
the Windows Hypervisor Platform (WHPX) on Windows (turn on the Windows
feature "Windows Hypervisor Platform"), and on its emulator (TCG)
otherwise; `POLY_QEMU_ACCEL=tcg` forces the emulator. `cargo xtask desktop`
sizes the screen for the host's display (a 200% display gets 2560x1600
drawn at scale 2, so it is sharp); `--size=WxH` and `--scale=N` override it.
`--memory=SIZE` and `--cpus=N` (on `desktop`, `run`, `chromium run`,
`toolkits`, and in the menu's Resources block) give the guest more RAM or
CPUs than its default, up to what the host has (the kernel starts at most 16
CPUs; it has no RAM limit).

On Windows, `cargo xtask run` builds and boots the kernel natively (QEMU
for Windows on the `PATH`). The parts that need a Linux host report so: the
diff runner against real Linux, root disks assembled from Linux packages,
and the boot tests that take screenshots or type through QEMU's Unix
sockets. Run those in WSL or a Linux container; `cargo xtask start` shows
them greyed out with the reason. In Git Bash, `winpty cargo xtask start`
gives the full menu (mintty is not a Windows console; without winpty it
asks with a numbered prompt instead).

## Repository layout

| Path | What |
|---|---|
| `kernel/` | The kernel (its own Cargo workspace): boot, CPU setup, memory, scheduler, drivers, file systems, network, process and syscall routing |
| `crates/` | `no_std` libraries shared with the kernel, unit-tested on the host |
| `personalities/` | Linux, NT (with the Windows DLLs under `nt/win32/`) and XNU system call layers, and the native Poly one |
| `userland/` | The desktop's native programs: compositor, Files, Terminal, notifications and a Wayland demo client |
| `tools/xtask/` | Build, run and test driver |
| `tools/pipeline/` | The agent pipeline: call tracer, diff runner, spec and hand-off tools ([docs](AGENT_PIPELINE.md)) |
| `coverage/` | The coverage map: one file per library or call table |
| `tools/coverage/` | Maintains the coverage map and the progress section of the README |
| `specs/` | Behaviour specs and visible tests, written from the real OS |
| `tests/` | Test programs run on the real OS and on Poly (Linux and Windows) |
| `docs/` | [Architecture](ARCHITECTURE.md), [decision records](adr/), [project brief](PROJECT_BRIEF.md), [agent pipeline](AGENT_PIPELINE.md), [Windows](windows/) and [macOS](macos/README.md) notes |

## Known limits

- 100% API coverage does not mean every app works. Undocumented quirks only
  show up in trace replays.
- Kernel-mode anti-cheat, DRM and driver-installing apps may never work.
- Apple services (iCloud, App Store, iMessage, Apple Pay) can be blocked server-side.
- Microsoft and Apple add APIs every year, so the coverage map is a treadmill.
- GPU output cannot match pixel for pixel. Visual tests use a noise floor
  measured on the real OS.
