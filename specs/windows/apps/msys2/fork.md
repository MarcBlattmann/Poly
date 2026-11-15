# MSYS2 runtime: what `sh.exe -c "/usr/bin/true; echo hi"` does on Windows (observation note)

Observed by spec agent `spec-win-apps-4`, 2026-11-15. Behaviour only: the calls of the unmodified program into
`kernel32` and `ntdll` as the repository's call tracer (`cargo xtask win trace-kit`, ADR 0090) records them, plus
test programs of our own. Nothing of MSYS2 or Windows was disassembled or read. MSYS2/Git binaries are not in the
repository.

## Setup

- PortableGit 2.56.1 (SHA-256 prefix `9dd04a60`), `usr\bin\sh.exe` and `usr\bin\msys-2.0.dll`. The kit was built with
  `trace-kit sh.exe msys-2.0.dll`: both files get their `kernel32`/`ntdll`/`user32` imports renamed to proxies; the
  runtime's own `LoadLibrary`/`GetProcAddress` calls are logged, but the functions it then calls through those pointers
  (the whole of `advapi32`, for example) are not.
- Launcher: our own program, `CreateProcessW` with `STARTF_USESTDHANDLES`, stdin = `NUL`, stdout and stderr = files
  (inheritable handles), working directory = the PortableGit root.
- Run on this PC (interactive session) and on homeserver2 in session 0 (SSH). Both: exit code 0, stdout `hi`, stderr
  empty, and the **same call order** in the parent process before and around the first thread creation (5242 calls
  recorded on the PC, 3666 on homeserver2: the numbers differ by environment size, not by sequence).

## Which call creates the signal thread, and what precedes it

`sh.exe` (the parent, thread 0) makes this sequence at start-up, identical on both machines (seq numbers of the homeserver2 run):

| seq | call | answer |
|---|---|---|
| ~190 | `NtQueryInformationProcess(NtCurrentProcess, 0x18 = ProcessSessionInformation, buf, 4)` | `STATUS_SUCCESS` (the session id) |
| 230 | `CreateNamedPipeA(name "\\.\pipe\msys-<id>-<pid>-sigwait", PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE (0x80001), PIPE_TYPE_MESSAGE | PIPE_REJECT_REMOTE_CLIENTS (0xc), 1 instance ...)` | a handle |
| 231 | `CreateFileA(same name, 0x40000080, share 0 ...)` (the client end) | a handle |
| 232-239 | four times `NtCreateEvent(... EVENT_ALL_ACCESS, ... )` followed by `SetLastError(0)` | `STATUS_SUCCESS` |
| 240 | `GetCurrentThread()` | the pseudo-handle -2 |
| 241 | `QueueUserAPC(routine, pseudo-handle of the calling thread, parameter)` | **1 (TRUE)** |
| 242 | `AddVectoredContinueHandler(0, handler)` | a non-NULL handle |
| 243 | `TlsAlloc()` | 2 |
| 244 | **`CreateThread(attributes, 0, start routine, parameter, 0, &tid)`** | a handle; `CloseHandle` of it follows at once |

So the second thread of `sh.exe` is made by `kernel32!CreateThread` (not `NtCreateThreadEx` from `sh.exe`'s own code), and it is made at
**start-up of every MSYS2 process**, long before any fork: the child `true.exe` makes the same call (its own trace: one
`CreateThread`, two `OpenThread`). The calls just before it are the ones in the table: the pipe pair, four events, a
queued APC to the caller's own thread (the pseudo-handle; the routine only runs when that thread waits alertably, see the
`QueueUserAPC` sheet), a vectored continue handler and a TLS slot.

The new thread (thread 1):

1. `NtQueryInformationThread(pseudo-handle -2, 9 = ThreadQuerySetWin32StartAddress, buf, 8)` -> success;
2. `TlsSetValue(2, ...)`, `TlsGetValue(2)`, `GetCurrentThreadId`;
3. `OpenThread(0x400 = THREAD_SET_LIMITED_INFORMATION, FALSE, its own id)` -> a handle; `MultiByteToWideChar`;
   `GetProcAddress(kernelbase, "SetThreadDescription")` found; `CloseHandle`;
4. `IsDebuggerPresent` (FALSE), `GetModuleHandleA("ntdll.dll")`, `GetTickCount`;
5. **`ReadFile(sigwait pipe server end, buffer, 192 bytes)`** and from then on the loop: every 192-byte record the
   other threads write to the client end (`sig_send`, under the mutex `sig_send.<pid>`) is read here; after each record
   it makes `AcquireSRWLockExclusive`/`ReleaseSRWLockExclusive` calls on one lock, `SetEvent` on two or three of the
   events made above, and reads again (`PeekNamedPipe` is used to see whether more is waiting).

This is the thread whose absence on Poly makes the parent wait forever: the parent writes its 192-byte record (to
tell the signal code about the child) and waits on an event the signal thread would set; with no thread nobody reads
the pipe.

## The rest of the fork path (parent, thread 0)

- Before fork, `GetThreadPriority(GetCurrentThread())` -> 0, `GetPriorityClass(NtCurrentProcess)` -> 0x20 (three times), `SetThreadPriority(duplicated handle of its own thread, 0)` -> TRUE.
- `CreateProcessW(...)` of the runtime's own image (seq ~4877 on the PC) -> TRUE, then `WaitForMultipleObjects(2, handles, FALSE, 300000)` -> 0;
  a **second** `CreateThread` with the same start routine follows after another `CreateFileA` (the thread that waits for the child and
  reports its exit), then a long wait (`WaitForMultipleObjects(2, ..., INFINITE)` in the main thread).
- `NtQueryInformationProcess` classes 4 and 3 at exit (times and VM counters), `DuplicateHandle(NtCurrentProcess, ... )` calls with `DUPLICATE_SAME_ACCESS`.

## Answers to the candidate calls of the hand-off (all recorded by test programs, sheets under `specs/windows/`)

| Call | Windows' answer |
|---|---|
| `NtQueryInformationProcess(NtCurrentProcess, 24, 4 bytes)` | `STATUS_SUCCESS`, the session id, returned length 4; any other length: `STATUS_INFO_LENGTH_MISMATCH` (Poly: `STATUS_INVALID_INFO_CLASS`) |
| `NtOpenThread(0x400)` then `NtQueryInformationThread(ThreadBasicInformation)` | the open succeeds, the query gives **`STATUS_ACCESS_DENIED` on Windows too** (class 0 needs `THREAD_QUERY_INFORMATION` or `THREAD_QUERY_LIMITED_INFORMATION`): this is not a difference of Poly |
| `GetThreadPriority(GetCurrentThread())`, `GetPriorityClass(GetCurrentProcess())` | 0 and 0x20; `SetThreadPriority(handle, 0)` -> TRUE (`GetThreadPriority` sheet) |
| `advapi32!LsaOpenPolicy(NULL, oa, POLICY_VIEW_LOCAL_INFORMATION, &h)` | `STATUS_SUCCESS`; the queries the runtime needs (`PolicyDnsDomainInformation`, `PolicyAccountDomainInformation`) succeed with buffers (`LsaOpenPolicy` sheet). The runtime reaches advapi32 through `LoadLibrary` + `GetProcAddress`, so its calls are not in the trace |
| `ntdll!RtlGetCurrentDirectory_U` | an ntdll export; byte length without the NUL, nothing written when the buffer cannot hold text + NUL (sheet) |
| `QueueUserAPC(routine, GetCurrentThread(), p)` | TRUE; the routine runs at the next alertable wait of that thread (sheet) |
| `AddVectoredContinueHandler` | a handle; continue handlers run after an exception handler that continued; never when no handler resolves the exception (sheet) |

## `.cygwin_dll_common` (an `IMAGE_SCN_MEM_SHARED` section) across processes

A test program (`shared_section`, in the kit; it is not a spec test because Poly's loader is not asked to do this by any entry yet) whose image has a
section with the shared flag (`/SECTION:.shr,RWS`): the parent writes 0x1234, starts its own image again, the child reads **0x1234**,
writes 0xB0B0, and after the child ended the parent reads **0xB0B0**. So on Windows a shared image section is **one copy for all processes
that run that image** (and, for a DLL, that load it), the child sees the parent's writes and the parent the child's, also across the
fork's `CreateProcessW`. The MSYS2 runtime keeps its process table (and the "installation id" used in the pipe names) in such a
section; on Poly the sections are private per process (`msys-2.0.dll`'s section characteristics `0xd0000040`), so a forked child cannot
see what the parent registered there.

## What this suggests for Poly (observation, not a recipe)

Poly's parent does reach the pipe pair and the 192-byte write, so the calls up to the four `NtCreateEvent`s work; the table above is
the list of what must then happen: `QueueUserAPC` to the caller's own pseudo-handle returns TRUE, `AddVectoredContinueHandler` returns a
handle, `TlsAlloc` gives a slot and `CreateThread` starts a thread that reads the pipe. Poly's own trace of the first of these that does not
return as above names the break.
