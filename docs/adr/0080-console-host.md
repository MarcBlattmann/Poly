# 0080: The console host: Windows consoles on Poly's terminals, console handles as file objects, standard handles in the process parameters

**Status:** Accepted. Builds on [ADR 0079](0079-our-kernelbase-and-kernel32.md)
(our `kernelbase`/`kernel32`; this replaces its interim standard handles),
[ADR 0067](0067-nt-file-io.md) (NT file and device objects, IRP-style
requests), [ADR 0065](0065-nt-threads-synchronisation-and-apcs.md)
(threads and waits), [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
(our `ntdll`, its service stubs), [ADR 0073](0073-windows-file-system-layout.md)
(the process parameters), [ADR 0046](0046-terminals-line-discipline-ptys-job-control.md)
(Poly's terminals, `poly-tty`) and [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md).

## Context

The Phase 5 item is "Console: our console host, `AllocConsole`,
`ReadConsole` / `WriteConsole`, standard handles". Every Windows console
program (7-Zip's `7z.exe`, the proof of concept after this) reads and
writes through console handles or redirected standard handles, asks
`GetFileType` which it has, sets modes and code pages, and expects Ctrl+C
to reach its handler. Until now `kernelbase`'s `GetStdHandle` opened
`\??\Z:\dev\console` (ADR 0079): a plain VFS character device with none of
the console's behaviour, and the same thing even when the program's output
was meant to go elsewhere.

On Windows a console is owned by a console host; a program reaches it
through handles to a console driver, gets standard handles from its
creator (Learn: "Creation of a Console", "Console Handles", the standard
handle inheritance of `CreateProcess`), and a redirected handle is an
ordinary file or pipe handle on which console functions fail. Poly
already has terminals (ADR 0046: the serial port, the screen, pseudo-
terminals) with a Linux line discipline, which is not the console's: the
console edits lines itself (`ENABLE_LINE_INPUT`), echoes itself, turns
Ctrl+C into a control event, and keeps a screen buffer whose cells,
cursor and attributes programs read and set.

Sources (clean room): Learn's "Consoles", "Console Functions" and the
reference page of each function implemented, "Console Handles", "Console
Virtual Terminal Sequences", "Low-Level Console Modes", "High-Level Console
Modes", "Creation of a Console", "Console Screen Buffers",
"HandlerRoutine callback function", "CTRL+C and CTRL+BREAK Signals",
"GetStdHandle"/"SetStdHandle", "CreateFile" (consoles), "INPUT_RECORD",
"KEY_EVENT_RECORD", "CONSOLE_SCREEN_BUFFER_INFOEX", "Code Page
Identifiers", "ZwDeviceIoControlFile"; the public headers' layouts and
constants (`wincon.h`, `consoleapi.h`, `wincontypes.h`, `winuser.h`,
`winioctl.h`, `winternl.h`, `ntstatus.h`); ECMA-48 for the terminal side;
the published IBM PC (437) and Windows-1252 character sets. Export names
from the coverage map. No Windows binary, conhost, Wine or ReactOS source
was looked at. There is no Windows oracle VM yet, so what Learn leaves
open is a labelled choice (below).

## Decision

### A console is an object of the NT personality, shown on one of Poly's terminals

Three layers, each doing one thing:

| Layer | Where | What |
|---|---|---|
| The console's logic | `crates/poly-console` (`no_std`, host-tested) | modes and their checks, code pages, terminal bytes → key events, cooked-mode line editing, the screen buffer and the VT bytes that mirror it, the control-handler list, the ioctl protocol |
| The console host | `personalities/nt/src/console` (host-tested with the fake kernel) | consoles in the file I/O state, console file objects, reads that wait as I/O request packets, `NtDeviceIoControlFile`, attaching, events for the kernel |
| The terminal side | `kernel/src/nt/console.rs` | which terminal shows which console, raw mode and a reader thread, sending output, control-handler threads, a new process's console and standard handles |

A console program's console therefore lives as long as some process is
attached, like a Windows console, but needs no host process: Poly has no
conhost; the NT personality is the host.

### Console handles are file objects on `\Device\PolyConsole`

| Name | Opens |
|---|---|
| `\Device\PolyConsole\New` | a new console and a connection (`AllocConsole`) |
| `\Device\PolyConsole\<id>` | a connection to console `<id>` |
| `\Device\PolyConsole\Process\<pid>` | a connection to the console of process `<pid>` (`AttachConsole(pid)`) |
| `Input` relative to a console handle | the input buffer (`CONIN$`) |
| `Output` relative to a console handle | the screen buffer (`CONOUT$`) |

A *connection* is a process's attachment (its `ConsoleHandle`); the
console closes when its last connection does (Learn: "FreeConsole"). Being
file objects, console handles close, duplicate and inherit like any
handle, `NtQueryVolumeInformationFile(FileFsDeviceInformation)` reports
`FILE_DEVICE_CONSOLE` (so `GetFileType` says `FILE_TYPE_CHAR`),
`NtReadFile` on an input handle is a read in the input code page (cooked
or not, as the mode says) and `NtWriteFile` on an output handle writes in
the output code page — so `ReadFile`/`WriteFile` on console handles behave
as Learn documents without `kernelbase` knowing. Everything else goes
through `NtDeviceIoControlFile` with Poly's own codes
(`poly_console::ioctl`: `CTL_CODE(FILE_DEVICE_CONSOLE, 0x800+n,
METHOD_BUFFERED, FILE_ANY_ACCESS)`): modes, Unicode and ANSI reads and
writes, input records, the screen buffer, fills, code pages, the title,
the control routine, `GenerateConsoleCtrlEvent`. Windows' own console
driver protocol is not public and is not imitated. `NtDeviceIoControlFile`
is a new service, `0x0D0` in the file I/O block (its own table,
`DEVICE_SERVICES`, so it merges beside other new services); on anything
but a console handle it is `STATUS_INVALID_DEVICE_REQUEST`, which
`kernelbase` reports as `ERROR_INVALID_HANDLE` — "WriteConsole fails if it
is used with a standard handle that is redirected to a file" (Learn).

A read that cannot finish (no line yet, no input record) is an I/O request
packet parked on the console and completed when input comes, through the
same completion path as ADR 0067's pipes: a synchronous handle waits, an
asynchronous one gets `STATUS_PENDING`.

### Standard handles come from the process's descriptors and go into its parameters

A process started by the kernel (the shell's `run`, `poly.init`, a test)
has a Linux-side file table. For each of descriptors 0, 1, 2:

- a **terminal** (by device number: `/dev/console`, `ttyS0`, `tty1`, a
  pty slave) → a console handle: the console of that terminal, made on
  first use and shared by every process on it (input for 0, the screen
  buffer for 1 and 2, as separate handles);
- **anything else** (a file, a pipe, the trace capture of `poly.init`) →
  an ordinary file handle on that open file (a new `Target::Host` of the
  file I/O layer: read and written as it is, appended so `2>&1` does not
  overwrite; `GetFileType` by what it is: disk, pipe, or character);
- **nothing** → NULL.

Only an `IMAGE_SUBSYSTEM_WINDOWS_CUI` image gets a console (Learn:
"Creation of a Console"); a console program none of whose descriptors is
a terminal still gets a console, shown nowhere (as a detached Windows
program's console is never shown), so `CONOUT$` works. A GUI program gets
none, and its terminal descriptors become NULL.

The handles go into `RTL_USER_PROCESS_PARAMETERS`: `winternl.h` names only
`ImagePathName` and `CommandLine` and leaves the 80 bytes before them as
`Reserved2[10]`. **Poly's own placement**: the console connection in
`Reserved2[0]` (+ 0x10), the standard input, output and error in
`Reserved2[2..5]` (+ 0x20, + 0x28, + 0x30) — to be confirmed by the spec
agent; programs use `GetStdHandle`. `kernelbase` reads them at
`DLL_PROCESS_ATTACH` (no more opening `\??\Z:\dev\console`), and
`CreateProcessW` (a later item) passes them the same way, inheriting or
using `STARTUPINFO`'s handles as Learn documents.

### The terminal side

- **Output**: the console's screen buffer is exactly the terminal's size
  (80×25 if the terminal does not know its size), its window the whole
  buffer, and every change is mirrored as VT bytes: characters in UTF-8,
  `CUP`/`SGR` only when the terminal's cursor or rendition differs from
  the buffer's, erase sequences for VT erases. A console takes the
  terminal over on its first output (reset, scrolling limited to its rows
  with `DECSTBM`, clear) so the two start equal, and gives it back when it
  closes (full scrolling region, the line below its cursor). Line feeds at
  the bottom are sent from the first column, so it does not matter whether
  the terminal adds a carriage return.
- **Input**: once a console needs input — a read, a peek, or a process
  registering its control routine (any program with `kernelbase`, so
  Ctrl+C reaches even a program that never reads) — the kernel saves the
  terminal's termios, sets raw mode (no `ICANON`, `ECHO`, `ISIG`,
  `IEXTEN`, `ICRNL`, `IXON`, `OPOST`) and starts a reader thread that feeds
  the console what the terminal sends (polling every 10 ms, so it stops
  without stealing a byte once the console closes). The settings come back
  when the console closes. A program that only writes never changes its
  terminal's mode.
- **Ctrl+C**: with `ENABLE_PROCESSED_INPUT` it is "processed by the system
  and is not placed in the input buffer" (Learn: "SetConsoleMode"); the
  console raises `CTRL_C_EVENT` for every attached process that registered
  a routine, and the kernel starts a thread at that routine in the process
  (Learn: "HandlerRoutine": "the system creates a new thread in the
  process to execute the function"). `kernelbase`'s routine calls the
  handlers last registered first until one returns TRUE, else ends the
  process with `STATUS_CONTROL_C_EXIT`. `SetConsoleCtrlHandler(NULL, TRUE)`
  ignores Ctrl+C.
- **`AllocConsole`** makes a console on Poly's system console terminal
  (`/dev/console`): there is no window to open.

### Behaviour of the console (`poly-console`)

- **Modes** (Learn: "SetConsoleMode"): a console starts with every input
  mode but `ENABLE_WINDOW_INPUT` and `ENABLE_VIRTUAL_TERMINAL_INPUT` (0x1F7)
  and output `PROCESSED | WRAP_AT_EOL` (3). Unknown bits, and
  `ENABLE_ECHO_INPUT` without `ENABLE_LINE_INPUT`, are
  `ERROR_INVALID_PARAMETER`; insert and quick-edit change only with
  `ENABLE_EXTENDED_FLAGS`.
- **Output**: processed output handles backspace, tab (spaces to the next
  multiple of 8), line feed (with a carriage return unless
  `DISABLE_NEWLINE_AUTO_RETURN`), carriage return and bell; other control
  characters, and all of them without processed output, show their IBM PC
  glyph. `ENABLE_VIRTUAL_TERMINAL_PROCESSING` acts on SGR (16 colours,
  256-colour and RGB mapped to the nearest), cursor moves, erases, save and
  restore, cursor visibility and the title (`OSC 0/2`); without it ESC is
  a character.
- **Input**: terminal bytes become key events (the table in
  `poly_console::input`: characters, Enter, Backspace, Tab, Ctrl+letters,
  Escape, Alt+key, arrows, Home/End, Insert/Delete, Page Up/Down, F1–F12),
  each a key-down and a key-up record (a terminal reports no releases).
  **Cooked reads** take records until Enter's key-down and return the line
  with `\r\n`: Backspace, Delete, Left, Right, Home, End, Insert
  (overwrite) and Escape (clear) edit it; it is echoed with
  `ENABLE_ECHO_INPUT`; a line longer than the caller's buffer is returned
  over several reads. Enter's key-up stays in the buffer, where a later
  `ReadConsoleInputW` finds it (as typing on a real console leaves it).
  Without `ENABLE_LINE_INPUT` a read returns the characters there are (at
  least one).
- **Code pages**: 437 (a new console's, the OEM code page of US Windows),
  1252, 20127 and 65001; others are `ERROR_INVALID_PARAMETER`. UTF-8 split
  across writes is joined; unmappable characters become `?`.
- **Screen buffer info**: `CONSOLE_SCREEN_BUFFER_INFO(EX)` from the
  buffer; the popup attribute (0xF5) and colour table (the classic 16
  colours) are Poly's.

### Functions

`kernelbase` (forwarded from `kernel32` through `api-ms-win-core-console-*`
contracts, as `poly_kernelbase::exports` lists): `AllocConsole`,
`FreeConsole`, `AttachConsole` (`ATTACH_PARENT_PROCESS`: the console the
process started on, while it exists), `GetConsoleMode`/`SetConsoleMode`,
`ReadConsoleW/A`, `WriteConsoleW/A`, `ReadConsoleInputW`,
`PeekConsoleInputW`, `WriteConsoleInputW`, `GetNumberOfConsoleInputEvents`,
`FlushConsoleInputBuffer`, `GetConsoleScreenBufferInfo(Ex)`,
`SetConsoleTextAttribute`, `SetConsoleCursorPosition`,
`FillConsoleOutputCharacterW/A`, `FillConsoleOutputAttribute`,
`GetConsoleCP`/`SetConsoleCP`, `GetConsoleOutputCP`/`SetConsoleOutputCP`,
`SetConsoleCtrlHandler`, `GenerateConsoleCtrlEvent`, `GetConsoleWindow`
(always NULL: a Poly console has no window, which Learn allows),
`SetConsoleTitleW`/`GetConsoleTitleW`, `GetStdHandle`/`SetStdHandle`, and
`CreateFileW` of `CONIN$`, `CONOUT$` and `CON`.

## To be confirmed by the spec agent against real Windows

1. Where `RTL_USER_PROCESS_PARAMETERS` keeps the console and standard
   handles (Poly: `Reserved2[0]`, `[2]`–`[4]`), and which handle values a
   new console process gets.
2. `SetConsoleMode` with `ENABLE_ECHO_INPUT` but not `ENABLE_LINE_INPUT`
   (Poly: `ERROR_INVALID_PARAMETER`), with unknown bits, and what
   `GetConsoleMode` reports for insert/quick-edit after a mode without
   `ENABLE_EXTENDED_FLAGS`.
3. A cooked `ReadConsoleW` ended by Ctrl+C (Poly: TRUE, 0 characters,
   `ERROR_OPERATION_ABORTED`; `ReadFile`: FALSE, `ERROR_OPERATION_ABORTED`).
4. The default handler's exit code (Poly: `STATUS_CONTROL_C_EXIT`), and
   whether `SetConsoleCtrlHandler(NULL, TRUE)` also stops handlers being
   called (Poly: yes, for Ctrl+C).
5. `FreeConsole` without a console (Poly: `ERROR_INVALID_PARAMETER`); what
   `FreeConsole`, `AllocConsole` and `AttachConsole` do to the standard
   handles (Poly: free leaves them, alloc and attach replace all three).
6. `AttachConsole(pid)` of a process without a console vs. one that does
   not exist (Poly cannot tell them apart: `ERROR_INVALID_PARAMETER`).
7. `GetConsoleTitleW`'s return value with a short buffer (Poly: the full
   length, the copy cut and NUL-terminated).
8. `FillConsoleOutput*` from outside the buffer (Poly:
   `ERROR_INVALID_PARAMETER`), the popup attribute and colour table, and
   the default code page of a console on a non-US system.
9. Which control characters cooked echo shows as `^X`, history keys, the
   key-up records of typed input, and the scan codes (Poly: 0).
10. The API-set contract of each console function (`poly_kernelbase::exports`).

## Consequences

- Host tests: `cargo test -p poly-console` (modes, code pages, input
  decoding, the VT parser and SGR mapping, the screen buffer and its
  terminal bytes, cooked editing, whole-console reads, writes, Ctrl+C and
  titles, the handler list, the ioctl layouts) and `cargo test -p
  poly-personality-nt console` (handles by name, modes, waiting reads,
  input records, Ctrl+C events, code pages and titles, the last connection
  closing the console, ioctls on files).
- The ring-3 test `nt::test_console` runs `contest.exe` (a fourth program
  of the Win32 workspace) on a pseudo-terminal with standard output
  redirected to a file and plays the user: about 90 checks of handles and
  `GetFileType`, modes, UTF-16 and UTF-8 output, attributes, cursor,
  fills, screen-buffer info, VT sequences, the title, two cooked lines
  (one edited with Backspace), raw records, a Ctrl+C handler, `CONOUT$`
  and freeing and allocating consoles; then the terminal's bytes, the
  file's content and the restored terminal settings.
- Every NT console program started on the system console now draws on it
  through the console (k32test's report included): the first output
  clears the terminal, as a new console window would start empty.
- Edits to shared files are additive: the file I/O target and device
  enums, two match arms in reads and writes, the volume device type, the
  parse hook's console branch, visibility of four helpers, a service table,
  a dispatch arm, two `params` constants and a writer, `start_with_std`
  in the kernel, `kernelbase`'s state and DLL entry, `CreateFileW`'s
  console names, and the Win32 build's program list.
- Left for later: `CreateConsoleScreenBuffer` and
  `SetConsoleActiveScreenBuffer`, `ReadConsoleOutput*`/`WriteConsoleOutput*`
  and `ScrollConsoleScreenBuffer`, the ANSI input-record functions, input
  history and `doskey` aliases, mouse and window events,
  `ENABLE_VIRTUAL_TERMINAL_INPUT` (accepted as a mode, but input is
  always decoded into key events),
  `CTRL_CLOSE_EVENT` when a terminal hangs up, process groups for
  `GenerateConsoleCtrlEvent`, and the pseudo-console API
  (`CreatePseudoConsole`).
