# Chrome S2: what the sandbox's interception needs from Poly (S2c)

Spec agent `spec-chrome-win-s2c`, 2026-11-15, for implementer `impl-chrome-s2`. Inputs: Chrome for Testing
157.0.8103.38 (the pinned package) running on Windows (this PC, and homeserver2 session 0 for the API tests) and on
Poly (experiment builds of `impl-chrome-s2-r2` b184ce5a with local changes that are **not** committed anywhere);
Poly's own call tracer (`--kit chrome_elf.dll,chrome.exe`) and kernel logs (`--exceptions`); export names and
section headers of Windows DLLs read as data; strings and data sections of Chrome (a third-party program) read
as data. **No Microsoft or Google binary was decompiled or disassembled, no Microsoft code bytes were read or
recorded** (the stubs varied below are Poly's own; the tests rewrite only their own code). Nothing of Chrome is
committed.

## 1. The null-pointer call at `chrome.exe+0x30ea5d`: cause

* At the fault `rcx` = `0x18027eab9` and `rdx` = `0x140364188`. Read as data: `rcx` is the export directory's
  name of the just-mapped `chrome_elf.dll` (`"chrome_elf.dll"`, RVA 0x27eab9), `rdx` is the text `"kernel32.dll"`
  in chrome.exe's `.rdata`. The target compares the new module's name with `kernel32.dll` through a function
  pointer that is 0.
* chrome.exe's data holds the names of the ntdll functions its sandbox code uses: `NtClose`,
  `NtAllocateVirtualMemory`, `NtCreateFile`, `NtCreateSection`, `NtDuplicateObject`, `NtFreeVirtualMemory`,
  `NtMapViewOfSection`, `NtOpenThread`, `NtOpenProcessTokenEx`, `NtProtectVirtualMemory`, `NtQueryAttributesFile`,
  `NtQueryFullAttributesFile`, `NtQueryObject`, `NtQueryVirtualMemory`, `NtSetInformationFile`,
  `NtSignalAndWaitForSingleObject`, `NtUnmapViewOfSection`, `NtWaitForSingleObject`, `NtQuerySection`,
  `RtlAllocateHeap`, `RtlAnsiStringToUnicodeString`, `RtlCompareUnicodeString`, `RtlCreateHeap`, `RtlDestroyHeap`,
  `RtlFreeHeap`, `RtlNtStatusToDosError`, **`_strnicmp`, `strlen`, `wcslen`**, `memcpy`. Poly's ntdll exports all of
  them except the three C functions in bold; Windows' ntdll exports all of them.
* How the value gets there (Poly's tracer, broker side): before a target runs, the broker copies a **248-byte
  block from its own `chrome.exe+0x3a79e0` to the same address in the target** (`WriteProcessMemory(target,
  0x1403a79e0, 0x1403a79e0, 248)`): a flag (1) and the addresses of those ntdll functions as the *broker*
  resolved them. A name Poly's ntdll lacks is a 0 in that table. (It also copies other variables of chrome.exe's
  `.data` the same way: 160 bytes at +0x3abc50, 8 at +0x3a79d8, 4 at +0x389e88, 8 at +0x3abac0/+0x3abac8/+0x3aba68,
  5 at +0x3abab8, ...) This only works because **ntdll and chrome.exe are at the same address in the broker and in
  every target** (Windows: one base per image per boot; the sheet `ntdll.dll/NtMapViewOfSection`, rule
  `image_other_process`; Poly: the preferred bases).
* Experiment on Poly (run 1): with `_strnicmp`, `strlen` and `wcslen` exported by Poly's ntdll the null call is
  gone in every sandboxed child.

**Requirement:** ntdll exports `_strnicmp`, `strlen` and `wcslen` as code (sheets `ntdll.dll/_strnicmp`,
`ntdll.dll/strlen`, `ntdll.dll/wcslen`, with tests: the exact values `_strnicmp` returns, ASCII-only folding to lower
case, NULL behaviour).

## 2. What comes next (found by running on with the fix)

| Run | Change on top of b184ce5a (local only) | What the sandboxed children did |
|---|---|---|
| 1 | ntdll exports `_strnicmp`, `strlen`, `wcslen` | 13 children fault **executing** `KERNEL32.dll+0xfdc4` (DEP), called from chrome.exe+0x311167 (below chrome.exe+0x300c85) |
| 3 | + `kernel32!CreateThread` as code instead of a forwarder | no sandboxed child faults any more; the GPU process and the network service run for the whole run |
| 4 | + `--enable-logging=stderr` | same; the children's own logs do not reach stderr |

* `KERNEL32.dll+0xfdc4` is the export entry of `CreateThread` in Poly's kernel32: a **forwarder** whose text is
  `api-ms-win-core-processthreads-l1-1-0.CreateThread`. Chrome's target code takes CreateThread's address from
  kernel32's export table itself and jumps into the forwarder's text. On Windows `kernel32!CreateThread` is code
  in kernel32 (an RVA in `.text`, `PAGE_EXECUTE_READ`; GetProcAddress gives the same address). **Requirement:**
  sheet `kernel32.dll/CreateThread` (one test, `k32_export_code`). Poly's kernel32 forwards 385 names; chrome.exe
  contains 180 kernel32 export names that are code on Windows and forwarders on Poly (risk list in section 7); only
  CreateThread is known to matter.
* **Still open after run 3/4 (not specified here):**
  1. Some children (one per run: the storage service or a renderer) stop at a handled breakpoint
     `0x80000003` at `chrome.dll+0x420b2` right after `LdrLoadDll(msdia140.dll)` fails (that happens on every
     stack-trace print). This was already in the implementer's last run (pid 69) and is independent of the
     interception. Its reason is a Chrome CHECK/dump request whose text the run cannot show (see 3).
  2. The **crash handler** (`chrome.exe --type=crashpad-handler`, the process without chrome.dll) then calls
     NULL: it resolves `ntdll!RtlGetUnloadEventTraceEx` by name (`LdrGetProcedureAddress` fails with 0xC000007A on
     Poly) and calls the result; Windows' ntdll exports it (Learn documents it: `RtlGetUnloadEventTraceEx`). The
     handler restarts once and dies the same way; the browser then hits its own breakpoint
     (`chrome.dll+0xc62f809`) and ends with `crash server did not respond, self-terminating`, exit `0xffff7001`.
     Next spec: `RtlGetUnloadEventTraceEx` (and, with the handler alive, the dump of the child shows what its
     CHECK was).

## 3. Question 1: what shape Poly's ntdll stubs must have

What the broker does per target (Poly tracer, every child type): `ReadProcessMemory(stub, 32)`,
`WriteProcessMemory(thunk + 0x40·i, the 32 bytes)`, `VirtualProtectEx(stub, 16, PAGE_WRITECOPY)`,
`WriteProcessMemory(stub, 16)`, `VirtualProtectEx(stub, 16, PAGE_EXECUTE_READ)`. The thunk page is
`VirtualAllocEx(NULL, 64 KiB, MEM_RESERVE, PAGE_NOACCESS)` + `VirtualAllocEx(reservation + 0x7000..0xd000, 4 KiB,
MEM_COMMIT, PAGE_EXECUTE_READWRITE)`. The functions patched (not the list in the S2-r2 hand-off, which named
offsets of another ntdll layout):

| Target | Functions whose stubs are read and rewritten, in order |
|---|---|
| GPU process | NtOpenThread, NtOpenProcess, NtOpenProcessToken, NtSetInformationThread, NtOpenThreadToken, NtOpenProcessTokenEx, NtOpenThreadTokenEx, NtMapViewOfSection, NtUnmapViewOfSection |
| network service, storage service | NtOpenThread, NtOpenProcess, NtOpenProcessToken, NtSetInformationThread, NtOpenThreadToken, NtOpenProcessTokenEx, NtOpenThreadTokenEx |
| renderers | NtCreateSection, then the nine of the GPU process |

When it refuses a stub it reads it, calls `GetModuleHandleW("ntdll.dll")`, `SetLastError(183)` and
`TerminateProcess(target, 1)` without writing anything (the "last error 183" of the S2-r2 hand-off).

Black-box variants of Poly's own stubs (each a build of Poly's ntdll; `syscall` = `0F 05`, `ret` = `C3`):

| Variant | Stub (Poly's) | Broker | Evidence |
|---|---|---|---|
| baseline (b184ce5a) | `4C 8B D1` + `mov eax, n` (`B8` imm32) + `syscall` + `ret`, 32-byte slots, NOP filler | accepted | S2-r2 runs, run 3 |
| implementer's earlier | `49 89 CA` (the other `mov r10, rcx` encoding) + ... | refused | S2-r2 hand-off |
| implementer's earlier | baseline spelling, stubs 11 bytes apart | the stub after a patched one is refused (its start was overwritten) | S2-r2 hand-off |
| A | baseline, **16-byte** slots | accepted, children run as with 32 | run `va` |
| B | `mov eax, n` **first**, then `4C 8B D1`, `syscall`, `ret` | **refused** at the first stub of every target | run `vbt` (trace) |
| C | baseline + 21 bytes of `CC` after `ret` (on NtMapViewOfSection) | accepted | run `vc` (trace) |
| E | baseline with a `nop` between `syscall` and `ret` (on NtOpenThread) | **refused** (every target, at NtOpenThread / after NtCreateSection) | run `vd` (trace) |
| F | `4C 8B D1`, `mov eax, n` in its other encoding (`C7 C0` imm32), `syscall`, `ret` (on NtCreateSection) | **refused** (every renderer, at its first stub) | run `ve` (trace) |
| D | `4C 8B D1`, `B8` imm32, then a relative `jmp` to a shared `syscall; ret` (on NtOpenThread) | **refused** (every non-renderer target) | run `ve` (trace) |

**Requirements found** (for the 10 functions above at least):

1. **Bytes 0..10 of the stub are exactly `4C 8B D1`, `B8` + the 32-bit system-call number, `0F 05`, `C3`** (mov r10, rcx
   in that encoding; mov eax, imm32 in the `B8` encoding; syscall; ret). Every other arrangement tried was refused:
   the other encoding of either `mov`, the two `mov`s swapped, a `nop` before `ret`, a jump instead of `syscall`.
   The number itself can be any value (Poly's own numbers were accepted).
2. The stub owns **at least 16 bytes**: the broker overwrites bytes 0..15; anything another function needs must
   not be in them. 32-byte slots are not needed (16 work); bytes 11..31 (filler `90`/`CC` or the next stub) are read
   and copied but not checked.
3. The 32 bytes are copied to the thunk page and the copy runs there in place of the original. With rule 1 the
   first 11 bytes are position-independent by construction; a stub that needed its own address (a relative branch
   out of it) is refused anyway (variant D).
4. Each stub is checked when it is patched, so the patch of one must not change the next (rule 2).
5. Same address in every process (ntdll's fixed base), as the broker reads in its own process what it writes in
   the target (section 1).

Not tested: whether the broker also accepts other, longer stub forms (that would need Windows' own stub bytes,
which this track did not record: owner's rule). Poly's baseline already satisfies 1-5; nothing in Poly's stubs needs
to change, only the sheets' rules should keep it so.

## 4. Question 2: PEB + 0x2d8

* winternl.h lays the PEB out up to `SessionId` at 0x2c0 (x64); 0x2d8 has no public name.
* **Windows, any normal process** (the tests' own, cmd, PowerShell, Chrome's browser, crash handler, GPU process,
  network service): a pointer to a committed **private read-write region of 8 KiB** of its own (allocation base =
  the pointer, zero-filled at the start), already there in a just-created suspended process.
* **Chrome's renderers and storage service on Windows: 0.** On Poly the tracer shows the broker writing **8 zero
  bytes** at `PEB + 0x2d8` of exactly those targets (storage service, renderers; not the GPU process or the network
  service), before they run, right after `NtQueryInformationProcess(ProcessBasicInformation)`.
* **What the child does with it:** nothing a test could see: a child whose parent wrote 0 there starts, loads a
  DLL, reads 0 there all its life and exits normally (test `pbi_peb`). Poly only needs a writable PEB at that offset
  and must not depend on the value. Sheet: `ntdll.dll/NtQueryInformationProcess`.

## 5. Question 3: `NtMapViewOfSection` of `chrome_elf.dll` in a target

* On Windows chrome_elf.dll (like chrome.exe and chrome.dll) is at the **same base in the browser and in every
  child** (read from outside in 10 processes), size 0x2f0000 = its SizeOfImage.
* The first view of an image section in a process, at the base the section chose: **STATUS_SUCCESS**, view size =
  SizeOfImage rounded to pages, already relocated (the header's ImageBase equals the base); a second view in the
  same process: `STATUS_IMAGE_NOT_AT_BASE` (0x40000003), not relocated; a view in another process: STATUS_SUCCESS at
  the same base. So Poly's 0 for chrome_elf.dll in the target matches Windows; the null call was not caused by
  the status. Sheet: `ntdll.dll/NtMapViewOfSection` (new `image_*` rules, test `image_views`).

## 6. Cross-process memory the broker relies on (sheets with tests)

* `kernel32.dll/VirtualProtectEx`: `PAGE_WRITECOPY` on an image code page (old 0x20), a write makes it a private
  `PAGE_READWRITE` page that stays `MEM_IMAGE`, back to `PAGE_EXECUTE_READ` (old 0x04); the whole broker sequence
  on the test's own stub, the child runs the rewritten code and the copy; `PAGE_READWRITE` on an image page is
  reported `PAGE_WRITECOPY`; **a target with the prohibit-dynamic-code policy does not stop any of it when done from
  outside** (Poly's ADR 0196 rule refuses executable protections only for the process itself: keep it that way).
* `kernel32.dll/WriteProcessMemory`: writes into `PAGE_EXECUTE_READ` pages without the caller changing the
  protection (NtWriteVirtualMemory does not: STATUS_PARTIAL_COPY), refuses read-only/no-access pages (998),
  partial copies (299 with the count).
* `kernel32.dll/VirtualAllocEx`: the 64 KiB reservation + one executable page at +0x7000, page rounding, the region
  layout VirtualQueryEx reports, refusals.

## 7. kernel32 forwarders (risk list)

Export names in chrome.exe's data that are **code** in Windows' kernel32 and **forwarders** in Poly's (b184ce5a):
AreFileApisANSI, CloseHandle, CompareStringEx, CompareStringW, ConnectNamedPipe, CreateDirectoryW, CreateEventW,
CreateFileMappingW, CreateFileW, CreateIoCompletionPort, CreateJobObjectW, CreateMutexW, CreateNamedPipeW,
CreateProcessW, CreateSemaphoreW, **CreateThread**, DeleteFileW, DisconnectNamedPipe, DuplicateHandle, ExitProcess,
ExpandEnvironmentStringsW, FileTimeToSystemTime, FindClose, FindFirstFileExW, FindFirstFileW, FindNextFileW,
FlsAlloc, FlsFree, FlsGetValue, FlsSetValue, FlushFileBuffers, FlushViewOfFile, FormatMessageA, FormatMessageW,
FreeEnvironmentStringsW, FreeLibrary, GetACP, GetCPInfo, GetCommandLineA, GetCommandLineW, GetComputerNameExW,
GetConsoleMode, GetConsoleOutputCP, GetConsoleScreenBufferInfo, GetCurrentDirectoryW, GetCurrentProcess,
GetCurrentProcessId, GetCurrentThread, GetCurrentThreadId, GetEnvironmentStringsW, GetEnvironmentVariableW,
GetExitCodeProcess, GetFileAttributesExW, GetFileAttributesW, GetFileInformationByHandle,
GetFileInformationByHandleEx, GetFileSizeEx, GetFileTime, GetFileType, GetFullPathNameW, GetHandleInformation,
GetLastError, GetLocalTime, GetLocaleInfoEx, GetLocaleInfoW, GetLogicalProcessorInformation, GetLongPathNameW,
GetModuleFileNameW, GetModuleHandleA, GetModuleHandleExW, GetModuleHandleW, GetNamedPipeClientProcessId,
GetNativeSystemInfo, GetOEMCP, GetProcAddress, GetProcessHandleCount, GetProcessHeap, GetProcessId,
GetProcessTimes, GetProductInfo, GetQueuedCompletionStatus, GetStartupInfoW, GetStdHandle, GetStringTypeW,
GetSystemDefaultLCID, GetSystemDirectoryW, GetSystemInfo, GetSystemTimeAsFileTime, GetSystemTimePreciseAsFileTime,
GetTempPathW, GetThreadId, GetTickCount, GetTimeZoneInformation, GetUserDefaultLCID, GetUserDefaultLangID,
GetUserDefaultLocaleName, GetVersionExW, GetWindowsDirectoryW, GlobalMemoryStatusEx, HeapDestroy,
HeapSetInformation, InitializeCriticalSectionEx, IsDebuggerPresent, IsProcessorFeaturePresent, IsValidCodePage,
IsValidLocale, IsValidLocaleName, IsWow64Process, LCMapStringEx, LCMapStringW, LoadLibraryExA, LoadLibraryExW,
LoadLibraryW, LocalFree, LockFileEx, MapViewOfFile, MoveFileW, MultiByteToWideChar, OpenProcess,
OutputDebugStringA, PeekNamedPipe, PostQueuedCompletionStatus, QueryInformationJobObject, QueryPerformanceCounter,
QueryPerformanceFrequency, RaiseException, ReadConsoleW, ReadFile, ReadProcessMemory, RegisterWaitForSingleObject,
ReleaseMutex, ReleaseSemaphore, RemoveDirectoryW, ReplaceFileW, ResetEvent, ResumeThread, RtlCaptureContext,
RtlLookupFunctionEntry, RtlPcToFileHeader, RtlUnwindEx, RtlVirtualUnwind, SetConsoleCtrlHandler, SetConsoleMode,
SetCurrentDirectoryW, SetEndOfFile, SetEnvironmentVariableW, SetEvent, SetFileAttributesW,
SetFileInformationByHandle, SetFilePointerEx, SetHandleInformation, SetInformationJobObject, SetLastError,
SetNamedPipeHandleState, SetProcessShutdownParameters, SetStdHandle, SetThreadAffinityMask,
SetUnhandledExceptionFilter, Sleep, SleepEx, SuspendThread, SwitchToThread, SystemTimeToTzSpecificLocalTime,
TerminateJobObject, TerminateProcess, TlsAlloc, TlsFree, TlsGetValue, TlsSetValue, TransactNamedPipe,
UnhandledExceptionFilter, UnlockFileEx, UnmapViewOfFile, UnregisterWaitEx, VerifyVersionInfoW, VirtualAlloc,
VirtualAllocEx, VirtualFree, VirtualFreeEx, VirtualProtect, VirtualProtectEx, VirtualQuery, VirtualQueryEx,
WaitForMultipleObjects, WaitForSingleObject, WaitNamedPipeW, WideCharToMultiByte, WriteConsoleW, WriteFile,
WriteProcessMemory. (Most of these are imports, which the loader resolves through the forward; only a program that
walks the export table itself is hurt.) Windows' own kernel32 has 211 forwarders (77 to NTDLL, 52 to kernelbase,
the rest to api-ms-win-* contracts).

## 8. Reproducing

* The experiments: a detached worktree at `origin/impl-chrome-s2-r2` (b184ce5a) with (a) `strlen`, `wcslen`,
  `_strnicmp` added to ntdll (`/EXPORT` in ntdll's `build.rs`, a small module), (b) CreateThread left out of
  kernel32's forward list and implemented in kernel32 by calling kernelbase's (and the forward check in
  `xtask win32.rs` told to skip it), (c) the stub variants in ntdll's `build.rs` (section 3). Then
  `cargo xtask chrome-win run --stage 2 --release --exceptions` (exceptions), or with
  `--launch-seconds 60 --kit chrome_elf.dll,chrome.exe --trace-only dump-dom` (the browser's own calls in
  `target/chrome-win/stage2.ptrace.jsonl`; the launcher writes the trace only if it ends Chrome itself).
  The mapping from `ntdll.dll+offset` to names needs the ntdll of that build (`target/ntdll-cache/` named in the run's
  output; the cache is shared between worktrees).
* Windows, Chrome's processes: start `chrome.exe --headless --user-data-dir=<new> about:blank`, wait 12 s, read each
  `chrome.exe` process's PEB (`NtQueryInformationProcess` class 0), 8 bytes at PEB + 0x2d8, `VirtualQueryEx` of what
  they point to, and the modules' bases (PowerShell as the same user; no elevation was needed).
