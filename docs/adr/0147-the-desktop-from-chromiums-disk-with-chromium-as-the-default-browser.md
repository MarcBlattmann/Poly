# 0147: The desktop from Chromium's disk, with Chromium the default browser

**Status:** Accepted. Roadmap Phase 6, after ADR 0130 (Chromium's disk, built
from pinned Debian packages) and ADR 0133 (the dev menu). The owner decided
(2026-11-06) that Chromium is the desktop's normal browser: no special
command, no flag to remember.

## Context

There were two separate boots. `cargo xtask desktop` (also the menu's first
entry) boots a small initramfs with Poly's own apps (Terminal, Files, ...,
ADR 0134) and has no browser. `cargo xtask chromium run` boots the Debian
root disk of ADR 0130 (228 MB of downloaded packages, a 1.3 GB image) with
Chromium in the launcher, but without Poly's apps: launching Terminal there
logged `/bin/poly-terminal: not found`, and the disk's settings, wallpapers
and shortcuts were not the desktop's. Chromium also was not the default for
anything: nothing opened a web page with it.

## Decision

**One desktop, with a switch.** `cargo xtask desktop` gets `--chromium`, and
the dev menu's Desktop entry gets an option **With Chromium**, on by default
(saved in `target/poly-start.json` like the others; a file without the key
has it on). On: the desktop boots from Chromium's disk with everything the
plain desktop has. Off: the desktop boots as before, from the initramfs. The
menu's other options (release, test apps, fresh settings, memory, CPUs,
scale, size, kernel arguments) apply to both; **Fresh disk** is Chromium's
disk (`--fresh-disk`, accepted with `--chromium` only), so it also drops the
browser's profile, bookmarks and logins. The menu calls the subcommand's own
functions (`desktop::prepare`, which downloads and builds what Chromium
needs, shown as steps of the build table, and `desktop::boot`). The first
entry of `chromium run` in the menu is gone; the subcommand stays, as does
"Build Chromium's disk" for fetching ahead of time.

The option's line says what the first start costs: **the download** (the
packages of the lock not yet in `target/chromium/debs`, in MB) and the
image, then that the disk is kept. It is **greyed with the reason where the
host cannot run it**: guest memory of Chromium's size (4G, more than the plain
desktop's 512M) is more than the host can give, or `curl` is missing while
packages are still to download. The setting stays on in the file; the run is
the plain desktop, and the printed command line shows it.

**Poly's files follow the checkout, the disk is kept.** Chromium's disk
holds the browser's profile, so it is made once and kept (`--fresh-disk`
starts again). The compositor, Files, Terminal, `notify-send`, the test apps,
the desktop and Chromium's launch scripts, the desktop files and the default
browser's lists change with every checkout, so they do not live in the
image: the **initramfs carries them in `/poly-overlay`**, and
`/etc/init.d/overlay`, run by the switch-root script (ADR 0130's amendment)
with the disk mounted and before it becomes the root, copies them onto it
(`cp -a`; modes and links kept) and removes what an earlier boot's test apps
left (`desktop::TEST_ONLY_PATHS`). The same files are also written into a
new disk, so it boots by itself, and a disk kept from before this ADR needs
no new image. Programs are copied to `/usr/bin` (the disk's `/bin` is a
link to it), so every `/bin/<name>` the launcher and Files start is there.
Fonts and wallpapers stay in the image (`chromium::collect`), as before.

**The settings.** The settings disk of the plain desktop
(`target/poly-desktop-state.img`) is attached as the second virtio-blk disk,
mounted by the kernel on the initramfs's `/settings` (`poly.mount=virtio1:/settings`)
and **bound into the root disk** by the overlay script (`mount -o bind`);
the compositor reads `/settings/poly/shell.conf`. So the same keyboard
shortcuts, wallpaper and scale apply with and without Chromium, and Fresh
settings means the same. The compositor's arguments (`--settings`,
`--scale`), which a first program of its own would get on the kernel's
command line, go through `/etc/poly-desktop.args`, which `poly-desktop` (the
disk's start script) reads.

**Default browser, by the freedesktop.org specifications.** Poly's Chromium
entry (`poly-chromium.desktop`, unchanged apart from what follows: Debian's
own entry stays out, it runs Chromium as root) now says `MimeType=text/html;
application/xhtml+xml;x-scheme-handler/http;x-scheme-handler/https;` and
takes `%U`, so it opens what it is given and the new tab page without
arguments. `/etc/xdg/mimeapps.list` (MIME Applications Associations
specification, `$XDG_CONFIG_DIRS`: the system's choices, below the user's own
in `$XDG_CONFIG_HOME`) names it the `[Default Applications]` for those four
types. Two readers use it:

- **Poly's Files** read only the user's list; they now read every
  `mimeapps.list` of the specification's locations, most important first
  (`poly_files_app::mimeapps_paths`). Opening a `.html` file starts Chromium.
- **`xdg-open`** (Debian's `xdg-utils`, on the disk already because Chromium
  depends on it) reads the same lists through `xdg-mime query default` and runs
  the entry's `Exec`. It needs `awk`, which Debian's `gawk` provides through
  an alternative that a maintainer script sets up; the disk gets the link
  (`/usr/bin/awk`) from the overlay. Where no list names a browser it falls
  back to `$BROWSER`, which the desktop's environment sets to the same
  command (otherwise it would try Debian's `chromium`, as root, and fail).

**While the window opens.** Chromium's window takes about 60 to 95 s under
WHPX (more under the emulator). Nothing in the desktop times out meanwhile:
the compositor has no ping or unresponsive-client kill, it waits for the
client's first buffer, and the host side (`desktop::boot`, the menu) waits
for QEMU without a limit. To tell the person what is going on, Poly's
launch script sends a notification ("Chromium is starting") when no Chromium
runs yet; a second launch while it runs opens a window there.

**No X display on this disk.** The compositor of this disk is started
without `--xwayland-lazy`, as the plain desktop's is: the disk has no
Xwayland, and a display offered with no server behind it made `xdg-open`
(which asks `xprop` about the desktop whenever `DISPLAY` is set, as it is in
Terminal) spin, connecting to it thousands of times a second, so a link opened
from Terminal never reached Chromium. Chromium and Poly's apps speak Wayland.
(The same loop on a real X server that closes connections at once is not
investigated here.)

## Not in this ADR: choosing a browser at install

Chromium is preinstalled on the development disk (ADR 0130); there is no
Software or store app (owner's decision). The **release installer** will later
ask which browser to install and download it, then write the matching
`poly-<browser>.desktop` and `/etc/xdg/mimeapps.list` the way this ADR does for
Chromium; nothing here depends on Chromium except those two files and the
launch script, which is why the default is data (the list), not code.

## Consequences

- A fresh checkout runs `cargo xtask start` (Enter) or `cargo xtask desktop
  --chromium` and gets the desktop with Chromium: about 230 MB downloaded and
  a 1.3 GB image made the first time, then kept. `cargo xtask desktop
  --chromium` is the same command the menu runs.
- CI is unchanged: the plain desktop's boot tests use the initramfs as
  before, and the new boot group `chromium-desktop` skips itself, like
  `chromium`, until the packages are downloaded.
- Boot group `chromium-desktop` boots `desktop --chromium`'s configuration
  (test apps, the settings disk) and checks that every app of the plain
  desktop is on the disk and every launcher entry's program is there, that `xdg-mime` names Chromium for pages and
  links, that the settings disk is bound, that Terminal opens from the
  launcher, and that `xdg-open` of a page starts Chromium (its window is shown).
- The disk is bigger than the initramfs and slower to start (the overlay's
  copy is a few megabytes; Chromium's own start is the minute); the plain
  desktop remains one click away for quick edits to the compositor.
- A person's own `mimeapps.list` (`~/.config/mimeapps.list`) changes the
  default browser without a rebuild.
- `xtask` gains `poly-files` as a dev-dependency, for the unit test that
  Files' choice for the lists the tool writes is Chromium.
