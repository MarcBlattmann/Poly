# 0130: Chromium from pinned Debian packages, unpacked in pure Rust on every host

**Status:** Accepted. Roadmap Phase 6, toward the milestone "Chromium (the
Linux build) runs on the desktop with its sandbox enabled". The spec side
(what Chromium needs) is `docs/chromium-linux-gaps.md`; the scenario tests
are `specs/linux/apps/chromium.yaml`.

## Context

The toolkits' root disk (ADR 0120) copies the build host's own GTK, Qt and
VLC packages, so it can only be made on a Linux host with those packages
installed. Chromium cannot come that way: the poly-dev image is Ubuntu
24.04, whose `chromium` is a snap, and Google Chrome's binaries may not be
in a Poly image. The owner also wants a build on Windows to give the same
desktop with Chromium in the launcher (ADR 0124: everything that ships
builds on Windows and Linux).

## Decision

- **Debian 13's Chromium.** `chromium`, `chromium-common` and
  `chromium-sandbox` (156.0.8078.73-1~deb13u1, trixie-security) and the
  packages they depend on, taken from two pinned snapshots of
  snapshot.debian.org (trixie at 13.7, trixie-security of 2026-10-31).
  Each snapshot's `Packages.xz` is pinned by the size and SHA-256 its
  `InRelease` lists; `cargo xtask chromium lock` follows `Depends` and
  `Pre-Depends` from a few seeds (the first alternative that exists, a
  virtual package's first provider) and writes `tools/xtask/chromium.lock`:
  one line per package with version, archive, size, SHA-256, set (`disk`,
  `boot` for the initramfs's `busybox-static`, `reference` for the spec
  agent's tools) and pool file name. Only the lock is committed.
- **Pure Rust on every host.** `cargo xtask chromium fetch` downloads with
  `curl` and checks every file with `sha256.rs`; a `.deb` is read as an
  `ar(5)` archive, its `data.tar.xz` decompressed with the `lzma-rs` crate
  (pure Rust, no C toolchain) and read as a tar archive by a small reader
  (ustar, GNU long names, pax paths). Regular files go to numbered blobs
  under `target/chromium/disk/`, never under their package names, so the
  host's file name rules, links and modes do not matter; the image's
  paths, modes (the set-user-ID `chrome-sandbox` included), link targets
  and times come from the archive entries. Merged `/usr` is applied (a
  package's `/bin/x` lands in `/usr/bin/x`; `/bin`, `/sbin`, `/lib`,
  `/lib64` are links). The image is written by `poly-ext2`'s formatter on
  every host (`rootdisk::write_image_poly_ext2`), not `mke2fs -d`, which
  would take modes from the staged host files.
- **No maintainer scripts.** What Chromium needs of them is written by the
  tool: users (`root`, `poly` uid 1000), a fixed `/etc/machine-id`. The
  GSettings schemas stay uncompiled (a GLib warning, nothing else).
- **The boot.** BusyBox's init from Debian's `busybox-static` (set `boot`)
  mounts the disk and chroots into it, as the toolkits' disk does; the
  desktop is the native compositor (ADR 0126). Chromium refuses its
  sandbox as root, so the launcher's entry runs `/usr/local/bin/poly-chromium`,
  which gives user `poly` a second name of the compositor's socket and
  changes to that user with util-linux's `setpriv`. `cargo xtask chromium
  run` boots it on any host with the desktop in QEMU's window.
- **The reference.** The spec agent observes the same packages on Linux:
  `cargo xtask chromium root --reference` unpacks every locked package
  with `dpkg-deb` (Linux only), with weston, Xvfb, Xwayland, xdotool, xwd
  and strace; `cargo xtask chromium apptest` runs the scenario tests there
  (pivot_root into the root, in a mount namespace) and on Poly, and
  compares stdout and exit status.

## Consequences

- `cargo xtask chromium disk` makes the same 1.3 GB image on Windows and
  Linux (checked: `e2fsck -fn` clean, `chrome-sandbox` mode 4755); about
  230 MB of packages are downloaded once.
- xtask gains one crate from crates.io, `lzma-rs` (host tool only).
- Debian's security updates mean a new lock now and then: change the
  snapshot dates and hashes in `chromium.rs`, run `cargo xtask chromium
  lock`, and the scenario tests on the reference.
- Boot group `chromium` skips itself until the packages are fetched, so CI
  does not download them.
- Poly's disk is entered with chroot, where Linux does not let a process
  make a user namespace: Chromium takes its set-user-ID sandbox there
  (docs/chromium-linux-gaps.md, "Which sandbox"). Whether the desktop
  disks should be entered with `switch_root` instead is left to the owner.

## Amendment: Chromium's disk becomes the root (switch_root), not a chroot

The owner decided (2026-11-02) that Chromium's disk is entered the way a
distribution's initramfs hands over to its root, so that Chromium uses
its **user-namespace sandbox**, which Poly implements, rather than the
set-user-ID helper. Only Chromium's disk changes; the root disk and the
toolkits' disk still chroot.

- **The first program is the initramfs's shell**, `poly.init=/bin/sh
  poly.init.arg=/etc/init.d/switch-root` (`initramfs::SWITCH_ROOT_INIT_ARGS`;
  the kernel's first program cannot be a `#!` script). As pid 1 it runs
  `rcS` and `rootdisk` as before (the disk on `/mnt/root`, `/dev`,
  `/dev/pts`, `/dev/shm`, `/proc`, `/sys` and `/tmp` bound into it), moves
  the disk's mount over `/` (`mount --move`, mount(2) `MS_MOVE`), changes
  its root to it from inside (`chroot .`) and execs BusyBox's `init` from
  the disk. That is switch_root(8)'s sequence; unlike switch_root it
  leaves the initramfs's files alone, because its `/dev` (the device nodes
  the kernel made) is what the disk's `/dev` shows.
- **The root is the mount namespace's top mount**, which Poly's
  `Vfs::is_chrooted` already treats as not chrooted (Linux likewise lets a
  process whose root is a mount stacked on the namespace's root make user
  namespaces). So `clone(CLONE_NEWUSER)` succeeds and Chromium's zygote
  sandboxes itself with user, pid and network namespaces
  (docs/chromium-linux-gaps.md, batch C).
- **BusyBox on the disk.** The same `busybox-static` binary of the lock is
  copied to `/usr/local/lib/poly/busybox` (`initramfs::DISK_BUSYBOX`), with
  `cttyhack` linked to it, and the disk gets an `/etc/inittab`
  (`chromium::INITTAB`): the desktop once, the disk's bash on the console,
  reboot on Ctrl-Alt-Del, `umount -a -r` at shutdown. The disk's
  `poweroff`/`reboot` (ADR 0059) signal this init as before.
- **Unchanged:** the boot groups, Machine's console protocol and the
  desktop (`poly-desktop` is started by init on the disk instead of
  through `chroot`). The set-user-ID helper is still on the disk
  (mode 4755): `--disable-namespace-sandbox` (the `chromium-setuid-sandbox`
  scenario test) still takes it.
