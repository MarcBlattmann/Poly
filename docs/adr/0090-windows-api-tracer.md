# 0090: The Windows API tracer: proxy DLLs generated from a program's imports, an instrumented copy that loads them, one JSON line per call, the same kit on Windows and on Poly, and a per-thread comparison

**Status:** Accepted. Amended by
[0110](0110-7-zip-traced-scenario-and-what-passing-its-trace-means.md): the
traced 7-Zip scenario's Windows trace is committed in canonical form, the
`wintrace` boot group runs that scenario instead of the three runs below,
and the comparison gained documented machine normalisations. Roadmap Phase
5's "Windows API tracer" (not ticked:
the milestone it serves, 7-Zip's recorded trace, stays open). Point 3 of
[ADR 0087](0087-7-zip-command-line-on-poly.md)'s "The recording on
Windows". Builds on [ADR 0075](0075-windows-test-oracle.md) (how Windows
programs are built here: `no_std` Rust, `raw-dylib`, `-Zbuild-std`,
`rust-lld`) and [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
(our loader, which binds the proxies on Poly).

## Context

ADR 0087 lists what a recording of 7-Zip on the oracle machine needs; the
API trace is the part nobody had built: the calls `7za.exe` makes into
`kernel32`, `advapi32`, `msvcrt`, `oleaut32` and `user32`, with arguments,
results and last errors, in order, per thread, taken the same way on
Windows and on Poly so that the two can be compared.

Constraints: nothing may be installed on the Windows machine (no
debugger, no Detours, no compiler); a debugger dialog must never appear;
the kit must also run on Poly unchanged, so it can only use what Poly's
loader and DLLs do for 7-Zip itself; and nothing inside a Windows DLL may
be looked at (CLEAN_ROOM.md): the tracer reads the *program's* import
names and observes calls, nothing else.

The owner directed the Windows runs to his own home machine
(`homeserver2`: Windows 11 Home build 26200, 6 CPUs, 8 GB, Defender only,
ANSI code page 1252, OEM 850, W. Europe Standard Time) instead of the
corporate PC, inside `C:\Users\server.HOMESERVER2\poly-oracle-work\`.

## Decision

### Proxy DLLs and an instrumented copy (no hooking at run time)

`cargo xtask win trace-kit <exe | 7zip> [--out <dir>] [--data <name>]...`
(`tools/xtask/src/wintrace.rs`) reads the program's import and delay-load
directories (`winimports::imports_of`, names and ordinals only) and writes
a self-contained directory (default `target/wintrace/kit`):

| File | What |
|---|---|
| `<program>.exe` | the program with only its import DLL names changed |
| `pt_<dll>.dll` | one proxy per imported DLL |
| `polytrace.dll` | the logger, which holds the one trace of the process |
| `polytrace-kit.json` | `polytrace-kit/1`: the program's SHA-256, each file's, every proxy's functions with signature and flags, the forwarded variables |

**The copy.** A new section `.ptname` (initialised, read-only data) is
appended with the new names (`pt_kernel32.dll` ...); each import (and
delay-load) descriptor's name field points there. Bound imports are
dropped (the directory and the descriptors' time stamps), as is an
Authenticode signature (invalid anyway), and the PE checksum is
recomputed. Code, data, the IAT and every other byte stay as they were,
so the program runs exactly its own code. Chosen over hooking the IAT at
run time (needs code injected before the program starts, which Windows
does only through a debugger, `CreateRemoteThread` or AppInit-style
registry settings) and over writing the names in place (`kernel32.dll` is
shorter than `pt_kernel32.dll`).

**A proxy** exports exactly what the program imports from the real DLL,
by the same names (C++-mangled ones included) and ordinals (`oleaut32` by
ordinal, `NONAME`), nothing else (its entry point and `_fltused` are
assembly symbols, not exports). Each function export is a two-instruction
thunk (`lea r10, [rip + record]; jmp pt_common`). The record (`Func`:
the real function's IAT slot, DLL name, function name or `#ordinal`,
signature, flags, stack slots) is written by the generator in assembly.
The real functions are **the proxy's own imports** (`raw-dylib` items),
bound by the loader as it binds the program's: no `LoadLibrary` in
`DllMain`, no `GetProcAddress` by ordinal, and on Poly nothing beyond
what 7-Zip itself needs. Exported variables (`msvcrt`'s `_iob`,
`__initenv`, `_commode`, `_fmode`; the list `DATA` is Learn's "Global
variables" plus our `msvcrt`'s statics, `--data` adds names) are **PE
forwarders** to the real DLL, so the program gets the real objects.

The proxies are Rust `no_std` cdylibs generated into
`target/wintrace/build` (one workspace with `polytrace`; `/EXPORT:` link
arguments from each `build.rs`) and built like ADR 0075's test programs
(`-Zbuild-std=core,compiler_builtins`, `rust-lld`, `/NODEFAULTLIB`,
`/Brepro`, offline). Emitting PE files directly with `poly-pe` was the
alternative; Rust plus `global_asm!` gave relocations, imports, exports,
forwarders and unwind data from the linker for free. 7-Zip's kit: the
logger 10 KB, proxies 4 to 17 KB, built in about 20 s the first time.

**The thunk** (`tools/wintrace/proxy/common.rs`, `pt_common`): one push
and a fixed 0x920-byte frame, described by `.seh_*` unwind codes so an
exception or `RtlVirtualUnwind` can pass through it. It saves the four
integer and four XMM argument registers, copies the stack arguments (the
signature's count beyond four, 16 at most, never past `NT_TIB.StackBase`)
to the bottom of its frame where the real function expects them, calls
`pt_enter`, restores the registers, calls the real function through its
IAT slot, keeps `rax`/`xmm0` across `pt_leave` and returns them.
Non-volatile registers are not touched. Copying the arguments (rather
than popping the return address and keeping it per thread) keeps a normal
frame: no return-address stack to get out of step when an exception
unwinds through, and nothing for `longjmp` to break. Functions that must
see their caller's own frame (`_CxxThrowException`, `setjmp`/`longjmp`,
the language-specific handlers `__C_specific_handler` and
`__CxxFrameHandler*`, `RtlUnwind*`, `RtlCaptureContext`: list `TAIL`)
take the frame down and *jump* to the real function, logged when they
begin. Functions that do not return (`ExitProcess`, `exit`, `_exit`,
`abort`, `?terminate@@YAXXZ`, ...: `NORETURN`) are logged when they begin
and write the buffer out; `TerminateProcess` writes it out first.

### The logger and the trace format

`polytrace.dll` (`tools/wintrace/polytrace`) opens `polytrace.jsonl` in
the current directory at `DLL_PROCESS_ATTACH` (`POLYTRACE_OUT` names
another file) and writes, after a `{"format":"polytrace/1"}` line, one
object per call:

```text
{"seq":2552,"t":0,"dll":"kernel32.dll","fn":"FindFirstFileW","args":[{"w":"new.7z"},"0xde4c7fe9e0"],"ret":"0xffffffffffffffff","err":2}
```

- `seq`: the order calls *began* in, over the process. A record is
  written when its call returns, so a nested call (a callback that calls
  an import, `_initterm`'s constructors) comes first in the file; readers
  sort by `seq`.
- `t`: a per-process thread number in order of first traced call (the
  thread id differs between runs and systems).
- `args`, `ret`: by the function's signature (`SIGNATURES` in
  `wintrace.rs`, written from Learn's prototypes for every function
  7-Zip imports and some more): integers as numbers (32-, 16- and 8-bit
  kinds masked), pointers and handles as `"0x…"`, and string arguments as
  `{"w":…}` / `{"a":…}` (read when the call begins, at most 200
  characters; a value below 0x10000 stays a number: an ordinal for
  `GetProcAddress`, a character for `CharUpperW`). A function without a
  signature is logged with four arguments of unknown kind. `ret` is
  `null` for `void` and for a call logged when it began.
- `err`: `GetLastError()` after the call, **only when the call changed
  it**; a function that leaves the last error alone has no `err`.

The logger only calls the real `kernel32` (its own imports), never a
proxy; a per-thread flag in a TLS slot (`TlsAlloc`) still lets a thunk
reached from inside the logger call straight through. It keeps the
caller's last error (`TlsGetValue` clears it, so it is saved first).
Records are formatted in the thunk's frame and appended under a critical
section to a 256 KB buffer, written when full, when a `NORETURN` function
begins and at `DLL_PROCESS_DETACH`.

A record is lost when an exception unwinds through the call (it is
written at the return), and `ExitProcess` ends other threads wherever
they are; both are visible as a `seq` with no record.

### Comparison: `cargo xtask win trace-diff`

`cargo xtask win trace-diff <a.jsonl> <b.jsonl> [--map-a from=to]
[--map-b from=to] [--limit n] [--check]` groups each trace by `t`, sorts
by `seq`, and walks both threads together. The same function (DLL and
name) on both sides is a match; its result, last error and integer and
string arguments are compared, pointers and handles only as NULL, all
ones (`INVALID_HANDLE_VALUE`) or "some value", and the results of
`GetTickCount`, `QueryPerformanceCounter`, the process and thread ids are
not compared. Where the functions differ, the nearest pair that matches
again (up to 64 calls ahead on each side) resynchronises, and the calls
skipped are reported as "only in A / only in B". `--map-*` replaces text
in strings (each side's working directory by one word). The report gives
the calls per DLL, per thread the counts and matches, and the first
divergences with the call before them.

### On Poly: boot group `wintrace` (CI shard `userland`, report only)

The group builds the kit for the cached `7za.exe`, puts it and
`hello.txt` in an initramfs under `/7zip` and boots three runs: `7za i`,
`7za a -bd new.7z hello.txt` and the same with `-mmt1`, each with
`poly.test.stderr=/7zip/polytrace.jsonl`, which makes the kernel report
the trace as `POLY-OUT: fd=2` records when the program ends (no kernel
change). Each trace goes to `target/wintrace/poly/7za-<step>.jsonl`, is
summarised (calls, the last three), and, when a Windows reference is in
`target/wintrace/reference` (or `POLY_WINTRACE_REF`), compared. It fails
only if the kit cannot be built. Boot logs echo the first 64 `POLY-OUT`
lines of a boot (a trace is about 1,500), then say how many were not
shown.

**Windows traces are not committed.** They are oracle observations (the
spec agent's side of CLEAN_ROOM.md, kept out of the repository like ADR
0075's run files), about 370 KB each, and depend on the machine (paths,
console or file output, CPU features). Re-recording is one PowerShell
command per run on the oracle machine (copy the kit and `hello.txt` to a
fresh directory, run, fetch `polytrace.jsonl`).

## Results (2026-10-30)

**On Windows (homeserver2).** The kit's copy of the pinned `7za.exe`
(SHA-256 checked on arrival) printed byte for byte what the original
printed, with the same exit code (0), for `7za i`, `7za a -bd new.7z
hello.txt` and `7za a -bd -mmt1 new.7z hello.txt`, standard output and
error redirected to files. Defender did not object to the kit. Traces:

| Run | Calls | Threads | advapi32 | kernel32 | msvcrt | oleaut32 |
|---|---:|---:|---:|---:|---:|---:|
| `i` | 4,038 | 1 | 6 | 79 | 3,953 | 0 |
| `a` | 4,137 | 3 | 12 | 236 | 3,879 | 10 |
| `a -mmt1` | 4,092 | 1 | 12 | 104 | 3,966 | 10 |

Most calls are the C runtime's `free`, `malloc` and `memcpy`. Two `a`
runs differ on Windows already: sizes that follow the working
directory's length (`malloc(202)` against `malloc(204)` for a directory
one character longer) and how often the worker threads wait and signal.

**On Poly** (TCG): `i` and `a -mmt1` run to the end under the tracer
with exit code 0, printing what the `7zip` boot test checks for
(`Formats:`, `Everything is Ok`).

| Run | Calls (Windows / Poly) | Matched without a difference | Divergences |
|---|---|---:|---:|
| `i` | 4,038 / 4,033 | 3,997 | 43 |
| `a -mmt1` | 4,092 / 4,079 | 3,938 | 160 |

The first divergences (in call order, both runs):

1. `__getmainargs` sets the last error to 0 on Windows; ours leaves it.
2. `IsProcessorFeaturePresent(17)` (`PF_XSAVE_ENABLED`) is 1 on Windows,
   0 on QEMU's TCG CPU, and 7-Zip then makes one call fewer.
3. `GetVersion` gives `0x6658000A` on Windows (10.0, build 26200) and
   `0x4A65000A` on Poly (10.0, build 19045).
4. `GetProcAddress(kernel32, "SetDefaultDllDirectories")` finds it on
   Windows; on Poly it is NULL with last error 127.
5. `_isatty` of the standard handles is 0 on Windows (redirected to files
   there) and 64 on Poly (its console), and 7-Zip's console path follows
   (`GetConsoleMode` only on Poly): a difference of the set-up, to be
   removed by redirecting on Poly too.
6. `AdjustTokenPrivileges` succeeds on Windows and fails with
   `ERROR_ACCESS_DENIED` on Poly (ours is a stub, ADR 0087).
7. `FindFirstFileW("hello.txt")` sets the last error to 0 on Poly, not
   on Windows; `GetCurrentDirectoryW` differs by the directories' length.

**`7za a` with its worker threads did not run on Poly under the tracer**,
and showed two Poly bugs (reported, not fixed here; they are not in the
loader):

- *Threads created by `NtCreateThreadEx` get stacks that do not grow.*
  `make_stack` reserves the stack and puts a guard page below the
  committed part, but only the first thread's allocation is marked as a
  stack (`vm.set_stack`, `personalities/nt/src/exec.rs`), so for any other
  thread the guard page raises `STATUS_GUARD_PAGE_VIOLATION` instead of
  growing it, and the page below is an access violation. 7-Zip's
  workers (`_beginthreadex` with 1 MiB reserved,
  `STACK_SIZE_PARAM_IS_A_RESERVATION`) never fault without the tracer;
  with it, the thunk's frame pushed one past its first page: an access
  violation writing just below it.
- *The kernel panics on that unhandled exception*: building the
  backtrace for the "unhandled exception" message
  (`nt::exception::finish` → `poly_seh::dispatch::walk`) overflows the
  kernel stack in nested user page faults (double fault), so the boot
  ends without `POLY-EXIT` and without the trace.

## Consequences

- `cargo xtask win trace-kit` and `trace-diff`; the boot group
  `wintrace`; `tools/wintrace/` (the logger and the thunk, excluded from
  the host workspace, formatted by `ci lint`'s `fmt (wintrace)` step).
- Host tests (`wintrace.rs`): the plan (one proxy per DLL, variables
  forwarded, duplicates once, flags, signatures by ordinal), the generated
  Rust, assembly and link arguments, the signature table's form, the
  import rename on a synthetic PE (names, the new section, signature
  dropped, size of image, checksum), the checksum, trace parsing (header,
  a cut-off last line), the diff (per-thread alignment, pointers as
  classes, path maps, resynchronisation) and the boot log reader.
- Shared files changed additively: `sevenzip.rs` (the two `win`
  subcommands), `main.rs` (the module, the boot group in shard
  `userland`, the lint step, the `POLY-OUT` echo limit), the root
  `Cargo.toml` (`tools/wintrace` excluded).
- Next: run the kit from ADR 0088's scenario driver as its "traced"
  variant (the kit is one directory: copy it beside the inputs); redirect
  the standard handles on Poly as on Windows; pin the working directory's
  length (or map it) so buffer sizes match; output-buffer strings for
  `GetCurrentDirectoryW`, `GetModuleFileNameW` and the like (read at the
  return); more signatures as other programs are traced; a mode that
  leaves out the heap and memory functions (most of the volume).
