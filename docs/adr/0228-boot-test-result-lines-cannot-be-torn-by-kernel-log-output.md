# 0228: Boot-test result lines cannot be torn by kernel log output

**Status:** Accepted. Track `serial-digest`. Builds on ADR 0046 (programs' terminals,
`/dev/ttyS0` on COM1), ADR 0080 (the NT console host, which draws on its terminal with VT
sequences), ADR 0155 (the Chrome launcher, `PolyDebugPrint`) and ADR 0214 (CI runs the Chrome
stages on every pull request). Inputs: ECMA-48 (control sequences), the PC16550D datasheet, and
Poly's own serial logs of the Chrome stages. No third-party source.

## What was there

The kernel log and the programs' terminal share one serial port, COM1, and the boot tests read
their results from it. Every write took the port's spin lock, but **once per call**:

- `kprintln!` was one call per line. `kprint!` pieces were separate calls, and so was each piece of
  a `writeln!` through the `Kprint` adapter. That is how `ktest` writes `test … ok`.
- A program's terminal output reached the port one system call at a time. The Chrome launcher
  wrote its digest line `LAUNCH-SHA256 stdout <64 hex digits>` **two hex digits per `WriteFile`**,
  and each one went NT console → VT bytes → `log::write_tty`. A kernel log line from any CPU could
  fall between two calls, into the middle of the digest.

The xtask parser rebuilt the console's text by cutting out kernel lines that start with six known
prefixes. Any other kernel line inside the digest row (`fs:`, `d3d11:`, …) lost the digest, and
the stage failed although Chrome's output was right. That is the flake that failed `chrome
(s2-sandbox)` on PR #302 and stage 7 twice locally. The NT console also wraps the 85-column
digest row with a cursor move, so even a whole write is two rows on the wire.

## Decisions

1. **One gate in front of the port** (`crates/poly-logline`, host-tested, no heap). It lives
   under COM1's lock together with the port. Everything the kernel log and `/dev/ttyS0` write goes
   through it. The shell's own prompt and echo do not (`write_console`): they must show at once.
2. **Kernel text goes out as whole lines.** It is collected per writer: the running thread, or a
   CPU's interrupt handlers. A writer with an unfinished line has a slot of its own (64 slots of
   512 bytes). A line goes out when its `\n` comes. A line that is longer than a slot, older than
   the window, or the oldest when every slot is taken goes out as it is. `Kprint` overrides
   `write_fmt`, so a `writeln!` is one call.
3. **Kernel lines wait while a program's terminal row is open.** A VT-aware tracker follows the
   terminal's output. The row is open once text was written after the last line feed or
   cursor-position sequence (`CSI … H`/`f`: the NT console starts every row with one). While the
   row is open and the program wrote in the last **250 ms**, whole kernel lines wait (up to
   32 KiB). They go out at the row's next boundary, inside the write that reaches it, or once the
   terminal has been quiet for the window. The boot CPU's timer tick polls the gate (`try_lock`,
   cheap when nothing waits). `qemu::exit`, the power-off and reset paths and the panic path flush
   it. Nothing is dropped: when the waiting room is full, what waits goes out first and tears that
   one row, as before. Kernel lines keep their order among each other.
4. **Result lines are framed and checksummed on a channel that is whole by construction.** The
   console's copy of a line can still be wrapped or cut: a row longer than 80 columns is two rows
   on the wire, and the gate lets kernel lines out at that boundary. So the launcher writes each
   line in one `WriteFile`. It also sends its result lines (`LAUNCH-DIGEST`, `LAUNCH-SHA256`,
   `LAUNCH-EXIT`) through `PolyDebugPrint` as `POLY-RESULT <line> crc32=<8 hex digits>`.
   `PolyDebugPrint` is one `kprintln!`, one whole kernel line. The frame can start in the middle
   of a console row, so the parser (`console::result_frames`) finds it anywhere in a line. It
   trusts a frame only when its CRC-32 (IEEE 802.3, as zlib) matches, and counts the others as
   bad. `Summary::parse` lets frames win over the console's rows. A stage whose standard output
   is the recorded one byte for byte counts as having printed the expected line.

## Why this and not a second port

A second serial port (or virtio-console) carrying only result lines would need a device in every
QEMU invocation of every boot test, a second log file, CI changes and a driver path in the NT
personality. The kernel log is already one atomic line per `kprintln!`, under the same lock as
everything else. A frame on it, with a checksum, is the smallest change that is robust even when
something unforeseen writes into the middle of a line: such a frame is rejected, never misread.
The gate makes the console's own rows whole as well, for every boot test that reads a program's
output (the Linux test programs print their results on `/dev/ttyS0`).

## Tests

- `poly-logline`: 12 host tests, among them many writers with lines in pieces that are never torn
  and keep their order, a 2-byte-per-write terminal row with kernel lines from three writers,
  the full waiting room, the quiet terminal, the slot taken by every writer.
- Kernel self-test `log::result_lines_stay_whole_while_other_cpus_log`. It drives a second,
  capture-only console through the same code (the self-test runs on every boot, also while a
  program draws on COM1). Three threads on other CPUs log while the test thread writes result
  lines in `kprint!` pieces and terminal rows two bytes per write. Every line must come out whole
  and in order. With the hold window set to 0 it fails ("a line was torn").
- xtask: `console::result_frames` finds whole frames and rejects torn, altered and cut ones.
  `Summary::parse` reads PR #302's failure shape (a `d3d11:` line inside the digest row) correctly
  from the frames.
