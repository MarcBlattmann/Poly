# Chrome's own `setup.exe` on Poly: what it calls, what Poly lacks, the sheets (batch B4) and notes for B5

Track `spec-win-chrome-setup` (spec agent, 2026-11-17). Batch B4 of `docs/windows/chrome-installer-plan.md`.
Owner decisions (2026-11-17): Google's installer runs unchanged; auto-update on; UAC with a separate prompt;
the installed Chrome replaces Chrome for Testing in the launcher. B0 (elevation, locale, system calls), B1 (services),
B2a (COM across processes) are specified by other tracks; this batch does not repeat their entries.

Inputs: the survey's traces and logs (`survey-chrome-installer`), the PE import **names** of `setup.exe`
157.0.8103.79 (and of `elevation_service.exe`, `notification_helper.exe`, `chrome.exe`), Chrome's own verbose
installer log of a `--system-level` install and uninstall, and our own test programs on the Windows oracle
(homeserver2). Nothing of Google's was decompiled or disassembled, and Google's installer was **not run again**
on the oracle. The oracle was only used for our own small test programs.

## 1. What `setup.exe --system-level` does (from its log)

Install (about 1.5 s of its own work, after the updater has unpacked it):

1. Writes its files under `C:\Program Files\Google\Chrome\Application\<ver>\` (+ `chrome.exe`, `chrome_proxy.exe`,
   `initial_preferences`, `PlatformExperienceHelper`), the registry (section 3), registers
   `GoogleChromeElevationService` (SCM, B1), the event-log provider and the firewall rule.
2. Starts a **child of itself, with the logged-on user's token**: `setup.exe --create-shortcuts=0 ...` ("Launching ... to
   create shortcuts"). That child makes the three links (section 2). The token comes from the shell process
   (`GetShellWindow`, `GetWindowThreadProcessId`, `OpenProcessToken`, `DuplicateTokenEx`, `CreateEnvironmentBlock`,
   `CreateProcessWithTokenW`; all imported). So a per-user link lands in the **user's** profile although the installer is elevated.
3. Writes Active Setup (section 3), asks the shell to refresh (`SHChangeNotify`), and exits; the updater then asks
   the user's shell to start `chrome.exe --from-installer` (Explorer is the parent on the oracle).

Uninstall (`setup.exe --uninstall --channel=stable --system-level --verbose-logging --force-uninstall`, exit 19, 6 s):

1. Deletes the shortcuts (`SHGetKnownFolderPath`/`SHGetFolderPathW` for the common and per-user folders), asks the
   OS-update handler to uninstall (`os_update_handler.exe --uninstall --system-level`).
2. Deletes its registry keys (every key listed in section 3, plus the COM classes, interfaces, type libraries and
   AppIDs the elevation and tracing services registered, `Applications\chrome.exe`, `OpenWithList\chrome.exe` of
   every extension).
3. **Walks the user profiles**: for each local account it uses the loaded hive or, when none is loaded, **loads
   `ntuser.dat` with `RegLoadKeyW` (after enabling SeRestorePrivilege/SeBackupPrivilege)** to delete the per-user Active
   Setup and Chrome keys, then `RegUnLoadKeyW`.
4. Deletes the files one by one, deletes the event-log source key, **moves its own running `setup.exe` to
   `C:\Windows\SystemTemp\<guid>.tmp`** (and deletes the rest), stops and deletes `GoogleChromeElevationService`, and
   finally starts the updater again with `--wake --system` so that it notices no app is left and removes itself.
5. Leaves the user's profile data, `HKCU\Software\Google`, `HKLM\SOFTWARE\Google\Update` state and the
   `ChromePDF` ProgID behind on Windows too (nothing to reproduce).

## 2. What Poly lacks for it (the stop list)

`cargo xtask win-app imports` of `setup.exe` (444 imports): **443 bind**; the one that does not is
`shlwapi!UrlCanonicalizeW`. But 56 more bind to code that the coverage map marks `stub` (a fail stub that returns
`FALSE`/`E_NOTIMPL`): the 64-bit `setup.exe` would therefore run into them one by one. The ones this batch
specifies (B1's SCM calls, B0's `IsUserAnAdmin`/`ShellExecute*`/locale calls and B2a's `CoRegisterInitializeSpy`
are other tracks'; the table says which sheet covers each):

| Used for | Function(s) | Poly today | Sheet (entry id below `windows/api/`) |
|---|---|---|---|
| **.lnk files** | `CoCreateInstance(CLSID_ShellLink)`, `IShellLinkW`, `IPersistFile`, `IPropertyStore` (AUMID, toast activator), `propsys!InitPropVariantFromCLSID` | **no ShellLink class at all** (`REGDB_E_CLASSNOTREG`); propsys fn stub | group A: `shell32.dll/ShellLink`, `IShellLinkW`, `IPersistFile`, `ShellLink.PropertyStore`, `propsys.dll/InitPropVariantFromCLSID` |
| known folders | `SHGetKnownFolderPath`, `SHGetFolderPathW` for Public Desktop, Common Programs/Start Menu, Quick Launch | green / claimed for the user folders; the common ones unspecified | group B: `shell32.dll/KnownFolders.Common` |
| shell refresh | `SHChangeNotify` | stub | `shell32.dll/SHChangeNotify` |
| default-browser registry as read by the OS | `SHCreateAssociationRegistration` (+ the registry patterns: ProgIDs, `StartMenuInternet`, `RegisteredApplications`, `App Paths`) | stub | `shell32.dll/SHCreateAssociationRegistration`, `shlwapi.dll/AssocQueryStringW` |
| starting Chrome through the shell | `IShellWindows` → `IShellDispatch2::ShellExecute` (Explorer is the parent) | none | `shell32.dll/ShellDispatch.ShellExecute` |
| event log | `RegisterEventSourceA`, `ReportEventA`, `DeregisterEventSource`; the `EventLog\Application\Chrome` key | A variants stub | group C: `advapi32.dll/RegisterEventSourceA`, `ReportEventA`, `EventLogSourceRegistration` |
| other users' hives (uninstall) | `RegLoadKeyW`, `RegUnLoadKeyW`, `RegSaveKeyW`, `RegEnumValueA`, `ntdll!NtOpenKeyEx/NtQueryValueKey/NtDeleteKey/RtlFormatCurrentUserKeyPath` | stub / unspecified | group C |
| file version reads | `version!GetFileVersionInfoSizeW/GetFileVersionInfoW/VerQueryValueW` (Poly has code, no sheet) | implemented, unspecified | group C: `version.dll/*` |
| firewall | `CoCreateInstance(CLSID_NetFwPolicy2)` + `INetFwRule` | none | group C: `hnetcfg.dll/NetFwPolicy2` (best effort) |
| copying/deleting files, snapshots of processes | `CopyFileW`, `GetShortPathNameW`, `GetLongPathNameW`, `CreateToolhelp32Snapshot`, `Process32FirstW/NextW`, `ReadDirectoryChangesW`, `MoveFileExW` (delayed delete) | stubs | group D |
| starting a process as the shell's user | `CreateProcessWithTokenW`, `CreateEnvironmentBlock`/`DestroyEnvironmentBlock` | stubs | group E |
| strings and URLs | `CommandLineToArgvW`, `UrlCanonicalizeW`, `shlwapi!IsOS` (#437) | implemented unspecified / **missing** | group E |
| network | `WinHttpSetOption`, `WinHttpSetStatusCallback` | stubs | group E |

Imported but **not on the `--system-level` install/uninstall path** (not specified; B7 will show whether any of them
is ever called): `ncrypt!NCrypt*` (12) and `tbs!Tbsi_GetDeviceInfo/Tbsip_Submit_Command` (key attestation for
Chrome's device-bound features), `urlmon!CreateURLMonikerEx`, `winhttp` (the online stub's download code),
`dbghelp!Sym*` (crash reports), the ETW calls, `advapi32` security-descriptor builders (B0/ADR 0171),
`GetThreadContext`, `HeapWalk`.

## 3. The registry contract (what the setup writes, from the survey's diffs)

All values as observed on Windows 11 build 26200 for 157.0.8103.79. Every key is also what uninstall deletes.
Where it lands: HKLM, **native view** unless noted.

| Key (below `HKLM\SOFTWARE\` unless noted) | Values |
|---|---|
| `Classes\ChromeHTML`, `Classes\ChromePDF` | default `Chrome HTML Document` / `Chrome PDF Document`; `AppUserModelId`=`Chrome`; `Application` (`AppUserModelId`, `ApplicationIcon` `chrome.exe,0`, `ApplicationName` `Google Chrome`, `ApplicationDescription` `Access the Internet`, `ApplicationCompany` `Google LLC`); `DefaultIcon` `chrome.exe,10` / `,11`; `shell\open\command` default `"C:\Program Files\Google\Chrome\Application\chrome.exe" --single-argument %1` |
| `Classes\.htm .html .mhtml .shtml .svg .xht .xhtml .webp .pdf` | only `OpenWithProgids\ChromeHTML` (or `ChromePDF`) = empty string: **no default value** is taken over |
| `Clients\StartMenuInternet\Google Chrome` (native **and** `WOW6432Node`) | default `Google Chrome`; `Capabilities` (`ApplicationName`, `ApplicationIcon`, `ApplicationDescription`, `FileAssociations` `.htm`…=`ChromeHTML`, `URLAssociations` http, https, irc, mailto, mms, news, nntp, sms, smsto, snews, tel, urn, webcal, google-chrome = `ChromeHTML`, `Startmenu\StartMenuInternet`); `DefaultIcon` `chrome.exe,0`; `InstallInfo` (`ReinstallCommand` `"…chrome.exe" --make-default-browser`, `HideIconsCommand`, `ShowIconsCommand`, `IconsVisible`=1); `shell\open\command` `"…chrome.exe"` |
| `RegisteredApplications` (native and WOW6432Node) | `Google Chrome` = `Software\Clients\StartMenuInternet\Google Chrome\Capabilities` |
| `Microsoft\Windows\CurrentVersion\App Paths\chrome.exe` (native and WOW6432Node) | default = full path of `chrome.exe`; `Path` = its folder |
| `WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\Google Chrome` | **only in the 32-bit view** (no native-view key): `DisplayName` `Google Chrome`, `DisplayVersion`/`Version` `157.0.8103.79`, `Publisher` `Google LLC`, `UninstallString` `"…\157.0.8103.79\Installer\setup.exe" --uninstall --system-level`, `InstallLocation` (`C:\Program Files\Google\Chrome\Application`), `DisplayIcon` `chrome.exe,0`, `EstimatedSize` (KB, dword), `InstallDate` `YYYYMMDD`, `VersionMajor` dword, `VersionMinor` dword, `NoModify`=1, `NoRepair`=1; no `QuietUninstallString`, no `SystemComponent` |
| `Microsoft\Active Setup\Installed Components\{8A69D345-D564-463c-AFF1-A69D9E530F96}` | default and `Localized Name` `Google Chrome`; `IsInstalled`=1; `Version` `43,0,0,0` (REG_SZ); `StubPath` `"…\Installer\chrmstp.exe" --configure-user-settings --verbose-logging --system-level` |
| `Microsoft\Active Setup\Installed Components\{49210152-871f-4ffa-961d-a172abcbc09d}` | `Google Platform Experience Helper`; `IsInstalled`=1; `Version` `153,0,8001,0`; `StubPath` `"…\PlatformExperienceHelper\platform_experience_helper.exe" --first-run` |
| `HKLM\SYSTEM\CurrentControlSet\Services\EventLog\Application\Chrome` | `EventMessageFile`, `CategoryMessageFile`, `ParameterMessageFile` = `…\<ver>\eventlog_provider.dll`; `CategoryCount`=1; `TypesSupported`=7 |
| `HKLM\SYSTEM\CurrentControlSet\Services\GoogleChromeElevationService` | B1's table (`Type` 0x10, `Start` 3, `ImagePath` REG_EXPAND_SZ, `DependOnService` `RPCSS`, `DisplayName`, `Description`) |
| firewall | one inbound rule, display name `Google Chrome (mDNS-In)` (UDP 5353, `chrome.exe`; `netsh advfirewall firewall show rule name=all`) |

**Active Setup** is run by Windows at a user's logon: for every component whose `Version` in HKLM is newer than the
user's copy under `HKCU\Software\Microsoft\Active Setup\Installed Components\<GUID>` it runs `StubPath` once (in
the user's session and token, before the shell starts), then copies the `Version` to HKCU. It is plain registry plus a
launcher, and it has no API a test program can observe without a new logon; **no sheet**, the contract is this table
and B5's note below. (What the stub does, `chrmstp.exe --configure-user-settings`, is Chrome's own program.)

## 4. Sheets of this batch

All entries are claimed for `impl-chrome-installer` (except `shlwapi!IsOS`, claimed earlier by `impl-chrome-s9`),
recorded on the oracle (every test ran twice with identical output, per-test token in `windows.json`), and handed off
(one record each under `specs/handoffs/`). Counts are visible / held out; ids are below `windows/api/`.

| Group | Entries |
|---|---|
| Shell links (5) | `shell32.dll/ShellLink` 2/1, `IShellLinkW` 3/2, `ShellLink.IPersistFile` 4/2, `ShellLink.PropertyStore` 1/1, `propsys.dll/InitPropVariantFromCLSID` 1/1 |
| Folders, shell, associations (5) | `shell32.dll/KnownFolders.Common` 2/1, `SHChangeNotify` 1/1, `SHCreateAssociationRegistration` 1/1, `ShellDispatch.ShellExecute` 2/1, `shlwapi.dll/AssocQueryStringW` 2/1 |
| Event log, hives, version, natives, firewall (15) | `advapi32.dll/RegisterEventSourceA` 1/1, `ReportEventA` 1/1, `EventLogSourceRegistration` 1/1, `RegLoadKeyW` 2/1, `RegUnLoadKeyW` 2/1, `RegSaveKeyW` 2/1, `RegEnumValueA` 1/1, `version.dll/GetFileVersionInfoSizeW`, `GetFileVersionInfoW`, `VerQueryValueW` 1/1 each, `ntdll.dll/NtOpenKeyEx`, `NtQueryValueKey`, `NtDeleteKey`, `RtlFormatCurrentUserKeyPath` 1/1 each, `hnetcfg.dll/NetFwPolicy2` (policy, rules, rule objects; adding a rule needs the full token, the filtered token gets `E_ACCESSDENIED`, which Chrome's setup tolerates; a table that enforces nothing is enough) 3/1 |
| Files and processes (11) | `kernel32.dll/CopyFileW` 2/1, `MoveFileExW.DelayUntilReboot` 2/1, `CreateToolhelp32Snapshot`, `Process32FirstW`, `Process32NextW`, `GetShortPathNameW`, `GetLongPathNameW`, `ReadDirectoryChangesW`, `GetProcessHandleCount`, `K32GetProcessMemoryInfo`, `GetThreadLocale` 1/1 each |
| Tokens and strings (8) | `advapi32.dll/CreateProcessWithTokenW` 3/1, `userenv.dll/CreateEnvironmentBlock` 2/1, `DestroyEnvironmentBlock` 2/1, `shell32.dll/CommandLineToArgvW` 2/1, `shlwapi.dll/UrlCanonicalizeW` 2/1, `IsOS` 1/1, `winhttp.dll/WinHttpSetOption`, `WinHttpSetStatusCallback` 1/1 each |

Total: 44 entries, 66 visible and 46 held-out tests. Not specified (and why): Active Setup (no API); `K32GetModuleInformation`, thread priority boost, `QueryThreadCycleTime`, `HeapLock/Unlock/Walk`,
`GetThreadContext` (diagnostic code, not on the install path); `ncrypt`/`tbs`/`CreateURLMonikerEx` (not on the
`--system-level` path: the log shows no key-attestation or TPM step).

## 5. Notes for B5 (Poly's front door, implementer only)

**Importing the `.lnk` files into the launcher** (reading what `IPersistFile::Save` wrote; the exact rules are the
`reading_a_link` rule of `ShellLink.IPersistFile.yaml`):

- **Scan, don't copy.** After the installer ends, and on every `rescan`, walk `C:\ProgramData\Microsoft\Windows\Start Menu\Programs\**`,
  `C:\Users\Public\Desktop`, the user's Start Menu and Desktop, and build the launcher entries from the links that exist
  *now*. Then an uninstalled app vanishes with its links; nothing to delete by hand. Skip Quick Launch (it only repeats
  the Start menu link) and links whose file name starts with `Uninstall`/`Remove`; one entry per (target, arguments).
  Desktop file name `poly-win-<hash of target+args>.desktop`, `X-Poly-WindowsLnk=<the .lnk's path>`.
- **Name** = the file name without `.lnk`. **Comment** = the link's description (StringData NAME_STRING).
- **Target**, in this order: LinkInfo `LocalBasePath` (+ `CommonPathSuffix`) when the link has LinkInfo (only if the target
  existed on a local disk when saved); else the ID list (drive letter from the 0x2F item, then each file item's name);
  else the EnvironmentVariableDataBlock (0xA0000001), expanding `%ProgramFiles%` etc.; the relative path alone is not
  enough (a Chrome-like link saved *before* `chrome.exe` exists has no LinkInfo: ID list only). Re-check that the target
  exists before showing the entry (a link whose target is gone is dead, e.g. mid-update).
- **Arguments** (`--source-shortcut-location=desktop` etc., kept verbatim), **working directory**, **show command** (only 1, 3
  and 7 are stored) and **icon**: IconLocation string plus the index in the header; if absent use the target's first icon
  group (ADR 0216 does it for Chrome for Testing: the largest PNG of the icon group). Strings are UTF-16 and may contain
  unexpanded `%variables%`.
- **AppUserModelID** and **ToastActivatorCLSID** come from the PropertyStoreDataBlock (0xA0000009), format id
  `{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}` pid 5 (string) and pid 26 (CLSID). Keep the AUMID with the entry: it groups
  windows and notifications of the app (`ChromeHTML` also names it: `Application\AppUserModelId`=`Chrome`).
- **Launching** an imported entry: start the target with the arguments from the user's session and the user's
  (unelevated) token, not as the installer's elevated process (see the shell-start note).

**Replacing Chrome for Testing in the launcher.** The installed Chrome's link set above gives the entry `Google Chrome`
with target `C:\Program Files\Google\Chrome\Application\chrome.exe`. Once it exists the launcher hides the Chrome for
Testing entry (`desktop --chrome-win` disk without it); the profile moves from `C:\poly-chrome-profile` to Chrome's own
`%LOCALAPPDATA%\Google\Chrome\User Data`.

**Starting the installed Chrome "through the shell".** Windows' updater asks Explorer (Shell.Application,
`ShellExecute`), so the child has the user's *filtered* token, medium integrity, the interactive session and Explorer as
parent, whatever the caller's token (`ShellDispatch.ShellExecute` sheet). `ShellExecuteExW` or an in-process
Shell.Application keep the caller's token. Poly's session manager should therefore start installed apps (and run
`CreateProcessWithTokenW(shell token)`) with the desktop user's own token. `CreateProcessWithTokenW` needs
SeImpersonatePrivilege (1314 without) and does **not** inherit environment, current directory or handles
(`CreateProcessWithTokenW`, `CreateEnvironmentBlock` sheets).

**Uninstall entry → Settings and Files' uninstall (ADR 0180).**

- Enumerate `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\*` in **both** registry views and `HKCU` (Chrome's key is
  only in the 32-bit view). Show an entry when it has `DisplayName` and an `UninstallString` and no `SystemComponent`=1
  and no `ParentKeyName`/`ReleaseType` (updates). Fields: name, `DisplayVersion`, `Publisher`, `DisplayIcon` (path,index),
  `InstallLocation`, `EstimatedSize` (KB), `InstallDate` (YYYYMMDD).
- ADR 0180's window is the confirmation. Then run `QuietUninstallString` if present, else `UninstallString`; for Google Chrome Windows' Apps
  settings appends `--force-uninstall` (the survey's uninstall: exit 19, 6 s; without it Chrome shows its own dialog).
  Run it elevated like the install, wait for the process and its children, then `rescan`; the `.lnk` scan makes the
  launcher entry disappear. Chrome's updater removes its two services and task about 20 s after the last app goes (B1/B3).
- `NoRemove`=1 hides the button; `WindowsInstaller`=1 means an MSI (`msiexec /x`, B6, not Google's path).
- On uninstall `setup.exe` needs: `RegLoadKeyW`/`RegUnLoadKeyW` of other users' `ntuser.dat` (sheets in this batch),
  `MoveFileExW` to move its own running image and a delayed delete, `ReadDirectoryChangesW`, `CopyFileW`, Toolhelp.
- **Active Setup**: after a user logs in (and once after the install for the installing user) Poly's session start runs,
  for every `Active Setup\Installed Components\<GUID>` with `IsInstalled`=1 whose HKLM `Version` is newer than the user's
  HKCU copy, the `StubPath` command line in the user's session, then copies `Version` to HKCU. Chrome's stubs
  (`chrmstp.exe --configure-user-settings`, `platform_experience_helper.exe --first-run`) are Chrome's own programs.
- **Default browser**: `SHCreateAssociationRegistration` sheets show Windows' API reads `Capabilities`/`RegisteredApplications`
  only for queries; `SetAppAsDefault` fails (`E_FAIL`) and `QueryAppIsDefault` answers TRUE, so Poly's Settings must keep
  its own default-browser state and read the `ChromeHTML`/`URLAssociations` registrations to offer "Google Chrome".

## 6. What B4 gives B5 (impl-chrome-installer, ADR 0227)

- **Reading links:** the host-tested crate `poly-lnk` is the `.lnk` format of both sides (shell32's
  `IPersistFile::Save` writes with it). `poly_lnk::parse(&bytes)` gives a `Link` with `target()` (LinkInfo base
  path, else the ID list, else the environment block, which may hold `%VAR%`), `arguments()`, `working_dir()`,
  `description`, `icon()` (path and index; the IconEnvironmentDataBlock wins), `show_command()` (1, 3 or 7),
  `app_user_model_id()` and `toast_activator_clsid()`; strings are UTF-16 without a NUL, and
  `poly_lnk::text::expand_env` expands `%VAR%`. An empty file is an empty link; a bad header is `Error::NotALink`.
- **Where the links are:** the known folders of `KnownFolders.Common` exist as real directories in Poly's layout;
  Chrome's links land in `C:\ProgramData\Microsoft\Windows\Start Menu\Programs` and `C:\Users\Public\Desktop`.
- **Uninstall entries** are plain registry keys (nothing new in B4); the hive functions Chrome's uninstall needs
  (`RegLoadKeyW`, `RegUnLoadKeyW`, `RegSaveKeyW`) are Poly's own hive container (ADR 0227, part 4).
- **Not done in B4:** Poly does not carry out `PendingFileRenameOperations` at the next boot (the request is
  recorded); `ShellWindows`' `ShellExecute` starts the program as the caller's child, not with the desktop user's
  token; Active Setup has no runner yet.
