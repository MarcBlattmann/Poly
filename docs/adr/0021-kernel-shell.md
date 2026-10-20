# 0021: The kernel shell: a host-tested line editor on `alloc`, a `Console` trait, serial input on IRQ 4

**Status:** Accepted. Amends [ADR 0003](0003-workspace-layout.md): shared
crates may use `alloc` now that the kernel has a heap. Builds on
[ADR 0016](0016-interrupt-dispatch-tick-and-kernel-locks.md) (`route_isa_irq`,
`WaitQueue`) and [ADR 0008](0008-in-kernel-tests-and-backtraces.md) (the
test harness the `test` command reuses).

## Context

Phase 2 builds a terminal into the kernel: a display driver and a text
grid, a keyboard driver, and a shell on top, until user-mode programs
exist. The display and keyboard are being written in parallel; the shell
must work before them (on the serial port, which `cargo xtask run`
connects to the terminal) and after them, unchanged. It needs a line
editor (cursor movement over UTF-8 text, history, Tab completion) that is
easy to get subtly wrong, a way to read input without polling, and a way
to keep the kernel log from scribbling over the line being typed.

## Decision

**The line editor is a host-tested crate, `poly-line`, and it uses
`alloc`.** History entries, completion candidates and argument words are
strings of any length; with a heap in the kernel since Phase 1, `String`,
`Vec` and `VecDeque` are simpler and safer than fixed buffers. Shared
crates stay `no_std`; `alloc` is allowed where it clearly simplifies (the
others stay allocation-free).

**Input is an abstract edit, output is ANSI text.** The editor takes an
`Edit` (insert a character, move, kill, recall history, complete, submit,
cancel, redraw) and writes text plus a small set of ECMA-48 sequences:
cursor left / right by n, erase to end of line, erase screen and cursor
home, and SGR colors in the prompt. Every cursor movement is relative, so
the editor never needs the prompt's width or the cursor's column; every
character is one column (no East Asian wide characters, no wrapping). A
device maps its input to edits: `TermDecoder` turns a terminal's byte
stream (UTF-8, `CR`/`LF`, `DEL`, `ESC [` and `ESC O` sequences, control
keys) into edits for the serial line, and a keyboard maps key events,
mostly through `Edit::control`. The text grid only has to implement the
sequences above. The crate's tests run the editor against a tiny terminal
emulator that rejects any other output, including a randomized comparison
with a reference model.

**The shell talks to a `Console`:** `core::fmt::Write` for output and
`read() -> Option<Edit>` (blocking) for input. `None` ends the shell, which
only a scripted console does: the kernel tests drive the real shell with
typed byte strings. `shell::spawn(name, Box<dyn Console + Send>)` runs it
in a kernel thread; the serial console is one implementation, the screen
and keyboard will be another.

**Serial input is interrupt-driven.** COM1's receive interrupt (ISA IRQ 4)
is routed through the I/O APIC with `route_isa_irq`; the UART raises it for
every byte (FIFO trigger level 1). The handler drains the UART into a
256-byte single-producer, single-consumer ring of atomics (the handler,
which never nests, and the one `SerialConsole`) and wakes a `WaitQueue`.
Without a route the console polls every 10 ms.

**Log output erases the prompt; the shell redraws it.** While the shell
waits for input, the serial console marks the prompt as shown. The next
kernel log write first erases that row (`CR`, `ESC [ K`) and marks it
erased; the waiting reader checks the mark every 50 ms (a timed wait, so
the log never has to wake a thread, which it could not do safely from
every context it prints in) and returns `Edit::Redraw`, which draws the
prompt and the line again under the log output.

**Commands are a static table** of name, usage, one line of help and a
handler `fn(&mut dyn Console, &[String]) -> Result<(), String>`. Lines are
split like a Unix shell without expansions (quotes optional). An unknown
command gets the closest name by edit distance (optimal string alignment,
at most 2 edits) as a suggestion.

**`test` reuses the boot harness.** `ktest::run(env, filter, when, out)`
writes the same lines as at boot to any `fmt::Write`. One run at a time, so
a test that runs the shell cannot run the tests inside itself. At runtime
the boot's `poly.selftest.*` flags are dropped (they make tests fail or
panic on purpose), and tests that check boot-time state are marked
`#[boot_only]` and left out.

**When the shell starts.** After the boot self-test, unless the boot is an
automated test run (`poly.qemu_exit`). `poly.shell` starts it in such a run
too, after the tests pass, so a boot test can type into QEMU's stdin (the
serial port) and end the boot with `reboot` (QEMU runs with `-no-reboot`).

**Reboot** writes back the disk caches, then tries the FADT's reset
register (I/O port or memory), the 8042's reset line, and a triple fault,
each after the previous one had half a second.

## Consequences

- The terminal integration needs only a `Console` over the text grid and
  the keyboard (key event to `Edit`) and a call to `shell::spawn`.
- Lines wider than the terminal are drawn wrong (the editor does not wrap
  or scroll horizontally), and wide or combining characters put the cursor
  in the wrong column. Both need the terminal's width and a character
  width table.
- A log line that arrives between two edits of the same line (rather than
  while the shell waits) is not erased first and can garble that row until
  the next redraw (Ctrl-L). The window is the few microseconds the editor
  takes to echo a key.
- The waiting shell wakes 20 times a second to look for log output.
- Commands run on the shell's thread; a long one (`test` without a filter)
  cannot be interrupted, and Ctrl-C only cancels the line being typed.
- Bytes typed while a command runs wait in the ring (256 bytes, then they
  are dropped and counted); QEMU stops reading its stdin when the UART's
  FIFO is full, so nothing is lost before the shell starts.
- `boot_only` is a new attribute of `kernel_test!`; a test that depends on
  boot-time state should use it.
