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

(The list, counts, branches and the recordings: see the hand-back and `specs/handoffs/`. It is filled in at the end of this
document by the lead.)

## 5. Notes for B5 (Poly's front door, implementer only)

(Next sections written after the groups reported: link import, launcher entries, uninstall mapping, shell start.)
