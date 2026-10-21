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

## Second slice: processes, TLS, code pages, file mappings

**Status:** Accepted. Same sources and method as above, plus Learn's pages
of every function below, "Thread Local Storage", "Fiber Local Storage",
"Code Page Identifiers", "File Mapping", "Creating Named Shared Memory",
"File Times", "Condition Variables", "One-Time Initialization", and the
public headers `processthreadsapi.h`, `fibersapi.h`, `stringapiset.h`,
`winnls.h`, `memoryapi.h`, `synchapi.h`, `timezoneapi.h`, `minwinbase.h`
for constants and layouts. The code page tables are the public charts
(Unicode Consortium `CP1252.TXT` and `CP437.TXT`).

### Processes: `CreateProcessW` over `NtCreateUserProcess`

- **`kernelbase` decides what runs** (`poly_kernelbase::process`, host
  tested): `lpApplicationName` is taken as it is (made a full path, never
  searched, no extension added); otherwise the command line's first token,
  quoted or tried at each space in turn (Learn's `c:\program files\sub
  dir\program name` example), gets `.exe` when it has no extension and no
  path, and a name without a path is searched for in the application's
  directory, the current directory, `System32`, `System`, the Windows
  directory and `PATH`. Not found is `ERROR_FILE_NOT_FOUND`.
- **The child's parameters** are built by `kernelbase` in its own memory
  as an `RTL_USER_PROCESS_PARAMETERS`: `ImagePathName` (+ 0x60) and
  `CommandLine` (+ 0x70) where `winternl.h` puts them, Poly's own
  environment pointer and size (+ 0x80, + 0x88, ADR 0073) and, new, Poly's
  own **current directory** `UNICODE_STRING` at + 0x90. `lpEnvironment`
  NULL gives the creator's block; a given block is ANSI unless
  `CREATE_UNICODE_ENVIRONMENT`. `lpCurrentDirectory` NULL gives the
  creator's. A new process's `kernelbase` starts in the directory its
  parameters name, else (a process the kernel started) the executable's.
- **`NtCreateUserProcess`** (new service `0x08B`; the WDK does not
  document it, and `PS_CREATE_INFO` / `PS_ATTRIBUTE_LIST` are in no public
  header, so Poly uses only what it can define itself): its eleven
  arguments are the commonly known shape, but Poly reads only the two
  handle out-pointers, the two desired accesses (0 = all), `ThreadFlags`
  and `ProcessParameters`. The kernel turns `ImagePathName` into an NT
  name (`X:\...` → `\??\X:\...`, `\\?\` → `\??\`), opens it for execute
  and read as the caller (so a missing file is the usual
  `STATUS_OBJECT_NAME_NOT_FOUND`), reads it, and asks the kernel
  (`NtKernel::create_process`, a default method) for the process: a new
  address space, the image and our `ntdll` mapped, PEB, TEB, stack and the
  copied parameters (`exec::start_child`), handles to the process and its
  first thread opened **in the creator's table before the thread runs**
  (`exec::child_handles`), then the thread started at
  `LdrInitializeThunk` like any process (ADR 0071). The client id is read
  back with `NtQueryInformationProcess` / `NtQueryInformationThread`.
- **Exit statuses.** `NtProcess::exit_status` is now set: by the first
  `NtTerminateProcess` of the process, else by its last thread's end.
  When a process ends, `NtSystem::process_exits` keeps its status while
  its process object lives (another process holds a handle), and
  `NtQueryInformationProcess(ProcessBasicInformation)` of another process
  reports it (`STATUS_PENDING`, i.e. `STILL_ACTIVE`, while it runs);
  the entry goes when the object is deleted (`NtSystem::reap`). The
  process object is signalled at exit (already so), which is what
  `WaitForSingleObject` on the handle waits for.
- **Not yet:** `CREATE_SUSPENDED` (`STATUS_NOT_SUPPORTED` /
  `ERROR_NOT_SUPPORTED`: resuming another process's thread needs
  cross-process suspension), handle inheritance (`bInheritHandles` is
  ignored; a child gets the console like every Poly NT process),
  `STARTUPINFO`'s fields, security attributes, `CreateProcessA`, process
  names in the object namespace, `TerminateProcess` of another process.

### Thread-local and fiber-local storage

TLS values live where `winternl.h` puts them: `TEB.TlsSlots[64]` at
TEB + 0x1480 and `TlsExpansionSlots` at TEB + 0x1780 (1,024 more, from the
process heap on a thread's first store; Learn: 1,088 indices).
`kernelbase` tracks the TEBs of the process's threads through its
`DllMain` (`DLL_PROCESS_ATTACH`, `DLL_THREAD_ATTACH`/`DETACH`, which our
`ntdll` sends) so `TlsFree` zeroes the index in every thread and a reused
index starts at zero. `TlsGetValue` clears the last error on success
(Learn). FLS: no fibers yet, so FLS values are per thread, kept by
`kernelbase`; a thread's callbacks run at its `DLL_THREAD_DETACH`,
`FlsFree` runs the index's callback for every thread's non-null value.

### Code pages and strings

`poly_kernelbase::nls`: code pages 1252 (`CP_ACP`, `CP_THREAD_ACP`), 437
(`CP_OEMCP`, "OEM United States") and UTF-8; `MultiByteToWideChar` and
`WideCharToMultiByte` with their documented flag checks
(`ERROR_INVALID_FLAGS`), the `-1` length convention, the size query and
`ERROR_INSUFFICIENT_BUFFER`, `MB_ERR_INVALID_CHARS` /
`WC_ERR_INVALID_CHARS` (`ERROR_NO_UNICODE_TRANSLATION`), U+FFFD for
invalid input otherwise, the default character and `lpUsedDefaultChar`
for the single-byte pages; `GetACP` (1252), `GetOEMCP` (437),
`GetCPInfo`, `IsValidCodePage`, `CompareStringOrdinal` (upcased with the
object manager's table for `bIgnoreCase`). `lstrlenW`, `lstrlenA`,
`lstrcmpW`, `lstrcmpiW` are `kernel32`'s own (their Learn pages name no
API set), compared ordinally.

### File mappings

`CreateFileMappingW`, `OpenFileMappingW`, `MapViewOfFile(Ex)`,
`UnmapViewOfFile`, `FlushViewOfFile` over the section services
(`poly_kernelbase::mapping` splits `flProtect` into the page protection
and `SEC_*` attributes, maps `FILE_MAP_*` to a view protection, checks the
64 KiB offset alignment: `ERROR_MAPPED_ALIGNMENT`). What the NT side
gained:

- **Sections of files of NT file I/O** (ADR 0067's file objects; until
  now only ADR 0063's minimal file objects could back a section). The
  file object must have been opened to read its data, and to write it for
  a section that writes shared pages (`STATUS_ACCESS_DENIED`, Learn's
  table of `flProtect` against the file's access). A writable section
  larger than the file grows the file (Learn: "the file on disk is
  increased"); otherwise a larger size is `STATUS_SECTION_TOO_BIG`. A data
  section's pages are the VFS file's own (`SectionSource::Vfs`: the
  kernel maps it with the `FileSource` a shared `mmap(2)` uses, so on a
  cached file system the views share the page cache's pages and a write
  through a view is the file's contents, seen by `ReadFile`).
- **`NtFlushVirtualMemory`** (new service `0x048`): checks that the range
  is in one view (`STATUS_NOT_MAPPED_VIEW`), rounds it to pages and
  returns it. The data is already in the file's cached pages; the page
  cache's writeback puts it on disk.

### Condition variables, one-time initialisation, file information, times

- A `CONDITION_VARIABLE` is a wake count: a sleeper notes it, releases
  its critical section or SRW lock, waits (the SRW lock's back-off) until
  it moves or the timeout passes (`ERROR_TIMEOUT`), and takes the lock
  again. Both wake functions move it, so `WakeConditionVariable` may wake
  every sleeper, which Learn's "spurious wakeups" allow.
- `INIT_ONCE`: bit 0 running, bit 1 done, the context above the two
  reserved bits. `InitOnceExecuteOnce`, `InitOnceBeginInitialize`,
  `InitOnceComplete` (synchronous only: `INIT_ONCE_ASYNC` is
  `ERROR_INVALID_PARAMETER`), `InitOnceInitialize`. A failed callback lets
  the next caller try again.
- `GetFileInformationByHandle` from the basic, standard and internal
  information and the volume's serial number; `GetFileTime`,
  `SetFileTime` (a null time is left alone); `FileTimeToSystemTime`,
  `SystemTimeToFileTime` (the Gregorian calendar from 1601, fields
  checked), `CompareFileTime`, `GetSystemTime`, `GetLocalTime`,
  `FileTimeToLocalFileTime`, `LocalFileTimeToFileTime`: Poly's local time
  is UTC until time zones exist.

### To be confirmed by the spec agent (second slice)

1. Every new forwarder's contract (as above): especially the condition
   variable and `InitOnce*` functions (ours: `api-ms-win-core-synch-l1-2-0`),
   `FileTimeToSystemTime`/`SystemTimeToFileTime`
   (`api-ms-win-core-timezone-l1-1-0`), and that the `lstr*` functions
   stay in `kernel32`.
2. `CreateProcessW`: the search order for a command line whose first
   token has a path but no extension (ours: no `.exe` added, as Learn
   says); what an empty environment block may look like; the error for a
   directory given as the program (ours: `ERROR_ACCESS_DENIED`).
3. `TlsGetValue` of an in-range index that is not allocated (ours: its
   slot, no error); `FlsGetValue` clearing the last error (ours: yes);
   when FLS callbacks run, and `FLS_MAXIMUM_AVAILABLE` (ours: 4080).
4. Code pages: best fit (ours: none, the default character), the five
   undefined 1252 bytes under `MB_ERR_INVALID_CHARS` (ours: valid, C1
   controls), how many U+FFFD an invalid UTF-8 sequence gives (ours: one
   per maximal invalid subpart), `lstrcmp(i)W`'s order (ours: ordinal, not
   the locale's linguistic sort).
5. `InitOnceBeginInitialize` with `INIT_ONCE_CHECK_ONLY` before completion
   (ours: FALSE, `ERROR_GEN_FAILURE`).
6. `FlushViewOfFile` (ours: returns once the range is checked, without
   waiting for the disk); `NtFlushVirtualMemory`'s `IoStatusBlock`.
7. The local time zone of the oracle machine (ours: UTC).

### Consequences (second slice)

- Host tests: `cargo test -p poly-kernelbase` (`nls`, `tls`, `process`,
  `mapping`, `time`, condition variables and `InitOnce` in `sync`, file
  information in `file`), `cargo test -p poly-personality-nt` (a child
  created through the service with its parameters read back, exit status
  and waits, the failure cases, sections of files that map their contents
  and grow them, `NtFlushVirtualMemory`).
- The ring-3 test (`k32test.exe`) runs itself twice as a child: once with
  an explicit environment, current directory and quoted command line,
  once found by search with what it inherits, and checks
  `GetExitCodeProcess` (`STILL_ACTIVE`, then the full 32-bit code),
  `GetProcessId` and the waits; plus TLS (expansion slots, another
  thread, reuse), FLS callbacks, conversions, mappings of a file (grown,
  written through a view, read back with `ReadFile`) and of the paging
  file (named, two views), condition variables with a thread, `InitOnce`,
  file information and times.
- Shared files changed (additively): the service table and dispatch, two
  `NtKernel` methods' neighbours (`create_process` is a default method),
  `NtSystem` (`process_exits`), `params` (the current directory),
  `SectionSource` (`Vfs`), the kernel's `start_with` (now over
  `start_params`), `backend.rs`, and `KERNEL32_FORWARDS` / `KERNEL32_OWN`.
- **Left** of `kernelbase`/`kernel32` after this slice: `CREATE_SUSPENDED`
  and handle inheritance in `CreateProcessW`, `CreateProcessA`, the other
  ANSI variants, overlapped I/O completion ports, thread pools and fibers,
  double-byte code pages, locales (`GetLocaleInfoEx`, linguistic
  `CompareStringEx`), time zones, console functions (with the console
  host, ADR 0080), module resources, `GetFileInformationByHandleEx`, and
  the rarer exports in the coverage map.
- The new entries are claimed by `kernel32-2` in `kernel32.dll` and
  `kernelbase.dll`, plus `NtCreateUserProcess` and `NtFlushVirtualMemory`
  (and their `Zw` names) in `ntdll.dll`.
