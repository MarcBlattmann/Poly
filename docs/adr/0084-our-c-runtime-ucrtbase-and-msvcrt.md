# 0084: Our C runtime: `ucrtbase.dll` behind the `api-ms-win-crt-*` API sets and `msvcrt.dll`, one shared runtime on `kernel32`, the formatting and conversions in a host-tested crate

**Status:** Accepted (first slice). Builds on [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(apps bring their versioned runtimes, `vcruntime140`/`msvcp140`; the system
runtimes are Poly's), [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
(PE DLLs from Rust, the loader's API-set table: `api-ms-win-crt-*` →
`ucrtbase.dll`, both CRTs known DLLs), [ADR 0079](0079-our-kernelbase-and-kernel32.md)
(the Win32 workspace, its content-hash cache, `kernel32` below us) and
[ADR 0080](0080-console-host.md) (standard handles, consoles).

## Context

The Phase 5 item is "C runtime: `ucrtbase` + `api-ms-win-crt-*` and
`msvcrt`", on the way to the 7-Zip milestone (`7z.exe`, an MSVC-built
console program). Since Visual Studio 2015 a program built with `/MD`
imports the Universal C Runtime through API-set names
(`api-ms-win-crt-runtime-l1-1-0.dll`, `…-stdio-…`, `…-heap-…`, `…-string-…`,
`…-convert-…`, `…-time-…`, `…-locale-…`, `…-math-…`, `…-environment-…`,
`…-filesystem-…`, `…-utility-…`), which Windows resolves to the system's
`ucrtbase.dll`; `vcruntime140.dll` (exceptions, `memcpy`, `__std_terminate`
…) comes with the app (ADR 0055). Programs built with older compilers or
the driver kits import the system `msvcrt.dll` instead, which has a
different start-up interface (`__getmainargs`), exported data (`_iob`,
`_environ`) and exported variadic functions (`printf`), where the UCRT's
headers format inline through `__stdio_common_vfprintf` and friends.

Sources (clean room, CLEAN_ROOM.md): the C standard (C11/C17: 7.4, 7.11,
7.12, 7.21, 7.22, 7.24, 7.27, 7.29, 7.30, annex F); Microsoft Learn's C
runtime reference: each function's page, "Format specification syntax:
printf and wprintf functions" (flags, size prefixes `I`/`I32`/`I64`/`w`, the
`%Z` "known issue" of the UCRT, infinity and NaN formatting, "Starting in
Windows 10 version 2004 … round to the closest even digit"), "scanf Width
Specification", "Parsing C command-line arguments" (its table of
examples), "Text and binary mode file I/O", "errno constants", "CRT
initialization", "Parameter validation", "x64 calling convention"
(varargs, home space), `_set_output_format` (three-digit exponents);
the layouts and constants of the public SDK headers
(`corecrt_stdio_config.h`'s `_CRT_INTERNAL_PRINTF_*`/`_SCANF_*` bits,
`corecrt_startup.h`'s `_onexit_table_t` and `_crt_argv_mode`, `stdio.h`'s
classic `_iobuf`, `fcntl.h`, `share.h`, `sys/stat.h`, `sys/timeb.h`,
`errno.h`, `ctype.h`, `float.h`, `locale.h`'s `lconv`, `math.h`'s `FP_*`,
`signal.h`, `time.h`, `ntstatus.h`, `winnt.h`); the Intel SDM for `MXCSR`;
Wikipedia's "Linear congruential generator" table for `rand`'s published
parameters. Export *names* and which DLL has which come from the coverage
map (Wine's `.spec` name lists). No Windows binary, UCRT source, Wine,
ReactOS or MinGW-w64 CRT source was looked at. There is still no Windows
oracle VM, so what Learn leaves open is a labelled guess (below).

## Decision

### Four crates in the Win32 workspace, the logic in a fifth on the host

| Crate | Output | What |
|---|---|---|
| `crates/poly-crt` (host workspace) | rlib, host-tested | the `printf` and `scanf` engines, exact float conversions, `strtol`/`strtod`, command lines, ctype, calendar time and `strftime`, `qsort`, `errno` values and texts, `fopen` modes and `_open` → `CreateFileW` arguments, text-mode translation, the `_s` rules, `MXCSR` ↔ control word, exact `math.h` functions, the export lists |
| `personalities/nt/win32/crt` (`poly-win32-crt`) | rlib | the process side shared by both DLLs: heap, descriptors, streams, arguments and environment, start-up and exit, signals, threads, and every function both DLLs export alike |
| `ucrtbase` | `ucrtbase.dll` | what only the UCRT exports: `__stdio_common_*`, `__acrt_iob_func`, the `__p_*` accessors, `corecrt_startup.h`'s functions, the handlers' setters, the C99 additions |
| `msvcrt` | `msvcrt.dll` | what only `msvcrt` exports: its data, `__getmainargs`, `_XcptFilter`, `__dllonexit`, the classic formatted I/O, `time`/`gmtime`… with a 64-bit `time_t`, the C++-named `terminate` |
| `crttest` | `crttest.exe` | the program the ring-3 test runs |

**Exports.** A `#[no_mangle]` item of an rlib is exported by every cdylib
that links it, so `poly-win32-crt` marks exactly the functions both DLLs
export with the same behaviour (about 400, the intersection of their
coverage lists); a function only one DLL exports is a plain Rust function
there, wrapped by that DLL. `memcpy`, `memmove`, `memset` and `memcmp` are
`compiler_builtins`' own, exported by each `build.rs` (`/EXPORT:`).
`cargo xtask win32` checks every export of each DLL is a name the coverage
map lists for that DLL (so nothing of one leaks into the other) and that
the start-up functions (`poly_crt::exports::UCRT_STARTUP`,
`MSVCRT_STARTUP`) are there.

**Imports.** The runtime calls `kernel32.dll` only (`sys.rs`, raw-dylib),
as the real one sits on `kernel32`'s API sets.

**Data.** The program-visible variables (`__argc`, `__argv`, `_environ`,
`_fmode`, `_acmdln`, `_pgmptr` …) are *cells*: the runtime's own statics,
which the UCRT hands out through `__p___argc` and friends, or — bound at
`msvcrt`'s `DLL_PROCESS_ATTACH` — `msvcrt`'s exported data symbols, so an
old program writing `_fmode` changes what `_open` uses. The same for the
standard streams' `FILE`s: `msvcrt`'s exported `_iob[20]`.

### `printf` and `scanf`

One engine for both widths (`poly_crt::printf::format<C>`, a `Unit` trait
over `u8`/`u16`), reading a Windows x64 `va_list` as 8-byte slots. It does
what C11 and Learn's "Format specification syntax" say, with the UCRT's
option bits from `corecrt_stdio_config.h`:

- `_CRT_INTERNAL_PRINTF_LEGACY_WIDE_SPECIFIERS`: `%s`/`%c` have the
  function's width, `%S`/`%C` the other; `%hs`/`%ls`/`%ws` are explicit.
- `%Z`: `ANSI_STRING`/`UNICODE_STRING` (`Length` in bytes, `Buffer` at
  + 8); without a prefix the UCRT's documented "known issue" (like `%S`),
  `%hZ` ANSI, `%wZ`/`%lZ` Unicode; `msvcrt`: ANSI.
- `inf`, `nan`, `nan(snan)`, and `nan(ind)` for the indefinite NaN (the
  default NaN, sign set); capitals for `E F G A`. `msvcrt`
  (`LEGACY_MSVCRT_COMPATIBILITY`): `1.#INF`, `1.#QNAN`, `1.#SNAN`, `1.#IND`.
- Exponents of two digits, three with `LEGACY_THREE_DIGIT_EXPONENTS`
  (`msvcrt`'s default, Learn: `_set_output_format`).
- Every decimal digit is exact: a `double` is `m × 2^e`, `poly_crt::float`
  expands it with a small big-integer type and rounds the digits; an exact
  tie goes to even with `_CRT_INTERNAL_PRINTF_STANDARD_ROUNDING`, away from
  zero without it (Learn's note on Windows 10 2004 / VS 2019 16.2).
- `%a`: 13 hexadecimal digits by default (Learn). `%n` only after
  `_set_printf_count_output(1)`, else the invalid parameter handler.
- The buffer rules of `__stdio_common_vsprintf` by option (standard
  `snprintf`: cut, terminated, full length returned; legacy `_snprintf`:
  unterminated when exactly full, −1 when longer), `_s` (the handler and
  `ERANGE` when it does not fit), `_snprintf_s` with `_TRUNCATE`
  (`poly_crt::printf::finish_*`, host-tested).

`scanf` (`poly_crt::scanf`) has the same prefixes, scansets with ranges,
`%n`, suppression, widths, and the secure forms' buffer sizes; its floats
go through the same correctly rounded parser as `strtod`.

`msvcrt.dll`'s variadic exports (`printf`, `sprintf`, `_snprintf`,
`swprintf`, `sscanf` …, `poly_crt::exports::MSVCRT_VARARGS`) are assembly
thunks with unwind data: spill RCX, RDX, R8, R9 into their home slots so
all arguments are one array (Learn, "x64 calling convention": varargs are
passed as unprototyped, floats also in the integer registers), point a
`va_list` at the first variadic slot, and call the `va_list` form.

### Conversions

`strtod`/`strtof`/`wcstod`/`atof` (`poly_crt::float::parse`): C11's
grammar (decimal, `0x` hexadecimal with `p` exponent, `INF`, `INFINITY`,
`NAN`, `NAN(chars)`), correctly rounded to nearest-even for any number of
digits (the first 800 significant digits exactly, a sticky digit for the
rest, which decides every tie); `ERANGE` on overflow (±`HUGE_VAL`) and on
an inexact zero or subnormal result. `strtol` and family: C11 7.22.1.4 with
the negation "in the return type" for the unsigned forms; `atoi` clamps
with `ERANGE` (Learn). `_itoa` and family: a sign only in radix 10.

### Streams and descriptors

- **Descriptors** (`lowio`): a table of handles with a translation mode;
  0–2 are the standard handles at start-up, a missing one is the handle −2
  on which I/O fails with `EBADF` (Learn, `_get_osfhandle`). Text mode
  writes `\n` as `\r\n`, reads `\r\n` as `\n` and stops at Ctrl+Z; the
  Unicode modes (`_O_WTEXT`, `_O_U16TEXT`, `_O_U8TEXT`) take UTF-16 units,
  written with `WriteConsoleW` to a console and as UTF-16LE or UTF-8 bytes
  elsewhere. `_open` maps its flags to `CreateFileW` (`poly_crt::fmode`).
- **Streams** (`stdio`): a `FILE` is the classic 48-byte `_iobuf` of the
  public `stdio.h` (so `msvcrt`'s `_iob` has the stride old programs
  expect; the UCRT's `FILE` is opaque); Poly keeps `_file` current and the
  stream's own state beside it. Each stream has a reentrant lock
  (`CRITICAL_SECTION`, for `_lock_file`). Reading keeps the raw bytes and
  translates text as it hands them out, so `ftell`/`fseek` are exact in
  text mode; switching between reading and writing moves the descriptor
  back over what was read ahead.
- **Buffering** (Poly's choice): files and pipes fully buffered (4 KiB);
  `stdout` on a console written at the end of each call; `stderr`
  unbuffered; `_IOLBF` is `_IOFBF` (Learn).
- Wide output in text mode is the "C" locale's single bytes (`EILSEQ` for
  other characters, as `wctomb` in that locale); a narrow function on a
  Unicode-mode stream is an invalid parameter (Learn, `_setmode`).

### Start-up and exit

`_configure_narrow_argv`/`_configure_wide_argv` parse `GetCommandLineA/W`
with Learn's rules (its table of examples is a host test);
`_initialize_narrow_environment` converts the environment block (ANSI code
page) leaving out the `=X:` entries; `_initterm` and `_initterm_e` call the
program's tables. `exit` calls the thread-local destructor callback
(`_register_thread_local_exe_atexit_callback`), the `atexit`/`_onexit`/
`_crt_atexit` functions last first, writes every stream, and ends the
process; `_cexit` does the same without ending it; `_exit` ends at once;
`_execute_onexit_table` runs a module's table last first. Each DLL's
`DLL_PROCESS_DETACH` writes its streams as well (a program that ends with
`ExitProcess`).

### Errors, handlers, signals

`errno` and `_doserrno` are per thread (TLS, made on first use, freed at
`DLL_THREAD_DETACH`; the last error is kept around `TlsGetValue`). A
failed `kernel32` call sets `_doserrno` and maps it to `errno`. The invalid
parameter handler is the thread's, else the process's, else the default,
which fails fast (`FAST_FAIL_INVALID_ARG`); if a handler returns, the
function fails with its documented `errno`. `signal`/`raise` keep a table
(SIGINT/SIGBREAK through `SetConsoleCtrlHandler`); a handler is reset to
`SIG_DFL` before it runs (but `SIGFPE`); `SIG_DFL` raised ends the process
with code 3, as does `abort` after `SIGABRT`'s handler. `_seh_filter_exe`
and `_XcptFilter` turn access violations, illegal instructions and float
exceptions (`ntstatus.h`) into `SIGSEGV`/`SIGILL`/`SIGFPE` when the program
installed a handler, else continue the search. `terminate` calls the
`set_terminate` handler then `abort`; `_purecall` the pure-call handler
then `abort`.

### The rest of this slice

The heap is the process heap (`HeapAlloc`, 16-byte aligned, as
`_get_heap_handle` says), `malloc(0)` a valid block, `realloc(p, 0)` frees,
`_set_new_mode(1)` makes `malloc` call the new handler; `_aligned_*` keep
the heap block's address before the aligned one. Time: `time_t` ↔ `tm`
proleptic Gregorian from 1970 to 3000-12-31 23:59:59 (`_gmtime64`'s
documented range), local time is UTC (ADR 0079), `strftime` in the "C"
locale with the `#` flag, `asctime` with a two-digit day as Learn's example.
Locale: only "C" (`setlocale` answers "C" for NULL, "C", "POSIX" and "",
NULL for any other name; `localeconv` gives C11's "C" `lconv`). Character
classes: the "C" locale's `_pctype` table (`ctype.h` bits, `EOF` entry
first); wide classes and case for Latin-1. `qsort` is an introsort,
`bsearch` a binary search. `_controlfp`/`_controlfp_s`/`_clearfp`/
`_statusfp`/`_fpreset` work on `MXCSR`. `math.h`: the exact functions
(`floor`, `ceil`, `trunc`, `round`, `fmod`, `modf`, `frexp`, `ldexp`,
`sqrt` by `sqrtsd`, `copysign`, `_dclass`). `rand` is the published
linear congruential generator, its seed per thread.

## To be confirmed by the spec agent against real Windows

1. The "C" locale's `LC_CTYPE`: Poly reads it as ISO-8859-1 (a byte is the
   code point of the same value, a wide character above U+00FF has no
   narrow form, so `printf("%ls")` of one fails with `EILSEQ`), `mbtowc`
   accordingly; `___lc_codepage_func()` 0; wide classes above U+00FF (none
   in Poly); `towupper` of Latin-1 letters; `isblank`/`_BLANK` of tab.
2. `setlocale(LC_ALL, "")` (Poly: stays "C" and answers "C", Windows names
   the user's locale, e.g. "English_United States.1252").
3. `printf`: `%p` as 16 upper-case digits; `%Lx` as 64-bit; `%C`/`%S` in
   the ISO (non-legacy) wide mode (Poly: wide); `%.0a` rounding to a
   leading `2`; the legacy `msvcrt` texts of infinities and NaNs cut or
   padded to the precision, its 17 significant digits followed by zeros,
   and an unknown conversion printed as its letter; `__stdio_common_vsprintf`
   with neither buffer option when the output does not fit (Poly: cut,
   terminated, −1).
4. `strtod`'s `ERANGE` on underflow (Poly: an inexact zero or subnormal
   result); `scanf`: an integer that overflows keeps its low bits, a
   secure buffer too small stops the scan with an empty string, `"1.25e"`
   consumes the `e`.
5. `strerror` texts (Poly: Learn's "errno constants" descriptions) and the
   Win32 error → `errno` table (`poly_crt::errno::from_win32`).
6. `_environ` without the `=X:` entries; an empty `argv[0]` for a command
   line starting with a blank; `_wenviron`/`_environ` built lazily by
   `getenv`/`_wgetenv` (Poly) or not.
7. Buffering of `stdout` on a console (Poly: written at the end of each
   call) and on a file (4 KiB); `_isatty`'s non-zero value (Poly: 0x40).
8. `abort`'s exit code and whether it reports (Poly: code 3, no report);
   `_amsg_exit` (Poly: exit 255, no message); `_purecall` without a handler.
9. `strftime`'s `%c`/`%x`/`%X` in the "C" locale (Poly: `%m/%d/%y
   %H:%M:%S`), `%Z` (Poly: "UTC") and the time zone variables
   (`_dstbias` −3600); `clock` from process start.
10. `rand`'s sequence (Poly: the published LCG, a = 214013, c = 2531011,
    bits 30..16) and its seed being per thread.
11. The `api-ms-win-crt-*` contract of each function (`crttest` imports
    each from the contract Poly believes holds it; our loader sends them all
    to `ucrtbase`, so a wrong guess costs nothing at run time).

## Consequences

- Host tests: `cargo test -p poly-crt` (57 tests: integers, pointers,
  strings, `%Z`, the wide functions and both specifier modes, fixed /
  exponent / general / hexadecimal floats, tie rounding by option,
  infinities and NaNs both ways, the legacy runtime, `%n`, the buffer
  rules; exact expansions, correctly rounded decimal and hexadecimal
  parsing in double and single precision; `strtol` bases, overflow and
  negation, `atoi`, `_itoa`; Learn's command-line examples; the ctype
  tables; calendar conversion, `mktime` normalisation, `asctime`,
  `strftime` with ISO weeks; `qsort` on many shapes and element sizes,
  heapsort, `bsearch`; `fopen` modes, `_open` arguments, text
  translation; the `_s` rules; `MXCSR`; the exact math functions against
  the standard library).
- The ring-3 test `nt::test_crt` runs `crttest.exe` from
  `C:\Program Files\PolyCrtTest` with a file as standard output: about 120
  checks (start-up as MSVC's code does it, arguments with quotes, the
  environment, the heap, strings and the `_s` handler path, `printf`/
  `swprintf`/`sscanf` through `__stdio_common_*`, conversions, files in
  text and binary mode, `ftell`/`fseek`/`ungetc`, descriptors, `_wfopen`,
  time, `qsort`/`bsearch`, locale, signals, a thread's own `errno`,
  `msvcrt.dll`'s variadic `sprintf`/`_snprintf`/`swprintf`/`sscanf`/
  `fprintf`, `_iob` and `__getmainargs`), then `exit` with an `atexit`
  order check; the test checks the lines on standard output (`\r\n`) and
  that `exit` wrote a stream left open.
- Build: the Win32 build makes three more files (`ucrtbase.dll` ~165 KB,
  `msvcrt.dll` ~165 KB, `crttest.exe` ~17 KB); the kernel embeds them and
  the layout writes both DLLs to `System32`.
- Shared files changed (additively): the Win32 workspace members,
  `xtask`'s `win32.rs` (outputs, inputs, `verify_crt`), `kernel/build.rs`'s
  embed list, three statics in `kernel/src/nt/mod.rs`, the layout's install
  loop, the host workspace members.
- The coverage entries of every export of both DLLs are claimed by `crt`.
- **Left** for the next slices: C++ exception support in `msvcrt`
  (`_CxxThrowException`, `__CxxFrameHandler3`, `__C_specific_handler`'s
  CRT side) and `setjmp`/`longjmp`; the transcendental `math.h` functions
  (`exp`, `log`, `pow`, `sin` …) and `_matherr`; locales other than "C"
  and multibyte code pages (`_mbs*`); wildcard expansion of `argv`;
  `_findfirst`/`_findnext`, `_spawn*`/`_exec*`/`system`, `_dup`/`_dup2`/
  `_pipe`, `tmpfile`/`tmpnam`; console I/O (`_getch`, `_cprintf`, `_cputs`);
  positional `printf` (`_printf_p`); `_beginthread`'s handle closing; the
  rarer exports in the coverage map.
