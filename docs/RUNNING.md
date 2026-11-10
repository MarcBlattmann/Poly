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
cargo xtask desktop # the desktop without Chromium, from a small initramfs (no test apps in the image or the launcher)
cargo xtask desktop --test-apps # add the test apps: Poly Demo, Wayland Test Client, Demo Window, Poly Fill, Send Test Notification
cargo xtask run     # boot the kernel in QEMU (Ctrl-A X to quit)
cargo xtask run --busybox   # boot into BusyBox's init and shell (`poweroff` quits; add console=tty1 for the screen)
cargo xtask test    # host unit tests + boot tests in QEMU
cargo xtask boot-tests --list   # the boot test groups; `boot-tests <group>` runs some
cargo xtask ci      # everything CI checks (`ci lint`, `ci host-tests`, ...: one part)
```

**Installing a Linux app** (ADR 0173): on the desktop with Chromium, download a `.deb`
from Debian 13's archive (for example from packages.debian.org) and click it in
Chromium's downloads, or double-click it in Files: Poly's installer shows what it
is, and Install has Debian's own `apt` and `dpkg` install it with what it needs (the
first time on a disk it configures the disk's own packages first, about half a
minute). The app is then in the launcher and in Files' Applications. A disk made
before this needs **Fresh disk** once (`--fresh-disk`). `cargo xtask boot-tests
linux-installer` runs the whole thing with galculator.

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
