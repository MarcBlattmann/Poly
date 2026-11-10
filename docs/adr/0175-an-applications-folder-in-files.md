# 0175: An Applications folder in Files: the launcher's catalog as a virtual folder, opened by the desktop

**Status:** Accepted (owner request, 2026-11-10: "one folder in Files that
shows all apps installed, like Apple does it"). Builds on
[ADR 0134](0134-files-and-terminal-polys-own-native-apps.md) (Files),
[ADR 0132](0132-installing-linux-apps-of-every-package-format.md) (installed
apps appear in the launcher; the installer is a later track), [ADR 0146](0146-app-icons-from-the-icon-theme-and-a-png-decoder.md)
(app icons), [ADR 0167](0167-busy-pointer-while-an-app-starts.md) (the busy
pointer and the control pipe) and the look of
[ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
and [ADR 0127](0127-the-alpine-shell-look.md).

## Context

The only list of installed apps was the launcher's, and it is a search
field: it shows what matches, not everything there is. The owner wants a
place to see all apps at once, as a folder in Files.

## Decision

### A virtual folder, not a directory of bundles

macOS can have a real `/Applications` because a Mac app *is* a directory (a
bundle). Poly's apps are not: a Linux app is a desktop file plus files all
over `/usr`, Poly's own Settings and Notifications are parts of the shell
with no file at all, and a Windows app is an `.exe` somewhere with a desktop
file that names it. A real directory would have to be filled with links or
copies that go stale, would show nothing for the shell's own apps, and
would invite dragging an "app" out of it, which moves a link and breaks
nothing or everything.

So Applications is a list Files makes up, at the location
`applications:///` (the path `applications:` inside Files: not absolute, so
it has no parent, cannot collide with a folder on any disk, the macOS
personality's real `/Applications` included, and no file operation can
reach a disk through it). It is a place in the sidebar (before Computer),
and its name or URI typed into the path bar goes there.

### One source, shared with the launcher

`poly_shell::apps::installed` is the one reader: Poly's built-ins plus the
desktop files of `/usr/share/applications`, in file-name order, each name
once. The compositor calls it for the launcher and Files calls it for the
folder (the directory-reading loop moved out of the compositor into
`poly-shell`, behind a two-method `AppFiles` trait, so it is host-tested
and there is one copy). Hidden entries (`NoDisplay`, `Hidden`), entries
that are not applications and the test apps (present only as desktop files
on a `--test-apps` image) are in or out of both by the same code.

`poly_shell::applications` is the folder's model: the items sorted by name,
what kind of app each is (`AppKind`: Poly, Linux, Windows), where it comes
from, the search rule, and what can be done with an item. Files shows it
through its ordinary browser: `poly-files-app`'s `applications::overlay`
puts the list over the file system as a folder of labels (`Fs::is_virtual`:
no name is hidden for a leading dot or refused for a `/`), so list and grid
views, sorting, search, type-ahead, selection and back/forward are the code
files already use.

**When the list updates.** Files reads the catalog each time the folder is
listed (opening it, F5, back and forward). The launcher still reads at the
desktop's start. To keep them one list, an `open` for an ID the shell does
not know makes the compositor read the catalog again
(`Shell::set_catalog`), so an app installed since then that was opened from
Files is in the launcher from then on. Neither watches the directory; the
installer track will tell both.

### Opening is the desktop's job

Opening an item writes `open <id>` to the compositor's control pipe (a new
line of `poly_wayland::launch::Control`, next to ADR 0167's `launch`, `pid`
and `cancel`). The compositor looks the ID up in the shell's catalog and
calls the same `start_app` the launcher's Enter does: the shell's own
windows open in the shell (Settings from Files was impossible otherwise),
programs are spawned with their activation token, and the pointer is busy
until the window is there. The line carries only an ID of the catalog, so
it lets a client start nothing the launcher could not.

A held Enter opens once: Files now ignores key repeat for "open" (the boot
test caught a repeat asking for Settings four times).

### What an item shows

The app's own icon (`poly_shell::appicon`, the launcher's cache and
lookup; `App::glyph` where there is none), its name, and in the list a Kind
column: "Poly app", "Linux app" or "Windows app". The status line gives the
selected app's details: kind, source (built into Poly, or its desktop
file's path) and its description. A desktop file does not say which world
its program is from, so the kind is read from the program's name: a
built-in or `poly-*` is Poly's, an `.exe` is Windows, the rest is Linux.

There is no Size column (an app has no one size) and no New folder button,
and rename, cut, copy, paste, delete and "Open in Terminal" do nothing
there; the overlay refuses writes too, whoever asks.

### Uninstall: not yet, and where it goes

Uninstalling needs to know which files an app owns, which is the
installer's knowledge (a later track). Until then there is no control for
it, not a disabled one. The hook is `poly_shell::applications::Action`:
an item's context menu is built from `Item::actions()`, which returns only
`Open` today. The installer adds `Action::Uninstall`, returns it for apps
a package record owns, and Files gets the menu row (and Delete on a
selected app) from that.

### The look

Alpine Shell has no file manager and no app grid, so there is nothing of
its own to match 1:1: the folder is Files' existing window (ADR 0134 in the
settings window's layout language of ADR 0127) with the launcher's app
icons in place of file glyphs: 20 pixels in a 34-pixel row, 40 in a tile,
the tokens and text styles unchanged. Compared with Alpine Shell's launcher
rows and settings window for the surfaces, the row hover and selection and
the icon treatment.

## Consequences

- One reader of the installed apps; the compositor lost its own copy.
- `Fs` has one more defaulted method (`is_virtual`); the file picker of
  Phase 8, which reuses `poly-files`, does not get the folder unless it
  asks for the overlay (an app is not a file to pick).
- Sorting by Kind in Applications orders by name: the browser sorts by
  file type, which every item shares. Grouping by world can come with the
  installer's metadata.
- Desktop files in `~/.local/share/applications` are not read, as in the
  launcher; adding the directory to `installed` adds it to both.

## Tests

Host: `poly-shell` (`applications`: order, kinds, details, search, the open
request, labels; `apps`: one order whatever the directory says; the shell's
`app_by_id` and `set_catalog`), `poly-wayland` (the `open` line round
trip), `poly-files` (a made-up folder's names are labels, the place),
`poly-files-app` (the folder equals the launcher's catalog and follows the
disk, the place and the location, opening by Enter and double click, held
Enter, search, nothing can be changed, icons drawn in both views).

Boot (`desktop-apps`, where Files is driven): Applications typed into the
path bar lists Files, Settings and Terminal; screenshots of grid and list;
Terminal opened from it is launched by the compositor and starts a shell;
Settings opened from it opens the settings window.
`POLY_DESKTOP_APPS_THEME=light` runs the same steps in the light theme.
