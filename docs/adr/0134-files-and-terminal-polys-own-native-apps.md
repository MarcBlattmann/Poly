# 0134: Files and Terminal, Poly's own native apps in the Alpine look; the browsing core that becomes the one file picker

**Status:** Accepted. Roadmap Phase 6, the owner's request of 2026-11-02:
the desktop had no file explorer and no terminal emulator (`poly-shell`'s
`apps.rs` skipped `Terminal=true` entries because "there is no terminal
emulator yet"). The owner chose **Poly's own native apps, in the Alpine
Shell look**, over porting GNOME's or KDE's. It builds on the native
runtime and desktop of [ADR 0125](0125-native-system-call-interface-and-poly-rt.md)
and [ADR 0126](0126-the-desktop-on-the-native-runtime.md), the look of
[ADR 0127](0127-the-alpine-shell-look.md) and the keymap of
[ADR 0129](0129-editable-keyboard-shortcuts.md). Files' browsing core is
the start of Phase 8's "One file picker for every app world".

These are Poly's own apps, not a reimplementation of an operating system's
interface, so the clean-room split of spec and implementer does not apply;
no code of other terminal emulators or file managers was read or copied
(the VT emulation follows ECMA-48, the DEC manuals and xterm's published
control sequences; the trash follows the freedesktop.org Trash
specification 1.0; desktop entries the Desktop Entry Specification 1.5).

## Decisions

### 1. Separate native processes, Wayland clients of the native compositor

Files (`/bin/poly-files`) and Terminal (`/bin/poly-terminal`) are native
Poly programs (`userland/native/files`, `userland/native/terminal`, three
lines each over the host-tested crates `userland/poly-files-app` and
`userland/poly-terminal`), started by the compositor like any app. Not
inside the compositor, unlike the settings window:

- a crash or a hang (a terminal reading a runaway program's output, a file
  operation on a slow disk) takes only the app down, never the desktop;
- they talk to the compositor exactly as Linux apps do (Wayland on
  `$XDG_RUNTIME_DIR/wayland-0`, server-side frames of ADR 0106), so window
  management, the switcher, snapping and the launcher's window list treat
  them like every other window;
- several windows are several processes (Ctrl+Shift+N, Super+E again).

Nothing in the architecture forced otherwise: the native ABI version 2
already has the sockets with handle passing, memory objects and `poll` a
Wayland client needs.

### 2. `poly-app`: the shared native app layer

`userland/poly-app` is what native apps share, `no_std` + `alloc` on
`poly-rt`:

- **`proto`**, a Wayland client without I/O: one `xdg_toplevel` in one
  `wl_shm` buffer at the scale `wl_surface.preferred_buffer_scale` (or the
  output's) asks for, pointer, keyboard, `wp_cursor_shape_v1`, the
  clipboard (`wl_data_device`: our selection as a source offering text
  types; a paste reads the offer through a pipe). Messages are encoded and
  decoded with `poly-wayland`'s wire format and generated tables, so the
  client speaks exactly the protocol the compositor implements. Its tests
  run it against `poly-wayland`'s own server in the same process
  (configure, HiDPI buffers, input, the clipboard handed to another
  client, close and titles).
- **`input`**: evdev codes with the seat's XKB bits to Poly's `Key` and
  the character on the desktop's layout (`poly-input`'s tables, as the
  compositor's launcher does; the layout is the settings' `keyboard`,
  which is also the layout of the keymap the compositor hands out); key
  repeat at `wl_keyboard.repeat_info`'s rate.
- **`ui`**: the Alpine look for app windows in the settings window's layout
  language (the ground and its 40-pixel header, the 224-pixel sidebar of
  36-pixel rows, the page on surface 1 with its rounded corner and
  hairline, fields of radius 8, ghost hovers, outline segmented controls,
  popovers with a shadow, menus), a one-line editor, context menus, and
  `Look`: the theme and layout from the shell's settings file
  (`$POLY_SHELL_SETTINGS`, which the compositor now gives the apps it
  starts), looked at again every 2 seconds so a theme change in Settings
  reaches open apps.
- **`run`**: the event loop: `poll` over the socket, the app's own handles
  (the terminal's pty), a paste's pipe; drawing when the compositor's frame
  callback says so. The compositor copies a buffer at commit and releases
  it at once, so one buffer is enough and keeps what was drawn: apps redraw
  what changed and damage only that.

### 3. Terminal

- **The pseudo-terminal** comes from the Linux personality's devices
  (`/dev/ptmx`, `TIOCSPTLCK`, `TIOCGPTN`, `/dev/pts/<n>`, `TIOCSWINSZ` on
  every resize, which sends `SIGWINCH`), through native `control`. The
  shell is `$SHELL`, else bash, else `/bin/sh`; `-e PROGRAM` runs a program
  instead (xterm's option), which is what the launcher now does for
  desktop entries with `Terminal=true` (`apps::parse_desktop_entry`
  prefixes `/bin/poly-terminal -e`); `--dir` starts in a folder (Files'
  "Open in Terminal"). `TERM=xterm-256color`, `COLORTERM=truecolor`.
- **The shell starts at the window's size.** The window's size and the
  mono face's cell size are known only at the first layout, so the shell is
  started then, on a pseudo-terminal that already has the grid's columns,
  rows and pixels (`pty::Launch::start`, called from `TermApp::relayout`).
  Started earlier, at a placeholder 80x24, the shell had printed its prompt
  when the first layout resized the terminal; the `SIGWINCH` made BusyBox's
  ash redraw it (carriage return, the prompt, erase to the end of the
  screen, a newline, the prompt again), so a new Terminal showed `~ #`
  twice whenever the shell was faster than the window. The `desktop-apps`
  boot test checks the shell starts at a size other than 80x24, is not
  resized (`poly-terminal-pty: resized`) while it starts, and that the
  screenshot of the page has exactly one short line (the prompt) under the
  banner.
- **The controlling terminal without a kernel change.** A Linux program a
  native process spawns is the root of its own process tree and leads its
  own session (ADR 0126); a session leader without a controlling terminal
  that opens a terminal without `O_NOCTTY` acquires it (POSIX;
  `poly-tty`). The standard handles the terminal passes were opened by the
  terminal, so the shell is started as
  `sh -c 'cd; exec "$@" <>/dev/pts/N >&0 2>&0' sh <shell> -i`: the shell
  opens its terminal itself, and `exec` keeps the session leader. Job
  control, `^C` and `^Z` work. A spawn flag for "new session with this
  controlling terminal" was the alternative; it would have meant a new ABI
  bit and kernel code for something a shell does in one line.
- **The emulation** is a new crate, `crates/poly-vt` (host-tested, no
  dependencies): a VT state machine (CSI, OSC titles, DCS/APC skipped,
  UTF-8 across reads), a resizable screen model with scroll regions, the
  alternate screen (`?47`, `?1047`, `?1049`), scrollback, autowrap with the
  pending-wrap flag, wide characters, DEC line drawing, insert mode,
  origin mode, tab stops, the replies (DSR, DA1, DA2), the modes a program
  sets (application cursor and keypad, bracketed paste, mouse 1000/1002/
  1003 with SGR 1006, focus events, reverse video), damage per row, and the
  encoders for keys, pastes and mouse reports. The kernel's own terminal
  (`poly-gfx`'s `TextGrid`) was not reused: it lives in caller-provided
  fixed storage for the kernel and has no scroll regions, alternate
  screen, resize or replies, which `less`, `vim` and `top` need.
- **The look**: a 40-pixel header (the terminal's icon, the OSC title,
  Copy, Paste and New window), the screen on the page with 12 pixels
  around it, Geist Mono 13 px in cells as wide as its advance and 1.35 em
  high, the ink and page as default colours, Tailwind's 400 tones as the
  ANSI colours on the dark theme and its 600 tones on the light (Alpine's
  palette is Tailwind's), bold as bright. Selection by drag, double click
  (word) and triple click (line); Ctrl+Shift+C/V, Shift+Insert and the
  middle button; the history with the wheel and Shift+Page Up/Down; on the
  alternate screen the wheel sends arrow keys.

### 4. Files, and a browsing core that becomes the file picker

The window (`userland/poly-files-app`) only lays out, draws and maps
input; everything it decides is in `crates/poly-files`, `no_std` +
`alloc` over a file system trait (`Fs`), no UI and no system calls, so the
Phase 8 picker can reuse it from any world:

- `model::Browser`: the listing (folders first, natural, case-insensitive
  names), sorting by name, size or kind, the search filter, hidden files,
  selection by click, Ctrl/Shift and keys (list and grid), type-ahead,
  history, and the picker's `PickFilter` (MIME patterns, folders only);
- `places`: Home and the XDG user folders (`user-dirs.dirs`), the root,
  the trash, mounted disks from `/proc/mounts`;
- `mime`: types by name and by content (magic numbers), descriptions,
  icons;
- `apps`: which app opens a file (`mimeapps.list` defaults, then
  `MimeType=` exact, then `type/*`), `Exec` field codes, and what a double
  click does (browse, launch a program or a `.desktop` file, open with an
  app; a `.desktop` file or a script launches only with its execute bit,
  so a downloaded `photo.jpg.desktop` does not run on a double click);
- `ops` (new folder, rename, copy/cut and paste with "name (copy)", moves
  across disks as copy and delete, refusing a folder into itself, deleting)
  and `trash` (the freedesktop.org home trash: the info file created
  exclusively first, `Path=` percent-encoded, `DeletionDate` in UTC because
  Poly has no local time zone in its user space yet; restore and empty).

The window, in the settings window's language: back, forward and up, the
path bar as breadcrumbs (Ctrl+L or `/` edits it as text), search, list or
grid, the hidden-files switch; the sidebar of places and disks; the page
with the folder's title, New folder (Empty Trash in the trash), column
headers that sort, and a status line; context menus, inline rename, a
confirmation before deleting for ever. Text files without an app open in
Terminal's `vi`. Native `stat` has no modification time yet, so there is
no date column; the native `Fs` finds links through the parent's listing
(there is no `lstat`), and reads a `rename` that fails with `Io` as a move
across disks.

### 5. In the desktop

- Both are built-ins of the launcher (`apps::builtins`: Files, Terminal,
  with keywords such as explorer, folder, downloads, shell, console, bash),
  with Lucide's `folder` and `square-terminal` (29 more Lucide icons in
  `poly-shell`, from lucide-react 0.575.0 as before).
- Two new actions in the keymap of ADR 0129: **Files, Win+E** (Windows'
  Explorer key) and **Terminal, Ctrl+Alt+T** (GNOME's and Ubuntu's),
  rebindable on the Keyboard shortcuts page. The page has two more rows,
  so the settings window grows from 900 × 620 to 900 × 720, as ADR 0127
  grew it for the longer pages.
- The desktop's image carries BusyBox when one is available (the host's
  static one, `POLY_BUSYBOX`, or a build), so Terminal has a shell; without
  it, Terminal says so in its window.

## Proof

| Check | Result |
|---|---|
| `poly-vt`, `poly-files`, `poly-app`, `poly-terminal`, `poly-files-app` host tests (VT parsing and the screen, the file model, operations on a temp folder, the trash, MIME choice, the client against `poly-wayland`'s server, the windows drawn) | pass |
| Boot group `desktop-apps`: Terminal from the launcher runs BusyBox's shell on a pty, `echo hi > /tmp/x` is typed, the file's contents come back through the terminal (an OSC title it reports); Files from the launcher lists `/tmp/x`; Ctrl+Alt+T and Win+E open them again; screenshots | see the hand-off |
| `cargo xtask desktop --release` on Windows, both apps open | see the hand-off |

## Consequences

- Native apps have a shared layer (`poly-app`); more of Poly's own apps can
  follow it.
- `Terminal=true` desktop entries are offered and run in Terminal.
- The file picker of Phase 8 starts from `poly-files`: a picker window
  (native) over `Browser` with a `PickFilter`, bridged to GTK's portal,
  Windows' common dialogs and macOS's open panel.
- Still to come: tabs in Terminal, a native `lstat` and file times (a date
  column), drag and drop between Files and other apps (`text/uri-list`),
  thumbnails, and copy progress for big trees (operations run in the
  window's loop today).
