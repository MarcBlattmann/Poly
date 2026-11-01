# 0114: 7-Zip's trace matched: privilege adjustment, processor groups, `msvcrt`'s `memcmp`, `FindFirstFileW(".")` and when the last error is set

**Status:** Accepted. Fixes the divergences ADR 0110's comparison reported
(`docs/windows/7zip-trace.md`: 0 of 15 steps, 395 divergences, 61
distinct). Now **15 of 15 steps** of the traced 7-Zip scenario match the
Windows reference, `TRACE_REQUIRED` is `true`, and with it the Phase 5
milestone "7-Zip command line passes its full recorded trace" is met as
ADR 0110 defines it. Extends [ADR 0083](0083-advapi32-registry-tokens-and-security-for-one-user.md)
(tokens: `NtAdjustPrivilegesToken`, the "later item" it names),
[ADR 0079](0079-our-kernelbase-and-kernel32.md) and
[ADR 0084](0084-our-c-runtime-ucrtbase-and-msvcrt.md).

## Context

ADR 0110 recorded 7-Zip's API trace on Windows (three agreeing runs) and
compared Poly's with it call by call: function, result, integer and string
arguments, and the last error (set, and to what, or left alone; the tracer
records `err` only when a call changed it). Its report listed what differed.
The implementer's inputs were that report and the output of `cargo xtask
win 7zip-trace diff` (the pipeline's red-diff feedback), Learn's reference
pages and the public SDK headers; nothing was run on Windows.

## Decision

### Privileges: `NtAdjustPrivilegesToken` and `AdjustTokenPrivileges`

7-Zip opens its process token with `TOKEN_ADJUST_PRIVILEGES` (no
`TOKEN_QUERY`) and enables `SeRestorePrivilege`, `SeCreateSymbolicLinkPrivilege`
and (for `a`) `SeSecurityPrivilege`, then calls `GetLastError`. The stub
failed with `ERROR_ACCESS_DENIED` (it read the token's privileges, which
needs `TOKEN_QUERY`). Now:

- `poly_security::token::adjust_privileges` (host-tested) is Learn's
  `AdjustTokenPrivileges`: `DisableAllPrivileges` disables every enabled
  privilege and ignores `NewState`; otherwise each entry enables
  (`SE_PRIVILEGE_ENABLED`), disables (no attribute) or removes
  (`SE_PRIVILEGE_REMOVED`, `winnt.h`) a privilege the token holds; one it
  does not hold is skipped and makes the result "not all assigned".
  `PreviousState` lists only the privileges that changed, with their former
  attributes, each once.
- The kernel gets the service `NtAdjustPrivilegesToken` (0x185, security
  group; not in the WDK, so `AdjustTokenPrivileges`' parameters in its
  order). It needs `TOKEN_ADJUST_PRIVILEGES`, and `TOKEN_QUERY` too for a
  `PreviousState` (Learn). A `PreviousState` buffer too small is
  `STATUS_BUFFER_TOO_SMALL` with the size in `ReturnLength` and nothing
  changed ("the function fails and does not adjust any privileges"); a
  privilege not held is `STATUS_NOT_ALL_ASSIGNED` (`ntstatus.h`, a success).
  The pseudo-handles of `GetCurrentProcessToken()` and
  `GetCurrentThreadEffectiveToken()` carry `TOKEN_QUERY` and
  `TOKEN_QUERY_SOURCE` only (Learn), so they cannot adjust.
- The state is the process's primary token's: `NtSystem::token_privileges`
  per process id, which `NtQueryInformationToken` (`TokenPrivileges`,
  `TokenStatistics`) reads, so every handle to the token sees a change. It
  is forgotten when the process ends. A process that has not adjusted
  anything has `Token::poly_user`'s privileges.
- `kernelbase!AdjustTokenPrivileges` calls the service and, on success,
  sets the last error to `ERROR_SUCCESS`, or `ERROR_NOT_ALL_ASSIGNED`
  (1300) for `STATUS_NOT_ALL_ASSIGNED` (Learn: "call GetLastError" to tell
  them apart). A `PreviousState` without `ReturnLength` is
  `ERROR_INVALID_PARAMETER` (Learn: it "can be NULL if PreviousState is
  NULL").

Poly's token holds all three privileges 7-Zip asks for, so `GetLastError`
after each returns 0, as on Windows.

### `kernel32`: `SetDefaultDllDirectories`, `GetActiveProcessorGroupCount`, `GetActiveProcessorCount`

7-Zip looks them up with `GetProcAddress`; they were missing (NULL, last
error 127), and 7-Zip took another path (Windows' extra `malloc`/`free`).

- `SetDefaultDllDirectories` forwards to `kernelbase` through
  `api-ms-win-core-libraryloader-l1-1-0` (its page's API set as Poly reads
  it). It takes a combination of the four `LOAD_LIBRARY_SEARCH_*` values
  Learn lists (`APPLICATION_DIR`, `USER_DIRS`, `SYSTEM32`, `DEFAULT_DIRS`,
  `libloaderapi.h`) and keeps them for the process. Poly's loader does not
  narrow its search by them yet (`LoadLibraryExW` ignores its search flags
  too).
- `GetActiveProcessorGroupCount` and `GetActiveProcessorCount` are
  `kernel32`'s own (their pages, `winbase.h`, name no API set): one group,
  holding `GetSystemInfo`'s `dwNumberOfProcessors`; `ALL_PROCESSOR_GROUPS`
  (`winnt.h`, 0xFFFF) counts every group; another group fails with 0.

### `msvcrt!memcmp` returns -1, 0 or 1

Learn promises only the sign; Windows' `msvcrt` returned 1 and -1 in all
203 compared calls, Poly (`compiler_builtins`) the difference of the bytes.
`msvcrt` now exports `memcmp` from the shared runtime
(`poly_crt_memcmp`, an assembly jump to `strings::memcmp_sign`, so that
`ucrtbase`, which links the same crate, does not export a Rust symbol);
`ucrtbase` keeps `compiler_builtins`' (`poly_crt::exports::MSVCRT_OWN_MEMORY`).

### `FindFirstFileW(".")` reports the directory's own name

7-Zip asks `FindFirstFileW(".")` for the current directory and copies the
name it gets: on Windows 28 bytes (`sevenzip-work` and its NUL in
UTF-16), on Poly 4 (`.`). `poly_kernelbase::path::split_find` now takes a
last component `.` or `..` as the directory it names: the full path is
resolved and the directory is looked up in its parent, so its entry
carries its own name. A root (`C:\.`) has no entry: `ERROR_FILE_NOT_FOUND`.

### When the last error is set

| Function | Before | Now | Source |
|---|---|---|---|
| `msvcrt!__getmainargs` | left alone | `ERROR_SUCCESS` on success | the trace |
| `advapi32!LookupPrivilegeValueW` | left alone | `ERROR_SUCCESS` on success | the trace |
| `oleaut32!SysAllocString`, `SysAllocStringLen` | left alone | `ERROR_SUCCESS` when they return a string | the trace |
| `kernel32!FindFirstFileW` | `ERROR_SUCCESS` on success | left alone on success | the trace |
| `kernel32!SetFilePointer` | `ERROR_SUCCESS` on success | `ERROR_SUCCESS` only when the result is `INVALID_SET_FILE_POINTER` | Learn ("you do not need to call GetLastError" for another value) and the trace |
| `kernel32!GetActiveProcessorGroupCount`, `GetActiveProcessorCount` | (missing) | a non-zero last error on success (placeholder `ERROR_INSUFFICIENT_BUFFER`) | the trace, below |
| `msvcrt`'s start (its `DllMain`) | left alone | a non-zero last error (placeholder `ERROR_INSUFFICIENT_BUFFER`) | the trace, below |

`CreateEventW` (ADR 0110's last row) already set `ERROR_SUCCESS`; its
divergence followed from `SetFilePointer`'s.

**The two placeholders.** The tracer sees only the calls 7-Zip makes
through its import table, and records a last error only when a traced call
changes it. Two divergences remained after everything above, and the trace
says why without saying the value:

1. `__getmainargs`, the first traced call that touches the last error,
   changes it to 0 on Windows, so it was not 0 when the program's start-up
   code reached it; on Poly it was 0.
2. `SysAllocString("")`, `SysAllocStringLen` and once `CreateFileW` change
   it to 0 on Windows where Poly's was 0 already. In every case the only
   calls between the last call that set 0 on both and that call are traced
   calls that left it alone on both, and 7-Zip's untraced calls of
   `GetActiveProcessorGroupCount` and `GetActiveProcessorCount` through
   the pointers `GetProcAddress` gave it. So one of those two leaves a
   non-zero last error on Windows when it succeeds.

Poly leaves `ERROR_INSUFFICIENT_BUFFER` (122) in both places
(`msvcrt::START_LAST_ERROR`,
`poly_kernelbase::system::PROCESSOR_QUERY_LAST_ERROR`): any non-zero value
gives the same trace, so the value is a placeholder, and both processor
functions set it because the trace cannot tell which of them does.

### Tests

- Host: `poly_security::token` (adjusting: enable, no-op, not held,
  disable, remove, disable all; `TOKEN_PRIVILEGES` round trip), the NT
  personality's `security` tests (`NtAdjustPrivilegesToken`: shared state
  across handles, access checks, the pseudo-handle, a short
  `PreviousState`, not all assigned), `poly_kernelbase::path` (`.` and
  `..` patterns), `poly_kernelbase::system` (processor groups, DLL
  directory flags), `poly_crt::string` (`memcmp`'s sign).
- Ring 3: `a32test` (`LookupPrivilegeValueW` clears the last error;
  `AdjustTokenPrivileges` enables and disables `SeRestorePrivilege` with
  `PreviousState`, `ERROR_NOT_ALL_ASSIGNED` for `SeLockMemoryPrivilege`,
  `ERROR_ACCESS_DENIED` without the right), `k32test` (`SetFilePointer` and
  `FindFirstFileW` leave the last error on success, `FindFirstFileW("k32dir\.")`
  is named `k32dir`, the processor-group functions,
  `SetDefaultDllDirectories`).
- The `wintrace` boot group: 15 of 15 steps, now required.

### The milestone

ADR 0110's four conditions: the untraced scenario passes its 16 steps and
is required (ADR 0101); the traced scenario's 15 steps have the
reference's exit codes and no divergence; `TRACE_REQUIRED = true`; the
reference has three agreeing runs on the current kit and driver (neither
changed here). The roadmap's milestone is ticked.

## To be confirmed by the spec agent

Observed only through ADR 0110's trace comparison, or Poly's choice where
Learn is silent:

1. `msvcrt!memcmp` returns exactly -1, 0 or 1 for every input (seen: 1
   and -1 in 203 calls of 16 bytes); whether `ucrtbase`'s does too.
2. `__getmainargs` sets the last error to 0 on success; whether
   `__wgetmainargs` does (Poly: unchanged, not seen).
3. The last error is not 0 when an `msvcrt` program's start-up code calls
   `__getmainargs`; which value (Poly's placeholder: 122), and where it
   comes from (Poly: `msvcrt`'s `DllMain`).
4. `GetActiveProcessorGroupCount` / `GetActiveProcessorCount` leave a
   non-zero last error on success; which function and value (Poly: both,
   122). `GetActiveProcessorCount` for a group that does not exist: 0 and
   `ERROR_INVALID_PARAMETER` (Poly's choice).
5. `LookupPrivilegeValueW`, `SysAllocString` and `SysAllocStringLen` set
   the last error to 0 on success (seen); whether `SysReAllocString(Len)`
   and `LookupPrivilegeNameW` do (Poly: unchanged).
6. `FindFirstFileW` leaves the last error alone on success (seen for
   names, `.` and full paths); `SetFilePointer` sets 0 only for a result of
   `INVALID_SET_FILE_POINTER` (seen: left alone for 0 and 32).
7. `FindFirstFileW` of a path ending in `.` or `..` reports the directory
   it names (seen for `.`: its name's length); a root (`C:\.`, Poly:
   `ERROR_FILE_NOT_FOUND`).
8. `SetDefaultDllDirectories(0)` or with other bits: Poly fails with
   `ERROR_INVALID_PARAMETER`.
9. `AdjustTokenPrivileges`: `ERROR_SUCCESS` when every privilege was held
   (seen: `GetLastError` 0 after enabling the three above, on the oracle's
   account); a child process's token starts with Poly's default privilege
   state rather than a copy of its parent's adjusted one; the access the
   two token pseudo-handles carry.

## Consequences

- New: `poly_security::token::{adjust_privileges, token_privileges,
  parse_token_privileges, Adjusted, SE_PRIVILEGE_REMOVED,
  STATUS_NOT_ALL_ASSIGNED}`; the service `NtAdjustPrivilegesToken`
  (`ntdll` stub generated from `services.rs`); `kernel32` exports
  `SetDefaultDllDirectories` (forwarder), `GetActiveProcessorGroupCount`,
  `GetActiveProcessorCount`; `poly_kernelbase::system::{active_processor_count,
  valid_default_dll_directories, ...}`; `poly_crt::exports::MSVCRT_OWN_MEMORY`.
- Changed: `kernelbase` (`AdjustTokenPrivileges`, `SetFilePointer`,
  `FindFirstFileW`, `SetDefaultDllDirectories`), `advapi32`
  (`LookupPrivilegeValueW`), `oleaut32`, `msvcrt` (`memcmp`,
  `__getmainargs`, its start), `poly_kernelbase::path::split_find`, the NT
  personality's token services; `TRACE_REQUIRED = true`;
  `docs/windows/7zip-trace.md` regenerated (15 of 15); coverage:
  `AdjustTokenPrivileges` (advapi32, kernelbase), `NtAdjustPrivilegesToken`
  (ntdll, ntoskrnl), `SetDefaultDllDirectories`, `GetActiveProcessorCount`,
  `GetActiveProcessorGroupCount` claimed by `impl-7zip-trace`.
- A change that alters any traced call of the 15 steps now fails the
  `wintrace` boot group.
