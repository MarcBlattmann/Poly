# 0133: `cargo xtask start`, a dev menu, and native Wayland test clients

**Status:** Accepted. The owner's request: "There are test apps etc. but
they can't be found. Can we make one start command with a nice GUI in the
terminal where I can select stuff, like a dev menu? Every time I run the
command it saves my settings and then starts with these when I just press
Enter, or else I can change them. Run things in parallel where possible."

## Context

- xtask has about twenty boot commands, each with its own flags
  (`desktop --scale=N --size=WxH --fresh-state`, `chromium run`,
  `toolkits`, `run --busybox`, `run --disk-root`, `boot-tests <group>`,
  `ci <part>`) and environment variables (`POLY_QEMU_ACCEL`,
  `POLY_QEMU_DISPLAY`). The only list of them was `main.rs`'s doc comment.
- On Windows the desktop's launcher listed "Poly Demo", "Wayland Test
  Client" and "Demo Window", but their programs were missing:
  `tests/linux/wayland/wl-demo.c` and `wl-client.c` were built with
  `cc -static`, which only a Linux host with a static C library has. The
  image left them out, and the desktop boot tests skipped themselves.
- A desktop boot builds, one after the other, things that do not depend
  on each other: the native programs (`userland/native`), `ntdll.dll`, the
  Win32 DLLs, the kernel, and the image's files (fonts, wallpapers).

## Decision

### The menu: `cargo xtask start` (`tools/xtask/src/start.rs`)

- **One list** of what can be run: the desktop, the desktop with
  Chromium's disk, building that disk, the toolkits' disk, the kernel's
  shell, BusyBox, the root disk's bash, boot test groups (picked from
  `BOOT_GROUPS`), CI parts (from `CI_PARTS`). Each has a one-line
  description and only the options that apply to it: release or debug, the
  accelerator, the desktop's scale and size, `--fresh-state`,
  `--fresh-disk`, extra kernel arguments, QEMU's display.
- **This host's limits are shown, not hidden.** What cannot run here is
  greyed out with the reason ("needs a Linux host: run it in the poly-dev
  container or WSL"). What needs something made first says so, and Start
  makes it first. (ADR 0147: the Desktop's "With Chromium" switch, on by
  default, replaced the entry "Desktop with Chromium"; the desktop itself
  downloads and builds Chromium's disk, so nothing is made first for it.)
- **It remembers.** The settings are saved in `target/poly-start.json` on
  each Start (per checkout, git-ignored with `target/`). The menu opens on
  them with Start focused, so Enter repeats the last run. A file that is
  corrupt or of another `format` gives the defaults, field by field.
  `--last` runs the saved choice without the menu; `--print` prints the
  command line(s) it stands for.
- **The same code as the subcommands.** The menu builds the subcommand's
  arguments and calls the subcommand's function in this process
  (`dispatch`, the match `main` used to have). The desktop is split into
  `desktop::prepare` (parse, build) and `desktop::boot`, so the menu can
  show the builds as a table and then hand the terminal to QEMU.
- **No terminal crate.** `tools/xtask/src/term.rs` puts the terminal in
  raw mode (termios on Unix; `SetConsoleMode` with virtual terminal input
  and output on Windows), reads keys as escape sequences, and draws with
  ANSI escapes. crossterm was tried first. On the owner's Windows host
  (the `x86_64-pc-windows-gnu` toolchain without MinGW's `dlltool`) it
  does not link, because `parking_lot_core` now reaches Windows through
  `windows-link`'s `raw-dylib`. Pinning older crates in the lock would
  break again at the next `cargo update`.
- **Without a console**, as in Git Bash's mintty (pipes, not a Windows
  console) or with piped output, a numbered prompt stands in. Enter
  repeats the last run, and a number runs another entry. `winpty cargo
  xtask start` gives the full menu in mintty.

### Builds at the same time (`tools/xtask/src/progress.rs`)

- `build_kernel` builds the native programs while it builds the kernel
  (separate target directories, so the two cargo runs never wait on each
  other's lock), after `ntdll.dll` and the Win32 DLLs, which are made at
  the same time. `native::build` keeps its result for the run, and
  callers on other threads wait for that one build.
- `desktop::prepare` builds the kernel, the image and the settings disk
  at the same time. The image gathers its files and builds the Windows
  test app while the native programs build.
- Measured on the owner's Windows PC, `cargo xtask desktop --release` up
  to the boot: from cold 191 s before, 117 s after (with the Windows test
  app built too); with nothing to rebuild, 4.4 s before and 1.6 s after.
- Every build step goes through `progress::step`. Inside
  `progress::show` (the menu's builds) each step writes to a log in
  `target/poly-start/`. The terminal shows a live table: one row per step,
  with its time and newest log line. What xtask says meanwhile is kept and
  printed after the table, with what each step took. Outside the menu,
  steps print as before.

### The test clients as native programs (`userland/poly-wl-demo`)

- `wl-demo` and `wl-client` are rewritten as native Poly programs on
  `poly-rt` (ADR 0125): the same wire protocol from the protocol XML (no
  libwayland), the same requests and the same output lines. Shared memory
  is a memory object passed with the request (not a file in `/dev/shm`).
  The keymap's handle is mapped and checked. `userland/native/wl-demo`
  builds both with the other native programs on any host. The desktop
  image, the toolkits' disk and `cargo xtask run`'s `/bin` carry them, so
  the launcher's test apps exist on every host, and the desktop boot tests
  no longer need a C toolchain.
- The C sources stay. Boot group `wayland` runs the Linux `wl-client`
  where it builds (the Linux personality's `SCM_RIGHTS` and `/dev/shm`),
  then the native one on every host.
- The desktop image also carries the Windows GUI test program of
  `desktop-win32` (`win_gui_fill.exe` behind "Poly Fill", ADR 0131).
  `tools/xtask/src/winapp.rs` builds it on every host, the way
  `pipeline win build` does: a generated package on
  `tests/windows/support`, `-Zbuild-std`, `rust-lld` as `lld-link`, with the
  same flags. The pipeline tool itself needs a Unix host. The menu lists
  the image's apps.

### Resources (ADR 0141)

- The interactive entries (desktop, Chromium, toolkits, the kernel shell,
  BusyBox, root disk bash) have a "Resources" block: **Memory** (auto, 512M to
  16G, any size typed) and **CPUs** (auto, 1 to the host's cores, 16 at most).
  Choices the host cannot give are greyed with the reason; both are saved in
  `target/poly-start.json` (`memory`, `cpus`; older files give `auto`) and
  passed as `--memory=SIZE --cpus=N`, which the subcommands take too. Boot
  tests and CI keep their fixed sizes. What the kernel does with them, and
  why a bigger disk is not offered, is in
  [ADR 0141](0141-memory-and-cpus-for-interactive-boots-and-the-dev-menu.md).

## Consequences

- One command to remember. The menu is also the list of what each host
  can run, with the reason for what it cannot.
- The desktop on Windows has the same launcher apps as on Linux, and the
  desktop boot tests check the native clients.
- Logic stays host-tested: the settings, command lines, availability,
  key handling and drawing of the menu (`start.rs`, `term.rs`), and the
  clients' encoding, parsing and drawing (`poly-wl-demo`).

## Amendment (test apps are opt-in)

The owner: the desktop's test apps do not work on Windows (Wayland Test
Client, Poly Demo and others); are they still needed? They are: the
desktop boot tests drive them, and they are the quickest check that a
change did not break drawing, moving or closing windows. But they do not
belong in a normal desktop's launcher.

- **Off means absent, not hidden.** Without `--test-apps` the desktop
  image has no `wl-demo`, no `wl-client`, no `win_gui_fill.exe` and none of
  the test desktop files (Poly Demo, Wayland Test Client, Demo Window, Send
  Test Notification, Poly Fill). The shell has no built-in entry for them
  either ("Poly Demo" and "Wayland Test Client" are no longer built into
  `poly-shell`): it learns of test apps only from desktop files, which xtask
  adds to the image when `--test-apps` is given or the dev menu's "Test
  apps" switch is on (saved with the other settings as `test_apps`,
  default off, remembered once turned on). The compositor is the same
  program either way and has no flag for it. Off is what normal and release
  runs and anything shipped from main get. The boot tests that use the
  test apps (desktop, desktop-shell, desktop-win32) build their image with
  the switch on themselves, so CI keeps covering them; boot group `wayland`
  puts `wl-client` in its own image. `notify-send`, the command-line
  notification sender (ADR 0126), stays in the image; only its test desktop
  file goes.
- **Wayland Test Client is interactive from the launcher.** `wl-client`
  is a one-shot protocol test that exits as soon as its checks pass, so
  the launcher made it look like no window opened. With `--interactive`
  it keeps the drawn surface and exits on `xdg_toplevel.close`; the
  launcher's desktop file passes the flag. Without the flag (boot group
  `wayland`) it behaves as before.
- **Poly Fill closes after a move (ADR 0131's desktop link, both ways).**
  `/dev/win32k` used to drop the compositor's records. Now the compositor
  sends a `Configure` with the frame's new client rectangle whenever a
  Windows window moved or was resized, and a `Request::Close` when its
  title-bar close button is clicked or Alt+F4 is pressed on it. win32k
  applies the geometry to the window (`Wm::configure_from_desktop`) so
  `GetWindowRect` agrees with the frame, and posts `WM_CLOSE` to the
  window's own thread (`Wm::post_system_message`), whose default handling
  destroys the window.
- **The launcher comes back** when a no-window app (Send Test
  Notification) exits, as it does when the last window closes
  (`desktop::launcher_should_reopen`).
