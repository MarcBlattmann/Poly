# 0218: Google Chrome for Linux from its `.deb`: the desktop's apps run as its user

**Status:** Accepted. It continues
[ADR 0173](0173-installing-a-deb-polys-installer-over-debians-own-apt-and-dpkg.md)
(the installer over Debian's own `apt` and `dpkg`, whose decision 6 left the
desktop's apps running as root) and
[ADR 0184](0184-x11-apps-a-session-bus-and-room-for-big-apps-on-the-desktop-disk.md)
(user `poly`'s session bus and X access, which listed "applications installed
from the launcher run as root" as not decided).

## What works

On the desktop with Chromium: download Google Chrome for Linux from
google.com (or `dl.google.com`'s direct link), click the download, **Install**
in Poly's installer, then **Google Chrome** in the launcher. Chrome runs with
its sandbox, shows its first-run dialogs and its pages; typing, scrolling,
maximizing and downloads work, and a downloaded file is one click away in
Files (**Downloads**).

## What was missing (found by walking that path)

| Stop | Cause | Fixed in |
|---|---|---|
| Chrome from the launcher exits at once | The desktop started every installed app as root; Chrome (like every Chromium or Electron app) refuses to run its sandbox as root | decision 1 |
| `google-chrome-stable: /dev/fd/63: Read-only file system` | Chrome's launch script redirects with bash's `exec > >(exec cat)`; a pipe's `/proc/<pid>/fd` link was not followed, so bash's `O_CREAT` open hit procfs | `Vfs::pipe_node`: a pipe's inode opens as that pipe (fifo(7) rules), as on Linux |
| Chrome dies with SIGTRAP after its sign-in page | The compositor sent `wl_pointer.leave` naming a surface the client had destroyed and whose ID it had reused for a `wl_buffer` | a destroyed surface drops the pointer and keyboard focus; a focus counts only while its ID is a `wl_surface` |
| Installing took 13 minutes, dpkg 514 s in the kernel | Once the page cache was at its limit, every page added made eviction walk the whole LRU past the same dirty and mapped pages, under a spinlock | second-chance eviction; an insert looks at 64 pages at most |
| `dpkg --configure -a` 2 min 45 s | `fsync` of a file synced the whole file system: every dirty page of every file, a journal commit, a disk flush; dpkg syncs hundreds of times | `FileSystem::sync_inode`: fsync writes back that file only |
| Writing 1 GiB panics the kernel | Writeback (and discarding a file) collected every dirty page's key into one 6 MiB list; the kernel heap's largest block is 4 MiB | writeback and every page-dropping loop work in batches |
| A download cannot be found in Files | Files runs as the desktop (root, Home `/root`); the browsers save into `/home/poly/Downloads` | decision 2 |
| The launcher comes up over each of Chrome's first-run dialogs | It reopened whenever the last window went, while Chrome was opening its next one | decision 3 |

## Decisions

### 1. The system's apps start as the desktop's user

The compositor takes `--app-user PROGRAM`; the desktop script passes
`/usr/local/bin/poly-as-user` where the disk has it (Chromium's disk). An app
whose desktop file is in `/usr/share/applications` (Debian's packages, Poly's
Chromium entry) starts through it, as user `poly` with that user's runtime
directory, session bus and X access (ADR 0184): `poly_shell::apps::runs_as_user`.
Poly's own programs (Files, Terminal, Settings, the installer, which runs
`dpkg`) keep running as the desktop, and so does an app of the user's own
applications folder (an AppImage the installer put in root's home, ADR 0180),
until the desktop itself runs as an ordinary user. `poly-as-user` does
nothing more for a program already running as `poly` (Chromium's launch
script calls it again).

### 2. Files shows the apps' user's Downloads

The desktop's environment names the home of the user apps run as
(`POLY_APPS_USER_HOME=/home/poly`); Files lists its **Downloads** after Home
when Home has no Downloads of its own. Files still runs as the desktop; a
Files of the desktop's user comes with that user.

### 3. The launcher waits for a running app's next window

When the last window goes while an app the desktop started still runs, the
desktop waits up to 10 s for that app's next window before it shows the
launcher; an app that exits brings it back at once.

## Not in this ADR

- **Chrome's GPU process.** Chrome starts its GPU process with its default
  flags; with no `libEGL.so.1` on the disk it exits and Chrome draws in
  software (Chromium on Poly runs `--disable-gpu`). Scrolling shows
  unpainted tiles for a moment.
- **Setting Chrome as the default browser** fails (`xdg-settings: $BROWSER is
  set`): `/etc/poly-desktop.env` names Chromium for `xdg-open`.
- **Google's apt repository.** Chrome's package adds its cron job
  (`/etc/cron.daily/google-chrome`); nothing runs cron on Poly's disk, so
  updates come from downloading a newer `.deb`.
- **Opening a file with an app from Files** spawns the app from Files, as
  the desktop, not through `--app-user`.

## Tests

Host: `poly-wayland` (no `leave` names a destroyed surface whose ID was
reused), `poly-procfs` (a pipe's descriptor link opens the same pipe),
`poly-pagecache` (batches, second chance, the insert's scan budget, fsync of
one file), `poly-shell` (which apps run as the user), `poly-files` (the apps'
user's Downloads), `poly-compositor` (the launcher's wait), `xtask` (the
scripts).

Boot group **`linux-chrome`** (shard `userland`; skipped where the Debian
packages or Chrome's `.deb` are not fetched: `cargo xtask chromium fetch`,
`cargo xtask chromium fetch-chrome`): Chrome's `.deb` in Downloads, opened
with `xdg-open` as user `poly`, installed by the installer over the guest's
network, `dpkg`'s state and the set-user-ID `chrome-sandbox` checked, **Open**
starts Chrome as uid 1000, and a test page's four colours fill its window;
bash's `exec > >(cat)` works.

## Addendum (2026-11-19, linux-chrome-polish): the owner's rough edges

- **The installer's window comes to the front.** A client's xdg-activation token (xdg-activation-v1
  leaves honouring a request to the compositor) is good only if the client had the focus when it
  made it (or nobody had), and only while the focus has not gone to another client or the shell
  since. A browser can no longer pull its window back over the installer it just started with a
  token from before. The desktop's own tokens (launches) are good until used.
- **Opening a download is fast.** The session sets `XDG_CURRENT_DESKTOP=X-Generic`: xdg-utils
  then asks no X client (`xprop`) about the desktop. That question made the compositor start
  Xwayland for the first download a browser opened (about 110 s). This was observed on xdg-utils
  as a black box; other values (`Poly`, `Poly:X-Generic`) still ask.
- **The default browser can be set.** `/etc/poly-desktop.env` no longer exports `$BROWSER`, with
  which `xdg-settings` refuses to change the default. `mimeapps.list` alone names the default
  browser, and "Make default browser" writes the user's own `~/.config/mimeapps.list`.
- **Chrome's GPU process starts.** `libegl1`, `libegl-mesa0` and `libgles2` are on the disk. Without
  `libEGL.so.1` the GPU process ended at start-up and Chrome fell back to software.
- **Files opens files with the system's apps as the desktop's user.** The desktop script exports
  `POLY_APP_USER` (the compositor's `--app-user`), and Files starts an app that opens a file through
  it. Poly's own programs (the installer, Terminal, Files) stay the desktop's.
- The boot group `linux-chrome` checks the first four.
