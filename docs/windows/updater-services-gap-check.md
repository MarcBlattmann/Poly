# Gap check: Google's updater services, scheduled task and Chrome `setup.exe` (B7)

Job `spec-win-updater-services` (2026-11-20). Question: what do the next stops of the Chrome installer run on Poly call
(the two services of the 32-bit updater 157 and 159, the update service's COM registration and activation, the scheduled
task, Chrome's `setup.exe`), and which of those calls have neither a sheet nor a green status?

## Method

- Imports of the five binaries that matter for the next stops: `updater.exe` 157 and 159 (32-bit), `elevation_service.exe`,
  `notification_helper.exe`, `setup.exe` (the PE import tables, names only, followed through forwarders).
- Each import compared with `coverage/windows/api/<dll>.json` (status) and `specs/windows/<dll>/<Symbol>.yaml` (sheet exists).
- The list is a point-in-time table of names; it says nothing about what the programs pass in. What a 32-bit program
  does through WOW64 is listed separately below.

## Result

624 distinct imports: 259 green, 260 claimed (64 of them with a sheet, 196 claimed by other tracks without one yet),
73 without any status and 32 stubs.

| Area | State before this job | Result |
|---|---|---|
| Service control (`OpenSCManagerW`, `CreateServiceW`, `ChangeServiceConfigW`, `ChangeServiceConfig2W`, `QueryServiceConfigW`, `QueryServiceStatusEx`, `DeleteService`, `CloseServiceHandle`, `OpenServiceW`, `RegisterServiceCtrlHandlerW`, `SetServiceStatus`, `StartServiceCtrlDispatcherW`) | all 12 have thick sheets and tests (`specs/windows/advapi32.dll/`, see `SERVICES.md` there) | no gap. The updater does not import `StartServiceW`: it starts its service through COM activation of a `LocalService` class. |
| COM service activation and registration (`CoCreateInstance` for a `LocalService` class, `CoRegisterClassObject`, AppID, class and type library registration, `CoInitializeSecurity`, `CLSID_GlobalOptions`) | green or specified by the B2/B2c/B2d jobs (PRs #307, #326, #419) | no gap |
| Scheduled task (`taskschd.dll` COM interfaces, `ITaskService`, `ITaskFolder.RegisterTaskDefinition`, `ITaskDefinition`, triggers, actions, principal, settings, `IRegisteredTask`) | about 150 sheets under `specs/windows/taskschd.dll/` (the updater reaches the API through COM, so there are no imports to compare) | no gap found by name; the 32-bit client view is open (below) |
| Chrome `setup.exe` | its imports are mostly green; the open ones are in the two lists below | see below |
| **Small functions that the updater, `setup.exe` and the helpers call and nobody specified** | `missing` or `stub` | **15 entries specified here** |

## Entries specified by this job

| DLL | Entries | Importers |
|---|---|---|
| `ole32.dll` | `CoCreateGuid`, `IIDFromString`, `StringFromGUID2` | updater 157/159 (and `IIDFromString`: elevation service) |
| `kernel32.dll` | `ProcessIdToSessionId`, `IsWow64Process`, `GetComputerNameExW`, `ReadProcessMemory` | updater 157/159 (and the others for `IsWow64Process`) |
| `kernel32.dll` | `ReplaceFileW`, `SetFileInformationByHandle`, `LockFileEx`, `UnlockFileEx`, `FindFirstFileExW`, `GetFileAttributesExW` | updater 157/159, `setup.exe`, `notification_helper.exe` |
| `userenv.dll` | `EnterCriticalPolicySection`, `LeaveCriticalPolicySection` (both were `stub`) | updater 157/159 |

## Still open after this job (not part of the 15)

Outside the services path; each group fits a job of its own.

| Group | Entries | Importers |
|---|---|---|
| Locale and string mapping | `CompareStringW`, `GetLocaleInfoW`, `GetStringTypeW`, `LCMapStringW`, `IsValidLocale`, `GetUserDefaultLCID`, `GetUserDefaultLangID`, `GetSystemDefaultLCID`, `GetThreadPreferredUILanguages` | all five |
| Process, heap, debugging support | `HeapSetInformation`, `HeapLock`/`HeapUnlock`/`HeapWalk`, `GetThreadContext`, `Wow64GetThreadContext`, `GetThreadPriorityBoost`, `SetThreadPriorityBoost`, `QueryThreadCycleTime`, `K32GetModuleInformation`, `GetModuleHandleExA/W`, `LoadLibraryExA`, `OutputDebugStringA/W`, `RemoveVectoredExceptionHandler`, `RtlCaptureStackBackTrace`, `RtlUnwind`, `RtlGetLastNtStatus`, `NtClose`, `SwitchToThread`, `GetCurrentThreadStackLimits`, `DiscardVirtualMemory`, `GetLogicalProcessorInformation(Ex)`, `GetProductInfo`, `VerifyVersionInfoW`, `WerRegisterRuntimeExceptionModule` | all five |
| Timers | `timeBeginPeriod`, `timeEndPeriod`, `timeGetTime` | all five |
| User interface of the updater | 14 GDI functions (`CreatePen`, `Rectangle`, `StretchBlt`, `PatBlt`, `FillRgn` ...), 17 user32 functions (`AdjustWindowRect`, `DrawIconEx`, `SetWindowsHookExW` ...), `SetWindowTheme` | updater 157/159, `setup.exe` (hooks) |
| Chrome `setup.exe` crypto | 14 `ncrypt.dll` functions, 2 `tbs.dll` functions, `CreateURLMonikerEx` | `setup.exe`, elevation service |

## The 32-bit gap

The updater 157 and 159 are 32-bit. The pipeline builds and runs 64-bit tests only (`tools/pipeline/src/windows.rs`), so every
sheet written for these entries describes what a 64-bit caller sees. What a 32-bit caller sees differs in places that were
specified for no entry yet and cannot be recorded by a committed test:

- **Service structures.** The 32-bit and the 64-bit layouts of `SERVICE_STATUS_PROCESS`, `QUERY_SERVICE_CONFIGW`,
  `SERVICE_TABLE_ENTRYW` and `ENUM_SERVICE_STATUS_PROCESSW` differ in pointer width; a Poly implementation that shares the
  64-bit code must convert them for a WOW64 caller.
- **Registry views.** A 32-bit service installer writes into `WOW6432Node` for COM classes and for `Uninstall` keys but not for
  `HKLM\SYSTEM\CurrentControlSet\Services`; the `WOW64` value of a class registration decides which server a 32-bit client starts.
- **COM and Task Scheduler.** A 32-bit client of the 64-bit Task Scheduler and of a 64-bit local server needs the 64-bit proxy
  for the interface; interfaces without a 64-bit proxy stub fail with `E_NOINTERFACE` or `REGDB_E_IIDNOTREG`.
- **`IsWow64Process`, `ReadProcessMemory`, `ProcessIdToSessionId`** give different answers or sizes for a 32-bit caller of a
  64-bit process (`wow_self` and `rpm_child` show the 64-bit view).

Evidence for a 32-bit view was produced by hand outside the pipeline: the 30 tests of this job built for
`i686-pc-windows-msvc` (a `kernel32`-only stand-in for the 64-bit-only helper crate) and run by the same oracle runner. 26 of
the 30 gave the same bytes as the 64-bit run; the other four: `IsWow64Process` answers TRUE for a 32-bit caller and its
32-bit child (`wow_self`, `wow_rights`), the 64-bit `FILE_RENAME_INFO` layout passed by a 32-bit program fails with
ERROR_INVALID_NAME 123 (`sfi_rename`; the 32-bit layout has a 4-byte `RootDirectory`), and `rpm_self` reads a "kernel half"
address that the 32-bit build truncates (a difference of the test, not of the function). The sheets say so where it matters. The results are in the kit (`results/x86-vs-x64.txt`), not committed.
The pipeline can build 32-bit tests since ADR 0259 (`// poly-arch: x86`, merged while this job ran); recording the 15 entries'
tests again as 32-bit twins is open work for a later job (the stack tests of `spec-win-stack-roundrect` are the first to do it).
