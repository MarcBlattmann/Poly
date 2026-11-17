# 0221: Windows services: a service control manager in the NT personality, `services.exe` as its host

**Status:** Accepted. Track `impl-chrome-installer-svc`, batch B1 of the Chrome-installer plan
(`docs/windows/chrome-installer-plan.md`). Builds on ADR 0066 (the registry), ADR 0171 (tokens,
descriptors, access checks), ADR 0192 (window stations), ADR 0079/0089 (`NtCreateUserProcess`).
Inputs (clean room): the spec sheets and visible tests of the 20 `advapi32` service entries and
their shared facts (`specs/windows/advapi32.dll/SERVICES.md`, handed over by `spec-win-services`),
Microsoft Learn ("Service Control Manager", "Service Security and Access Rights", the function
pages), `winsvc.h`, `winnt.h`, `winerror.h`. No third-party source was read.

## Context

Google's Chrome installer (run unchanged, auto-update on: owner decisions) creates two
auto-start services for its updater and Chrome adds `GoogleChromeElevationService`; the updater
and the elevation service run *as* services (`StartServiceCtrlDispatcherW`,
`RegisterServiceCtrlHandlerW`, `SetServiceStatus`). Poly had no service control manager: the
20 functions were fail stubs.

## Decision

### The manager's logic is a crate; its state lives in the NT personality

`crates/poly-scm` (host-tested) is the manager: the database (`Config`: every value of
SERVICES.md section 1, written to and read from `HKLM\SYSTEM\ControlSet001\Services\<name>`,
which `CurrentControlSet` links to), the checks of each call (names, types, start types, error
control, accounts, the error codes of the sheets), the handles (numbers per process, closed with
the process), the security descriptors of the services and of the manager (the default SDDL of
section 2, checked with `poly_security::access::check` against the caller's effective token, so a
restricted or impersonating caller gets 5 where the sheets say), the status of each service and
the life of its process (start, the dispatcher's connection, controls, STOPPED, crash = 1067,
failure actions with their counter, the 30-second rule, dependencies started first, deletion when
the last handle closes), and the listing. One `Scm` per machine is kept in `NtSystem::scm`
(`personalities/nt/src/scm.rs`).

On Windows `advapi32` talks to `services.exe` through RPC. On Poly it sends **one Poly system
call**, `PolyServiceCall(Operation, In, InLength, Out, OutLength, ReturnLength*)` (table 3,
index 5; exported by `ntdll` for `advapi32` and `services.exe`; 32-bit form `V P V P V P`): the
request and reply are messages of `poly_scm::wire`, the same for 32-bit and 64-bit callers.
`advapi32` lays the replies out in the `winsvc.h` structures of its own width (`services.rs`).

Why not a manager in user space with RPC: Poly has no RPC runtime yet (batch B2 brings COM
across processes), and everything the manager needs to decide is already in the NT personality:
the caller's token, the registry, process ends. The part that needs a process context is in one.

### Waits are on real events

Nothing blocks in the kernel. `StartServiceW` gets an event in the caller's table that the
manager signals when the start is decided (the process connected, ended, timed out, or could
not be started), then asks for the result. `ControlService` gets one that is signalled when the
service's handler returned, then reads the status. A service process's dispatcher gets an
auto-reset event that is set when a control is queued for it or its service reported STOPPED;
it runs the handler on its own (the main) thread, the service main on a new thread.

### `services.exe` is the host

A process has to start the service processes, at boot, after a failure action's delay and on
`StartServiceW`; and one that does not call the dispatcher within 30 s has to be ended.
`services.exe` (`personalities/nt/win32/services`, imports only `ntdll`, installed in `System32`)
is that process: the kernel starts it from a kernel thread when the manager first needs it (and
at the first NT use when the registry holds auto-start services). It loops on
`PolyServiceCall(HOST)`, which starts the processes that are due **in its own call** (so their
creator is `services.exe`), ends the ones past their deadline, and returns its event and how long
it may wait. On its first call it becomes LocalSystem; only `System32\services.exe` run by an
administrator or SYSTEM, while no other host lives, may become the host.

### What a service process is

- **Identity** (`poly_security::service`, kept apart from the user's token, which the UAC work
  may change): `Token::service(account, service_sid)`: user `S-1-5-18` (or `-19`, `-20` for
  LocalService and NetworkService), session 0, System integrity, Everyone and Authenticated
  Users, and with `ServiceSidType` 1 or 3 the group `S-1-5-6` and the service SID
  `S-1-5-80-<SHA-1 of the upper-cased name>` with attributes `0xE`.
- **No desktop:** its window station is `Service-0x0-3e7$` (per account) with a `Default`
  desktop, made when first needed, not visible.
- **Command line:** the `ImagePath` with its variables expanded, current directory `System32`.

### Start types

- **Auto (2):** started when the host first runs, dependencies first (the host starts at the
  first NT use if the registry has any). `DelayedAutostart` is stored; Poly starts delayed
  services with the others (not tested by the sheets).
- **Demand (3):** `StartServiceW` (and, with B2, COM `LocalService` activation).
- **Disabled (4):** 1058. A service marked for deletion is disabled.

`RpcSs` is provided by the manager itself: always RUNNING, auto start, with Windows' values in
the registry, so that Google's services (which depend on it) start.

### Shutdown

Not done yet: Poly's power-off does not send `SERVICE_CONTROL_SHUTDOWN`/`PRESHUTDOWN` to running
services (the sheets do not cover it; `ControlService` refuses 5 with 87 as Windows does). A
follow-up when Poly's shutdown sequence gets hooks for the NT world.

### Settings

A read-only list of the services in Settings is **a follow-up**: the native Settings app has no
way yet to ask the NT personality anything; the list would need a Poly call of its own (or a read
of the `Services` keys of the `SYSTEM` hive file). Noted in the hand-back.

## Consequences

- The 20 entries are implemented; their fail stubs are gone from `apps.rs` and `delayed.rs`.
  `RegisterServiceCtrlHandlerExW` is a new export.
- Every process exit calls the manager (`scm::process_ended`) once it was used since boot.
- The registry gains `Services\RpcSs` the first time a program uses the manager.
- Not covered (as the sheets): A variants, share-process services started by name (each service
  gets a process of its own), drivers, `LockServiceDatabase`, group and tag ordering, the event
  log, the service's descriptor in the registry (`Security` subkey: kept in memory only), the
  failure counter's reset period across boots, reboot and run-command actions (accepted, not run).
