# 0123: Flow control on the serial port's input, and what the toolkits found on the desktop: inert role objects, activation before the first map, SIGCHLD in the compositor, VLC's Qt interface through Xwayland

**Status:** Accepted. Roadmap Phase 6, the milestone "GTK and Qt apps run;
VLC plays a video with sound". Follows up the toolkits' findings
([ADR 0120](0120-gtk-qt-and-vlc-from-the-toolkits-root-disk.md)) on the
compositor on KMS ([ADR 0116](0116-compositor-on-kms-vsync-toolkit-protocols-and-backdrops.md)),
the shell ([ADR 0117](0117-shell-window-management-notifications-and-settings.md))
and Xwayland ([ADR 0118](0118-xwayland-and-polys-x11-window-manager.md));
changes the serial console's input of
[ADR 0021](0021-kernel-shell.md) (which the terminal on COM1 of
[ADR 0046](0046-terminals-line-discipline-ptys-job-control.md) shares).

## Context

ADR 0120 left five things open before the milestone could be ticked:

1. On the compositor on KMS, VLC's Qt interface was once cut off with
   `get_viewport: object 24 is a wl_subsurface, not a wl_surface`: it
   looked as if the server's object table kept a stale entry.
2. A window started from the launcher after another closed was drawn
   inactive (grey title), also after a click, though keys reached it.
3. The compositor reaped the apps it launched only when it woke for
   something else.
4. The kernel's serial input ring (256 bytes) dropped what did not fit:
   a long line a boot test typed into bash arrived cut short on KVM.
5. VLC's Qt interface died of a `SIGSEGV` at the same instruction in three
   of five runs, and aborted with "free(): double free detected" at exit.

Sources: the Wayland protocol XML (`wayland.xml`: `wl_display.delete_id`,
`wl_subsurface` "becomes inert" when its surface is destroyed;
`xdg-activation-v1.xml`), signal(7), sigaction(2), signal-safety(7),
pipe(2), waitpid(2), the PC16550D datasheet (IER, the receive FIFO, LSR),
QEMU's documentation of its serial device and character devices, and the
programs' own behaviour: `WAYLAND_DEBUG=1`, `/proc/<pid>/maps` at the
crash, and the same programs on the build container's real Linux kernel
against a host build of `poly-compositor`. No VLC, Qt, libwayland, Linux
or QEMU source was read (CLEAN_ROOM.md).

## Decision

### The object table follows `delete_id`; role objects go inert

What VLC did, from `WAYLAND_DEBUG=1` on Poly and on Linux: when its video
starts, Qt makes the video widget's native window (a `wl_surface` and a
desynchronized `wl_subsurface` of the main window) and, as VLC resizes
the main window to the video, destroys both and makes them again at once.
VLC's video output thread can be handed the first surface; it then uses
a surface the client library already freed. The ID in that freed proxy
is whatever the client gave out next (here a new `wl_subsurface`), which
is exactly the protocol error the server raised, and in the other runs
the client library itself faults on the freed memory: the faulting
instruction is in `libwayland-client.so.0` (`/proc/<pid>/maps`), and
**the same happens on Linux** against the same compositor build: in
batches of runs of `vlc -I qt --play-and-exit` 3 of 5 (two aborts, one
`SIGSEGV`), 4 of 4, and 2 of 5 with `--aout dummy`. The server's table
was right: it only changes on requests and on its own `delete_id`.

Two real stale references were found on the way and are fixed:

- A `wl_subsurface` or `xwayland_surface_v1` whose surface is destroyed
  first kept the surface's ID; once the client made a new surface with
  that ID, the old role object's requests (`set_position`, `destroy`,
  `set_serial`) reached the new surface. Both hold `Option<ObjectId>` now,
  cleared when the surface goes (`surface_gone`, as viewports and
  fractional scale objects were): inert, as `wayland.xml` says.
- Frame callbacks and presentation feedback waiting for a page flip are
  answered only if the ID still is that callback or feedback (an
  answered callback is deleted with `delete_id`; answering another
  object would delete it under the client).

Host tests: the VLC sequence with the IDs reused crosswise (the new
`wl_subsurface` gets the old surface's ID) makes the viewport and shows
the subsurface, and only the stale ID gets the error; an inert
subsurface's requests leave a new surface with its old surface's ID
alone.

### A toplevel activated before its first map is drawn active

GTK and Qt present a new window with `xdg_activation_v1.activate` before
its first buffer. The server made it the active toplevel then, with no
window to draw active yet; at the map, `activate` found it active already
and returned without drawing it so, and a click (`activate_window`) also
found it active. Keys went to it, the frame stayed grey. `activate` now
draws the window active in that case too. Host test: one app's window
closes, the next activates its toplevel before mapping it: drawn active,
reported active, still active after a click. The toolkits boot test
checks Poly's title bar is the active colour after the click.

### SIGCHLD wakes the compositor

`poly-compositor` installs a `SIGCHLD` handler (`SA_RESTART |
SA_NOCLDSTOP`) that writes a byte to a non-blocking, close-on-exec pipe
whose read end is in the main loop's `poll` set (the self-pipe way: a
child ending just before `poll` still wakes it; `signalfd` would need
`SIGCHLD` blocked, which launched apps would inherit). The loop empties
the pipe and reaps what ended (launched apps, Xwayland, the `--exec`
client, which no longer needs a 50 ms poll). Host test: a child that
ends wakes a `poll` with an hour's timeout, and is reaped with its status.

### Flow control on the serial port's input

The receive ring is now `poly_tty::rx::RxRing` (host-tested, lock-free
between one producer and one consumer). The interrupt handler only reads
the UART while the ring has room: a full ring leaves the rest in the
UART's receive FIFO and switches the receive interrupt off (IER), and the
ring is marked throttled. QEMU's UART takes no more from its character
device while its FIFO is full, so the input waits on the host side
instead of being lost. A reader (the shell, or the terminal's input
thread) that takes the ring down to half full, or finds it empty before
waiting, drains the UART again and switches the interrupt back on. The
mark is set before the producer looks at the room a second time, so a
reader that empties the ring meanwhile sees it; readers wait with
deadlines (50 ms) and look again. A real 16550 without hardware flow
control would overrun its FIFO instead (LSR's overrun bit); RTS/CTS is
for real serial ports, later. Tests: the ring's host tests (a full ring
reads nothing past its room; a 1000-byte line through a 16-byte ring
with a slower reader arrives whole; producer and consumer on two
threads), the kernel test of the ring, and a step of boot group `shell`
that types a 700-byte line ahead while the shell runs a kernel test that
spins for 200 ms: with the old ring it arrived cut short (and its Enter
with it), now whole.

### VLC's Qt interface through Xwayland

VLC 3's Qt interface embeds its video by handing the video output the
native window of a Qt widget; on Wayland that is the surface Qt may make
again (above), and VLC crashes on Linux as on Poly. Through Xwayland
(`QT_QPA_PLATFORM=xcb`, VLC's X11 video output into the Qt window) it
played 5 of 5 times on Linux against the same compositor and exited
cleanly. So the toolkits disk gets Qt 5's X11 platform plugin
(`platforms/libqxcb.so`, `xcbglintegrations`) and VLC's X11 video output
and window plugins; the desktop runs `poly-compositor --desktop
--xwayland-lazy`; the launcher's VLC entry starts it with
`QT_QPA_PLATFORM=xcb`. GTK and Qt 6 apps stay on Wayland.

Xwayland runs as root (the compositor's user) and takes local
connections from its own user only: VLC, run as `poly`, was refused
("Authorization required, but no authorization protocol specified") and
Qt fell back to Wayland. `poly-runas`, which already gives the user the
compositor's Wayland socket, now also lets the user onto the X display:
still root, it connects to `$DISPLAY` and adds the server-interpreted
address `localuser:<user>` (ChangeHosts; X11 protocol encoding,
Xsecurity(7)), then checks the answer with a GetInputFocus round trip.
Only that user gets in, as with the Wayland socket.

The check of VLC's Qt interface is counted now
(`POLY_TOOLKITS_VLC_GUI=optional` reports it only) and wants status 0
after Ctrl+Q: on Poly the window shows the video in Poly's frame and
VLC ends with status 0.

## Consequences

- Nothing in Poly's kernel was behind VLC's crash or its double free;
  they are VLC's on Wayland, and it runs through Xwayland, as its X11
  interface is the one it is built around.
- Input typed or pasted into the serial console is never lost; a reader
  that stops reading stops the sender.
- Launched apps are reaped as they end (the boot tests see the
  compositor's exit line right after the kernel's).
- Open: VLC's own Wayland window modules still need `xdg_shell` v5 or
  `wl_shell` (ADR 0120); RTS/CTS on real serial ports.
