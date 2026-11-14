# 0180: Uninstalling apps from Files, and installing AppImages

**Status:** Accepted. Roadmap Phase 8, the next step of the Linux installer
(owner approved: uninstall, AppImage, a boot test). It continues
[ADR 0173](0173-installing-a-deb-polys-installer-over-debians-own-apt-and-dpkg.md)
(the installer over Debian's own `apt` and `dpkg`, which named uninstall as
not done) and [ADR 0175](0175-an-applications-folder-in-files.md) (the
Applications folder, which left `Action::Uninstall` as the hook), and does the
AppImage part of [ADR 0132](0132-installing-linux-apps-of-every-package-format.md).
Poly ships a browser and no store ([ADR 0147](0147-the-desktop-from-chromiums-disk-with-chromium-as-the-default-browser.md)):
apps come from double-clicking what was downloaded, and now they also go the
other way.

## What works

- In Files' Applications folder, an app the installer owns has **Uninstall**
  in its right-click menu (and on the Delete key). Poly's own apps and
  Chromium have no such row.
- The row asks the desktop, which opens the installer's window for that app:
  its icon, name, what it is, **Cancel** and **Uninstall** (in the danger
  colour). Nothing is removed before that click.
- For a `.deb` app, Debian's own `apt-get remove`, then `apt-get autoremove`
  (the packages that only it pulled in). For an AppImage, its files. The
  launcher entry and the Files entry are gone at once, without a restart.
- A double click on a downloaded `.AppImage` opens the installer, which says
  what it is and asks once: **Run** (from where it is), **Install**, or Cancel.
  Install puts it in the launcher and in Applications; Applications can
  uninstall it again.

## Decisions

### 1. The installer's window is the confirmation

Alpine Shell (`apps/desktop`, Settings, Apps, Installed) expands an app's row
into *Open*, *Open file location* and *Uninstall*; Uninstall is a
`ConfirmButton`: a soft danger button whose first click turns it into a solid
danger button reading "Uninstall <name>", and the second click removes the
app. The launcher has the same action (`runUninstall`: "Removing X…",
"X removed"). Poly's list is Files' Applications folder, which has a menu and
no expanding rows, so the two steps are a menu row, then a window that says
what goes, with the same solid danger button (`pal().danger`, white text, as
Files' own delete confirmation and Alpine's confirm state). The window is
also where the progress and a failure are shown, exactly as for an
installation (ADR 0173), and it cannot be closed while `apt` runs.

Files writes **`uninstall <id>`** to the compositor's control pipe, the same
way it asks for `open <id>` (ADR 0175) and as an installation is asked for
(`install <path>`, ADR 0173); the compositor starts `poly-installer --window
--uninstall <id>`. The line carries an app ID of the catalog and nothing
else: a client that writes it gets a window that asks, never a removal. The
window works out for itself, from the disk, what the app is and whether it
may go (`poly_pkg::remove::plan`); it does not trust the menu that asked.

### 2. What may be uninstalled

| The app is | Uninstall |
|---|---|
| one of Poly's own (`poly:Files`, Settings, ...) | never |
| an app of a package of the **base root** (the packages the disk is built from) | refused, with the reason |
| a desktop file nobody installed (Poly's own Chromium entry, test apps) | refused |
| an app of a package installed after the disk was made (by the installer) | yes |
| an AppImage the installer put in the user's folder | yes |

The build writes the names of the packages it unpacked to
`/usr/share/poly/base-packages` (`poly_pkg::remove::BASE_PACKAGES`); dpkg's own
file lists (`/var/lib/dpkg/info/<package>.list`) say which package owns a
desktop file. An app is removable when a package that is not in the base list
owns its desktop file. The desktop and its browser are of the base root, so
they stay. A disk made before this change has no base list and nothing of
dpkg's is removable on it (the installer already asks for a fresh disk, ADR
0173).

`autoremove` can only remove what `apt` itself marked as installed to satisfy a
dependency: the base root was unpacked by the build, not asked for by `apt`,
so none of it is ever among them. In the boot test the library `apt` fetched
for galculator (`libquadmath0`) is gone with it, and Chromium is untouched.
The configuration files of a removed package stay (`remove`, not `purge`),
as on Debian; the user's own files and the app's data in `~/.config` stay.

### 3. AppImage: where the files go

Type 2 only (an ELF runtime with a squashfs image appended; type 1 is
refused with a sentence). The installer reads the image **without running
it**: `poly_pkg::appimage` finds the squashfs where the ELF file ends (the end
of its section header table, from the ELF header, as the AppImage
specification describes), mounts it with `poly-squashfs` (the reader the
kernel mounts squashfs with) over reads of the file, and takes out the
top-level `.desktop` file and the icon (`.DirIcon`, else `<Icon>.png`, else
the biggest PNG of the icon theme in the image). The window shows its name,
summary, version (`X-AppImage-Version`) and size. A real CPU-X image reads
this way (gzip squashfs; an ignored host test reads any image named by
`POLY_TEST_APPIMAGE`).

Install puts these in the **desktop user's data directory** (`$XDG_DATA_HOME`,
root's today, as the desktop's user installs, ADR 0173 decision 6):

```
$XDG_DATA_HOME/poly/appimages/<slug>/<slug>.AppImage   the image, mode 755
$XDG_DATA_HOME/poly/appimages/<slug>/icon.png          its icon, if it had one
$XDG_DATA_HOME/applications/poly-appimage-<slug>.desktop
```

`<slug>` is the app's name in lower case letters, digits and `-`. The desktop
file is the image's own entry reduced to what the launcher reads (`Name`,
`Comment`, `Categories`, `Keywords`, `Terminal`), started through the copy
(`Exec`), with the extracted icon as an absolute path and
`X-Poly-AppImage=<image path>`, which is what tells an installed AppImage from
any other desktop file. The copy is written under a `.part` name and renamed,
so a cut-off copy is never the app. Installing it again replaces it.
Uninstall removes the directory and the desktop file, and nothing else (the
download stays where it was).

The launcher and the Applications folder now also read
`$XDG_DATA_HOME/applications` (`poly_shell::apps::installed_for_user`: the
user's file wins for one ID), which ADR 0175 had left out. Files opens a
downloaded AppImage with the installer (`application/vnd.appimage` joins the
package types of its desktop file and of `/etc/xdg/mimeapps.list`), also when it
has the execute bit; only the installer's own copy runs when double-clicked.

### 4. Running an AppImage on Poly: which path works

Tried in the boot test, on CPU-X 5.4.1:

- **FUSE mount (the runtime's normal way): the mount happens, the app does
  not start.** The type 2 runtime mounted its image under
  `/tmp/.mount_*` (squashfuse in the runtime, on Poly's FUSE), then failed:
  `Failed to execute "/tmp/.mount_…/AppRun": I/O error (os error 5)` and
  `fuse: failed to unmount …: Invalid argument`. Executing a file that a FUSE
  file system serves fails with `EIO` on Poly today. That is a kernel gap in
  how an `execve` (or the mappings of the new program) reads from FUSE; it was
  found here and **not fixed here**.
- **Extract and run (the AppImage project's `APPIMAGE_EXTRACT_AND_RUN=1`,
  documented for exactly the systems that cannot mount): works.** The runtime
  unpacks the image to a temporary directory, runs `AppRun` from it, and
  removes it when the app ends. CPU-X's window appears (screenshot in the
  hand-over).

So Poly starts AppImages by extracting: the installed desktop file's `Exec` is
`env APPIMAGE_EXTRACT_AND_RUN=1 <image>`, and the installer's **Run** sets the
variable. `poly_pkg::appimage::RUNS_BY_EXTRACTING` is the one switch: when
`execve` from FUSE works, set it to `false` and the desktop files are written
without the variable (installed ones keep it, harmlessly). The roadmap's
"AppImages install and run, without FUSE (Poly mounts their squashfs)" stays
unticked: Poly *reads* the squashfs (to install) but does not mount it for the
app. A start costs the unpacking each time (seconds for a 27 MB image).

## Not in this ADR

- **Flatpak and RPM:** still recognised and refused (ADR 0173).
- **Updates** of installed `.deb` and AppImage apps (AppImage's `zsync` /
  `AppImageUpdate`, `apt upgrade`), and any version check.
- **A preview of what `autoremove` takes** (`apt-get -s`): the window says "what
  only it needed" without naming it.
- **Uninstalling the base root's apps**, and apps of kept disks without a base
  list; **purge** (configuration files) and removing the app's data.
- **AppImage integration beyond the launcher entry:** file types
  (`MimeType`), desktop actions, the AppImage's own `--appimage-signature`
  check, type 1 images, 32-bit images.
- **The copy blocks the window** for as long as it takes (about a second for
  27 MB); a progress bar for it needs a chunked copy in the window's tick.
- **Removing an app while it runs:** the files go; the running process
  continues. The boot test stops it first.
- **A user other than root:** as in ADR 0173, the desktop's user installs and
  removes; with real accounts the package tools need a privileged helper
  behind the same window.

## Tests

Host: `poly-pkg` (`appimage`: the squashfs offset from ELF headers of both
classes, a squashfs image written by the test (uncompressed, one metadata
block each) with desktop file, icon through `.DirIcon`, by name and from the
icon theme, links followed and looped, refusals, slugs and paths, the desktop
file; `remove`: base list, ownership, plans for a package, an AppImage, Poly's
own and refused apps; `installer`: the removal's steps, messages, failures,
the AppImage window's buttons and steps; `apt`: the commands), `poly-shell`
(the folder's `Uninstall` action and request, the user's applications
directory, the uninstaller's entry), `poly-wayland` (the `uninstall` line),
`poly-files` (an AppImage goes to the installer unless it is the installed
copy), `poly-files-app` (the menu row and the Delete key, Poly's apps without
them, nothing asked without a desktop), `poly-installer-app` (the window for
removing a package, an AppImage, a refusal; both themes; the danger button;
Run and Install for an AppImage), `xtask` (the boot test's helpers).

Boot group **`linux-installer-2`** (shard `userland`; skipped where the
packages are not fetched, so it does not run in CI, as `linux-installer`
does not): Chromium's disk with galculator's `.deb`. It installs it as
Chromium's "open" does, then uninstalls it from Files' Applications through
the right-click menu and checks that the window only asked, that `apt` removed
it and its library, that the launcher entry, the binary and the Files entry
are gone, that dpkg's state is clean and Chromium untouched; that asking to
uninstall Chromium is refused. Then it installs `curl` from Debian's archive,
**downloads CPU-X's AppImage in the guest** from its GitHub release page
(`v5.4.1`, pinned by URL and SHA-256, never committed), double-clicks it in
Files, installs it, checks the files, starts it (its window appears) and
uninstalls it from Applications. About 160 seconds on KVM.
