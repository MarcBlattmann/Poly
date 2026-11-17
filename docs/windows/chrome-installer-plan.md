# Google Chrome's Windows installers on Poly: survey and plan

Track `survey-chrome-installer` (spec agent, 2026-11-17). Goal of the owner: in Chrome on Poly, download
Chrome for Windows from google.com, double-click the installer, have it install as on Windows, and find a
clean Chrome for Windows in the launcher. Today Poly runs Chrome for Testing unpacked from a zip
(ADR 0155, 0216). Running `.exe` installers through Poly's installer is ADR 0132's open item.

Inputs: Google's two consumer installers run on the Windows oracle (`homeserver2`, Windows 11 build 26200.9974,
an elevated SSH session) with Process Monitor, snapshots before and after, and the updater's and
Chrome's own logs; then the same installers (and the updater taken from them) run on Poly in QEMU
(`cargo xtask win-app run`). No Google binary was decompiled or disassembled; the PE import *names* and
the programs' own logs are the inputs. Nothing of Google's is committed.

## 1. What the installers do on Windows

Both installers are small 32-bit programs (`ChromeSetup.exe` 13 MB online stub, `ChromeStandaloneSetup64.exe`
170 MB offline). Both are Authenticode-signed (Google LLC), `requestedExecutionLevel asInvoker`: **they
do not ask Windows for elevation themselves**. The updater they carry does, when it finds it is not
elevated (`ShellExecuteEx` with the `runas` verb on itself). On the oracle the session was already
elevated (`IsUserAdmin: 1, IsElevatedWithUACOn: 1`), so no UAC prompt was seen; the consent step itself
was not traced (an SSH session cannot show it).

Process tree of the stub (53 s from double click to a running Chrome; the standalone takes 34 s):

| Step | Process | What it does |
|---|---|---|
| 1 | `ChromeSetup.exe` | unpacks Google's updater to `C:\Windows\SystemTemp\Google<pid>_<n>\bin\updater.exe` (the standalone also unpacks `bin\Offline\{...}\{appid}\157.x_chrome_installer.exe`), runs `updater.exe --install=appguid=...&iid=...&lang=en&browser=4&usagestats=0&appname=Google%20Chrome&needsadmin=prefers&ap=x64-statsdef_1&installdataindex=empty` |
| 2 | `updater.exe --install` (32-bit, "opt") | shows its own small progress window; tries `CoCreateInstance` of its COM class (fails, `0x80040154`: not registered yet); runs *Setup* in place: copies itself to `C:\Program Files (x86)\Google\GoogleUpdater\<ver>\`, registers COM classes, **creates two Windows services** and **one scheduled task**, writes `prefs.json` |
| 3 | `services.exe` starts `updater.exe --system --windows-service --service=update-internal` and `--service=update` (LocalSystem, each with a `--crash-handler` child) | the *update* service is the COM server (`--com-service`) the install talks to |
| 4 | service `update` | HTTPS `POST https://update.googleapis.com/service/update2/json` (protocol 4.0, 12 requests), downloads the `.crx3` from `dl.google.com`/`www.google.com`/`edgedl.me.gvt1.com` (stub only), unpacks it with a `--unzip-worker` child (a Mojo process), starts `<ver>_chrome_installer_uncompressed.exe --verbose-logging --do-not-launch-chrome --channel=stable --installerdata=...` |
| 5 | Chrome's mini installer, `setup.exe` (64-bit) | `setup.exe --mini-installer-path=...`, then `setup.exe --channel=stable --system-level --verbose-logging --installerdata=...`: copies `C:\Program Files\Google\Chrome\Application\` (269 files), writes the registry, shortcuts, the elevation service, the event-log source, a firewall rule |
| 6 | `Explorer.EXE` starts `chrome.exe --from-installer` | the updater asks the shell of the user's session to start Chrome; first start creates `%LOCALAPPDATA%\Google\Chrome\User Data` (550 files), starts the crash handler, GPU, network, storage, renderer and extension processes; `services.exe` starts `elevation_service.exe` |

### Things created (observed: before/after snapshots and the logs; the machine was clean before)

- **Services (3)**: `GoogleUpdaterService<ver>` and `GoogleUpdaterInternalService<ver>` (own process, auto start,
  LocalSystem, depend on `RPCSS`, `WOW64=0x14c`), `GoogleChromeElevationService` (own process, manual start,
  LocalSystem, depends on `RPCSS`). Written by `services.exe` as `HKLM\SYSTEM\CurrentControlSet\Services\<name>`
  with `Type`, `Start`, `ErrorControl`, `ImagePath`, `DisplayName`, `DependOnService`, `ObjectName`, `Description`.
  The updater **updates itself**: the standalone installs updater 157.0.8089.0 and a minute later 159.0.8140.0 has
  its own two services and task beside it (one new versioned set per update).
- **Scheduled task (1)**: `\GoogleSystem\GoogleUpdater\GoogleUpdaterTaskSystem<ver>{GUID}`, SYSTEM, action
  `updater.exe --wake --system`, registered through the Task Scheduler 2.0 COM API (`RegisterTaskDefinition`).
- **COM**: 7 `AppID`s with `LocalService` + `ServiceParameters=--com-service` (the updater's classes are
  started by the SCM on activation), 4 `CLSID` keys, one `LocalServer32` (`notification_helper.exe`),
  `GoogleUpdate.Update3WebMachine`, 37 `Interface` keys whose `ProxyStubClsid32` is `{00020424-...}` (the
  type-library marshaller, "PSOAInterface") and 37 `TypeLib` keys pointing at `updater.exe\4` (the library is a
  resource of the updater's exe), in both the 64-bit and the `WOW6432Node` view.
- **Shell and file associations**: `ChromeHTML` and `ChromePDF` ProgIDs, `.htm .html .mhtml .shtml .svg .xht
  .xhtml .webp .pdf` OpenWithProgids, `Clients\StartMenuInternet\Google Chrome` (capabilities, URL
  associations http/https/mailto/...), `RegisteredApplications`, `App Paths\chrome.exe`, Active Setup
  (`chrmstp.exe --configure-user-settings` and the Platform Experience Helper).
- **Shortcuts (`.lnk`)**: `C:\Users\Public\Desktop\Google Chrome.lnk`,
  `C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Google Chrome.lnk`, a per-user Quick Launch link.
- **Other**: uninstall entry `HKLM\...\Uninstall\Google Chrome` (`setup.exe --uninstall --channel=stable
  --system-level`), event-log source `Application\Chrome`, firewall rule "Google Chrome (mDNS-In)",
  `C:\Program Files (x86)\Google\GoogleUpdater\{crx_cache (528 MB), prefs.json, updater.log,
  updater_history.jsonl}`.
- **Network**: only HTTPS to `update.googleapis.com`, `dl.google.com`/`www.google.com`/`edgedl.me.gvt1.com`
  (also plain HTTP) and `clients2.google.com/cr/report` (crash reports); Chrome itself then uses QUIC.

### Uninstall (the normal way)

`setup.exe --uninstall --channel=stable --system-level --verbose-logging --force-uninstall` (what Apps
settings runs) exits with 19, deletes the program files, shortcuts, services, ProgIDs and the uninstall entry;
**the updater removes itself about 20 s later** (its services, task and folder) because no app is left.
Left behind on the oracle: the user's Chrome profile and `HKCU\Software\Google`, the empty `C:\Program
Files (x86)\Google`, `HKLM\SOFTWARE\WOW6432Node\Google\Update` (the updater's state), the `ChromePDF` ProgID,
one Active Setup entry, two log files in `C:\Windows\SystemTemp`. Windows leaves them too; they are not
a problem for later oracle runs.

### First start of the installed Chrome

`chrome.exe --from-installer`: crash handler (`--type=crashpad-handler`), GPU, network, storage, data-decoder,
5 renderers. It is the same program as Chrome for Testing 157 (same `chrome.exe` + versioned folder layout),
so everything of stages S1 to S5 (ADR 0155, 0206, 0211, 0216) applies; what is new is `elevation_service.exe`
(COM, started by the SCM) and the registered `notification_helper.exe` COM server.

## 2. Where Poly stops

Poly runs both installers as 32-bit WOW64 programs (ADR 0186). With `cargo xtask win-app run` (console
mode, user networking) on this tree (`origin/main` 9e3188251):

| # | Stop | Evidence |
|---|---|---|
| 1 | `shell32!IsUserAnAdmin` and `ShellExecuteExW` are fail stubs. The stub calls them to relaunch itself elevated (`runas`). A 32-bit fail stub takes no arguments, so `stdcall` leaves them on the stack and the program jumps to a stack address: `0xc0000005` at `0x12b348`, parameter `[8, ...]` | stub: `win-app run`, `launcher: code=0x0`, exception line |
| 2 | **Workaround 1** (`IsUserAnAdmin` returns TRUE; Poly's `Token::poly_user()` already is the elevated administrator's token, ADR 0171): the installer runs on. `C:\Windows\SystemTemp` does not exist on Poly's `C:` (it falls back to `C:\Program Files\Google<pid>_<n>\bin`), opening the volume `\??\C:` returns `0xc00000ba` (a directory, not a volume), then `kernel32!GetDriveTypeW` and **`SetProcessWorkingSetSize`** are fail stubs. The 32-bit stub then **ends the process with `0xc0000002`** | stub and standalone alike; same list of 9 loader reasons |
| 3 | Loader stubs the installer binds to and that Poly lacks (both installers): `advapi32!{CreateServiceW, DeleteService, ChangeServiceConfigW, ChangeServiceConfig2W, QueryServiceConfigW}`, `oleaut32!#161 #184`, `kernel32!{EnumSystemLocalesW, GetDateFormatW, GetTimeFormatW, GetUserPreferredUILanguages, GetDriveTypeW, QueryFullProcessImageNameW, SetProcessWorkingSetSize, WTSGetActiveConsoleSessionId}`, `kernelbase!{EnumSystemLocalesEx, GetDateFormatEx, GetTimeFormatEx, LCIDToLocaleName, LocaleNameToLCID}`, `ole32!{CoAddRefServerProcess, CoGetCallContext, CoRegisterClassObject, CoReleaseServerProcess, CoResumeClassObjects, CoRevokeClassObject}`, `wtsapi32!WTSEnumerateSessionsW`, `winhttp!WinHttpSetOption` | `cargo xtask win-app imports` of both installers and `win-app run` loader reasons |
| 4 | The **64-bit updater** (`updater.exe` 159.0.8140.0, taken out of the installed tree and run by hand with the stub's command line, workaround 1 on): imports bind except `oleaut32!#424`, `ole32!CoGetCallContext`; it stops at `ole32!CoIncrementMTAUsage` (stub), `CoCreateInstance` of `{0000034B-...}` (GlobalOptions) for `{0000015B-...}` not registered, then an `int3` (a failed check) at `updater.exe+0x9a3a6` | `win-app run --exe updater.exe --args "--install=..."` |
| 5 | The **32-bit updater** (157.0.8089.0) has the same imports plus `advapi32!{ImpersonateNamedPipeClient, StartServiceCtrlDispatcherW}`, `gdi32!{FrameRgn, OffsetWindowOrgEx, RoundRect}`, `msimg32!AlphaBlend`, `comctl32!#410 #412 #413`, `kernel32!K32QueryWorkingSetEx`; it ends at `ole32!CoIncrementMTAUsage` (a 32-bit fail stub ends the process) | same |
| 6 | The Windows installer front door: Poly's installer (`poly-pkg::format::detect`) knows deb, rpm, AppImage, Flatpak, Snap; **an `.exe` is not a format to it**. Files sniffs `MZ` as `application/x-msdownload` ("Windows program"), and `poly-files::apps::open_action` opens it with *whatever handler claims that MIME type*: **none does**, so a double click does nothing useful (text editor / "no app"). `application/x-msi` is named but handled by nobody either | `crates/poly-files/src/{mime,apps}.rs`, `crates/poly-pkg/src/format.rs` |

Everything beyond stop 4 (a service host, out-of-process COM with type-library marshalling, the Task
Scheduler, Chrome's `setup.exe`) was not reachable. Poly *does* already have: registry, tokens, WinHTTP, WOW64
(ADR 0186), `CoCreateInstance` for in-process classes, `wintrust`, `version`, the Chrome S1-S5 stack.
Import check of every Google binary (`win-app imports`): `ChromeSetup.exe` has 436 imports, 1 not present in our DLLs
(`ole32!CoGetCallContext`); `ChromeStandaloneSetup64.exe` 437 and the same one. The loader's list above is longer
because functions that exist only as fail stubs count as bound.

## 3. The plan, in batches (each = spec sheets, implementer, integrator)

Sizes: **S** one track of 10 entries or fewer, **M** one track of 10 to 25, **L** two tracks, **XL** three or
more; "track" = one spec session, one implementer session and the integration.
All function lists are from the survey; the coverage map has an entry for each name (`stub` today).

| Batch | Content | Entries (approx.) | Size | Unlocks |
|---|---|---|---|---|
| **B0 The installer starts** | `shell32!IsUserAnAdmin` (FALSE in an ordinary process, TRUE after a consent: decision 1), `ShellExecuteExW/W/A` (verb `open` = the caller's token; `runas` from a non-elevated caller = the consent dialog; `SEE_MASK_NOCLOSEPROCESS`), `C:\Windows\SystemTemp` in the layout and what `GetTempPath` returns for an elevated process, opening a volume (`\\.\C:`, and `\??\C:` as a volume), `GetDriveTypeW`, `SetProcessWorkingSetSize`, `K32QueryWorkingSetEx`, `WTSGetActiveConsoleSessionId`, `WTSEnumerateSessionsW`, `QueryFullProcessImageNameW`, `GetTimeFormatW`/`GetDateFormatW`/`EnumSystemLocalesW` and their `...Ex`/`LCIDToLocaleName`/`LocaleNameToLCID` kernelbase forms, `GetUserPreferredUILanguages`, `WinHttpSetOption`, `CoGetCallContext`, `CoIncrementMTAUsage`, `oleaut32 #161 #184 #424`. Plus, for the integrator, not an entry: **a 32-bit fail stub must pop its arguments** (today it ends the process, so one missing function ends a 32-bit installer) | ~25 | M | both installers extract and start the updater |
| **B1 Services (SCM)** | `advapi32`: `OpenSCManagerW`, `CreateServiceW`, `OpenServiceW`, `ChangeServiceConfig(2)W`, `QueryServiceConfig(2)W`, `QueryServiceStatus(Ex)`, `StartServiceW`, `ControlService`, `DeleteService`, `CloseServiceHandle`, `EnumServicesStatusExW`, `StartServiceCtrlDispatcherW`, `RegisterServiceCtrlHandlerExW`, `SetServiceStatus`; the services registry layout above; a **service host** in the NT personality: starts `Start=2` services at boot as LocalSystem, starts a service on demand (`StartServiceW`, COM `LocalService`), ends and restarts them | ~18 + the host | L | `CreateServiceW` of the updater and of Chrome's elevation service; the updater's `Setup succeeded` |
| **B2 COM across processes** | `CoRegisterClassObject`, `CoResumeClassObjects`, `CoRevokeClassObject`, `CoAddRefServerProcess`, `CoReleaseServerProcess`, `CoInitializeSecurity`, `CoImpersonateClient`, `CoSetProxyBlanket`, `CoGetPSClsid`/`CoRegisterPSClsid`, activation of `LocalServer32` and `AppID\LocalService` classes (with `ServiceParameters`), **marshalling through the type library** (`oleaut32` `LoadTypeLib`/`LoadTypeLibEx` of a resource `exe\4`, `RegisterTypeLib`, the PSOAInterface proxy and stub, `rpcrt4` as far as the updater's interfaces need), the 32/64-bit registry views | ~25-30 | **XL** | the install's talk to the update service; Chrome's `IElevator` (cookie encryption) |
| **B3 Task Scheduler** | `taskschd`: `CLSID_TaskScheduler`, `ITaskService`, `ITaskFolder` (`GetFolder`, `CreateFolder`, `RegisterTaskDefinition`, `DeleteFolder`), `ITaskDefinition` and the registration/trigger/action/principal/settings objects, `IRegisteredTask` (`Run`, `Enabled`); the task store (registry `Schedule\TaskCache` and the XML), a scheduler that runs `--wake` tasks | ~12 interfaces | M | the updater's `RegisterTaskDefinition`; also needed by other installers |
| **B4 Chrome's own `setup.exe`** | `IShellLinkW` + `IPersistFile` (`.lnk` writing and reading), `SHGetKnownFolderPath` for the common Desktop/Start Menu, `INetFwPolicy2` (the firewall rule; best effort), event-log source registration, Active Setup, `MoveFileEx` with delayed delete, hard links, `chrmstp.exe`, uninstall (`--uninstall`), the `notification_helper` COM server; tested by running the payload the standalone extracts (`win-app run --dir`) | ~15 | M | files, registry and shortcuts of Chrome |
| **B5 Poly's front door for `.exe`** (implementer only, no oracle) | `poly-pkg` format `WindowsProgram` (`MZ`) and `WindowsInstaller` (`.msi` OLE compound), the installer's desktop file claims `application/x-msdownload` and `x-msi`; shows publisher (Authenticode through `wintrust`), name, version, size from the PE version resource; the **Install click is the consent** (as for `.deb`, ADR 0173); runs the program in the NT personality as the elevated user with its own windows visible, a "running" state, waits for it and its children; afterwards **reads the `.lnk` files the installer created** (Common/Start Menu/Programs, Public Desktop; MS-SHLLINK format), makes launcher entries (Exec = the target, icon from the PE resources as ADR 0216), registers the `Uninstall` key in Settings → Apps; `Run` without installing for plain programs | n/a | M | double click to a launcher entry |
| **B6 MSI** | `msi.dll` (`MsiInstallProductW`, `MsiOpenDatabase`, the action engine, tables, custom actions), the Windows Installer service (`msiserver`) | ~40+ | **XL** | the enterprise `.msi` (`googlechromestandaloneenterprise64.msi`, 172 MB, found and downloaded, not run): **not used by Google's consumer installers; defer**. |
| **B7 End to end** | one boot check that runs the stub from the Downloads folder with the front door, finds Chrome in the launcher, starts it, uninstalls it and finds the services and task gone; auto-update self-swap (157 → 159: a second versioned service set), default-browser registration seen by Poly's settings | n/a | M | acceptance |

### What can be deferred or skipped

- **MSI (B6)**: not on Google's path. Skip until another installer needs it.
- **The updater's persistence**: cannot be skipped with Google's installers unchanged. Setup creates two services and a task
  in one step and the install is *driven through* the update service over COM, so B1, B2 and B3 are all on
  the path. Whether the updater accepts a failing `RegisterTaskDefinition` (so that B3 could be a no-op first)
  is unknown and cheap to test once B1 and B2 exist.
- **Auto-update**: can be made inert after the install by policy (`HKLM\SOFTWARE\Policies\Google\Update`:
  `UpdateDefault`, `AutoUpdateCheckPeriodMinutes`, `Update{8A69D345-...}`) without changing the installer; Poly
  can write them at the end of B5's run if the owner decides "off".
- **Elevation service** (`IElevator`, B2 follow-up): the oracle's service was started by Chrome's first run. Whether
  Chrome starts and browses without it was **not tested**; it is expected to fall back (it serves the encryption
  of "app-bound" data). Test it when B1 and B2 exist.
- **Firewall, event log, Active Setup, Platform Experience Helper**: best effort, need no function beyond B4's.

### Where the plan is least certain

The chain after the updater's first COM call was not reached on Poly (stop 4), so B1 to B3 are sized from the
oracle trace, not from Poly failures. The unknowns that move a size: (a) how much of the type-library marshaller
(`ITypeInfo` based) and `rpcrt4` the 37 updater interfaces need (XL may be L, or more); (b) whether the updater
survives without its scheduled task or without `CoInitializeSecurity` specifics; (c) the `Offline` payload and the
`--unzip-worker` Mojo process of the stub (the standalone avoids the download; the stub also needs
`WinHttp` against Google's servers on Poly's network stack).

## 4. Owner decisions

### Decided on 2026-11-17 (they replace the recommendations below where they differ)

1. **UAC is a separate prompt, as on Windows.** Normal programs run **not elevated**. A program that asks for
   administrator rights gets a consent dialog of Poly's own: `ShellExecute(Ex)` with `runas`, a `requireAdministrator`
   manifest (or `highestAvailable` for an administrator), COM elevation. Yes: the program runs with the elevated
   token. No: the documented failure (`ERROR_CANCELLED` 1223 with `hInstApp` 5 from `ShellExecuteExW`, 5 from
   `ShellExecuteW`; `ERROR_ELEVATION_REQUIRED` 740 from a plain `CreateProcess`). This changes Poly's token model
   (ADR 0171: today every process has `Token::poly_user()`, the elevated administrator's token): an administrator gets a
   **split token**, a filtered standard token plus a linked elevated token. The observable behaviour is specified by
   `specs/windows/advapi32.dll/UacStandardToken.yaml`, `UacElevatedToken.yaml`, `ObjectIntegrityLabel.yaml`,
   `kernel32.dll/CreateProcessElevation.yaml`, `shell32.dll/{IsUserAnAdmin,ShellExecuteExW.Elevation,ShellExecuteW,ShellExecuteA}.yaml`
   (spec track `spec-win-elevation`; recorded on the oracle with the filtered and with the full token, `poly-oracle.ps1 -Token
   standard|elevated`, docs/AGENT_PIPELINE.md). What the consent dialog looks like is Poly's own design.
2. **Auto-update is on from the start.** Google's updater services and task stay (2 services + 1 task per version).
3. **Google's installers run unchanged** (route A).
4. **The installed Chrome replaces Chrome for Testing** in the launcher (and the `desktop --chrome-win` disk starts without it
   once B7 passes).

The rest of this section is the survey's original list; items 1 to 3 and 5 are answered by the above.


1. **Elevation (UAC).** Recommended: every Poly process already has the elevated administrator's token (ADR
   0171). `IsUserAnAdmin` is TRUE, `ShellExecute` with `runas` is a plain launch, and the **Install click in
   Poly's installer is the consent** (as for `.deb`, ADR 0173); no secure-desktop prompt, no second prompt
   inside a program. Alternative: a real consent dialog for `runas` and for services starting a program (more
   Windows-like, a day or two more, and a lower-integrity token for ordinary programs, which the S2 sandbox
   work could use).
2. **Auto-updates.** On (Google's updater as on Windows: 2 services + 1 task per version, 528 MB `crx_cache`, a wake every
   few hours, background CPU/disk and network on a small desktop) or **off** (policy keys written after the
   install, and Chrome's own "update" check still works by downloading a new installer)? The recommendation is *off first*, on
   after B7 passes.
3. **Route.** (A) run Google's installers unchanged (this plan; services, COM across processes and the
   Task Scheduler are the cost, and they also serve every later installer) or (B) a Poly-specific shortcut that
   unpacks Chrome's payload without the updater (cheaper, but not "installs normally", and breaks on every
   Google change). The plan is route A.
4. **What a service is on Poly.** Start `Start=2` services at boot or on first use? Always LocalSystem? Visible
   in Settings? A service host that runs `services.exe`-like logic as a native Poly process is assumed.
5. **Replace Chrome for Testing.** When B7 passes, should the launcher's "Chrome" be the installed Chrome (and the
   `desktop --chrome-win` disk start without Chrome for Testing), or both?
6. **Proprietary software.** Poly never ships Google's installer or Chrome (as ADR 0132/0130); the user downloads it
   from google.com in Chrome on Poly. Confirm.

## 5. Workarounds used in this survey (not committed)

- `shell32!IsUserAnAdmin` returning 1 (workaround 1).
- `tools/xtask/src/win32.rs`: on this PC `std::fs::rename` of the new `win32-cache` set failed twice and the set
  was deleted ("cached as ..." in the log, then `kernel` build: `kernelbase.dll: The system cannot find the
  path specified`); retrying the rename and then copying fixed it. **A real bug of the cache on this host**
  (a retry or copy fallback is a few lines).
- The patches are in the kit (`patches/`); the worktree is back to `HEAD`.
