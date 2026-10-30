# 0088: The 7-Zip scenario driver: one Windows test program runs `7za.exe` step by step on Windows and on Poly, its recording is committed, and the two runs are compared per step

**Status:** Accepted. Builds the recording ADR 0087 asks for ("The
recording on Windows", points 1, 2, 4 and 5; the API trace of point 3 is
not part of it) toward the Phase 5 milestone "7-Zip command line passes its
full recorded trace", which stays unticked. Builds on
[ADR 0075](0075-windows-test-oracle.md) (the Windows test oracle: test
programs, `poly-oracle.ps1`, `poly-winrun/1`) and
[ADR 0087](0087-7-zip-command-line-on-poly.md) (the pinned `7za.exe`, the
`7zip` boot test).

## Context

ADR 0087 runs `7za.exe` on Poly once per boot and checks that the output
contains a phrase. The milestone needs the same program run the same way
on real Windows and compared in full: exit codes, the bytes it prints, the
files it makes. The oracle runner (ADR 0075) runs *our* test programs and
reads the file they write; it does not run third-party programs, ship data
files or redirect a child's standard handles.

## Decision

### One driver, a Windows test program like the others

`tests/windows/sevenzip_scenario.rs` is a `no_std` Rust test program
(ADR 0075) that imports only from `kernel32.dll` and `ntdll.dll`, by name.
It runs beside `7za.exe`, in a directory of its own that is also the
current directory of every step, and:

1. records **machine facts** that 7-Zip's output can depend on:
   `RtlGetVersion`'s version and platform, `GetACP`/`GetOEMCP`, the local
   offset `FileTimeToLocalFileTime` applies to the pinned time (what 7-Zip's
   listings show), `GetTimeZoneInformation`'s id and biases, the processor
   count (`GetSystemInfo`), the physical memory (`GlobalMemoryStatusEx`) and
   whether `HKCU\Software\7-Zip` exists (`RegOpenKeyExW`). Functions that
   only feed a fact (`GetTimeZoneInformation`, the two registry functions
   through `LoadLibraryW("advapi32.dll")`) are found with `GetProcAddress`,
   so a function Poly lacks is a recorded fact (`unavailable`), not a load
   failure of the driver. It also records the size and SHA-256 of `7za.exe`
   (`input`), so a run with another binary is visible;
2. writes `hello.txt` (`Hello from 7-Zip on Poly.\r\n`, `sevenzip.rs`'s
   `SAMPLE_TEXT`) and sets its creation, access and last-write times to
   2026-01-01 00:00:00 UTC (`SetFileTime`), so the archives are the same on
   every run, and records the tree;
3. runs the **steps** of its `STEPS` table, in order, each with
   `CreateProcessW` (application `<dir>\7za.exe`, command line `7za <args>`,
   `bInheritHandles`, `STARTF_USESTDHANDLES`, the directory as current
   directory): standard input an empty file, standard output and error two
   files, all three inheritable; it waits (killing the step with
   `TerminateProcess` after 120 s) and records the exit code (or
   `spawn-error error=<GetLastError>`, `timeout`);
4. after each step records standard output and error (size, SHA-256, the
   bytes up to 1 MiB) and **every file and directory** below the directory
   except its own (`7za.exe`, the driver, `poly-test.out`, `poly-step.in`,
   `.out`, `.err`): path, size, SHA-256, last-write `FILETIME`, attributes,
   and the bytes of files up to 64 KiB (base64), sorted by path.

The steps (all but `usage` and `bad-command` get `-bd -sccUTF-8` after the
command, so no progress display and UTF-8 output whatever the console code
page):

| Step | Arguments |
|---|---|
| usage | (none) |
| info | `i` |
| hash | `h hello.txt` |
| add | `a -mmt1 new.7z hello.txt` |
| list | `l new.7z` |
| list-tech | `l -slt new.7z` |
| test | `t new.7z` |
| extract | `x -y -oout new.7z` |
| add-mx9 | `a -mx9 -mmt1 all.7z hello.txt` |
| add-zip | `a -tzip -mmt1 new.zip hello.txt` |
| extract-zip | `e -y -ozip new.zip` |
| missing-archive | `x -y missing.7z` |
| not-an-archive | `t hello.txt` |
| bad-switch | `a -zzz x.7z hello.txt` |
| bad-command | `q` |

**Times without a shared clock.** An archive's own last-write time is "now".
The driver writes a time at or after the start of a step as that step's
name (`mtime=step:add`); older times (the pinned one, times 7-Zip restores
on extraction) as the value. Before each step it waits until the system
time has moved past the end of the previous step, so a file closed at the
end of one step cannot carry the next step's start time on a coarse clock.
Two runs on Windows give the same bytes (the runner's `stable`).

**The observation file**, `poly-test.out` (format `poly-scenario/1`, one
record per line: `machine`, `input`, `step`, `command`, `exit`, `stdout`,
`stdout-b64`, `stderr`, `stderr-b64`, `file`, `file-b64`, `dir`, `end`,
`done`; documented in `tools/pipeline/src/scenario.rs`). Both runners
already carry the file (`output_b64` of `poly-winrun/1`), so nothing in the
result format changed but an optional `notes` field (below). `poly-wtest`
gained `write_output`, an incremental `Sha256` (FIPS 180-4) and a streaming
`Base64Writer` (RFC 4648), written for this; the file capture is generic,
so the planned API tracer (a "traced" variant) can reuse it.

### Data files beside a test

- **Bundle**: `win bundle --data-dir DIR` adds every `DIR/<test>/<file>`
  of the bundle's tests as a line `data <test> <name> <sha256> <base64>`
  after the executables; the end line counts them too. `win unbundle`
  writes them to `<out>/data/<test>/`. A bundle without data lines is what
  it was; an older runner refuses one with them ("unknown bundle line"),
  so the format name stays `poly-winbundle/1`.
- **Runner** (`poly-oracle.ps1 2`): `-DataDir DIR` copies `DIR\<test>\*`
  beside the test into each scratch directory before each run; a bundle's
  data lines go to `<Work>\data`, the default then. `-TestTimeouts
  'sevenzip_scenario=300'` overrides `-TimeoutSeconds` per test (the result
  file records them). One run of the scenario takes about a second here.
- `cargo xtask win 7zip-diff` stages the cached `7za.exe` (ADR 0087) as
  `target/pipeline/windows/data/sevenzip_scenario/7za.exe` and builds the
  driver into `target/pipeline/windows/scenario-bin` (not `win build`'s
  directory, whose other executables `--tests` would remove).

### The recording

Recorded on the oracle PC (Windows 11 Enterprise 25H2, build 26200.9448,
x64; ANSI 1252, OEM 850, UTC+1 on the recording date; 32 processors) with
`poly-oracle.ps1 -TestsDir ... -DataDir ... -TestTimeouts
sevenzip_scenario=300` (2 runs per invocation), twice: all four runs gave
the same bytes. The result is committed as
`tests/windows/oracle/sevenzip_scenario.windows.json` (ADR 0075's
`tests/windows/oracle/`). It holds 7-Zip's output on Windows; like a spec
sheet it is an observation, and it is the reference CI compares with.

The reference names the driver's SHA-256 (`c46fc5c4...`). The build is
reproducible, so the committed source gives that executable; **any change
to the driver or to `poly-wtest` needs a new recording** (the comparison
reports "different drivers" otherwise). On the recording PC, after the
recordings, the endpoint protection began refusing to start the driver
("Access is denied" from `CreateProcess`, the same for a bundle-unpacked
copy; `7za.exe` still starts): recording again there needs the owner to
allow it (ADR 0075: an exclusion is the owner's decision).

### The comparison: `pipeline win scenario diff`

`win scenario diff --reference <windows.json> [--target poly|<file>]
[--markdown <file>]` runs the scenario drivers on Poly (or reads a result
file) and compares each with the reference **per step**:

- **exit code**: exactly (the driver's text: `0x2`, `spawn-error error=2`);
- **stdout, stderr**: byte for byte after the normalizations below; the
  first differing line is shown, or "the target wrote nothing";
- **files**: by path: kind, size, SHA-256, last-write time (value or step
  label), attributes; a missing or extra file is a difference; contents
  are shown by the first differing byte;
- a **setup** pseudo-step compares the tree before the first step
  (`hello.txt` with its pinned times: does `SetFileTime` work);
- machine facts are listed side by side, not compared; inputs that differ
  (another `7za.exe`) and the drivers' own notes are listed.

**Normalizations** (only values that legitimately differ between two correct
systems; each named in the report, the step then `same*`):

1. **Local time**: 7-Zip prints local times. If the drivers' local offsets
   differ (the PC +60 minutes, Poly 0), the reference's `YYYY-MM-DD
   HH:MM:SS` times in stdout and stderr are shifted by the difference
   before comparing. A target that reports no offset is taken as UTC.
2. **ZIP MS-DOS times**: a ZIP's local headers and central directory hold
   local MS-DOS times (PKWARE APPNOTE 4.4.6; the format of Learn's
   "DosDateTimeToFileTime"); the reference's are shifted the same way
   before the hashes are compared (the NTFS extra field holds UTC and is
   compared as is).

No other normalization is needed: no step prints the processor count, the
memory size or a CPU name (`-mmt1` fixes the thread count; the benchmark is
not a step). If a step that prints them is added, a normalization for it is
added with it, here.

Verdicts per step: PASS, FAIL, MISSING (the target's driver did not get to
it). A reference that is not `stable`, a different driver, or a target
without scenario observations is a problem reported above the steps. It
writes `<test>.report.txt` and `summary.json` (`poly-scenario-diff/1`) in
the run directory and exits 1 unless every step passed. `win scenario
show <results.json>` prints a run step by step with the outputs as text;
`win scenario poly` runs Poly only. `show` and `diff` read real Windows'
observations and refuse to run as an implementer, like `win show`/`diff`.

### Poly's side

`scenario::run_poly` boots once per driver with an initramfs holding
`/<test>/<test>.exe` and its data files and `poly.qemu_exit
poly.init=/<test>/<test>.exe poly.test.stdout=/<test>/poly-test.out`
(no `poly.test.capture`, so only the observation file is reported as fd 1,
whatever the steps print on the console), with a 900 s timeout. The
kernel's lines about unhandled exceptions, unimplemented calls and programs
it cannot start are kept as the result's `notes` and listed in the report.

### In CI: report only

The `7zip` boot group (shard `userland`, which fetches `7za.exe`) runs the
scenario after its five required boots: it stages and builds the driver,
runs `win scenario diff` on the boot tests' kernel and prints the report.
It never fails the job while `SCENARIO_REQUIRED` in
`tools/xtask/src/sevenzip.rs` is `false`; errors in running it are printed
as warnings. **To make it required**: once `cargo xtask win 7zip-diff`
passes every step, set `SCENARIO_REQUIRED = true` (a difference or a
failure to run then fails the `7zip` group), and tick nothing else: the
milestone also needs the API trace (ADR 0087, point 3).

`cargo xtask win 7zip-diff [--markdown docs/windows/7zip-recording.md]`
runs the same comparison by itself (it builds the kernel unless `--kernel`
or `--target` is given). [docs/windows/7zip-recording.md](../windows/7zip-recording.md)
is the first report, generated that way.

## The first comparison (Poly at this ADR's revision)

16 steps (with setup): 1 pass, 15 fail. Every file every step makes is the
same as on Windows (`7z` archives byte for byte, the ZIP after the time
normalization), `SetFileTime` works, and the exit codes of all eleven steps
that succeed on Windows are the same (0). What differs:

- **No step's standard output or error reaches the files.** Poly's
  `CreateProcessW` ignores `bInheritHandles` and `STARTUPINFO`
  (`STARTF_USESTDHANDLES`): the child writes to the console. This is
  workstream B's change (standard handles and inheritance in
  `kernelbase`); every step's stdout/stderr comparison waits on it.
- **The error paths end in an unhandled C++ exception**: `missing-archive`
  (Windows 2), `bad-switch` and `bad-command` (Windows 7) exit with
  `0xe06d7363` on Poly. 7-Zip reports these errors by `throw`; Poly's
  `__CxxFrameHandler` is a stub (ADR 0087), so no `catch` runs. The kernel
  notes name the exceptions. `not-an-archive` exits 2 on both (no throw).
- Machine facts (not compared, listed): Poly reports Windows 10.0.19045,
  OEM code page 437, local offset 0, no `GetTimeZoneInformation`, 2
  processors and 248 MiB in the test VM.

## Consequences

- New: `tests/windows/sevenzip_scenario.rs`,
  `tests/windows/oracle/sevenzip_scenario.windows.json`,
  `tools/pipeline/src/scenario.rs` (parser, normalizations, comparison,
  reports, Poly run; host-tested, including the committed reference
  against itself and its ZIP shifted and back), `pipeline win scenario
  poly|diff|show`, `cargo xtask win 7zip-diff`,
  `docs/windows/7zip-recording.md`.
- Changed additively: `poly-wtest` (`write_output`, `Sha256`,
  `Base64Writer`), `windows.rs` (bundle `data` lines, `data_files`,
  `RunResult::notes`), `poly-oracle.ps1` (`-DataDir`, `-TestTimeouts`,
  version 2), the `7zip` boot group (the report-only scenario), the
  pipeline and xtask command lists, AGENT_PIPELINE.md.
- The driver is frozen with its recording: changing it means recording
  again on the PC (see above).
- Not done: the API trace (ADR 0087, point 3; the tracer workstream can
  run its traced variant through the same driver); a console variant of
  the steps (7-Zip draws progress differently on a console); recording on
  a second Windows build.
