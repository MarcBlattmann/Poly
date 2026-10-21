# 0087: 7-Zip's command line on Poly: the official x64 `7za.exe` fetched and checked by `xtask`, its imports measured against our DLLs, the gaps filled, a boot test, and what the Windows recording needs

**Status:** Accepted. Works toward the Phase 5 milestone "proof of concept b:
7-Zip command line (`7z.exe`) passes its full recorded trace", which stays
unticked (see "The recording on Windows"). Builds on
[ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md) (our `ntdll`,
its loader and API-set table), [ADR 0075](0075-windows-test-oracle.md) (the
Windows test oracle), [ADR 0079](0079-our-kernelbase-and-kernel32.md)
(`kernelbase`/`kernel32`), [ADR 0080](0080-console-host.md) (the console
host), [ADR 0083](0083-advapi32-registry-tokens-and-security-for-one-user.md)
(`advapi32`), [ADR 0084](0084-our-c-runtime-ucrtbase-and-msvcrt.md) (our C
runtime) and [ADR 0027](0027-agent-pipeline-tools-and-test-run-protocol.md)
(the test-run records `POLY-INIT` / `POLY-EXIT`).

## Context

The milestone names 7-Zip's command line. 7-Zip is LGPL-2.1 software:
running it, and reading the *names* in its PE import table, is allowed
(CLEAN_ROOM.md); copying its code is not, and nothing of it is read to learn
how Windows behaves. Its behaviour on Windows will come from the oracle
machine (ADR 0075), not from here.

Sources for what was implemented (clean room): Microsoft Learn's reference
page of every function below, including each page's "APILocation" metadata
(the DLLs and API-set contracts that host it, as ADR 0083 reads them), the
"BSTR", "VARIANT structure", "x64 exception handling" and "Interlocked
Singly Linked Lists" pages; the layouts and constants of the public SDK
headers (`winnt.h`: `CONTEXT`, `EXCEPTION_RECORD`, `DISPATCHER_CONTEXT`,
`SCOPE_TABLE_AMD64`, `SLIST_HEADER`, `PF_*`; `excpt.h`; `oaidl.h` /
`wtypes.h`: `VARIANT`, `VARTYPE`; `sysinfoapi.h`: `MEMORYSTATUSEX`;
`processthreadsapi.h`: `STARTUPINFOW`; `ntifs.h`/`wdm.h`:
`FILE_FS_FULL_SIZE_INFORMATION`, `FILE_LINK_INFORMATION`; `ntddk.h`:
`ProcessTimes`/`KERNEL_USER_TIMES`; `winerror.h`), the Intel SDM for
`CPUID`/`XGETBV`, Raymond Chen's "Decoding the parameters of a thrown C++
exception (0xE06D7363)" (Microsoft Dev Blogs, 2010) for the exception code
of a C++ `throw` and its parameters, and the coverage map's export names
(Wine's `.spec` lists; `oleaut32`'s ordinals from its `oleaut32.spec` at the
map's revision, export-table data). No Windows binary, Wine, ReactOS, 7-Zip
or MSVC runtime source was looked at.

## Decision

### The binary: `7za.exe` (x64) of 7-Zip Extra 26.05, fetched and pinned

7-zip.org's download page (read 2026-10-21) offers 7-Zip 26.05. Its
standalone `7zr.exe` is an **x86** program, which the NT personality (x64
only) does not run, so the program is `x64/7za.exe` of the "7-Zip Extra"
package (`7z2605-extra.7z`, 1,763,900 bytes; the page links it to the
project's GitHub releases, fetched over HTTPS only). The site publishes no
hashes and GitHub's asset digests were not reachable from here; the SHA-1
and MD5 sums SourceForge's `sevenzip` project lists for the same files
matched the downloads, and `tools/xtask/src/sevenzip.rs` pins the SHA-256
and size of each file it fetches and of `7za.exe` itself (1,334,784 bytes).

`cargo xtask win fetch-7zip` downloads with `curl` (`--proto =https`),
checks size and SHA-256, unpacks the `.7z` with the same release's Linux
console version (`7z2605-linux-x64.tar.xz`, also pinned; no host tool here
reads `.7z`), checks `7za.exe`, makes `sample.7z` (one small text file) for
the extraction steps with that Linux `7zz`, deletes both archives and keeps
`<main checkout>/target/7zip/26.05/{7za.exe,sample.7z}` (1.3 MB, shared by
worktrees like the `ntdll` cache). Nothing of 7-Zip is committed.

### The import gap tool: `cargo xtask win imports <exe | 7zip>`

`tools/xtask/src/winimports.rs` reads the executable's import and
delay-load directories with `poly-pe` and follows every import the way our
loader binds it: an API-set name to its host (`poly_ldr::apiset`), a
forwarder of one of our DLLs to its target, an ordinal to the export at
that ordinal, until code is found. Our DLLs are the ones the kernel embeds,
read from the same content-hash caches (`ntdll`, the Win32 workspace), so
the report is about the current sources. Each import is **present** (with
the coverage map's status of the entry), a **stub** (status `stub`),
**missing function** (and whether the coverage map knows the name for that
DLL) or **missing DLL**. `--markdown <file>` writes the report as
[docs/windows/7zip-gaps.md](../windows/7zip-gaps.md) (generated; run it
again after a change), `--check` fails unless every import binds. Host
tests cover the classification on synthetic tables.

**The gap list at the start** (`7za.exe`: 145 imports from five DLLs, no
delay-loads): 107 present; **2 missing DLLs** — `oleaut32.dll` (6 imports,
all by ordinal: 2, 4, 6, 7, 9, 10) and `user32.dll` (`CharUpperW`,
`CharPrevExA`); **30 missing functions** — `kernel32`: `ResumeThread`,
`SetThreadAffinityMask`, `MoveFileW`, `GetLargePageMinimum`,
`SetFileApisToOEM`, `GetProcessTimes`, `SetProcessAffinityMask`,
`OpenEventW`, `FileTimeToDosDateTime`, `DosDateTimeToFileTime`,
`IsProcessorFeaturePresent`, `GetProcessAffinityMask`, `GetModuleHandleA`,
`GlobalMemoryStatusEx`, `GetDiskFreeSpaceExW`, `GetDiskFreeSpaceW`,
`MoveFileWithProgressW`, `CreateHardLinkW`, `FindFirstStreamW`,
`FindNextStreamW`, `GetLogicalDriveStringsW`, `DeviceIoControl`;
`advapi32`: `SystemFunction036`, `GetFileSecurityW`, `SetFileSecurityW`,
`AdjustTokenPrivileges`; `msvcrt`: `??1type_info@@UEAA@XZ`,
`__C_specific_handler`, `__CxxFrameHandler`, `_CxxThrowException`. No
function was present but stubbed (no `stub` entries existed).

**After this ADR**: every import binds; 138 are present and 7 are stubs
(below).

### What was filled

`kernelbase` (forwarded from `kernel32` through the contract each page's
APILocation names; `poly_kernelbase::exports`), the logic host-tested in
`poly-kernelbase` (`cpu`, `slist`, `system`, `oleaut`, `time`):

- `ResumeThread`, `SuspendThread` (`NtResumeThread`/`NtSuspendThread`);
  `GetProcessTimes` over a new `ProcessTimes` class of
  `NtQueryInformationProcess` (the creation time the kernel notes when it
  makes an NT process, the CPU time `cputime` counts; exit time 0 — Poly
  keeps no times of ended processes); `GetStartupInfoW` (`cb` and zeros:
  `CreateProcessW` passes no `STARTUPINFO` on yet).
- `IsProcessorFeaturePresent`: `CPUID` and `XCR0` mapped to the `PF_*`
  values Learn lists (AVX only when the OS saves its state); PAE and
  fast-fail always present; virtualization features absent.
- `GetProcessAffinityMask`, `SetProcessAffinityMask`,
  `SetThreadAffinityMask`: masks validated (Learn: a non-empty subset of the
  system's) and recorded, **not enforced** (Poly's scheduler does not pin
  threads yet).
- `GlobalMemoryStatusEx`: physical memory from Poly's own `/proc/meminfo`
  read through `Z:` (the NT personality has no documented class for memory
  sizes); commit limit = physical memory (no paging file); virtual = the
  user address range; `dwLength` must be 64. `GetLargePageMinimum` is 0.
- `OpenEventW`, `GetModuleHandleA`; `GetLogicalDrives` (probing
  `\??\A:\`..`\??\Z:\`), `GetLogicalDriveStringsW`, `GetDiskFreeSpaceW`,
  `GetDiskFreeSpaceExW` (`FileFsFullSizeInformation`; cluster counts above
  32 bits report `0xFFFFFFFF`); `DeviceIoControl` over
  `NtDeviceIoControlFile` (file-system control codes too; they fail with
  `ERROR_INVALID_FUNCTION` where no device takes them).
- `SetFileApisToOEM`/`ToANSI`, `AreFileApisANSI`: the switch is recorded
  and reported; Poly's few ANSI file functions still use the ANSI page.
- `MoveFileW` (`MoveFileExW` with `MOVEFILE_COPY_ALLOWED`),
  `MoveFileWithProgressW` (no progress calls for a rename),
  `FileTimeToDosDateTime`, `DosDateTimeToFileTime`.
- MSVC start-up imports the task listed beyond 7-Zip's:
  `SetUnhandledExceptionFilter`, `UnhandledExceptionFilter` (no debugger:
  the filter decides, else `EXCEPTION_EXECUTE_HANDLER`, no message box),
  `RaiseException`, `InitializeSListHead` and the interlocked list
  functions (`CMPXCHG16B` with the header's sequence number), and
  `kernel32`'s forwarders `RtlCaptureContext`, `RtlLookupFunctionEntry`,
  `RtlVirtualUnwind`, `RtlUnwindEx`, `RtlPcToFileHeader` to `ntdll`
  (`api-ms-win-core-rtlsupport-*`, which Poly's API-set table gives to
  `ntdll`: `KERNEL32_NTDLL_FORWARDS`). `IsDebuggerPresent`,
  `GetSystemTimeAsFileTime` and `QueryPerformanceCounter` existed.
- Six of these (`MoveFileW`, the two MS-DOS time functions and the three
  affinity functions) are in Windows' `kernel32`/`kernel32legacy`, not its
  `kernelbase`; Poly's API-set table sends every `api-ms-win-core-*`
  contract to `kernelbase`, so they live there (the coverage map's
  `kernelbase` list does not have them; their `kernel32` entries are
  claimed).

`ntdll`: **`RtlCaptureContext`** is now assembly: every integer register as
at the call, `Rsp`/`Rip` as after the return, segments, `EFlags`, `MxCsr`
and the `FXSAVE` image, `ContextFlags` `CONTROL | INTEGER | SEGMENTS |
FLOATING_POINT` (the Rust version read its own frame). **`RtlRaiseException`**
captures its context, unwinds one frame and raises from the caller (the
exception address is the return address there; a handler that continues
execution makes the call return) — the old one raised from an address the
unwinder could not walk from. The dispatcher now fills
`DISPATCHER_CONTEXT`'s `LanguageHandler`, `HandlerData` and `ScopeIndex`
and reads `ScopeIndex` back. **`__C_specific_handler`** (`msvcrt` forwards
to it), its decisions in `poly_seh::scope` (host-tested): Learn leaves the
handler data to the handler, so Poly reads `winnt.h`'s `SCOPE_TABLE_AMD64`
field names — a record covers `[BeginAddress, EndAddress)`; with a
`JumpTarget` it is an `__except` whose `HandlerAddress` is the filter
(`filter(EXCEPTION_POINTERS*, frame)`), or the constant
`EXCEPTION_EXECUTE_HANDLER` itself when it is 1; it unwinds to `JumpTarget`
on `EXCEPTION_EXECUTE_HANDLER`, dismisses on `CONTINUE_EXECUTION`; without
one it is a `__finally` run during an unwind (`handler(TRUE, frame)`)
unless the unwind targets a place inside the same scope; `ScopeIndex` moves
past a termination handler before it runs (to be confirmed).

`advapi32`: `SystemFunction036` (Learn's `RtlGenRandom`): bytes of the
kernel's `/dev/urandom`.

`msvcrt`: `_CxxThrowException` raises `0xE06D7363` (noncontinuable) with
four parameters — an internal value the article does not give (Poly: 0),
the object, the throw information and the throwing module's base;
`??1type_info@@UEAA@XZ` frees nothing (Poly's runtime never caches a
name).

New DLLs in the Win32 workspace, installed in `System32` and embedded like
the others (`oleaut32.dll` added to the known DLLs):

- `user32.dll`: `CharUpperW` (a string in place, or one character in the
  low word; `ntdll`'s upcase table) and `CharPrevExA` (Poly's code pages
  have no lead bytes).
- `oleaut32.dll`: `SysAllocString`, `SysAllocStringLen`, `SysReAllocString`,
  `SysReAllocStringLen`, `SysFreeString`, `SysStringLen` (the 4-byte length
  prefix of Learn's "BSTR", the process heap), `VariantInit`,
  `VariantClear`, `VariantCopy` (`BSTR`s duplicated and freed, interfaces
  `AddRef`/`Release`d, references copied as bits, bad types
  `DISP_E_BADVARTYPE`); exported at the ordinals programs bind
  (`/EXPORT:Name,@n`). Arrays and records (`SAFEARRAY`, `IRecordInfo`)
  answer `E_NOTIMPL` for now.

**Stubs** (coverage status `stub`: they exist and fail plausibly):
`FindFirstStreamW` (`ERROR_HANDLE_EOF` for an existing file: Poly's volumes
have no NTFS streams), `FindNextStreamW`, `CreateHardLinkW`
(`FileLinkInformation`, which Poly's NT file I/O does not have yet:
`ERROR_INVALID_FUNCTION`), `AdjustTokenPrivileges` (nothing adjusted:
`ERROR_NOT_ALL_ASSIGNED` unless the token holds every privilege asked for),
`GetFileSecurityW`/`SetFileSecurityW` (`ERROR_NOT_SUPPORTED`: no security
descriptors on files yet) and **`__CxxFrameHandler`**, which continues the
search: Poly has no C++ exception handling (its tables are the compiler's,
documented nowhere public), so no `catch` takes an exception and no
destructor runs during an unwind; a C++ exception ends the process with
`0xE06D7363`. This is the largest gap left; the roadmap's own `vcruntime140`
item (ADR 0055) is where it would be solved.

### The kernel: `poly.init.arg=`

The first program (`poly.init=<path>`) gets its arguments from
`poly.init.arg=<arg>` words, one per argument, in order (no spaces inside
one; the command line is 512 bytes). The NT side puts them in the process
parameters' command line as for any program.

### The boot test: `7zip` (CI shard `userland`)

`sevenzip::boot_test` puts `7za.exe`, `hello.txt` and `sample.7z` into
`/7zip` of an initramfs (`Z:\7zip`, the program's directory and so its
current one) and boots once per step with `poly.init=/7zip/7za.exe`:

| Step | Command line | Must print |
|---|---|---|
| usage | `7za` | `Usage: 7za` |
| info | `7za i` | `Formats:` |
| add | `7za a -bd new.7z hello.txt` | `Everything is Ok` |
| test | `7za t -bd sample.7z` | `Everything is Ok` |
| extract | `7za x -bd -y -oout sample.7z` | `Everything is Ok` |

For each it reports whether the program started, its exit code and
`NTSTATUS` (`POLY-EXIT`), what it printed (the console host's screen turned
back into lines) and the kernel's notes (unimplemented services, unhandled
exceptions, `POLY-INIT: unsupported`). **All five pass** (exit status 0
and the expected text), so all five are required (`REQUIRED`); a step for
something Poly cannot do yet can be added as report-only.

**No network in the boot tests:** without the cached `7za.exe` the test is
skipped with the reason and the command that fetches it. CI's `userland`
job restores `target/7zip` from an Actions cache keyed `7zip-26.05` and runs
`cargo xtask win fetch-7zip` before the boot tests (GitHub's runners reach
github.com); if the fetch fails the job warns and the test is skipped.
`cargo xtask ci lint` checks the workflow does this with the version
`sevenzip.rs` pins.

The ring-3 test of `kernel32` (`k32test.exe`, ADR 0079) gained checks of
everything above that a program can observe from `kernel32` alone (and
`oleaut32`, `user32`, `advapi32` loaded with `LoadLibraryW`, `oleaut32` by
ordinal): processor features, the interlocked list, `RtlCaptureContext`
through the forwarder, `RaiseException` continued by a vectored handler,
the top-level filter, MS-DOS times, process times, memory status, drives
and disk space, `OpenEventW`, a suspended thread resumed, affinity, the
file API switch, the moves and the stubs' errors, `BSTR`s and `VARIANT`s,
`CharUpperW`, `RtlGenRandom` (589 checks in all).

## The recording on Windows (what the milestone still needs)

"Passes its full recorded trace" needs a recording of the same program on
the oracle machine (ADR 0075: the owner's Windows 11 PC, a PowerShell
runner, results as a file), compared with Poly's run. The runner today runs
*our* test programs, which write an observation file; for 7-Zip it needs:

1. **Inputs, fixed and shipped in the bundle**: the pinned `7za.exe`
   (SHA-256 above), `hello.txt` (its exact bytes), `sample.7z` (the bytes
   from the cache), each file's modification time set to a fixed value
   before the run (7-Zip stores and prints it), in a fresh work directory
   that is the current one; the same file tree Poly's initramfs has.
2. **Commands**: the five steps above, plus `7za l sample.7z` (a listing:
   names, sizes, times), `7za h hello.txt` (CRC32), `7za a -bd -mx9 -mmt1
   all.7z hello.txt` (one thread so the output does not depend on the CPU
   count), `7za x -bd -y missing.7z` and `7za t -bd hello.txt` (the error
   exits, whatever codes Windows gives), and `7za a -bd -sccUTF-8 ...` to pin the console
   charset. Every command with `-bd`, standard output and error redirected
   to files (7-Zip draws progress differently on a console), and
   `-sccUTF-8` (or a known console code page: the owner's machine is Swiss
   German, OEM 850, Poly's 437).
3. **What to capture per command**: the exit code; standard output and
   standard error as bytes; the files it made or changed (names, sizes,
   SHA-256, times, attributes); and the API trace — the calls `7za.exe`
   makes into `kernel32`, `advapi32`, `msvcrt`, `oleaut32` and `user32`
   (name, arguments, return value, last error), in order, per thread.
   That trace is the roadmap's "Windows API tracer" item, not built yet: an
   import-address-table hook of the program's own imports (the names this
   tool lists) is enough for the milestone; it must run without a
   debugger dialog (the runner's error mode).
4. **What to pin or record about the machine**: the time zone (listings
   print local times; run with UTC or record the offset), the ANSI and OEM
   code pages, the processor count and memory size (7-Zip's defaults
   depend on them: `-mmt1` and an explicit `-md` remove that), the Windows
   build, and whether `HKCU\Software\7-Zip` exists (`7za.exe` imports the
   registry functions).
5. **Comparison**: exit codes and output bytes exactly, except the first
   line's version and date (the same binary prints the same) and times;
   archives by content (`7za l -slt` of each) rather than bytes until times
   are pinned; the API traces by call sequence and results, ignoring
   pointers (as the Linux diff runner does for syscalls, ADR 0027).

Until that recording exists and passes, the milestone stays open.

## To be confirmed by the spec agent against real Windows

1. Every new forwarder's contract (from APILocation), and that Windows keeps
   the six "legacy" functions in `kernel32`.
2. `__C_specific_handler`'s reading of `SCOPE_TABLE_AMD64` (records,
   constant filter 1, `JumpTarget` 0 as `__finally`, `ScopeIndex`).
3. `RaiseException` with more than 15 arguments (ours: the first 15) and
   its exception address (ours: the return address in `RaiseException`).
4. `DosDateTimeToFileTime` with fields out of range (ours: fails,
   `ERROR_INVALID_PARAMETER`), `GlobalMemoryStatusEx` with a wrong
   `dwLength` (ours: `ERROR_INVALID_PARAMETER`) and its commit and virtual
   figures, `GetDiskFreeSpaceW` beyond 32 bits, `GetLogicalDriveStringsW`'s
   result when the buffer is short.
5. `FindFirstStreamW` on a volume without streams, `CreateHardLinkW`'s
   error on one without links, `AdjustTokenPrivileges` for a standard user.
6. `UnhandledExceptionFilter` without a filter and without a debugger
   (ours: `EXCEPTION_EXECUTE_HANDLER`, as with `SEM_NOGPFAULTERRORBOX`).
7. `_CxxThrowException`'s first parameter and flags; `VariantClear` /
   `VariantCopy` of arrays (ours: `E_NOTIMPL`).

## Consequences

- `cargo xtask win fetch-7zip` (network, about 3.4 MB downloaded, 1.3 MB
  kept) and `cargo xtask win imports <exe | 7zip> [--markdown f] [--check]`;
  the `7zip` boot group (five boots of a few seconds).
- Host tests: `poly-kernelbase` (`cpu`, `slist`, `system`, `oleaut`, MS-DOS
  times, the `rtlsupport` forwards), `poly-seh` (`scope`),
  `poly-personality-nt` (`ProcessTimes`), `xtask` (`winimports`, the boot
  test's output reader, the workflow check).
- Shared files changed additively: the Win32 workspace members and
  `OUTPUTS`, `kernel/build.rs` and `nt::USER32`/`OLEAUT32`, the layout's
  install list, `poly_ldr::path::KNOWN_DLLS` (`oleaut32.dll`), the kernel's
  NT state (creation times) and `NtKernel::process_times`, the services'
  `ProcessTimes` class, `program.rs` (`poly.init.arg`), `ntdll`'s SEH
  exports, `k32test.exe`, the CI workflow and `BOOT_GROUPS`.
- Coverage: the entries above are claimed by `7zip` in `kernel32.dll`,
  `kernelbase.dll`, `advapi32.dll`, `msvcrt.dll`, `ntdll.dll`, `user32.dll`
  and `oleaut32.dll`; the seven stubs have status `stub`.
- Left for the milestone: the Windows recording (above), C++ exception
  handling, and whatever the recorded trace shows 7-Zip does that Poly
  answers differently.
