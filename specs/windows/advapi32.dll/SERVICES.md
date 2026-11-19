# The service control manager: what the sheets of the service entries share

Written by `spec-win-services` (2026-11-17) from observations of Windows 11 Home 25H2 (build 26200.9974, x64) on the
oracle machine, made with test programs of our own and Microsoft Learn's descriptions of the calls (`winsvc.h`,
`winerror.h`). Behaviour only: no code, no internals. The sheets of the entries named at the end say what each call
does; this file holds the facts they have in common, so that they are written once.

Why it exists: Google's updater (and `setup.exe` of Chrome, and Chrome's `elevation_service.exe`) create, start,
query, change and delete Windows services, and the updater and the elevation service run *as* services. The calls the
programs import are: `OpenSCManagerW`, `CreateServiceW`, `OpenServiceW`, `DeleteService`, `ChangeServiceConfigW`,
`ChangeServiceConfig2W`, `QueryServiceConfigW`, `QueryServiceStatusEx`, `CloseServiceHandle` (the installers, the
updater, `setup.exe`), and `StartServiceCtrlDispatcherW`, `RegisterServiceCtrlHandlerW`, `SetServiceStatus`,
`QueryServiceStatusEx` (the services themselves). The other entries (`StartServiceW`, `ControlService`,
`RegisterServiceCtrlHandlerExW`, `QueryServiceStatus`, `QueryServiceConfig2W`, `EnumServicesStatusExW`, the object
security calls) are what other programs and the test programs use.

## 1. The service database and the registry

A service is a key `HKLM\SYSTEM\CurrentControlSet\Services\<name>`; the names are case-insensitive, 1 to 256 UTF-16
units, no `\` or `/` (ERROR_INVALID_NAME 123), unique among services. `CreateServiceW` writes exactly these values
(and no subkeys); a program can read them with the registry calls:

| Value | Kind | Written | Content |
|---|---|---|---|
| `Type` | DWORD | always | 0x10 own process, 0x20 share process, 0x110 own process interactive, 0x1 kernel driver |
| `Start` | DWORD | always | 2 auto, 3 demand, 4 disabled (0 boot and 1 system are valid for drivers only) |
| `ErrorControl` | DWORD | always | 0 ignore, 1 normal, 2 severe, 3 critical |
| `ImagePath` | REG_EXPAND_SZ | always | the text as given (quotes, arguments, `%VAR%` kept); QueryServiceConfigW returns it with variables expanded |
| `DisplayName` | REG_SZ | only when a display name was given | a service created without one has no such value, and its display name is its name |
| `ObjectName` | REG_SZ | always | `LocalSystem` for a NULL account and for `LocalSystem`; otherwise the account as given (`NT AUTHORITY\LocalService`) |
| `DependOnService` | REG_MULTI_SZ | only for a non-empty list | the names, in order; an empty list removes the value |
| `Description` | REG_SZ | `ChangeServiceConfig2W` level 1 with text | removed for an empty text and for a NULL pointer |
| `FailureActions` | REG_BINARY | level 2 | see the sheet of `ChangeServiceConfig2W` (reset period, offsets 0, count, offset 0x14, then type and delay pairs) |
| `FailureActionsOnNonCrashFailures` | DWORD | level 4 | the flag |
| `DelayedAutostart` | DWORD | level 3 | 1 or 0 (written for both) |
| `ServiceSidType` | DWORD | level 5 | 0 none, 1 unrestricted, 3 restricted |
| `PreshutdownTimeout` | DWORD | level 7 | milliseconds (the default, 10000, is not stored) |
| `DeleteFlag` | DWORD 1 | `DeleteService` | the service is marked for deletion; the key goes when the last handle closes |
| `WOW64` | DWORD 0x14c | when a **32-bit** program created the service | seen on the services Google's 32-bit updater creates (survey, 2026-11-17); the test programs here are 64-bit |

What Google's programs leave in the database (survey `docs/windows/chrome-installer-plan.md`): the two updater
services per version `GoogleUpdaterService<ver>` and `GoogleUpdaterInternalService<ver>` (own process, `Start` 2,
`ErrorControl` 1, account LocalSystem, `DependOnService` `RPCSS`, a `Description`, `WOW64` 0x14c, `ImagePath` =
`"…\updater.exe" --system --windows-service --service=update` or `update-internal`), and Chrome's
`GoogleChromeElevationService` (own process, `Start` 3, `ErrorControl` 1, LocalSystem, depends on `RPCSS`). No
`FailureActions`, no `DelayedAutostart`, no `ServiceSidType` value: of `ChangeServiceConfig2W` only the description is
used. The updater **deletes and creates its services again** when it updates itself and when it uninstalls.

## 2. Handles, rights and the default security descriptors

- **Manager handle** (`OpenSCManagerW`): rights SC_MANAGER_CONNECT 0x1, CREATE_SERVICE 0x2, ENUMERATE_SERVICE 0x4,
  LOCK 0x8, QUERY_LOCK_STATUS 0x10, MODIFY_BOOT_CONFIG 0x20, ALL_ACCESS 0xF003F. `CreateServiceW` needs CREATE_SERVICE,
  `EnumServicesStatusExW` ENUMERATE_SERVICE; `OpenServiceW` needs no particular right.
- **Service handle**: SERVICE_QUERY_CONFIG 0x1, CHANGE_CONFIG 0x2, QUERY_STATUS 0x4, ENUMERATE_DEPENDENTS 0x8, START
  0x10, STOP 0x20, PAUSE_CONTINUE 0x40, INTERROGATE 0x80, USER_DEFINED_CONTROL 0x100, ALL_ACCESS 0xF01FF, plus DELETE,
  READ_CONTROL, WRITE_DAC, WRITE_OWNER. Each call checks its right on the handle and fails with ERROR_ACCESS_DENIED (5)
  without it.
- **Default descriptor of a new service** (owner and group `SY`; as SDDL):
  `D:(A;;CCLCSWRPWPDTLOCRRC;;;SY)(A;;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;BA)(A;;CCLCSWLOCRRC;;;IU)(A;;CCLCSWLOCRRC;;;SU)`
  and the SACL `S:(AU;FA;CCDCLCSWRPWPDTLOCRSDRCWDWO;;;WD)` (readable only with ACCESS_SYSTEM_SECURITY). So SYSTEM may
  query the configuration and status, list dependents, start, stop, pause, interrogate and read the descriptor but
  not change or delete the service; Administrators have every right; interactive and service logon users may query
  the configuration and status, list dependents, interrogate, send user-defined controls and read the descriptor;
  nobody else gets any right.
- **Default descriptor of the manager**: `D:(A;;CC;;;AU)(A;;CCLCRPRC;;;IU)(A;;CCLCRPRC;;;SU)(A;;CCLCRPWPRC;;;SY)(A;;KA;;;BA)(A;;CC;;;AC)(A;;CC;;;S-1-15-3-1024-528118966-3876874398-709513571-1907873084-3598227634-3698730060-278077788-3990600205)`
  `S:(AU;FA;KA;;;WD)(AU;OIIOFA;GA;;;WD)`. Everybody authenticated may connect; only Administrators may create
  services, lock the database or change its boot configuration.
- Closing the last handle of a service that was marked for deletion removes it (and its key) a few milliseconds
  later: a program that deletes a service and creates one of the same name at once can still get
  ERROR_SERVICE_MARKED_FOR_DELETE (1072) for a moment.

**What a caller that is not elevated gets.** On Windows a process that is not elevated has Administrators only as a
deny-only group, and the descriptors above then give it no creating, starting, stopping, changing, deleting or
descriptor-writing right. Observed with a really non-elevated token (the oracle's interactive session, UAC filtered):
`OpenSCManagerW(SC_MANAGER_ALL_ACCESS)` fails with 5 on the first call; CONNECT, ENUMERATE_SERVICE, QUERY_LOCK_STATUS,
GENERIC_READ and MAXIMUM_ALLOWED work; CREATE_SERVICE, LOCK, MODIFY_BOOT_CONFIG fail with 5; on an existing system
service QUERY_CONFIG, QUERY_STATUS, INTERROGATE, READ_CONTROL work and START, STOP, PAUSE_CONTINUE, CHANGE_CONFIG,
DELETE, WRITE_DAC, USER_DEFINED_CONTROL fail with 5; the enumeration works. The test programs make their non-elevated
caller with a restricted copy of their own token (Administrators deny-only, privileges removed), which gives the same
refusals for the rights above. **Poly's design (ADR 0171, the owner's elevation decision) decides whether Poly's
programs are elevated** (the survey recommends: every process is, and the Install click is the consent); the sheets
state what a caller without Administrators gets, whichever way it comes to be one.

## 3. The life of a service process

1. `StartServiceW` on an own-process service whose start type is not disabled starts the image of `ImagePath` (the
   whole text is the command line: `"<image>" --child run` is seen by the process as its command line) in session 0 as
   the account of the service, **with the dependencies started first**. It returns TRUE when the process has
   connected (called `StartServiceCtrlDispatcherW`) and its service main function has begun; at that moment the
   status is START_PENDING with checkpoint 0 and wait hint 2000 until the service reports itself. A service that
   reports RUNNING at once may already be RUNNING when the call returns: programs that need RUNNING wait for it.
2. Errors of the start: image not found 2 (the status stays STOPPED); disabled 1058; already running or starting 1056;
   a dependency missing 1075, a dependency disabled 1068; rights 5.
3. **The 30 second rule.** If the process does not call the dispatcher, `StartServiceW` fails with
   ERROR_SERVICE_REQUEST_TIMEOUT (1053) after **30 seconds (measured: 30000 ms)**, and the manager has then ended the
   process (status STOPPED, no process id). If the process ends **before** it calls the dispatcher, the same error 1053
   comes at once (about 50 ms) and the status is STOPPED.
4. The dispatcher (`StartServiceCtrlDispatcherW`, called by the process's main thread) fails with
   ERROR_FAILED_SERVICE_CONTROLLER_CONNECT (1063) in a process the manager did not start; the table's name is ignored for
   an own-process service; the service main function is called on a thread of its own with argc = 1 + the number of
   arguments of `StartServiceW`, argv[0] the service name. The dispatcher returns when the service has reported
   STOPPED. A later call of the dispatcher in a process that already tried returns 1056.
5. The service registers its handler with `RegisterServiceCtrlHandlerW`/`ExW` (the name does not matter for an
   own-process service, not even NULL) and reports with `SetServiceStatus(handle, &status)`: state, accepted controls
   (STOP 1, PAUSE_CONTINUE 2, SHUTDOWN 4, …), Win32 exit code, service-specific exit code, checkpoint, wait hint. What
   it reports is what `QueryServiceStatusEx` shows. A state that is not 1 to 7, and an accepted-controls mask with
   bits no control uses, are refused with ERROR_INVALID_DATA (13) and change nothing.
6. **Controls.** `ControlService` (right per control: STOP, PAUSE_CONTINUE, INTERROGATE, USER_DEFINED_CONTROL): on a
   stopped service 1062 (ERROR_SERVICE_NOT_ACTIVE; also for INTERROGATE and for user-defined codes); STOP, PAUSE or
   CONTINUE that the service did not announce in its accepted-controls mask is 1052 (ERROR_INVALID_SERVICE_CONTROL)
   — also while it is START_PENDING and has announced nothing; the codes 0, 99 and 256 are 87 (invalid parameter) and
   SHUTDOWN (5) cannot be sent (87); INTERROGATE (4) and user-defined codes 128 to 255 always reach the handler of a
   running service, whatever the mask. A service that announced STOP while START_PENDING gets STOP then and
   INTERROGATE, and PAUSE is 1061 (ERROR_SERVICE_CANNOT_ACCEPT_CTRL); a control sent while a service is
   STOP_PENDING was also seen to give 1061. The call
   returns the status the service has when it returns. A service that other running services depend on cannot be
   stopped (1051, ERROR_DEPENDENT_SERVICES_RUNNING). The value an extended handler returns does not change what
   `ControlService` returns.
7. **Stopping.** The handler of STOP reports STOP_PENDING (wait hint) and then STOPPED; the status becomes STOPPED
   with exit code 0, process id 0 (the process has ended); a service can be started again at once.
8. **Exit codes.** Never started: Win32 exit code 1077 (ERROR_SERVICE_NEVER_STARTED); reported with STOPPED: kept
   as reported (1066 ERROR_SERVICE_SPECIFIC_ERROR with the specific code, or a plain code); a process that ends without
   reporting STOPPED: STOPPED with 1067 (ERROR_PROCESS_ABORTED).
9. **Failure actions.** When a service ends as in 8 without reporting STOPPED (a crash), the manager runs the action of
   the current failure number from the list of `ChangeServiceConfig2W` level 2: the first failure the first action,
   the second failure the second, and so on (observed with the lists restart, none and restart, restart, restart,
   none: four starts in all, then it stays stopped); the count is reset after the reset period without failures (not
   tested: the period was a day). A failure while no actions were configured is counted too: a service that crashed
   once without actions and then got the list restart, none did not restart at its next crash. RESTART starts the
   service again after the delay (observed: a new process, RUNNING again, about the delay plus the time to start); NONE
   leaves it stopped. A service that reports STOPPED with a non-zero exit code is a failure only when level 4
   (FailureActionsOnNonCrashFailures) is set; otherwise it stays stopped.
10. **Dependencies** (`DependOnService`): names are not checked when the service is created or changed (a name that
    does not exist is accepted), a service that lists itself is refused (1059, ERROR_CIRCULAR_DEPENDENCY); at start the
    dependencies are started first and must exist and not be disabled. Google's services depend on `RPCSS`: on Windows
    it is a running, auto-start share-process service of NetworkService; **Poly needs a service of that name that is
    RUNNING** or the updater's services cannot start.

## 4. Who the service is

A LocalSystem service's process (observed through its own token): user `S-1-5-18`, session 0, integrity
`S-1-16-16384` (System), groups Everyone and Authenticated Users (and no `S-1-5-6`, unless it has a service SID),
window station `Service-0x0-3e7$` (not visible: `WSF_VISIBLE` 0), desktop `Default` of that station. LocalService and
NetworkService processes are `S-1-5-19` and `S-1-5-20` in session 0, integrity System, the stations
`Service-0x0-3e5$` and `Service-0x0-3e4$`.

**Service SID**: with `ServiceSidType` unrestricted (1) the process token has a group `S-1-5-80-a-b-c-d-e` (attributes
0xE: mandatory, enabled by default, enabled; and the group `S-1-5-6` is added too), where a to e are the five 32-bit
little-endian words of the SHA-1 of the **upper-cased service name in UTF-16 little endian**; verified for the test
service names. With the type none (the default) there is no such group.

## 5. Listing

`EnumServicesStatusExW` (SC_ENUM_PROCESS_INFO only; 124 for other levels) lists services by type and state filter
(the type needs one of the SERVICE_* masks, state 1 active, 2 inactive, 3 all; 0, 4 and a type of 0 are 87), in an
unspecified but stable order; a buffer that is too small gives ERROR_MORE_DATA (234) with the entries that fit and a
resume handle, and the pages together are the whole list once. A group name that no service is in gives 1060.

## Entries

`OpenSCManagerW`, `CreateServiceW`, `OpenServiceW`, `DeleteService`, `CloseServiceHandle`, `ChangeServiceConfigW`,
`ChangeServiceConfig2W`, `QueryServiceConfigW`, `QueryServiceConfig2W`, `QueryServiceStatusEx`, `QueryServiceStatus`,
`StartServiceW`, `ControlService`, `StartServiceCtrlDispatcherW`, `RegisterServiceCtrlHandlerW`,
`RegisterServiceCtrlHandlerExW`, `SetServiceStatus`, `EnumServicesStatusExW`, `QueryServiceObjectSecurity`,
`SetServiceObjectSecurity` (all `advapi32.dll`, sheets beside this file). Tests: Rust programs, one per file, in the
`<symbol>.tests/` directories; they create a throwaway service named `PolySpecSvc…` (their own image is the service:
started with `--child <mode>` it plays a service and writes what it sees to a file beside itself, which the test reads
back), and delete it again. A service that the test program could not end is ended by its next run.
