# 0167: A busy pointer while an app starts: launch tracking by activation token and process, a spinner drawn from distances

**Status:** Accepted (owner request, 2026-11-09). Builds on
[ADR 0106](0106-wayland-server-core-and-software-compositor.md) (the
software cursor and `xdg_activation_v1`),
[ADR 0107](0107-desktop-fbdev-input-routing-and-shell.md) and
[ADR 0117](0117-shell-window-management-notifications-and-settings.md)
(the shell starts apps), [ADR 0124](0124-native-desktop-alpine-shell-look-and-builds-on-windows-and-linux.md)
and [ADR 0127](0127-the-alpine-shell-look.md) (the look),
[ADR 0126](0126-the-desktop-on-the-native-runtime.md) (the compositor as a
native program) and [ADR 0128](0128-windows-gui-win32k-in-the-nt-personality-and-the-desktop-link.md)
(the desktop link).

## Context

Starting a big app (Chromium, a Windows program, anything from a slow disk)
takes seconds in which nothing on the screen changes, and a click that
looks like it did nothing gets repeated. The owner wants the pointer to say
"working": **only the pointer changes** (an arrow with a small spinner),
**no pulsing launcher or taskbar entry**.

Nothing in Poly knew which window belonged to which launch. The launcher
([`Shell`](../../crates/poly-shell/src/shell.rs)) only handed an `App` to the
compositor, which spawned it and forgot it, and
`Server::activation_token()` existed but nothing used it.

## Decision

### What "starting" means

A **launch** (`poly_wayland::launch::Launches`) begins when Poly starts an
app and ends at the first of:

| End | When | Covers |
|---|---|---|
| activated | a client calls `xdg_activation_v1.activate` with the launch's token | toolkits that read `XDG_ACTIVATION_TOKEN` (GTK, Qt, Chromium's Wayland backend) |
| window | a window is mapped by the launched process or by a process it started, any depth | every Wayland client (its peer's pid), X windows (`_NET_WM_PID`), Windows windows (the process win32k reports) |
| failed | the process ended with a non-zero status before any window | crashes, missing libraries |
| quiet exit | the process ended with status 0 and no window appeared in 3 s | a program with no window to show (`notify-send`), a launcher that handed over to a running instance whose window did not show |
| timed out | 30 s without any of the above (logged) | apps that never show a window |
| cancelled | the program could not be started | |

The pointer is busy while **any** launch is pending, so launching several
apps at once keeps it busy until the last has a window (or ends otherwise).
The bookkeeping is `poly-wayland/src/launch.rs`: no clock of its own (the
caller passes milliseconds), no I/O, so every rule above is a host test.

The launched process's descendants are found by walking parents from the
window's process (`/proc/<pid>/stat`, at most 32 generations, a loop stops
it). A `sh -c` wrapper or a browser's forked processes therefore still end
the launch; an app that daemonises (its parent is gone) is caught by the
token, the quiet exit or the timeout.

### The token (xdg-activation-v1)

`Server::begin_launch(name)` makes a token (the existing
`activation_token()`), registers the launch, and the compositor puts it in
the new app's environment as `XDG_ACTIVATION_TOKEN` **and**
`DESKTOP_STARTUP_ID` (startup-notification's name, for toolkits that still
read it), replacing any such variable it inherited. `activate` with a known
token ends the launch whatever surface it names, and still activates the
toplevel as before (the token works once). The protocol was already served
by ADR 0106; what is new is that tokens belong to launches.

### Programs that start apps themselves (Files)

Files starts apps without the compositor knowing. It writes to the
compositor's control pipe (`$POLY_COMPOSITOR_CONTROL`, already in every
client's environment; the compositor opens it for reading and writing, so a
writer never blocks or fails for want of a reader), one line each:

```text
launch <token> <name>     before starting; the token goes to the app
pid <token> <pid>         after starting
cancel <token>            starting failed
```

`poly_app::startup::Startup` writes them and gives the token variables
(`poly-launch-<pid>-<n>`); Files' `spawn` uses it and drops the token
variables it inherited. The parser is `launch::Control` (round-trip tested
with the writer). A pipe that is full or gone loses the line: the wait ends
by itself, it cannot hang anything.

### Which pointer where

`CursorLook::Busy(phase)` is the look when a launch is pending **and no
client has set the cursor**: over the empty desktop, the shell's panels and
the launcher. A client's own request always wins, over its windows: a
cursor surface, "no cursor", or a named shape. A client that has not
called `set_cursor` (nor `wp_cursor_shape`) yet shows the desktop's pointer,
busy or not. Leaving a client resets the request (ADR 0106), so the busy
pointer is back on the desktop at once. Windows apps' cursors reach the
scene through their own surfaces (ADR 0128) and so win over their windows too.

### The artwork

Original, drawn from distances at the output's scale
(`compose/busy.rs`; no image files, no fonts): Poly's arrow
(white with a black outline, unchanged) and, at its lower right, a ring of
5 px radius and 2.5 px stroke centred 18 px right and 18.5 px down from the
hot spot, anti-aliased to one device pixel. A 135° arc turns clockwise
over it in 16 pictures of 60 ms (a turn per 0.96 s) and fades toward its
tail (35 % opacity at the tail end). Every colour is Alpine Shell's:

| Theme | Arc | Track | Halo (1 px, outside the ring) |
|---|---|---|---|
| dark (title bar `#1a1a19`) | ink `#ebebe7` | white at 25 % | black at 43 % |
| light (title bar `#d6d6d6`) | ground `#202020` | black at 22 % | white at 59 % |

Which one is read from the scene's frame theme (the same one the shell
switches with the settings, `Scene::set_theme`), so the pointer follows
light and dark without a new setting. The pointer's rectangle grows from
12 × 19 to 27 × 28 logical pixels while busy; a new picture damages only
that rectangle (a test checks it), so a busy desktop costs one small blit
every 60 ms and nothing else; the compositor's poll wakes for the next
picture or timeout and only while a launch is pending.

### X11 and Windows apps

Xwayland's clients all connect as the one Xwayland process, so Wayland's
peer pid says nothing about them: the compositor reports X windows from the
X window manager when one is mapped (`_NET_WM_PID`, not override-redirect).
Windows windows: the desktop link (ADR 0128) gets a new record
`ToCompositor::Process { surface, pid }`, sent after `Create` by
`Desktop::set_process` (from the process win32k reports for the window) and
replayed on reconnection; a reader that does not know the kind skips it
(the link's rule), so no version bump. The compositor reports the process
when the surface is first shown. The NT personality's process id is the kernel's (`Caller::user(pid, tid)`
in `kernel/src/nt`), so it is the pid the launch has; the `desktop-win32`
boot test checks that the Windows test app's launch ends by its window,
not by the timeout.

### What the log says

For the boot test and for people: `busy pointer on: launching <name> (<n>
pending)`; `launch <name>: <how> after <ms> ms (<n> pending)` where *how* is
`activated by its token`, `window mapped by pid <pid>`, `exited with
status <s>, no window`, `timed out after 30000 ms, no window` or `not
started`; `busy pointer off` when none is left.

### Test apps

`wl-demo` takes `--delay <ms>` (waits before it connects) and `--activate`
(activates its window with `$XDG_ACTIVATION_TOKEN` before the first
buffer, as toolkits do); `--test-apps` adds "Slow Window" and "Slow
Activated" to the launcher.

## Not done

- **Pulsing launcher or taskbar entries**: the owner asked for the pointer
  only.
- **Waiting for the app to finish loading** (a splash, a window that stays
  empty): the launch ends when a window is mapped, as every desktop does.
- **Per-app timeouts**: 30 s for all, from the request.
- **Single-instance apps handing over to a running instance that sends no
  token back**: the wait ends 3 s after the launcher process exits.
- **Hardware cursor plane**: the cursor is software-drawn (ADR 0106, 0116);
  a plane could take the 27 × 28 busy pictures later.

## Consequences

- `poly-wayland` gains `launch` and `compose/busy` and a few `Server`
  methods (`begin_launch`, `adopt_launch`, `launch_pid`, `launch_clock`,
  `launch_window_mapped`, `set_client_pid`, `launch_wait_ms`,
  `take_launch_events`); the compositor's loop calls them. The cursor's rules
  are in one place (`server/cursor.rs`).
- A launched app's environment has two more variables. An app that is
  launched by an app (Files, a shell) must not pass its own token on:
  Poly's own code drops them; other programs pass a used token, which does
  nothing.
- The desktop boot test has a step (6) for it: screenshots of the busy
  pointer (two, spinner turned), of the normal pointer after (spinner gone,
  nothing turning), and the log lines for both ways a launch ends.
