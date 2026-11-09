# 0124: The desktop becomes native Poly user space, looks 1:1 like Alpine Shell, and everything that ships builds on Windows and Linux

**Status:** Accepted (owner decisions, 2026-11-01). Roadmap Phase 6. It
supersedes the part of [ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md)
that keeps the ridge a pure status strip with nothing dropping down from
it, and changes where the compositor of
[ADR 0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md)
runs. Window frames ([ADR 0106](0106-wayland-server-core-and-software-compositor.md))
and per-window menus (ADR 0107) are unchanged.

## Context

Phase 6 delivered a working desktop: the compositor, the shell (ridge,
launcher, window management, notifications, settings), XWayland, GTK, Qt
and VLC. The compositor and the shell's helpers are **Linux programs**:
static glibc binaries that run on Poly's Linux personality, because that
was the only working user-space interface. That has three costs:

- Building the desktop needs a Linux host (a Linux C library and C
  compiler), so `cargo xtask desktop` does not work on Windows, while
  `cargo xtask run` does (since `tools/xtask/src/hostfs.rs`).
- The desktop depends on one guest personality. Windows and macOS apps
  would reach Poly's own desktop through Linux's.
- The look was a first approximation of Alpine Shell, and the owner finds
  it far from the original (shadows, spacing, fonts, the panels).

## Decisions (owner)

1. **The desktop is native Poly user space.** The compositor, the shell
   (ridge, launcher, panels, notifications, settings, window management)
   and their helpers become native Poly programs: pure Rust on Poly's own
   system call interface, through a small runtime, with no Linux, Windows
   or macOS personality in between. They run as **user-space processes,
   not in the kernel**, so a fault in the desktop cannot take down the
   system. Apps of every world keep reaching it through their own
   protocols: Linux apps through Wayland and X11 (XWayland), later Windows
   apps through win32k and macOS apps through their window server
   interfaces, all bridged to the native compositor.
2. **The shell looks 1:1 like Alpine Shell**, the owner's Windows shell
   (`alpine-shell` repository; a running copy is on the Windows machine
   `homeserver2`). That includes what ADR 0107 left out: panels that
   extend from the ridge (quick settings, calendar and the like, as in
   Alpine Shell), notifications at the top right, Alpine Shell's settings
   window, fonts, colours, spacing, radii, soft shadows and blur. "1:1" is
   checked by comparing Poly's screenshots with Alpine Shell's at the same
   size.
3. **Poly may reuse Alpine Shell's assets.** The owner, who owns both
   projects, grants Poly the use of Alpine Shell's icons, colour and
   spacing tokens, measurements and wallpaper. Its fonts are Geist, Geist
   Mono and Averia Serif Libre under the SIL Open Font License; Poly takes
   them from their upstream releases (pinned and checked, like DejaVu) and
   ships their licences. Alpine Shell's code is TypeScript/React for
   Electron, so it is re-written in Rust rather than copied.
4. **Everything that ships in Poly builds on a Windows host as well as a
   Linux host**, with the same commands. Native programs are pure Rust
   linked by `rust-lld`, which needs no host C toolchain. Only tooling
   that needs Linux by nature stays Linux-only, and reports so on other
   hosts instead of breaking the build: the diff runner against real Linux,
   root disks assembled from a Linux distribution's packages (the toolkits'
   GTK, Qt and VLC), and boot tests that drive QEMU through Unix sockets.
5. **The work runs in parallel**: the 1:1 look (the drawing code in the
   host-tested shell and graphics crates) and the native runtime with the
   desktop's move onto it.

## Consequences

- New roadmap milestones in Phase 6, in order: the native interface and
  runtime, the desktop running natively, the Alpine Shell look, and the
  Windows build of the desktop. A preinstalled browser to download and
  install apps of every world is a later milestone (Phase 8).
- The rendering crates need what Alpine Shell's look uses and the first
  shell did not: anti-aliased rounded shapes, Gaussian shadows, backdrop
  blur, and Alpine Shell's fonts at its sizes.
- The native runtime defines how a native process talks to the kernel
  (the native system call table, how a native ELF is recognised, display,
  input, sockets shared with the personalities so Linux clients can still
  connect to the compositor). Its own ADR records those choices.
- `CLAUDE.md` gains the rules: build on Windows and Linux; the desktop is
  native; the UI follows Alpine Shell 1:1.
