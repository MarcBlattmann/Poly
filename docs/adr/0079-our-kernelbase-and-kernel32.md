# 0079: Our `kernelbase` and `kernel32`: real PE DLLs from Rust over our `ntdll`, `kernel32` forwarding through API sets

**Status:** Accepted. Builds on [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
(our `ntdll`: a real PE DLL built from Rust, its loader and API-set table,
its build and content-hash cache in `xtask`), [ADR 0073](0073-windows-file-system-layout.md)
(`C:\Windows\System32`, the environment block and process parameters),
[ADR 0067](0067-nt-file-io.md) (NT file I/O, `Z:` as the whole VFS),
[ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (dispatcher
objects and waits), [ADR 0063](0063-nt-processes-object-manager-and-memory.md)
(NT memory, the TEB) and [ADR 0055](0055-microsoft-redistributables-run-apps-own-copies.md)
(Poly provides the system DLLs; apps bring their own runtimes).

## Context

The Phase 5 item is "Our `kernelbase` / `kernel32`: files, processes,
threads, memory, synchronisation, errors". Almost every Windows program
imports `kernel32.dll`; since Windows 7 most of its functions live in
`kernelbase.dll` and `kernel32` reaches them through API-set contracts
(Learn: "Windows API sets"; each function's reference page names its
contract in the "API set" row of its requirements). The runtimes apps
bring (`vcruntime140`, `msvcp140`, `ucrtbase` replacements, ADR 0055)
import `kernel32` and `api-ms-win-core-*` contracts directly, so both
paths must work.

Sources (clean room): Learn's reference pages of every function
implemented, "Windows API sets", "Naming Files, Paths, and Namespaces",
"File path formats on Windows systems", "System Error Codes", "Environment
Variables", "Changing Environment Variables", "Kernel object namespaces",
"Targeting your application for Windows", "Operating System Version",
"Slim Reader/Writer (SRW) Locks", the PE format specification, and the
layouts and constants of the public SDK headers (`winerror.h`,
`winbase.h`, `winnt.h`, `minwinbase.h`, `fileapi.h`, `processenv.h`,
`sysinfoapi.h`, `ntifs.h`, `wdm.h`, `winternl.h`). Export *names* come
from the coverage map (Wine's `.spec` name lists). No Windows binary,
Wine or ReactOS source was looked at. There is still no Windows oracle
VM, so what Learn leaves open is a labelled guess (below).

## Decision

### Build: one workspace for the Win32 modules, built and cached like `ntdll`

`personalities/nt/win32` is one Cargo workspace (excluded from the host
workspace, like `ntdll`'s) with four crates:

| Crate | Output | What |
|---|---|---|
| `rt` (`poly-win32-rt`) | rlib | what every module needs without a C runtime: `__chkstk`, `_fltused`, the panic handler (a fail-fast, `int 0x29`) |
| `kernelbase` | `kernelbase.dll` | the implementations (below) |
| `kernel32` | `kernel32.dll` | forwarders, and the functions `kernel32` keeps |
| `k32test` | `k32test.exe` | the program the ring-3 test runs |

They are `no_std` Rust compiled for `x86_64-pc-windows-msvc` with
`-Zbuild-std` and linked by `rust-lld` (lld-link flavour), exactly ADR
0071's recipe; one `cargo build` compiles `core`/`alloc` once for all
four. **Imports use `#[link(kind = "raw-dylib")]`**: rustc writes the
import entries itself, so `kernelbase` imports `ntdll.dll` and `kernel32`
imports `kernelbase.dll` with no import library. Entry points
(`kernelbase_main`, `kernel32_main`, `k32test_main`) are set by each
crate's `build.rs` (`/ENTRY:`), since there is no CRT startup.

`cargo xtask win32` builds and verifies them; every kernel build runs
`win32::ensure` (`tools/xtask/src/win32.rs`), which hashes the inputs
(the four crates, every repo crate they reach through path dependencies,
the workspace manifest and lock file, the service table, the toolchain
pin, the flags — `ntdll.rs`'s `inputs_from`/`hash_files`, shared) and
takes `win32-<hash>/` from `<main checkout>/target/win32-cache` (shared by
every worktree, beside `ntdll-cache`), building only on a miss and then
deleting the build directory (`POLY_WIN32_KEEP_BUILD=1` keeps it). The
check: `kernelbase` exports every forwarded function as code; `kernel32`
exports each as a forwarder string to its contract and its own functions
as code; `k32test.exe` is an executable importing only `kernel32.dll`.

The kernel embeds the three files (`kernel/build.rs`, from
`POLY_WIN32_DIR`; empty files without `xtask`) as `nt::KERNELBASE`,
`nt::KERNEL32`, `nt::K32TEST`, and the layout writes the two DLLs into
`C:\Windows\System32` at the first NT use with ADR 0073's
`install_file` (rewritten only when the bytes differ), next to `ntdll`.
The loader finds them there as known DLLs (ADR 0071's list already names
both). `ntdll` stays mapped by the kernel; these two are loaded by
`ntdll`'s loader like any DLL.

### `kernel32` forwards through API-set contracts

`poly_kernelbase::exports::KERNEL32_FORWARDS` lists each forwarded
function with the contract its Learn page names
(`CreateFileW` → `api-ms-win-core-file-l1-1-0`, `CloseHandle` →
`api-ms-win-core-handle-l1-1-0`, `Sleep` → `api-ms-win-core-synch-l1-2-0`,
`LocalAlloc` → `api-ms-win-core-heap-l2-1-0`, ...). `kernel32`'s
`build.rs` turns each row into `/EXPORT:Name=contract.Name`, which
lld-link writes as a forwarder; `ntdll`'s loader resolves the contract
with `poly_ldr::apiset` (every `api-ms-win-core-*` is hosted by
`kernelbase.dll`, ADR 0071) and binds the import to `kernelbase`'s code.
A host test checks every contract resolves to `kernelbase`.
`KERNEL32_OWN` are the functions whose pages name no API set
(`GlobalAlloc`, `GlobalFree`, `GlobalSize`, `GlobalLock`,
`GlobalUnlock`): `kernel32` implements them over `kernelbase`'s
local-memory functions.

### The logic in a host-tested crate

`crates/poly-kernelbase` (`no_std` + `alloc`) holds everything that can
be decided without a process: DOS paths (kinds, `GetFullPathNameW`'s
normalisation per "File path formats on Windows systems": legacy device
names, the current directory and per-drive `=X:` directories, separators,
`.`/`..`, trailing periods and spaces; the `\??\` NT form;
`FindFirstFileW`'s directory/pattern split; `GetTempPathW`'s order), the
environment block (lookup without case, sorted insertion, deletion,
`ExpandEnvironmentStringsW`), `CreateFileW`'s disposition/access/flag
mapping and the `ERROR_ALREADY_EXISTS` / `ERROR_FILE_EXISTS` rules, the
file pointer, `WIN32_FIND_DATAW` from `FILE_BOTH_DIR_INFORMATION`,
`FILE_RENAME_INFORMATION`, wait timeouts and results, named-object names,
the SRW lock, `VirtualFree`/`LocalAlloc`/`GlobalAlloc` argument rules,
`FormatMessageW` (the system message table from Learn's "System Error
Codes" and the insert/escape/width rules), the version and the manifest
rule, `SYSTEM_INFO`, the buffer-size conventions, and the export table.
The `NTSTATUS` → Win32 mapping is `poly_rtl::status`, shared with
`ntdll` (`RtlNtStatusToDosError`), plus the few documented exceptions
(`CREATE_NEW` on an existing file is `ERROR_FILE_EXISTS`).

### Behaviour choices

- **The last error** is `ntdll`'s `RtlGetLastWin32Error` /
  `RtlSetLastWin32Error` (`TEB.LastErrorValue`). Every failure sets it;
  successes set it where Learn says (`CreateFileW`'s dispositions,
  `CreateEventW`/`CreateMutexW`/`CreateSemaphoreW` on an existing name,
  `GetFileSize`/`SetFilePointer` when the low DWORD is ambiguous).
- **The environment** is the block the kernel put in the process
  parameters (ADR 0073, Poly's own + 0x80/+ 0x88). `SetEnvironmentVariableW`
  builds a new sorted block in the process heap and points the parameters
  at it, so `ntdll` (the `Path` of the DLL search) sees the change.
  `GetEnvironmentStringsW` returns a copy.
- **The current directory** is `kernelbase`'s, behind its process lock,
  absolute and without a trailing separator unless a root. It starts as
  the executable's directory: the kernel does not yet pass a starting
  directory (that comes with `CreateProcess` and the console host).
- **The standard handles**: a console host is a later Phase 5 item. Until
  then a process gets what every Poly process starts with on descriptors
  0, 1 and 2, the system console: `GetStdHandle` opens
  `\??\Z:\dev\console` (ADR 0067: `Z:` is the whole VFS) on first use,
  for reading (input) and writing (output, error). If that fails the
  handle is NULL, which Learn allows. `SetStdHandle` replaces them.
- **Find handles** are the address of `kernelbase`'s own find state (the
  open directory, the pattern, the last `NtQueryDirectoryFile` batch).
- **Named objects**: Poly has one session, so `Global\` and `Local\`
  both name `\BaseNamedObjects`; creation uses `OBJ_OPENIF`.
- **The SRW lock** is one word (bit 0 exclusive, bit 1 a writer waiting,
  the rest counting readers); waiters spin, then yield, then sleep 1 ms.
  It has no wait queue yet; it is fair to writers.
- **Critical sections** are `ntdll`'s `Rtl*CriticalSection`.
- **Local and global memory** come from the process heap; a moveable
  block is fixed (its handle is its address).
- **The version**: Learn: a program not manifested for Windows 8.1 or 10
  is told 6.2 by `GetVersion`/`GetVersionExW`. `kernelbase` reads the
  executable's `RT_MANIFEST` resource (PE format, `.rsrc`) for the
  `supportedOS` GUIDs Learn lists: Windows 10 → 10.0.19045 (as
  `RtlGetVersion` and the registry report), 8.1 → 6.3.9600, none →
  6.2.9200. `RtlGetVersion` always gives 10.0.19045.
- **`LoadLibraryExW`** passes the name to `LdrLoadDll` (a relative path
  made absolute first); its flags (`LOAD_LIBRARY_AS_DATAFILE`, the search
  flags) are ignored for now.
- **`MoveFileExW`** renames on one volume; `MOVEFILE_COPY_ALLOWED` across
  volumes and `MOVEFILE_DELAY_UNTIL_REBOOT` are not supported yet.

### Three new system services

`GetSystemTimeAsFileTime`, `GetTickCount(64)`, `QueryPerformanceCounter`
and `GetSystemInfo` need time and the processor count, which no service
gave. New table `SYSINFO_SERVICES` (`0x140`–`0x17F`, ADR 0063's
numbering) in `personalities/nt/src/sysinfo.rs`: `NtQuerySystemTime`,
`NtQueryPerformanceCounter` (the time since boot in 100 ns units, so a
fixed 10 MHz frequency, the value Learn's "Acquiring high-resolution time
stamps" gives for current Windows) and `NtQuerySystemInformation`
(`SystemBasicInformation` only, its documented `NumberOfProcessors`). Two
`NtKernel` methods with defaults (`monotonic_100ns`, `processor_count`)
are the kernel's side. `ntdll`'s stubs come from the table automatically.

## To be confirmed by the spec agent against real Windows

1. Each forwarded function's contract (the Learn "API set" rows as Poly
   reads them; a real `kernel32.dll`'s forwarder strings are export-table
   data the spec agent may compare), and which functions `kernel32` keeps
   (ours: the global-memory ones).
2. `GetFullPathNameW` on an empty string (ours: `ERROR_INVALID_NAME`),
   `\??\` paths given to Win32 functions (ours: passed through like
   `\\?\`), legacy device names with extensions (`CON.txt` → `\\.\CON`).
3. `FindFirstFileW` on a path ending in a separator (ours:
   `ERROR_FILE_NOT_FOUND`), the `*.`/`<`/`>` DOS wildcard translation
   (ours: only `*.*` → `*`), and whether `.`/`..` are reported.
4. `GetEnvironmentVariableW` of an empty value (ours: 0 with the last
   error cleared); deleting a variable that does not exist (ours:
   success); names with `=` (ours: `ERROR_INVALID_PARAMETER`).
5. `SetFilePointer` without the high DWORD past 4 GiB (ours:
   `ERROR_INVALID_PARAMETER`); `GetVersionExW` with a wrong size (ours:
   `ERROR_INSUFFICIENT_BUFFER`); the build numbers reported to
   unmanifested programs.
6. `FormatMessageW`: line widths other than 0 and `MAX_WIDTH_MASK` (ours:
   no wrapping), `IGNORE_INSERTS` with escapes (ours: everything copied),
   the languages served (ours: neutral and English), the exact texts.
7. `SYSTEM_INFO`'s processor level and revision (ours: 6 and 0); the
   performance counter's frequency on the oracle VM.
8. Standard handles of a process started without a console, and what
   `GetFileType` says for them.

## Consequences

- Build cost: a miss builds the four crates and `core`/`alloc` for the
  Windows target in about 20–30 s with ~150 MB of temporary artifacts,
  deleted afterwards; the cache keeps ~75 KB per version
  (`kernelbase.dll` ~60 KB, `kernel32.dll` ~11 KB, `k32test.exe`
  ~14 KB). A hit costs one hash of the inputs per kernel build.
- Host tests: `cargo test -p poly-kernelbase` (paths, environment, files,
  waits, the SRW lock across threads, memory flags, messages, version,
  manifests, the export table against `poly_ldr::apiset`), and
  `cargo test -p poly-personality-nt sysinfo` for the new services.
- The ring-3 test `nt::test_kernel32` runs `k32test.exe` from
  `C:\Program Files\PolyK32Test`: 187 checks of files and directories,
  the environment and command line, the last error after failing calls,
  heaps, local/global and virtual memory, a thread with events, a
  critical section and an SRW lock, mutexes, semaphores, named events,
  time, the version, `FormatMessageW` and modules (`GetProcAddress`
  through a forwarder returns `kernelbase`'s code), ending with
  `ExitProcess(0x4B)`.
- Edits to shared files are additive: the services table and dispatch,
  two `NtKernel` default methods, the kernel's two implementations, the
  layout's install loop, `kernel_cargo`, `ci`'s format step, and
  `ntdll.rs`'s hashing helpers made reusable (the `ntdll` cache key is
  unchanged).
- What is left of `kernelbase`/`kernel32` for the next items: the ANSI
  variants beyond the few here, `CreateProcessW` and process handles,
  overlapped I/O completion ports, `CreateFileMappingW`/`MapViewOfFile`,
  condition variables and `InitOnce`, thread-pool and fiber APIs, TLS
  (`TlsAlloc`), the NLS functions (`MultiByteToWideChar`, code pages,
  locales), console functions (with the console host), module resources
  (`FindResource`), `GetFileInformationByHandle(Ex)` and file times, and
  the hundreds of rarer exports in the coverage map.
- The coverage entries of the exports implemented here are claimed by
  `kernel32` in `kernel32.dll` and `kernelbase.dll`, plus the three new
  `ntdll` services; they turn green when the Windows oracle and held-out
  tests exist.
