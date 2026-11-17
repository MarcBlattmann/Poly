# The Task Scheduler 2.0 COM API: what the sheets of taskschd.dll share

Observed on homeserver2 (Windows 11 25H2, build 26200.9974) by the tests of `spec-win-taskschd`, every test run twice
with identical output; the interface and method orders, enumerations and error codes are from the public SDK headers
`taskschd.h` and `winerror.h` (Windows Kits 10.0.26100.0) and Microsoft Learn ("Task Scheduler 2.0 Interfaces").
No Microsoft or Google binary was decompiled or disassembled, Wine and ReactOS were not read.

## 1. What Google's updater does with it (batch B3 of docs/windows/chrome-installer-plan.md)

Evidence: the updater's log line `task_scheduler.cc: RegisterTaskDefinition:<task name>: "<updater.exe>" --wake --system`
(a call of the object model, not of `RegisterTask`), the registry difference of the installer run (`HKLM\SOFTWARE\Microsoft\Windows
NT\CurrentVersion\Schedule\TaskCache`, `Tree`, `Tasks`, `Logon`), the `schtasks`-style list before and after, and the
GUIDs that appear in the data of `updater.exe` 157 and 159 (found by a byte search for the 16-byte values of the public IIDs;
nothing was disassembled).

| Fact | Evidence |
|---|---|
| it creates the scheduler with `CoCreateInstance(CLSID_TaskScheduler {0F87369F-A4E5-4CFC-BD3E-73E6154572DD}, IID_ITaskService {2FABA4C7-4DA9-4013-9697-20CC3FD40F85})` | both GUIDs are in the data of both updaters |
| it asks the triggers it creates for `IDailyTrigger {126C5CD8-B288-41D5-8DBF-E491446ADC5C}` and `ILogonTrigger {72DADE38-FAE4-4B3E-BAF4-5D009AF02B1C}`, and the action for `IExecAction {4C3D624D-FD6B-49A3-B9B7-09CB3CD3F047}` | the three IIDs are in the data of both updaters; the IIDs of `ITimeTrigger`, `IBootTrigger`, `IEventTrigger`, `IWeeklyTrigger`, `IMonthlyTrigger`, `IIdleTrigger`, `IRegistrationTrigger`, `ITaskFolder`, `IRegisteredTask`, `ITaskDefinition`, `IPrincipal`, `ITaskSettings`, `IRegistrationInfo`, `IActionCollection`, `ITriggerCollection`, `IRepetitionPattern` are **not** (a method that returns an interface pointer needs no IID) |
| one task per updater version, name `GoogleUpdaterTaskSystem<version>{<GUID>}`, in the folder `\GoogleSystem\GoogleUpdater\` (neither folder existed; both appear in the cache with their own descriptors, so a registration through a path with folders makes them) | task list and `TaskCache\Tree` before/after |
| principal SYSTEM (stored as `S-1-5-18`, displayed `SYSTEM`) | task list (`User` SYSTEM), the stored `Author` `NT AUTHORITY\SYSTEM` |
| action: command line `"C:\Program Files (x86)\Google\GoogleUpdater\<version>\updater.exe" --wake --system`; the stored command has the quotes around the path and the arguments `--wake --system` apart | the stored `Actions` value |
| description `GoogleUpdater Task System <version>`; the task's schema version is 1.2 | stored `Description`, `Schema` 0x10002 |
| two triggers, a calendar (daily) trigger and a logon trigger, both with a repetition (an interval of 3600 s appears twice in the stored triggers); the task is listed in the cache's `Logon` list | stored `Triggers`, `TaskCache\Logon` |
| state `Ready` after the registration; the first `--wake` run was started by the scheduler 5 minutes after the registration (10:03:00 registered, 10:08:00.141 the `--wake` process logged "UpdaterMain (--wake)") | the task list and the updater's log |
| the standalone installer's run ended with **two** tasks (versions 157 and 159, each with its own GUID) a minute after the update: the new version registers its own task; the owner's brief for this track says the updater deletes and registers its task again when it updates itself (its own clean-up of an older version's task was not seen in the snapshot) | task list after the standalone run |
| on uninstall the updater removed its services, task and folder about 20 s after the last application was gone | uninstall run |

What Poly needs from it: `CoCreateInstance` of the class, `Connect`, `GetFolder`, `NewTask`, the principal/settings/
registration/trigger/action objects as the updater fills them, `RegisterTaskDefinition` (with a path that makes the folders),
`GetTask`, `DeleteTask`, `DeleteFolder` and `GetTasks` for the clean-up, the XML text of the registered task, and a scheduler that runs the task by its
triggers and by `Run`. Which of these the updater really calls beyond the one log line is **not known**: the sheets
cover all of them, and say in each sheet whether the updater is known to use it.

## 2. Object model and interface slots

Every interface is a dual interface: `IUnknown` (slots 0-2), `IDispatch` (slots 3-6), then its own methods from slot 7,
in the order below (taskschd.h). A property is a pair `get_X` then `put_X` in this order; strings are BSTRs, a
`VARIANT` argument is passed as the 24-byte structure, Booleans are `VARIANT_BOOL` (-1 true, 0 false).

| Interface | Methods from slot 7 |
|---|---|
| ITaskService | GetFolder, GetRunningTasks, NewTask, Connect(4 VARIANTs), get_Connected, get_TargetServer, get_ConnectedUser, get_ConnectedDomain, get_HighestVersion |
| ITaskFolder | get_Name, get_Path, GetFolder, GetFolders, CreateFolder(BSTR, VARIANT sddl), DeleteFolder, GetTask, GetTasks, DeleteTask, RegisterTask(path, xml, flags, user, password, logon, sddl), RegisterTaskDefinition(path, definition, flags, user, password, logon, sddl), GetSecurityDescriptor, SetSecurityDescriptor |
| ITaskFolderCollection, IRegisteredTaskCollection, IRunningTaskCollection | get_Count (slot 7), get_Item(VARIANT index, 1-based or a name) (slot 8), get__NewEnum |
| IRegisteredTask | get_Name, get_Path, get_State, get_Enabled, put_Enabled, Run(VARIANT params, IRunningTask**), RunEx, GetInstances, get_LastRunTime, get_LastTaskResult, get_NumberOfMissedRuns, get_NextRunTime, get_Definition, get_Xml, GetSecurityDescriptor, SetSecurityDescriptor, Stop, GetRunTimes |
| IRunningTask | get_Name, get_InstanceGuid, get_Path, get_State, get_CurrentAction, Stop, Refresh, get_EnginePID |
| ITaskDefinition | get_RegistrationInfo, put_RegistrationInfo, get_Triggers, put_Triggers, get_Settings, put_Settings, get_Data, put_Data, get_Principal, put_Principal, get_Actions, put_Actions, get_XmlText, put_XmlText |
| ITriggerCollection | get_Count, get_Item(LONG index), get__NewEnum, Create(TASK_TRIGGER_TYPE2), Remove(VARIANT), Clear |
| ITrigger | get_Type, get_Id, put_Id, get_Repetition, put_Repetition, get_ExecutionTimeLimit, put_, get_StartBoundary, put_, get_EndBoundary, put_, get_Enabled, put_Enabled |
| IDailyTrigger / ILogonTrigger / ITimeTrigger / IBootTrigger | ITrigger's 13 methods, then: DaysInterval (SHORT), RandomDelay / Delay, UserId / RandomDelay / Delay (get, put each) |
| IRepetitionPattern | get_Interval, put_, get_Duration, put_, get_StopAtDurationEnd, put_ |
| IActionCollection | get_Count, get_Item(LONG), get__NewEnum, get_XmlText, put_XmlText, Create(TASK_ACTION_TYPE), Remove(VARIANT), Clear, get_Context, put_Context |
| IAction / IExecAction | get_Id, put_Id, get_Type; then get_Path, put_, get_Arguments, put_, get_WorkingDirectory, put_ |
| IPrincipal | get_Id, put_, get_DisplayName, put_, get_UserId, put_, get_LogonType, put_, get_GroupId, put_, get_RunLevel, put_ |
| ITaskSettings | AllowDemandStart, RestartInterval, RestartCount, MultipleInstances, StopIfGoingOnBatteries, DisallowStartIfOnBatteries, AllowHardTerminate, StartWhenAvailable, XmlText, RunOnlyIfNetworkAvailable, ExecutionTimeLimit, Enabled, DeleteExpiredTaskAfter, Priority, Compatibility, Hidden, IdleSettings, RunOnlyIfIdle, WakeToRun, NetworkSettings (each get_ then put_) |
| IRegistrationInfo | Description, Author, Version, Date, Documentation, XmlText, URI, SecurityDescriptor, Source (each get_ then put_) |

The class `CLSID_TaskScheduler` is a CoCreateInstance-able in-process class (`taskschd.dll`, threading model Both); objects
are returned for the interface asked (`ITaskService`, `IDispatch`, `IUnknown`).

Enumerations (taskschd.h): TASK_STATE 0 unknown, 1 disabled, 2 queued, 3 ready, 4 running. TASK_CREATION flags 1 validate only,
2 create, 4 update, 6 create or update, 8 disable, 0x10 don't add principal ACE, 0x20 ignore registration triggers.
TASK_LOGON_TYPE 0 none, 1 password, 2 S4U, 3 interactive token, 4 group, 5 service account, 6 interactive token or password.
TASK_RUNLEVEL 0 limited (LUA), 1 highest. TASK_TRIGGER_TYPE2 0 event, 1 time, 2 daily, 3 weekly, 4 monthly, 5 monthly day of week, 6 idle,
7 registration, 8 boot, 9 logon, 11 session state change, 12 custom. TASK_ACTION_TYPE 0 exec, 5 COM handler, 6 e-mail, 7 message.
TASK_INSTANCES_POLICY 0 parallel, 1 queue, 2 ignore new, 3 stop existing. TASK_COMPATIBILITY 0 AT, 1 V1, 2 V2 (the default), 3 V2.1 ... 6 V2.4.

## 3. Result codes seen

| HRESULT | Meaning where seen |
|---|---|
| 0x80070002 | the task or folder does not exist (also update of a missing task, delete of a missing one) |
| 0x800700B7 | already exists (create of an existing task or folder) |
| 0x8007007B | the path or name has a character that is not allowed (`:` `/` doubled or trailing `\`, leading space) |
| 0x80070057 | invalid argument (flags not allowed, a name too long, an index out of range, a number out of range) |
| 0x80070091 | the folder is not empty |
| 0x80070005 | access denied (not elevated, or an account refused) |
| 0x80070035 | the network path was not found (a server name that is not this machine) |
| 0x800704E3 | the service object is not connected |
| 0x80004003 | NULL out pointer (E_POINTER) |
| 0x80004001 | not implemented (the XML text of a collection, of the registration info and of the settings) |
| 0x80040110 | aggregation refused; 0x80040154 class not registered for the context (local server only) |
| 0x80041326 | the task is disabled (Run); 0x80041328 the task cannot be started on demand; 0x8004130B the task is not running |
| 0x80041319 | a required element is missing (a definition with no action, an exec action with an empty command, a weekly or monthly trigger without days) |
| 0x8004131A malformed XML; 0x80041316 an element that is not allowed there; 0x80041317 a namespace that is not the task namespace; 0x80041318 a value that is not valid (a version, a date) |
| 0x00041303 | `LastTaskResult` of a task that has not run yet (SCHED_S_TASK_HAS_NOT_RUN); 0x00041301 while running; 0x00041306 after Stop (terminated) |
| 0x80070534 | no mapping of the account name (an account that does not exist); 0x8007052E logon failure |

## 4. The task XML (what `get_Xml` and `get_XmlText` return)

UTF-16 text, lines end with CR LF, two spaces of indentation, no byte order mark in the BSTR, this declaration and root:

```
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
```

A **new definition** (`NewTask`) gives exactly (27 lines; `RegistrationInfo`, `Triggers`, `Actions` empty elements, the settings in this order):

```
  <RegistrationInfo />
  <Triggers />
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>true</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>true</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>false</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <Duration>PT10M</Duration>
      <WaitTimeout>PT1H</WaitTimeout>
      <StopOnIdleEnd>true</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <RunOnlyIfIdle>false</RunOnlyIfIdle>
    <WakeToRun>false</WakeToRun>
    <ExecutionTimeLimit>PT72H</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions />
</Task>
```

The **registered task's XML** (`IRegisteredTask::Xml`) is normalised and shorter: elements still at their defaults are left out
(`AllowStartOnDemand`, `Enabled`, `Hidden`, `RunOnlyIfIdle`, `WakeToRun`, `AllowHardTerminate`, `RunOnlyIfNetworkAvailable`, `Priority`,
`StartWhenAvailable` when false, `ExecutionTimeLimit` when PT72H), a `URI` is added to the registration info, and the principal is named
`Author`. A SYSTEM task registered the way the updater does it (SYSTEM, logon type SERVICE_ACCOUNT, run level highest, start when
available, battery flags false, no time limit, a daily trigger repeating every hour for a day and a logon trigger repeating every hour,
one exec action) reads back as (49 lines):

```
<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>PolySpec updater-like task 1.0</Description>
    <URI>\PolySpec\tfg\Gu\Task1{9BC308BB-98D3-4838-9AE6-66AF62C749BD}</URI>
  </RegistrationInfo>
  <Principals>
    <Principal id="Author">
      <UserId>S-1-5-18</UserId>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <StartWhenAvailable>true</StartWhenAvailable>
    <IdleSettings>
      <Duration>PT10M</Duration>
      <WaitTimeout>PT1H</WaitTimeout>
      <StopOnIdleEnd>true</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
  </Settings>
  <Triggers>
    <CalendarTrigger>
      <StartBoundary>2030-01-01T00:00:00</StartBoundary>
      <Repetition>
        <Interval>PT1H</Interval>
        <Duration>P1D</Duration>
      </Repetition>
      <ScheduleByDay>
        <DaysInterval>1</DaysInterval>
      </ScheduleByDay>
    </CalendarTrigger>
    <LogonTrigger>
      <Repetition>
        <Interval>PT1H</Interval>
      </Repetition>
    </LogonTrigger>
  </Triggers>
  <Actions Context="Author">
    <Exec>
      <Command>"C:\path\to\the\program.exe"</Command>
      <Arguments>--child mark</Arguments>
    </Exec>
  </Actions>
</Task>
```

The element names of the trigger types: time `TimeTrigger`, daily `CalendarTrigger` with `ScheduleByDay`, weekly `CalendarTrigger` with
`ScheduleByWeek` (`DaysOfWeek`, `WeeksInterval`), monthly `CalendarTrigger` with `ScheduleByMonth`, monthly day-of-week `CalendarTrigger` with
`ScheduleByMonthDayOfWeek`, boot `BootTrigger`, logon `LogonTrigger`, idle `IdleTrigger`, registration `RegistrationTrigger`, event
`EventTrigger`, session state change `SessionStateChangeTrigger`. A principal of the interactive user is
`<Principal id="Author"><UserId>S-1-5-21-...</UserId><LogonType>InteractiveToken</LogonType></Principal>`; the SYSTEM principal writes no `LogonType`.
The action types: `Exec` (`Command`, `Arguments`, `WorkingDirectory`), `ComHandler`, `SendEmail`, `ShowMessage`; the `Actions` element has
`Context="Author"` (the id of the principal).

## 5. How the tests work

Each test is a program that makes its own tasks under the folder `\PolySpec\<test>`, deletes everything of its own at the start and at the end,
never touches any other task, and prints observations (result codes in hex, counts, flags, texts with the path of its own image replaced
by `<image>` and the machine's account SIDs by `<user-sid>`). The task's action is the test's own image started with `--child <mode>` by the
scheduler as SYSTEM: `mark` writes `<image>.mark`, `exit7` writes it and ends with 7, `hold` writes `<image>.run`, waits for `<image>.go`
(at most 20 s) and ends. A waiting loop never exceeds 8 s.
