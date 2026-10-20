# 0035: The screen terminal: a status bar and a text grid on the display, a second shell on the keyboard, the kernel log mirrored through a ring

**Status:** Accepted. Builds on [ADR 0019](0019-display-framebuffer-and-drawing.md)
(display, `poly-gfx`), [ADR 0020](0020-ps2-keyboard-and-layouts.md)
(keyboard) and [ADR 0021](0021-kernel-shell.md) (the shell and its
`Console` trait).

## Context

Phase 2 ends with the kernel booting into a full-screen terminal that is
usable with the keyboard. The parts exist: a display with a RAM back
buffer and a drawing library whose `TextGrid` knows the escape sequences
the line editor writes and which cells changed; a PS/2 keyboard driver
that turns scancodes into characters and special keys; a shell that talks
to any `Console`. Joining them raises four questions: how keys reach the
line editor, how the kernel log gets onto the screen without deadlocks,
how often to draw (a full redraw of 1280x800 takes about 40 ms under
QEMU's TCG, a full flush 20 ms), and what happens to the serial console,
which the boot tests and `cargo xtask run` depend on.

## Decision

**A second shell, not a replacement.** The serial shell stays exactly as
it is. When there is a display, `terminal::start` (after the kernel tests,
before the serial shell, so its log line comes before the serial prompt)
puts a terminal on it and runs another shell, `shell-screen`, on a
`ScreenConsole`. Both shells share the command table; each has its own
history. `poly.display=off`, no VGA, or `poly.terminal=off` (the boot
screen stays, which the boot-screen screenshot test uses) start nothing.

**Logic in a host-tested crate, `poly-term`;** the kernel module is glue
(memory, locks, threads, devices). The crate has the screen layout
(`Screen`: 8x16 font, 12x24 from 1920x1200 up; the status bar on the top
row; the grid below; scrollback up to 1000 lines within a 3 MiB cell
budget), the status line (`status::format`, `status::draw`), the key
encoding (`keys::translate`), the log ring (`LogRing`) and `Terminal`, a
`TextGrid` plus the rules for sharing it between a shell and the log.

**Keys become the bytes a terminal sends** (xterm conventions: UTF-8,
`CR`, `DEL`, `ESC [ A`, `ESC [ 1 ; 5 D`, `ESC [ 3 ~`, ...), which the
console decodes with the line editor's own `TermDecoder`, as the serial
console does. Mapping keys straight to `Edit`s would have been shorter,
but the byte encoding is what a TTY needs later (Phase 4: programs read
these bytes from `/dev/tty`), it keeps one decoder for both consoles, and
it is tested on the host through the real keyboard pipeline (scancodes,
layouts, modifiers). Shift+Page Up / Page Down are the terminal's own:
they scroll the view into the scrollback; any key that types scrolls back
to the screen.

**The log never draws.** `log::_print` runs in any context: interrupt
handlers, with spinlocks held, with interrupts off, inside the scheduler.
Rendering there could deadlock (the display lock, the terminal's mutex)
and would hold interrupts off for tens of milliseconds. So `_print` only
appends to a 64 KiB `LogRing` behind its own spinlock, which overwrites
the oldest bytes and is read by position: a reader that fell behind learns
how much it lost, and a reader that starts late (the terminal, after the
tests) gets the last 64 KiB of the boot log. Readers hold the lock only to
copy bytes out, and never print while holding it. Whoever holds the
terminal drains the ring into the grid: the shell before writing its own
output (so the two stay in order), the render thread otherwise.

**The prompt survives the log as on the serial line.** The console marks
the prompt shown while it waits for keys; log output first erases the
prompt row, and within 50 ms the console hands the shell an `Edit::Redraw`,
which draws the prompt and the line again below the log. Log text is grey
(`LOG_STYLE`) so the shell's own output stands out.

**One render thread draws, in batches.** Writers change cells (under the
terminal's sleeping mutex) and poke the render thread, which renders what
changed (`TextGrid`'s dirty tracking; scrolling moves pixels) at most once
per 15 ms, looks for log output every 25 ms, and refreshes the status bar
(version, uptime, free memory, keyboard layout, CPUs, scrollback position)
every second. It flushes the changed rectangle in bands of 64 rows, so the
display's spinlock (interrupts off) is never held for a whole-screen copy.

**Kernel tests and the terminal stay out of each other's way.** The screen
console reads the keyboard with `keyboard::next_shared_event_until`, which
takes no keys while kernel tests run (`ktest::running`), since the
keyboard tests read the keyboard themselves; the render thread pauses
while a `display::` test runs (`ktest::current`), since those draw on the
screen and check it.

## Consequences

- `cargo xtask run` opens a QEMU window with a usable shell; the serial
  shell, the boot log on the serial port and every existing boot test are
  unchanged (the boot-screen screenshot test boots with
  `poly.terminal=off`).
- A boot test types a command through QEMU's monitor (`sendkey`) and
  compares screenshots with what `poly-gfx` and `poly-term` draw on the
  host: the output row cell for cell, the status bar's fixed start and its
  colours. It takes a screenshot every half second until they match, so a
  loaded host makes it slower, not flaky.
- Log output reaches the screen up to 25 ms later than the serial port,
  and a burst of more than 64 KiB between two drains loses its oldest part
  (the terminal says how many bytes).
- Each `kprintln` formats its arguments twice (serial and ring). Cheap next
  to the serial port itself.
- The status bar says "1 of N CPUs" until SMP brings up the others.
- The byte encoding is ready for a TTY layer: the terminal's input side
  becomes a line discipline's input later, and its output side what
  programs write to the console.
