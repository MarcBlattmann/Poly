# 0226: The Task Scheduler: taskschd.dll, a task store on disk and a scheduler process

**Status:** Accepted. Batch B3 of [the Chrome-installer plan](../windows/chrome-installer-plan.md) (track
`impl-chrome-installer-taskschd`). Google's updater registers a task
`\GoogleSystem\GoogleUpdater\GoogleUpdaterTaskSystem<version>{GUID}` through the Task Scheduler 2.0 object model
(`RegisterTaskDefinition`, a daily and a logon trigger repeating hourly, one exec action `updater.exe --wake
--system`, SYSTEM) and relies on the scheduler to start it (the first `--wake` five minutes after the registration).
Inputs: the 133 `taskschd.dll` sheets of `spec-win-taskschd` (PR #299) and their 44 visible tests,
`specs/windows/taskschd.dll/TASKSCHD.md`, Microsoft Learn ("Task Scheduler 2.0 Interfaces", the page of each
interface, "Task Scheduler Schema", `TASK_CREATION`, `TASK_LOGON_TYPE`, `TASK_STATE`, "Trigger Types"), the SDK
headers `taskschd.h` and `winerror.h`, and ADR 0220 (COM across processes) and ADR 0221 (Windows services, B1,
not merged yet). No third-party source.

## Context

What the sheets require:

- **An in-process COM class.** `CoCreateInstance(CLSID_TaskScheduler)` works with `CLSCTX_INPROC_SERVER` and fails
  with `CLSCTX_LOCAL_SERVER` alone: the class is `taskschd.dll`'s, threading model Both. Every interface is dual
  (`IUnknown`, `IDispatch`, then the methods in `taskschd.h` order). The test programs and the updater call the
  vtables, never `IDispatch::Invoke`.
- **Definitions are local objects.** `NewTask` works before `Connect`; a definition's parts (registration info,
  principal, settings, triggers, actions, a trigger's repetition) are one object each, the same pointer on every
  call; the setters store any text and validate nothing; `get_XmlText` is the full 27-line document of a new
  definition (TASKSCHD.md section 4) and follows every change.
- **A machine-wide task store.** Folders and tasks keyed without regard to case and kept with the case they were
  made with; a registration makes the folders on its way; `CREATE`/`UPDATE`/`VALIDATE_ONLY`/`DISABLE`; the stored
  form is a normalised XML text (defaults left out, a `URI`, the principal `Author`, SIDs); the store survives
  reboots and is shared by every process.
- **A scheduler.** `Run` starts the action's program as the task's account and the task is State 4 while it runs;
  `Stop` ends it (`0x00041306`); the exit code becomes `LastTaskResult`; a second `Run` while one instance runs is
  ignored (policy IgnoreNew); deleting a running task leaves its process running. Triggers start tasks by
  themselves (the updater's daily trigger with an hourly repetition and its logon trigger).

On Windows the store and the scheduler are the `Schedule` service (`svchost`), and `taskschd.dll` talks to it over
RPC. Nothing of that protocol is observable, so Poly chooses its own (as ADR 0220 did for COM).

## Decision

### 1. Three parts

| Part | Where | Does |
|---|---|---|
| The logic | `crates/poly-taskschd` (host-tested, `no_std`) | the definition model and its two XML forms, the XML parser and the registration's validation, paths and their errors, the store (folders, tasks, instances, states), trigger times, the persisted form, the wire messages |
| `taskschd.dll` | `personalities/nt/win32/taskschd` | the COM class and its ~30 interfaces; definitions live here; folder and task calls go to the scheduler process |
| `schedsvc.exe` | `personalities/nt/win32/schedsvc` | Poly's `Schedule` service: owns the store, persists it, starts and watches the tasks' processes, fires triggers |

### 2. The definition model

A definition is plain data (`poly_taskschd::def::Definition`): registration info, principal, settings (with idle
and network settings), triggers (one struct for every type; the fields a type does not use stay empty), actions,
the actions' `Context` and `Data`. Text properties are `Option<String>` (NULL vs. empty is observable);
`VARIANT_BOOL`s are kept as the 16-bit value given (`put_Hidden(1)` reads back 1, neither true nor false); numbers
as given, with the few range checks the sheets list at the setter (Priority 0–10, Compatibility 0–6 and -1,
DaysInterval ≥ 1, LogonType ≠ 0). Account names are canonicalised at the setter (`S-1-5-18`, `NT AUTHORITY\SYSTEM`
→ `SYSTEM`, `S-1-5-32-545` → `Users`, …) and written as SIDs in the XML.

`taskschd.dll` keeps each part in a shared cell so that the part objects are stable and a removed trigger's object
still answers; it snapshots the cells into a `Definition` to write XML or to register.

The two XML forms (UTF-16 in the BSTR, CR LF, two spaces of indentation):

- **Definition form** (`ITaskDefinition::XmlText`): the 27-line text of a new definition; triggers carry
  `<Enabled>true</Enabled>`; the principal is written only when something in it was set; settings in the order of
  TASKSCHD.md section 4, always all of them.
- **Registered form** (`IRegisteredTask::Xml`, what the store keeps): RegistrationInfo (with the task's `URI`),
  Principals (`id="Author"`), Settings with defaults left out, Triggers, Actions (`Context="Author"`); the element
  orders the recorded runs show (e.g. a daily trigger is `StartBoundary`, `Repetition`, `ScheduleByDay`; a monthly
  day-of-week trigger lists `Months`, `Weeks`, `DaysOfWeek` here but `Weeks`, `DaysOfWeek`, `Months` in the
  definition form).

`put_XmlText` and `RegisterTask` parse with a small XML reader of Poly's own (elements, attributes, text, the five
entities, comments and the declaration); the errors are the sheets': malformed `0x8004131A`, unknown or misplaced
element `0x80041316`, foreign namespace `0x80041317`, a value that is not valid (a version 1.7 or above, a bad
date) `0x80041318`, a required element missing `0x80041319`.

### 3. The store and its file layout

`poly_taskschd::store::Store` is a tree of folders with tasks; every operation of `ITaskFolder` and
`IRegisteredTask` is a method returning the sheets' result codes. Each task holds its definition (the registered
form is written from it), whether it is enabled (`put_Enabled(false)` disables without writing `<Enabled>` into
the XML; a `DISABLE` registration or a definition with `Settings.Enabled` false writes it), its last run time and
result, its missed runs and its running instances. The root has `\Microsoft\Windows` from the start (the tests and
real programs open it).

On disk, as on Windows, a task is a file and a folder a directory under `C:\Windows\System32\Tasks` (the file
holds the registered XML, UTF-16LE with a byte order mark). Poly adds one file of its own,
`C:\Windows\System32\Tasks\.poly-state` (a dot name is not a valid task or folder name, so it can never collide),
with what is not part of the XML: enabled flags, last run times and results, missed runs. Windows' registry cache
(`Schedule\TaskCache\Tree`, `Tasks`, `Logon`) is not written: no program the sheets saw reads it, and the files are
enough to rebuild the store at start. The scheduler writes through on every change.

### 4. The scheduler process

`C:\Windows\System32\schedsvc.exe` (Poly's own file, installed by the kernel's layout like `comlaunch.exe`) serves
the pipe `\\.\pipe\poly-taskschd`, one request and one reply per connection (framed as ADR 0220's: a `u32` length,
then the payload of `poly_taskschd::wire`). A thread per connection answers from the store under a lock; one more
thread, the **clock**, runs every 100 ms: it reaps the tasks' processes that ended (exit code → `LastTaskResult`,
State back to 3), and fires the triggers whose time has come.

**Running a task.** The command is the exec action's `Path` (quoted when it has a space and no quotes, environment
references expanded) and its `Arguments`, started with `CreateProcessW` in the action's working directory. The
process runs with the scheduler's identity: when B1 runs the scheduler as a service that identity is LocalSystem
(ADR 0221), which is what a SYSTEM task needs; until then it is the identity of whoever started the scheduler. A
task for the interactive user runs the same way (Poly has one user). `Stop` and `IRunningTask::Stop` end the
process (`TerminateProcess`) and set `0x00041306`; `DeleteTask` forgets the instance and leaves the process
running, as the sheet observed.

**Triggers, in this order of work** (the updater needs the first two):

| Trigger | Fires |
|---|---|
| daily (`CalendarTrigger`/`ScheduleByDay`) | at `StartBoundary` + n × `DaysInterval` days, and inside each day at every `Repetition.Interval` for `Repetition.Duration` (the whole day when the duration is absent and an interval is given) |
| logon | when the scheduler sees the user session start: at its own start after boot (Poly logs its one user on at boot), then every `Repetition.Interval` |
| boot | at the scheduler's start after boot, after `Delay` |
| time | once at `StartBoundary`, then the repetition |
| registration | when the task is registered (after `Delay`) |

Weekly, monthly, idle, event and session triggers are stored and written but do not fire yet. `EndBoundary` ends a
trigger; a disabled task or trigger never fires; `MultipleInstances` IgnoreNew (the default and the updater's) skips
a firing while an instance runs, Parallel starts another, Queue and StopExisting are treated as IgnoreNew for now.
Times are local time, which is UTC on Poly (`TimeZoneKeyName` UTC); a `StartBoundary` with an offset or `Z` is
converted.

**Missed runs.** The scheduler stores, per task, the time up to which it has handled the triggers. At start it
counts the firings that fell into the time it was not running: with `StartWhenAvailable` (the updater's setting) it
runs the task once at once; the count goes to `NumberOfMissedRuns` either way.

**Who starts it.** Until B1 lands, `taskschd.dll` starts `schedsvc.exe` on demand under a named mutex (as `ole32`
starts `comlaunch.exe`) the first time a service object connects; it then runs until the machine stops. That covers
the installer: it registers the task, which starts the scheduler, and the first `--wake` follows from the clock.
After a reboot nothing fires until a program connects. **The switch to B1** (ADR 0221): register
`HKLM\SYSTEM\CurrentControlSet\Services\Schedule` (own process, auto start, `ImagePath`
`%SystemRoot%\System32\schedsvc.exe`, `ObjectName` LocalSystem) in the registry defaults, give `schedsvc.exe` a
service main (`StartServiceCtrlDispatcherW`, status RUNNING, stop on `SERVICE_CONTROL_STOP`), and let
`taskschd.dll` call `StartServiceW("Schedule")` instead of `CreateProcessW` (keeping the mutex path only as the
fallback when the manager answers `ERROR_CALL_NOT_IMPLEMENTED`). The kernel then starts `services.exe` at the first
NT use because an auto-start service exists, `services.exe` starts the scheduler as LocalSystem, and boot and
logon triggers fire after a reboot without any program asking.

### 5. `taskschd.dll`

- The class factory and the service object as the `DllGetClassObject` sheet says (no aggregation, `LockServer`
  S_OK, `QueryInterface` for `IUnknown`, `IDispatch`, `ITaskService` only).
- `IDispatch` is answered with no type information: `GetTypeInfoCount` gives 0, the rest `E_NOTIMPL`. Scripting
  clients would need a type library (later work).
- `Connect` accepts the local forms (empty, NULL, empty string, `localhost`, `127.0.0.1`, the computer name) and
  refuses the rest with `0x80070035`, as observed; `HighestVersion` is `0x00010006`.
- A folder or task object holds the path as asked (its `Name`/`Path` keep that text) and asks the scheduler on each
  call, so a deleted task's object answers its name and fails its state with `0x80070002`, and collections are
  snapshots whose items are live.
- `CLSID_TaskScheduler` is registered by the layout's registry sync at every boot:
  `HKLM\SOFTWARE\Classes\CLSID\{0F87369F-A4E5-4CFC-BD3E-73E6154572DD}\InprocServer32` = `taskschd.dll`, threading
  model Both.

## Consequences

- Google's updater can register its task and the scheduler starts `updater.exe --wake --system` from the daily
  trigger's hourly repetition while Poly runs; the logon and boot triggers fire after a reboot once B1 starts the
  scheduler as a service.
- Two new system files, `taskschd.dll` and `schedsvc.exe`, built with the Win32 modules and embedded in the kernel.
- Tasks run with the scheduler's identity until B1; a task for another account than SYSTEM or the user is not
  separated from them.
- Not done: `RunEx`, `GetRunTimes`, security descriptors of folders and tasks (accepted, a fixed DACL text is
  returned), the 32-bit view, the COM handler / e-mail / message actions at run time, the triggers of section 4 that
  do not fire, `IDispatch` with type information, the Task Scheduler 1.0 interfaces (`mstask`).
