# 0173: Installing a `.deb`: Poly's installer over Debian's own `apt` and `dpkg`

**Status:** Accepted. Roadmap Phase 8, "`.deb` packages install with their
dependencies and maintainer scripts". It carries out the `.deb` part of
[ADR 0132](0132-installing-linux-apps-of-every-package-format.md) (one
installer for every package format) and decides what that ADR left open:
where the Debian base root is, how it gets `dpkg`'s database, how the
installer is started for a file, who installs, and what the window shows.
It builds on [ADR 0130](0130-chromium-from-pinned-debian-packages.md) and
[ADR 0147](0147-the-desktop-from-chromiums-disk-with-chromium-as-the-default-browser.md)
(the desktop boots from a disk of pinned Debian packages),
[ADR 0134](0134-files-and-terminal-polys-own-native-apps.md) (native apps),
[ADR 0146](0146-app-icons-from-the-icon-theme-and-a-png-decoder.md),
[ADR 0167](0167-busy-pointer-while-an-app-starts.md) and
[ADR 0175](0175-an-applications-folder-in-files.md) (the one list of apps).

## What works

A person downloads a `.deb` in Chromium and clicks it in Chromium's
downloads, or double-clicks it in Files. Poly's installer opens and says
what the package is. One click on **Install** installs it with its
dependencies; the app is then in the launcher and in Files' Applications,
with its icon, without a restart of the desktop, and **Open** starts it.

## What was missing (found by trying it)

`dpkg` and `apt` ran on Poly at the first try: no kernel work was needed for
`.deb` (the namespaces, squashfs and FUSE of ADR 0168 are for the other
formats). What was missing was on the disk:

1. **No package tools and no database.** The disk was Chromium's dependency
   closure, unpacked by the build tool: no `dpkg`, no `apt`, no
   `/var/lib/dpkg`.
2. **No package had ever been configured.** The build unpacks Debian's
   packages on any host and runs none of their maintainer scripts, so
   everything those create was absent: alternatives (`which`, `awk`,
   `pager`), Debian's users and groups, `ld.so.cache`, the MIME database,
   the gdk-pixbuf loader cache, compiled GSettings schemas, GIO's module
   cache. Chromium did not need them; the first GTK app installed aborted at
   start (no image loaded, because the MIME database was missing, because
   `update-mime-database` was skipped, because `which` did not exist).

## Decisions

### 1. The Debian base root is the desktop's root disk

ADR 0132 puts `.deb` packages into "a shared base root of the distribution".
Since ADR 0147 the desktop boots from a disk that is Debian 13: that disk is
the Debian base root, and packages install into `/`. A base of its own behind
a mount namespace is needed when there is a second base (Ubuntu) or when the
desktop's root stops being Debian; neither is the case, so nothing is built
for it.

### 2. Debian's tools, and a database that tells the truth

The disk's seeds gain `apt`, `dpkg`, `debian-archive-keyring` and the rest
of Debian's Essential set (Debian Policy 3.8: every maintainer script may
use it without saying so), and `file`, which `xdg-mime` asks for a file's
type. About 7 MB of downloads more.

The build writes `dpkg`'s database for the packages it unpacked
(`poly_pkg::dpkgdb`, written from dpkg(1), deb-control(5), deb-triggers(5)
and from what a database made by `dpkg` looks like): `status`, with every
package **`install ok unpacked`**, `info/<package>.list`, every package's
control files (maintainer scripts, `md5sums`, `conffiles`, `triggers`) and
the trigger interests. *Unpacked* is exactly what the build did; claiming
*installed* was tried first and is what hid gap 2.

**Debian's own `dpkg --configure -a` then configures the base root, on
Poly.** It runs every package's `postinst` in dependency order and the
triggers: 251 packages in about half a minute. This is what a Debian
bootstrap's second stage does, done by the distribution's tool, so nothing
about what a package needs is rewritten in Poly. The installer runs it before
its first installation on a disk ("Preparing this computer for Linux apps
(once)"), not at boot: the desktop and Chromium do not need it, and a boot
stays as quick as it was.

Three things the build still does, because the scripts expect them from a
system that was bootstrapped the usual way:

- `/etc/passwd` and `/etc/group` are `base-passwd`'s master files plus Poly's
  user (`base-files`' script gives a directory to group `mail`);
- `/var/run` and `/var/lock` are links into `/run` (`base-files`' script
  makes them only while the directories are empty, and init has written
  `/var/run/utmp` by then);
- `/usr/sbin/policy-rc.d` forbids starting services (invoke-rc.d(8)): init
  here is BusyBox's with Poly's own table.

`awk`, which `xdg-mime` needs before anything is configured, keeps its link
from the boot overlay only until `update-alternatives` owns it.

### 3. The archive: the snapshots the disk is made of, signed

`/etc/apt/sources.list.d/debian.sources` names the two pinned
snapshot.debian.org archives of `chromium.rs` (trixie and trixie-security),
verified with Debian's archive keys. What `apt` resolves a dependency to
therefore fits what is on the disk. A snapshot's `Release` file is past its
`Valid-Until` by nature; that check is off for these two sources only.
`apt`'s defaults apply otherwise (recommended packages are installed, as on
Debian). Downloads are retried three times.

### 4. The installer: `poly-installer`, a native app; `poly-pkg`, its logic

`crates/poly-pkg` (`no_std`, host-tested) holds everything that is not I/O:

- the format of a file **from its content** (ADR 0132, decision 2): `.deb`
  is installed; RPM, AppImage, Flatpak references and snaps are recognised
  and the window says that Poly cannot install them (yet, or, for Snap, at
  all) and to look for a `.deb`;
- reading a `.deb` (ar(5), tar, deb(5), deb-control(5)); only the control
  member is read, which is at the start of the file, so a large package is
  described from its first kilobytes; `control.tar` in gzip, xz (every block
  of the stream: `poly-compress` learned that) or zstd;
- the `apt-get` and `dpkg` command lines, the progress lines of
  `APT::Status-Fd`, and what went wrong from their output;
- the model: what the window says and which command is next.

`userland/poly-installer-app` is the window, in the settings window's layout
language (ADR 0127): a package glyph in a rounded box (the app's own icon
once it is installed), the package's name and summary, its facts (version,
installed version if any, size, who it is from, the kind of package, what it
needs), one sentence about what is happening, a progress bar, and the
buttons at the bottom right, the main one solid: **Cancel / Install**, then
**Close / Open**. A failure is the sentence in the danger colour, with the
tool's own words. The window cannot be closed while `dpkg` runs.

The commands run as children with their output on a pipe and their progress
on a second one; nothing is asked (`DEBIAN_FRONTEND=noninteractive`, a
changed configuration file keeps its contents) and a second installer waits
for `dpkg`'s lock.

### 5. Opening a package is the desktop's job

`poly-installer.desktop` (`NoDisplay`: it is no app of the launcher) names
the package MIME types, and `/etc/xdg/mimeapps.list` makes it their default.
Two readers use that, as for the default browser (ADR 0147):

- **Files** opens a double-clicked `.deb` with it;
- **`xdg-open`**, which is what Chromium runs for a clicked download, finds
  the type with `file` and the app with `xdg-mime`.

Both end up running `poly-installer FILE`. That program does not open a
window: it writes **`install <path>`** to the compositor's control pipe and
ends (and withdraws the launch Files announced for it, so the pointer is not
busy for a window that will not come). The compositor starts
`poly-installer --window FILE`, as it starts any app. So the window is
always the desktop's own, whoever asked. Chromium runs as user `poly`; its
launch script gives that user a name for the control pipe, as it does for
the Wayland socket (`poly-as-user`, split out of `poly-chromium`).

The line carries a path and nothing else. The installer installs nothing
before its **Install** is clicked.

### 6. Who installs

The desktop's user installs: root on today's disks, where every app but
Chromium runs as root. The click on **Install** is the consent. There is no
password prompt because there is no account to ask for one; when the desktop
runs as an ordinary user, the window stays the same and the package tools
need a privileged helper with a prompt in front of it. (Native processes
have no credentials of their own yet, ADR 0125, so the hand-over of decision
5 does not rest on a user ID.)

### 7. The app is there without a restart

When the installation is done the installer writes **`rescan`** to the
control pipe: the compositor reads the installed apps again
(`poly_shell::apps::installed`) and the launcher has the new one. Files'
Applications reads the list each time it is shown (ADR 0175). The
installer's **Open** asks the desktop for the app (`open <id>`), with the
desktop file the package's file list names.

## Not in this ADR

- **Uninstall and updates.** The hook for uninstall is
  `poly_shell::applications::Action` (ADR 0175); `dpkg`'s database now knows
  which files an app owns.
- **An Ubuntu base** for packages built for Ubuntu (ADR 0132's second
  base), and with it base roots apart from `/`.
- **RPM, AppImage, Flatpak:** recognised, not installed.
- **Kept disks:** a disk made before this change has no package tools and
  no database; the installer says so and asks for a fresh disk.
- **Stopping an installation** half way, and choosing whether recommended
  packages come along.
- **Two title bars on GTK 3 apps:** GTK 3 draws its own header bar inside
  the compositor's frame. It is how every GTK 3 app looks on Poly's desktop
  today (the `toolkits` group's apps too), not something installing does.

## Tests

Host: `poly-pkg` (formats by content; ar, tar, control data; a `.deb` in
every compression; the database's files and what is read back from them; the
commands, progress and error lines; every state of the model), `poly-compress`
(a multi-block xz stream through the `.deb` test), `poly-installer-app` (the
window with a made-up system: one click installs, three commands on a new
disk, nothing closes it while it runs, failures, both themes),
`poly-wayland` (`install` and `rescan` lines), `xtask` (the lock's `install`
set, the overlay's `awk` link, the launch scripts).

Boot group **`linux-installer`** (shard `userland`; skipped where the
packages are not fetched): Chromium's disk with **galculator**'s `.deb` in
Downloads. Galculator is a small real GTK 3 app from Debian's archive that
is not on the disk and depends on one library (`libquadmath0`) that is not
there either, so `apt` has to resolve and download a dependency. The test
opens the file with `xdg-open` as user `poly` (what Chromium does) and by a
double click in Files, clicks Install, waits for the three commands, checks
`dpkg`'s state, starts the app from the installer and from the launcher, and
finds it in Files' Applications. It takes about 90 seconds on KVM. The guest
downloads from snapshot.debian.org, as the `chromium` group loads a public
page.

The `toolkits` group's GDK error on the owner's PC ("Truncating shared memory
file failed") did not show here: galculator runs with Debian 13's GTK
3.24.49 from the pinned packages, while the toolkits disk is made of the
*host's* GTK packages, which differ between that PC's build container and
CI's runner. That failure is therefore not on this path and was not looked
into further.
