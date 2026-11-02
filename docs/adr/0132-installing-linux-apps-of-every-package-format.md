# 0132: Installing Linux apps of every package format, as simply as on macOS or Windows

**Status:** Accepted as a goal (owner, 2026-11-02); two choices are open
for the owner (marked ⚖️ below and in the roadmap). Roadmap Phase 8. It
extends the milestone "A preinstalled browser (Chromium) downloads and
installs Linux and Windows apps, which then appear in the launcher"
(ADR 0124).

## Context

Linux apps come in many package formats: `.deb` (Debian, Ubuntu and their
derivatives), `.rpm` (Fedora, openSUSE, RHEL), Flatpak (Flathub), Snap
(Snap Store) and AppImage, with plain tarballs besides. On a Linux
distribution each needs its own tool, and some only work on the
distribution they were built for. On macOS or Windows a user downloads an
app, opens it and it is installed; it shows up with the other apps and can
be removed again.

The owner wants Poly to install every Linux format that way. Poly runs
Linux programs unchanged on its Linux personality (no Linux kernel
underneath), so the formats' own tools (`dpkg`, `apt`, `rpm`, `dnf`,
`flatpak`, `bubblewrap`, `snapd`) can run on Poly as they are. Poly does not
rewrite them. What Poly has to provide is the kernel interface they need,
the desktop integration, and one front door for the user.

## Decisions

1. **One installer for every format.** Opening a package from a download,
   the file manager or a link in the browser starts Poly's installer, a
   native Poly app drawn like the rest of the shell (ADR 0124, 0127). It
   shows the app's name, icon, version, publisher, size and, where the
   format declares them, its permissions. Then one button installs it. The
   app then appears in the launcher (desktop file, icons and MIME types
   registered), and it is removed from the launcher or from Settings → Apps.
   Updates use the format's own channel (apt/dnf repositories, Flathub, the
   Snap Store, AppImage's update information) and surface as Poly
   notifications. Windows installers (`.exe`, `.msi`, Phase 7) and later
   macOS images and packages (`.dmg`, `.pkg`, Phase 10) go through the same
   front door.
2. **The format is recognised from the file's content, not its name:**
   - `ar` archive with `debian-binary`: `.deb`;
   - RPM lead magic `ed ab ee db`: `.rpm`;
   - ELF with the AppImage magic `AI` and type byte at offset 8: AppImage;
   - `.flatpakref`/`.flatpakrepo` keyfiles and `.flatpak` bundles: Flatpak;
   - squashfs with `meta/snap.yaml`: Snap.
3. **Each format keeps its own world, not one merged `/usr`.**
   - **`.deb`:** installs into a shared base root of the distribution it
     targets. Poly's real `apt` and `dpkg` resolve dependencies from that
     distribution's archive and run the maintainer scripts there. The
     package's `Depends` and version strings choose between bases (e.g.
     Debian or Ubuntu). This is how Poly already assembles Debian roots
     (ADR 0120, 0130), but done on Poly by the distribution's own tools.
   - **`.rpm`:** the same, with `rpm` and `dnf` in a Fedora-family base.
   - **AppImage:** copied into the user's applications folder. Poly
     mounts its embedded squashfs image read-only itself, so AppImages run
     without FUSE (the AppImage runtime's FUSE path stays supported where it
     is asked for).
   - **Flatpak:** `flatpak` and `bubblewrap` run unchanged, with Flathub
     as the default remote. Their sandbox is real (user, mount and PID
     namespaces, seccomp filters). The desktop portals they talk to over
     D-Bus (`xdg-desktop-portal`: file chooser, open URI, notifications,
     screenshots) are answered by Poly's shell. This is the Phase 8 file
     picker and notifications, shared with Windows and macOS apps.
   - **Snap:** see the open choice below.

   An installed app runs in its world's root through a mount namespace (or
   chroot), with the user's home, fonts, the Wayland/X11 sockets, audio and
   the session bus shared into it, so it looks and behaves like any other
   app on the desktop.
4. **What Poly's kernel and Linux personality need**, specified and tested
   through the usual pipeline (spec agent on real Linux, implementer from
   the spec):
   - squashfs (xz, zstd, lz4, lzo; read-only);
   - loop devices or direct file mounts;
   - `/dev/fuse`;
   - overlayfs where a format uses it;
   - user, mount, PID, network and UTS namespaces with `pivot_root`,
     `setns` and the new mount API;
   - seccomp filters;
   - enough of what `dpkg`/`rpm` maintainer scripts call (users and groups,
     `ldconfig`, triggers).

   Chromium's sandbox (Phase 6) already needs the namespaces and seccomp.

## Open choices (owner) ⚖️

- **Which distributions back `.deb` and `.rpm`.** Proposal:
  - Debian stable for `.deb`, plus Ubuntu LTS for packages built for Ubuntu
    (recognised from their dependencies);
  - Fedora for `.rpm`; openSUSE packages could get their own base later.

  More bases mean more packages work unchanged, at the cost of disk space
  per base.
- **How Snap runs.** `snapd` expects systemd, mounts each snap's squashfs,
  and confines apps with AppArmor and seccomp. The options:
  - (a) run `snapd` itself, with enough of systemd's service interface and
    of AppArmor for it;
  - (b) Poly unpacks snaps, mounts their squashfs and runs them with
    Flatpak-style namespaces and seccomp, without `snapd` (no Snap Store
    updates unless Poly speaks its API);
  - (c) leave Snap until the others work.

  Proposal: (c), then (b).

## Consequences

- New Phase 8 milestones, in order:
  - the kernel features above;
  - the installer front door;
  - AppImage;
  - `.deb`;
  - `.rpm`;
  - Flatpak;
  - the Snap decision and Snap;
  - Windows and macOS installers through the same front door.
- Base roots cost disk space (a few hundred MiB each), shared by every app
  of that family. Uninstalling the last app of a base offers to remove the
  base.
- Packages are only as compatible as the base they land in: a `.deb` built
  for a newer Ubuntu than the installed base may need that base's newer
  release. The installer says so instead of half-installing.
- Proprietary packages (e.g. Google Chrome's `.deb`, Spotify) are
  downloaded by the user from their publishers. Poly never ships them in
  its images (as with Chromium, ADR 0130).
