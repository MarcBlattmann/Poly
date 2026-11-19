# 0235: System processes run as LocalSystem

**Status:** Accepted. Track `system-processes` (asked for by B2c, `impl-com-security`). Builds on
ADR 0171 (tokens, descriptors and access checks), ADR 0220 (COM across processes and
`comlaunch.exe`), ADR 0221 (the service control manager), ADR 0222 (UAC: split tokens) and
ADR 0226 (the Task Scheduler). Inputs (clean room, `CLEAN_ROOM.md`): the spec sheets
`specs/windows/advapi32.dll/SERVICES.md` (section 4, who a service is),
`rpcrt4.dll/I_RpcOpenClientProcess` and `ole32.dll/AppID.Security`; Microsoft Learn ("LocalSystem
Account", "Well-known SIDs", `CreateProcessAsUserW`, `GetNamedPipeClientProcessId`,
`DuplicateTokenEx`); `winnt.h`. Nothing of Wine, ReactOS or Chromium was read.

## Context

Windows runs its system services as **LocalSystem**: the DCOM launcher and RPCSS from boot, and
every service whose configuration names that account. Poly's service control manager already
gives its services their account's token (ADR 0221, `Token::service`). Two of Poly's own system
programs did not get it:

- **`comlaunch.exe`**, Poly's COM activation process (ADR 0220), was started by the first
  client's `ole32` with `CreateProcessW`, so it ran with that client's token. The sheet
  `I_RpcOpenClientProcess` records that during an activation the server's caller is the system's
  activation process, which an elevated server can open (`PROCESS_QUERY_LIMITED_INFORMATION`,
  `PROCESS_VM_READ`) and a standard one cannot (5). With the client's token, a standard server
  opened it.
- **`schedsvc.exe`**, the Task Scheduler (ADR 0226), runs as the `Schedule` service (LocalSystem)
  when `taskschd` can start that service. A standard user may not start it (the service's
  descriptor gives interactive users no start right), so `taskschd` started the program itself,
  with the user's token. On Windows the service runs from boot and nobody needs to start it.

## Decision

1. **The SYSTEM token is `Token::service(Account::LocalSystem, …)`** (ADR 0221): user
   `S-1-5-18`, Administrators (owner), Everyone, Authenticated Users, System integrity
   (`S-1-16-16384`), session 0, logon session `0x3E7` (SERVICES.md section 4); every privilege,
   enabled (Learn: "LocalSystem Account"). No field is added that a sheet or Learn does not give.
2. **`personalities/nt/src/sysproc.rs`** starts a program with a service account's token:
   `spawn_as(command, account, service SID, place, …)`. The service control manager's services
   use it (their station `Service-0x0-<luid>$`, unchanged), and so does the new Poly service:
3. **`PolyStartSystemProcess(Program)`** (`TABLE_POLY` 7, exported by `ntdll`): a DLL asks the
   kernel to start one of the system's programs the way the system runs it.
   - **1, the COM activation process:** `C:\Windows\System32\comlaunch.exe` as LocalSystem in
     session 0, on the caller's desktop (the servers it starts run there, as before), outside the
     caller's jobs. Its process descriptor is its token's default (SYSTEM all access,
     Administrators read and execute) with a System label that only stops writing up: this gives
     the sheet's results (an elevated server opens it with both rights, a standard one gets 5).
   - **2, the Task Scheduler:** the service manager starts the `Schedule` service for the system,
     as its configuration says (`Scm::start_for_system`: no handle, no caller's rights, nobody
     waits on the result; already running is not an error).
   - Anything else is `STATUS_INVALID_PARAMETER`. The caller does not choose the program, the
     account or the token, so the call gives it nothing it could use against the system: both
     programs run on Windows from boot anyway.
4. **`ole32` and `taskschd` use it.** `ole32` asks for program 1 when the activation pipe is not
   there (the named mutex still lets one client at a time ask). `taskschd` asks for program 2
   when it cannot start `Schedule` itself; it starts `schedsvc.exe` as a plain process only when
   there is no such service at all.
5. **`comlaunch.exe` starts a server as the client that activates it.** It finds the client from
   the activation pipe (`GetNamedPipeClientProcessId`), takes a primary copy of its token
   (`OpenProcessToken`, `DuplicateTokenEx`) and starts the server with `CreateProcessAsUserW`.
   Without the client's token it starts nothing (`CO_E_SERVER_EXEC_FAILURE`): a server never
   runs as SYSTEM by accident. Services named by an `AppID`'s `LocalService` go through the
   service manager, which starts them as their configuration says.
6. **A process made with a given token gets that token's default descriptors** (process and first
   thread, the label included): Learn, `CreateProcessAsUserW`, "The default security descriptor
   is that of the user referenced in the hToken parameter". Until now Poly used the creator's
   token. They are applied after the creator's handles are opened, as explicit descriptors
   already were, so the creator keeps the handles the call returns. Without this, a server
   started by the SYSTEM activation process would carry SYSTEM's descriptor and its own user
   could not open it.

## Consequences

- A standard server's `I_RpcOpenClientProcess` during an activation gets 5, an elevated one a
  handle (with B2c's `ole32`).
- `comlaunch.exe` and `schedsvc.exe` no longer end with the job of the program that caused them to
  start; they run until the machine stops, as on Windows.
- Poly's named pipes do not check descriptors yet, so the SYSTEM-owned activation and scheduler
  pipes stay open to every client; when they do, both servers will need explicit pipe
  descriptors.
- Poly still starts these programs on demand rather than at boot, so a boot that never uses COM
  or the Task Scheduler does not pay for them.
