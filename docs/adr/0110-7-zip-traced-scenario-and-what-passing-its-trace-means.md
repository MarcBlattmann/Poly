# 0110: The traced 7-Zip scenario: the scenario's fifteen steps under the API tracer, recorded on the oracle machine and committed in canonical form, compared call by call with Poly's, and what "passes its full recorded trace" means

**Status:** Accepted. The last part of the recording ADR 0087 asks for
("The recording on Windows", point 3: the API trace) toward the Phase 5
milestone "7-Zip command line passes its full recorded trace", which stays
unticked: Poly's trace does not match yet (below). Builds on
[ADR 0088](0088-7-zip-scenario-driver.md) (the scenario driver and its
steps), [ADR 0090](0090-windows-api-tracer.md) (the tracer: kit, trace
format, `trace-diff`), [ADR 0097](0097-oracle-machine-over-ssh-and-wine-test-seeds.md)
(the oracle machine `homeserver2`, `win remote`) and
[ADR 0101](0101-system-message-table-and-formatmessage.md) (the scenario
required; what the milestone still lacks). Amends ADR 0090 in two points:
a Windows trace reference **is** committed (in canonical form), and the
`wintrace` boot group runs the traced scenario instead of three single
runs.

## Context

ADR 0101 left three things between the 16/16 scenario and the milestone:
no Windows trace of the scenario's steps, traces that differ (some of it
the set-up, some of it Poly), and nothing that checks. ADR 0090 had traced
three separate runs and found that 7-Zip's buffer sizes follow the working
directory's length, that `_isatty` on Poly's console changes 7-Zip's path,
and that two Windows runs differ where worker threads wait. Its traces were
about 370 KB each and stayed out of the repository.

## Decision

### The traced driver: `tests/windows/sevenzip_trace.rs`

A second scenario driver, a copy of ADR 0088's (not a variant of the same
source: that driver is frozen with its recording, its SHA-256 is in the
reference). Its data files are the trace kit (`cargo xtask win 7zip-trace
stage` runs `win trace-kit` for the cached `7za.exe` into
`target/pipeline/windows/data/sevenzip_trace/`: the instrumented `7za.exe`,
`polytrace.dll`, `pt_{advapi32,kernel32,msvcrt,oleaut32,user32}.dll`). It

1. makes a **work directory** `<dir>\p<x...>\sevenzip-work` whose full path
   is exactly 120 characters on every system: the padding directory takes
   up what the runner's directory leaves (the oracle's scratch directory is
   90 characters, Poly's `Z:\sevenzip_trace` 17), and the work directory's
   own name is the same everywhere, because 7-Zip asks for it
   (`FindFirstFileW(".")`) and copies it (a buffer of its length). The
   kit's files are copied into it;
2. runs ADR 0088's fifteen steps there, each `<work>\7za.exe` with the work
   directory current, standard input, output and error redirected to files
   in the driver's own directory exactly as ADR 0088 does (so `_isatty` is
   0 and the console path is not taken on either system);
3. after each step copies the kit's `<work>\polytrace.jsonl` into the
   observation file between `trace size=<n> sha256=<hex> lines=<n>` and
   `trace-end`, deletes it, and records the exit code, both outputs and the
   tree as ADR 0088 does (the kit left out).

The observation file's first line is `poly-tracescenario/1`, so the
scenario tools (`win scenario show/diff`) do not take it for a scenario;
`tools/xtask/src/sevenzip_trace.rs` reads it.

### The recording

`win remote` (ADR 0097) with the kit as data files, one run per invocation:

```sh
polyd 'cargo xtask win 7zip-trace stage'
POLY_RUN=~/poly-dev/polyd tools/pipeline/windows/poly-remote.sh --host homeserver2 --tests sevenzip_trace \
    --data-dir target/pipeline/windows/data --runner-arg -Runs --runner-arg 1 \
    --runner-arg -TestTimeouts --runner-arg sevenzip_trace=1800
cp .oracle-remote/homeserver2/results.json <somewhere>/windows-N.json     # once per run, at least twice
polyd 'cargo xtask win 7zip-trace record windows-1.json windows-2.json windows-3.json'
```

`record` reads each result file's observation, puts every step's trace into
canonical form (below) and compares the runs with the first one exactly as
Poly is compared; **if the runs differ it refuses** (unless
`--allow-noise`, which the reference then records as `noise_allowed`). So a
kind of run-to-run noise is either normalised and documented here, or
visible. It then writes the reference.

On 2026-10-31 on `homeserver2` (Windows 11 Home, build 26200.9711; ANSI
1252, OEM 850, UTC+1, 6 processors), bundle `45a0f74a7575`: three runs
(about 0.7 s each), identical after normalisation; two earlier runs of the
driver before the work directory got its fixed name were identical to each
other too. Every step runs on one thread on Windows (the `a` steps use
`-mmt1`, the others do not start workers for one small file), so ADR 0090's
thread noise does not arise. Raw: a 4.3 MB observation file per run, 46,338 calls
(after normalisation) in 15 steps, 72 functions; about 94 % of the calls are the C
runtime's `free`, `malloc`, `memcpy`, `fputc`, `memset`.

### The reference: `tests/windows/oracle/sevenzip_trace.reference.json`

Format `poly-tracereference/1`, 157 KB. What it holds:

- `recorded`: the system, the number of runs and when they started, the
  driver's SHA-256, the work directory's length, every kit file's size and
  SHA-256 (the kit build is reproducible, `/Brepro`; a changed tracer or a
  changed signature table means recording again), the machine facts;
- `steps`: per step the command, exit code, output digests and, per
  thread, the call sequence as indices into `calls`, a run of one index
  written `index*count`;
- `calls`: each distinct canonical call once,
  `[dll, function, arguments, result, last error]` (715 of them).

**Canonical form** (`wintrace::canonical`): what the comparison looks at
and nothing else. The normalisations below applied; pointers and handles
as `"NULL"`, `"-1"` or `"ptr"`; integers as numbers; strings with the work
directory replaced by `<work>`; results the comparison does not compare as
`"*"`; each call's position as its index in its thread. Two traces compare
the same in canonical form exactly when the raw traces compare the same
(`trace-diff`'s rules), so the raw Windows traces need not be kept. They
are not committed (oracle run files, 4.3 MB per run, with machine paths);
the reference names everything needed to regenerate them.

### The normalisations (all, and only these)

From ADR 0090 (unchanged): pointers and handles only as NULL, all ones or
"some value"; results of `GetTickCount`, `GetTickCount64`,
`GetCurrentProcessId`, `GetCurrentThreadId`, `QueryPerformanceCounter`,
`QueryPerformanceFrequency`, `GetSystemTimeAsFileTime` not compared; each
side's working directory as one word. New here, each because two correct
systems on different machines differ in it:

1. **Machine: `IsProcessorFeaturePresent`** calls are left out of both
   traces before alignment. The answer describes the processor
   (`PF_XSAVE_ENABLED`, 17, is 1 on the oracle, 0 on QEMU's CPU), and how
   often 7-Zip asks follows `CPUID`, which is not a traced call (on the
   oracle 7-Zip asks once more, early). That Poly reports its processor
   truthfully is a spec test's to check, not the trace's.
2. **Machine: `GetVersion`** is compared as its major and minor version;
   the build number in its high word is the machine's Windows build
   (26200 on the oracle; two correct Windows installations of different
   builds differ). Poly reports 10.0 like the oracle.
3. **Machine: `GetOEMCP`'s result** is not compared: the OEM code page of
   the machine's locale (850 on the oracle, 437 on Poly; ADR 0088 already
   lists both code pages as machine facts). `GetACP` is 1252 on both and
   stays compared.
4. **`memset`'s fill value** is compared as a byte: `memset` converts it to
   `unsigned char` (Learn), and 7-Zip leaves whatever its register held in
   the other bits (`0xFFFD1001` on Poly where Windows' register held `1`).
5. **Local times in strings** (`YYYY-MM-DD HH:MM:SS`) are put into UTC by
   each run's own local offset (`machine local-offset-minutes`), ADR 0088's
   local-time normalisation applied to the trace (7-Zip's listings pass
   their lines to `fputs`).

Results and last errors of every other call are compared exactly,
including "left alone" against "set to the same value it had": the tracer
records `err` only when a call changed the last error, and that is
behaviour (`SetLastError(0)` on success or not). `trace-diff` applies 1-4
too (5 needs the offset, which only the driver records).

### The comparison: `cargo xtask win 7zip-trace diff`

```sh
cargo xtask win 7zip-trace diff [--kernel <elf>] [--build-kernel] [--target <poly.json>] \
    [--markdown docs/windows/7zip-trace.md] [--check]
```

It stages, runs the driver on Poly (`pipeline win scenario poly --tests
sevenzip_trace --timeout 5400`: one boot, the same initramfs layout as
ADR 0088's scenario) or reads a result file, and per step compares the exit
code and the trace thread by thread (`wintrace::diff_traces`: a call only
one side made, or the same call with another result, last error, integer
or string argument; resynchronising within 64 calls). The outputs and
files are not compared here: that is the untraced scenario's, which is
required (ADR 0101), and the trace would only repeat it without its
normalisations. It writes `target/wintrace/scenario/report.txt` (every
divergence), Poly's per-step traces next to it, prints a summary and with
`--markdown` the implementers' feedback: per step the matched calls, the
divergences per function, and each distinct divergence once with the
calls of both sides (non-pointer arguments, result, last error), where and
how often. `record` and `diff` read Windows' observations and refuse
`POLY_ROLE=implementer` (CLEAN_ROOM.md), like `pipeline win diff`.

### In CI: the `wintrace` boot group, report only, and the switch

The `wintrace` group (shard `userland`, where `7za.exe` is fetched) now
runs the traced scenario on the boot tests' kernel and compares it with
the reference. It fails if the kit or the driver cannot be built; a
difference, or a failure to run, is reported and does not fail the job
while `TRACE_REQUIRED` in `tools/xtask/src/sevenzip_trace.rs` is `false`.
ADR 0090's three single runs (`i`, `a`, `a -mmt1`) are gone: the scenario
covers `i` and `a -mmt1`, and the multi-threaded `a` is not a scenario
step (its run-to-run wait counts would need a normalisation of their own).

**To make it required:** once `cargo xtask win 7zip-trace diff --check`
passes every step, set `TRACE_REQUIRED = true`. A difference or a failure
to run then fails the `wintrace` group.

### What "passes its full recorded trace" means

ADR 0087 point 3 asks for the calls into `kernel32`, `advapi32`, `msvcrt`,
`oleaut32` and `user32`, "name, arguments, return value, last error, in
order, per thread". ADR 0088 made the other points concrete as a per-step
comparison with documented normalisations; this does the same for the
trace. The milestone is met when **all** of these hold:

1. the untraced scenario passes all 16 of its steps and is required
   (ADR 0101: true today);
2. the traced scenario on Poly gives, for each of the 15 steps, the
   reference's exit code and a trace with **no divergence** from the
   reference: the same calls in the same order on every thread, each with
   the same result, last error behaviour and integer and string arguments,
   after exactly the normalisations listed above (no allowance for a count
   of divergences, no per-function exemptions);
3. that comparison is a required check: `TRACE_REQUIRED = true` in the
   `wintrace` group;
4. the reference was recorded with at least two runs that agree (`record`
   enforces it) on the current kit and driver.

A new normalisation needs the same justification (two correct systems
differ in it; listed here with the values seen) and an amendment of this
ADR. The roadmap's "left:" text says the same.

## Results (Poly at this ADR's revision)

The traced driver runs all 15 steps on Poly (TCG, one boot of about half a
minute) with the reference's exit codes. **0 of 15 steps match**: 45,924
of the 46,338 Windows calls are matched without a difference, 395
divergences, 61 distinct ones. By function (calls only on Windows / only
on Poly / differing):

| Function | Only Windows | Only Poly | Differ | What |
|---|---:|---:|---:|---|
| `msvcrt!memcmp` | 0 | 0 | 203 | Windows returns 1 or -1, Poly the difference of the bytes |
| `kernel32!GetProcAddress` | 0 | 0 | 47 | `SetDefaultDllDirectories`, `GetActiveProcessorGroupCount`, `GetActiveProcessorCount` are NULL on Poly (last error 127) |
| `msvcrt!free`, `msvcrt!malloc` | 38 | 0 | 16 | follow from the missing processor-group functions and from `FindFirstFileW(".")` |
| `advapi32!AdjustTokenPrivileges` | 0 | 0 | 34 | 1 on Windows, 0 on Poly (last error 5, `ERROR_ACCESS_DENIED`) |
| `kernel32!GetLastError` | 26 | 0 | 0 | 7-Zip calls it after a successful `AdjustTokenPrivileges` only |
| `msvcrt!__getmainargs` | 0 | 0 | 15 | sets the last error to 0 on Windows, leaves it on Poly |
| `kernel32!FindFirstFileW` | 0 | 0 | 10 | sets the last error to 0 on success on Poly, not on Windows |
| `advapi32!LookupPrivilegeValueW` | 0 | 0 | 7 | sets the last error to 0 on success on Windows, not on Poly |
| `kernel32!SetFilePointer` | 0 | 0 | 7 | sets the last error to 0 on success on Poly, not on Windows |
| `oleaut32!SysAllocStringLen` (#4), `SysAllocString` (#2) | 0 | 0 | 7 | set the last error to 0 on Windows, not on Poly |
| `msvcrt!memcpy` | 0 | 0 | 3 | after `FindFirstFileW(".")` 7-Zip copies 28 bytes on Windows (`sevenzip-work` and its NUL in UTF-16), 4 on Poly |
| `kernel32!CreateEventW` | 0 | 0 | 1 | sets the last error to 0 on Windows where Poly leaves it |

[docs/windows/7zip-trace.md](../windows/7zip-trace.md) has every distinct
divergence with both sides' calls. Some follow from others (a last error
one side left set decides whether a later call "changes" it). These are
observations for implementers; they need spec sheets (CLEAN_ROOM.md) before
Poly changes, and this ADR changes nothing in Poly.

## Consequences

- New: `tests/windows/sevenzip_trace.rs`,
  `tests/windows/oracle/sevenzip_trace.reference.json`,
  `tools/xtask/src/sevenzip_trace.rs` (`cargo xtask win 7zip-trace
  stage|record|diff`, the boot test, host tests: the observation parser,
  canonical steps, the reference's round trip and the committed reference
  against itself, the comparison and reports, local times),
  `docs/windows/7zip-trace.md`.
- Changed: `tools/xtask/src/wintrace.rs` (`canonical`, `canonical_value`,
  `normalise` with the machine and byte normalisations, also applied by
  `trace-diff`; the boot test delegates to the traced scenario; ADR 0090's
  `TRACED` runs, `read_boot` and the `POLY_WINTRACE_REF` directory removed),
  `sevenzip.rs` (the `7zip-trace` subcommand, `pipeline` shared),
  `main.rs` (the module), AGENT_PIPELINE.md, the roadmap's "left:" text.
- A change to the driver, to `poly-wtest`, to the tracer (`tools/wintrace`)
  or to the signature table changes what the reference names: record again
  (about five minutes with three runs).
- Not done: the multi-threaded `a` under the tracer (ADR 0090 saw it fail on
  Poly); recording on a second Windows build; output-buffer contents of
  calls (the tracer logs arguments when a call begins, so what
  `FindFirstFileW` wrote is seen only through 7-Zip's next calls).
