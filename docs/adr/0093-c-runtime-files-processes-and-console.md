# 0093: The C runtime's files, processes and console: `_findfirst`, `_dup`/`_pipe` on anonymous pipes, `_spawn*`/`system`/`_popen` passing descriptors in `lpReserved2`, wildcard `argv`, console I/O on `CONIN$`/`CONOUT$`

**Status:** Accepted. A slice of the Phase 5 item "C runtime" after
[ADR 0084](0084-our-c-runtime-ucrtbase-and-msvcrt.md) (our `ucrtbase.dll`
and `msvcrt.dll`, the shared runtime crate, the host-tested `poly-crt`);
builds on [ADR 0089](0089-createprocess-handle-inheritance-startupinfo-create-suspended.md)
(`CreateProcessW` with inherited handles and the `STARTUPINFO`, its
`lpReserved2` kept in the child), [ADR 0080](0080-console-host.md) (the
console host) and [ADR 0067](0067-nt-file-io.md) (`\Device\PolyPipe`).

## Context

ADR 0084 left, among others: wildcard expansion of `argv`; `_findfirst` /
`_findnext`; `_spawn*` / `_exec*` / `system`; `_dup` / `_dup2` / `_pipe`;
console I/O (`_getch`, `_cprintf`, `_cputs`). Console programs use them
for directory listings, running tools (build drivers, archivers calling
helpers), redirection, and keyboard prompts. `_pipe` and `_popen` need
anonymous pipes, which `kernelbase` did not have (`CreatePipe`, left by
ADR 0089), and pipes need an end of stream when the writer goes, which
Poly's stand-in pipe device did not give.

Sources (clean room, CLEAN_ROOM.md): Microsoft Learn's pages of each
function (`_findfirst` and its forms, `_findnext`, `_findclose`, `_dup`,
`_dup2`, `_pipe`, "_spawn, _wspawn functions", "_exec, _wexec functions",
`_cwait`, `system`, `_popen`, `_pclose`, `_getch`, `_getche`, `_kbhit`,
`_ungetch`, `_putch`, `_cputs`, `_cgets`, `_cgets_s`, `_cprintf`),
"Expanding Wildcard Arguments", "Console and port I/O", "Parsing C
command-line arguments", `CreatePipe`, "Anonymous Pipes", "Anonymous Pipe
Operations", `ReadFile` (an anonymous pipe whose write handle was
closed), `STARTUPINFOW` (`cbReserved2` / `lpReserved2` "reserved for use
by the C Run-time"), `CreateProcessW`, `ReadConsoleInputW`,
`KEY_EVENT_RECORD`; the public headers' layouts and constants
(`corecrt_io.h`'s `_finddata*_t` and `_A_*`, `process.h`'s `_P_*` and
`_WAIT_*`, `fcntl.h`, `corecrt_startup.h`'s `_crt_argv_mode`, `conio.h` /
`corecrt_wconio.h`'s `__conio_common_vcprintf`, `wincontypes.h`,
`winuser.h`'s virtual-key codes, `consoleapi.h`, `processthreadsapi.h`,
`winbase.h`, `winnt.h`, `ntstatus.h`); the PC BIOS keyboard service's
extended key codes (IBM Personal Computer AT Technical Reference). Export
names from the coverage map. No Windows binary, UCRT source, Wine or
ReactOS source was looked at; there is no oracle yet, so what Learn
leaves open is a labelled choice.

## Decision

### Logic in `poly-crt`, the process side in the shared runtime

| Where | What |
|---|---|
| `poly_crt::find` | `WIN32_FIND_DATAW` → the eight `_finddata*_t` layouts (times 32/64-bit, size 32/64-bit, narrow/wide names) |
| `poly_crt::wildcard` | expanding `argv` from `args::parse_marked` (which now says whether a quote delimited an argument) |
| `poly_crt::spawn` | the command line, the file search, the interpreter's line, `_popen` modes, environment blocks, the descriptor block |
| `poly_crt::conio` | which console key events `_getch` returns, as what; `_cgets`' line |
| `poly_kernelbase::pipe` | `CreatePipe`'s pipe names and access |
| `crt::find`, `crt::spawn`, `crt::conio` | the functions, over `kernel32` |
| `ucrtbase` / `msvcrt` `files_console.rs` | what only one DLL exports |

Variadic functions both DLLs export (`_spawnl*`, `_execl*`) are thunks in
the shared runtime (`poly_win32_crt::va_thunk!`, the same assembly as
`msvcrt`'s, listed in `poly_crt::exports::SHARED_VARARGS` for both
`build.rs`); `msvcrt`'s `_cprintf` family in `MSVCRT_CONIO_VARARGS`.

### `_findfirst` / `_findnext`

`FindFirstFileW` / `FindNextFileW` / `FindClose`; the search handle is
`FindFirstFileW`'s. A failure is `ENOENT` when nothing (more) matched,
`EINVAL` for other errors (Learn). `msvcrt`'s `_findfirst` / `_findnext`
use `_finddata64i32_t` and its `i64` forms `__finddata64_t`, as its
`time_t` is 64-bit (ADR 0084). Poly's choices: `attrib` keeps Learn's
`_A_*` bits; a zero `FILETIME` is −1 ("-1L for FAT"), as is a time a
32-bit field cannot hold; a 32-bit size keeps its low bits.

### `_dup`, `_dup2`, `_pipe`, and anonymous pipes

`_dup` / `_dup2` duplicate the handle (`DuplicateHandle`) so both
descriptors share the file and its pointer; `_dup2` closes what the
target had and grows the table. `_pipe` is `CreatePipe`, in the given
translation (`_fmode`'s with none); `_O_NOINHERIT` marks both ends.
Descriptors now remember `_O_NOINHERIT` (`_open`, `_open_osfhandle`,
`_pipe`).

`CreatePipe` (new in `kernelbase`, forwarded by `kernel32` through
`api-ms-win-core-namedpipe-l1-1-0`): Learn says anonymous pipes are named
pipes with unique names; Poly has no named pipes, so it opens a uniquely
named `\Device\PolyPipe\Anonymous.<pid>.<n>` twice, the read end with
read access and the write end with write access, inheritable as the
security attributes say. **The pipe device gains an end of stream**: a
stream counts its opens with write access; once there was one and the
last closes, the reads waiting fail with `STATUS_PIPE_BROKEN` and so does
a read of the empty stream (`ERROR_BROKEN_PIPE`, which the runtime reads
as the end). Data written before stays readable. `nSize` is not used (the
stream grows; a write never waits: Poly's choice).

### `_spawn*`, `_exec*`, `_cwait`, `system`, `_popen`

- **Command line**: the arguments joined by single spaces, unquoted (Learn:
  "hi there" becomes two arguments). Longer than `CreateProcessW`'s 32,767
  characters: `E2BIG` (Learn's `_spawn` page names 1,024 bytes; to be
  confirmed).
- **File**: a name with an extension as it is, ending in `.` without it,
  else `.com`, `.exe`, `.bat`, `.cmd` (Learn); with a drive or slash only
  there, else the current directory, and for the `p` forms each `PATH`
  directory. A `.bat` / `.cmd` runs through the interpreter. Missing:
  `ENOENT`. An unknown mode, a NULL name or argument list, an empty
  `argv[0]`: the invalid parameter handler and `EINVAL`.
- **Descriptors**: every descriptor not marked `_O_NOINHERIT` is
  duplicated as an inheritable handle for the call and passed in
  `STARTUPINFO.lpReserved2`, and 0, 1, 2 also as the standard handles
  (`STARTF_USESTDHANDLES`), so a child not on this runtime writes where
  `_dup2` pointed descriptor 1 (Poly's choice); `bInheritHandles` is TRUE;
  the copies are closed after. **The block's layout is Poly's own**
  (Learn gives none): a `u32` count, one flag byte per descriptor (open;
  translation; append), then an unaligned 64-bit handle each. A child's
  runtime builds its table from it at start-up (`lowio::init`), filling 0,
  1, 2 from its standard handles where the block has none.
- **Modes**: `_P_WAIT` returns the exit code; `_P_NOWAIT` / `_P_NOWAITO` the
  process handle (Learn), which `_cwait` waits for, reads the code of and
  **closes** (Poly's choice); `_P_DETACH` starts it with `DETACHED_PROCESS`
  (accepted, not acted on, ADR 0089) and returns 0; `_P_OVERLAY` and
  `_exec*` start the child and end this process at once with exit code 0,
  without `atexit` functions or writing streams (Learn: flush first).
- **The command interpreter** (`system`, `_popen`): `COMSPEC` if it names a
  file, else `cmd.exe` in a `PATH` directory (Learn), run as `<interpreter>
  /c <command>` (the interpreter's path quoted if it has a blank). **Poly
  has no `cmd.exe`**, so by default `system(NULL)` is 0 with `ENOENT` and
  `system`, `_popen` and batch files fail with `ENOENT` — what Learn
  documents for a missing interpreter. A program (or a future `cmd.exe`)
  named by `COMSPEC` works as one; the ring-3 test uses its own program.
- **`_popen`**: a pipe; the child gets this process's standard handles but
  the pipe's end for the stream it reads or writes, and no other
  descriptors; our end is a `_O_NOINHERIT` descriptor under a stream.
  `_pclose` closes the stream, waits, and returns the exit code; a stream
  `_popen` did not make is `EINVAL`.

### Wildcard `argv`

`_configure_narrow_argv` / `_configure_wide_argv` with
`_crt_argv_expanded_arguments` (what MSVC's start-up asks when the
program links `setargv.obj` / `wsetargv.obj`), and `msvcrt`'s
`__getmainargs` / `__wgetmainargs` with `DoWildCard` non-zero, expand
each argument with `*` or `?` to the names `FindFirstFileW` gives for it.
Poly's choices: `argv[0]` and an argument a double quote delimited part
of are not expanded; each name keeps the argument's directory part; `.`
and `..` are left out; the order is the directory's; no match leaves the
argument.

### Console I/O

Straight to the console (`CONIN$` / `CONOUT$`, opened once), whatever the
standard handles are:

- `_getch` reads key presses with `ReadConsoleInputW` in a mode with only
  `ENABLE_PROCESSED_INPUT` (Ctrl+C stays a signal, as Learn says it cannot
  be read), restoring the mode. A press with a character gives it (the
  console input code page's bytes for the narrow form, the UTF-16 unit for
  `_getwch`, a repeat count giving it that often); a function or cursor key
  two calls, prefix 0xE0 for the separate cursor block (`ENHANCED_KEY`)
  and 0 otherwise, then the BIOS code by virtual key (F1–F12 and their
  Shift / Ctrl / Alt forms, the cursor keys and their Ctrl / Alt forms),
  since Poly's console leaves the scan code 0. Releases and modifier keys
  give nothing.
- `_getche` echoes characters (not a function key's prefix and code);
  `_kbhit` drops the records `_getch` would skip at the front (so they do
  not hide a later key) and says whether a key waits; `_ungetch` keeps one
  character per width.
- `_putch`, `_cputs`, `_cprintf` (`msvcrt`, `MSVCRT_OPTIONS`) and
  `__conio_common_vcprintf` (`ucrtbase`, the caller's options; the `_s`
  forms the same) write with `WriteConsoleA`, the wide forms with
  `WriteConsoleW`; nothing translates `\n` (Learn).
- `_cgets` / `_cgets_s` / `_cgetws` / `_cgetws_s` read a cooked, echoed
  line with `ReadConsoleW` (line, echo and processed input for the call);
  a longer line leaves its rest for the next read.
- The `_nolock` forms (`ucrtbase`) skip the runtime's console lock.

## To be confirmed by the spec agent against real Windows

1. The `lpReserved2` layout the runtime writes and reads (Poly's own here),
   so programs with a statically linked runtime and ours pass descriptors
   both ways; whether a spawned child's standard handles follow descriptors
   0–2 (`STARTF_USESTDHANDLES`).
2. `_P_OVERLAY` / `_exec*`: the parent's exit code and whether it waits;
   `_P_DETACH`'s return value; whether `_cwait` closes the handle; the
   `E2BIG` limit.
3. The extension order without one and whether the bare name is tried;
   batch files without an interpreter (`ENOENT`).
4. Wildcard expansion: quoted arguments, `.`/`..`, ordering (sorted or the
   directory's), hidden files, the directory prefix.
5. `_findfirst`'s `attrib` mask, a zero `FILETIME`, the 32-bit size.
6. `_getch`'s codes for modified keys, keypad keys and Alt+letter; `_kbhit`
   dropping records; `_getche` echoing extended keys; `_ungetch` shared
   between the widths or not.
7. Anonymous pipes: writing when every reader is gone (Poly: buffered),
   `nSize`, a full pipe blocking the writer.

## Consequences

- Host tests: `poly-crt` (`find`: layouts and offsets of all eight
  structures; `wildcard`; `spawn`: joining, extensions, `PATH` order, batch
  files, the interpreter's line, environment blocks, `_popen` modes, the
  descriptor block's round trip; `conio`: characters, releases, function
  and cursor keys with modifiers, records, lines; `args`: quote marks),
  `poly-kernelbase` (`pipe`), `poly-personality-nt` (a pipe's reads end
  when its last writer closes, data written before still read).
- The ring-3 test `nt::test_crtio::nt_c_runtime_files_processes_and_console`
  runs the new `crtiotest.exe` from `C:\Program Files\PolyCrtIoTest` with
  a file as standard output: about 100 checks — `_findfirst`/`_findnext`
  in four layouts and `msvcrt`'s; `_dup`/`_dup2` (one file pointer,
  stdout to a file and back), `_pipe` in binary and text with its end of
  stream; `_spawnl`/`_spawnv`/`_wspawnv`/`_spawnle` (exit codes, joined
  arguments, only the given environment), descriptors inherited and
  `_O_NOINHERIT` ones not, a child's stdout through `_dup2`, `_P_NOWAIT`
  with `_cwait`, `_spawnlp` on `PATH`, `ENOENT`/`EINVAL`, `_execl` from a
  child; `system` without an interpreter and with itself as `COMSPEC`;
  `_popen` reading and writing with `_pclose`'s exit codes; wildcard
  `argv` in a child (narrow, wide, `__getmainargs`); console keys written
  into its own input buffer for `_kbhit`/`_getch` (characters, cursor and
  function keys), `_ungetch`, `_getwch`, `_getche`, output measured by the
  cursor (`_putch`, `_putwch`, `_cputs`, `_cputws`, `__conio_common_vc*printf`,
  `msvcrt`'s `_cprintf`), and `_cgets`/`_cgets_s`/`_cgetws`. The kernel test
  checks a child's line reached the inherited standard output.
- Shared files changed additively: the Win32 workspace members, `xtask`'s
  `win32.rs` outputs and inputs, `kernel/build.rs`'s embed list, a static
  and a module in `kernel/src/nt/mod.rs`, `KERNEL32_FORWARDS`, both DLLs'
  `build.rs`, `poly_crt::exports`, the pipe record in `personalities/nt/src/io`,
  `lowio::Fd` (`noinherit`), `startup::main_args` (a `wildcards` argument).
- Coverage: the exports above in `ucrtbase.dll` and `msvcrt.dll`, and
  `CreatePipe` in `kernel32.dll` / `kernelbase.dll`, claimed by
  `impl-crt-io`.
- Left: `_cscanf` / `__conio_common_vcscanf`, the positional
  `__conio_common_vc*printf_p` (with positional `printf`), named pipes and
  `PeekNamedPipe`, a `cmd.exe`.
