# 0196: AppContainers: lowbox tokens, two-pass access checks, and image views under prohibit-dynamic-code

**Status:** Accepted. Works toward stage S2 of the Chrome-for-Windows ladder
([0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)),
Chrome with its sandbox on, after [0192](0192-chrome-s2-window-stations-per-process-mitigations-and-known-dlls-under-a-token.md)
(the broker's window stations, mitigation policies and known DLLs). Builds on the security model
([0171](0171-windows-security-model-tokens-descriptors-and-access-checks.md)) and on DLLs being
image section views ([0188](0188-dlls-are-image-section-views-and-share-their-clean-pages.md)). The rules
are the sheets of `spec-chrome-win-s2b` (`kernelbase.dll/AppContainerRegisterSid`,
`CreateAppContainerToken`, `GetAppContainerNamedObjectPath`, `DeriveCapabilitySidsFromName`;
`userenv.dll/CreateAppContainerProfile`, `GetAppContainerFolderPath`; `ntdll.dll/LdrRegisterDllNotification`;
`mdmregistration.dll/IsDeviceRegisteredWithManagement`); nothing here was read from Chrome or from
Microsoft's binaries.

## Context

Round 1 of S2 (0192) let Chrome's broker build its window stations, desktops and restricted tokens. The
run then stopped at `LdrLoadDll(MDMRegistration.dll)`, `LdrGetProcedureAddress(ntdll!LdrRegisterDllNotification)`
and `kernelbase!AppContainerRegisterSid` (0xC000007A): the browser makes AppContainers (`cr.sb.net<40 hex>`,
display name "Chrome Sandbox") for its network service and other targets, registers their SIDs, derives
capability SIDs and makes lowbox tokens for the children. Round 1 also left a hole: the loader's own
`NtProtectVirtualMemory` calls carried `PAGE_POLY_LOADER` (0x0100_0000), a bit any caller could set to make
private memory executable in a process that prohibits dynamic code.

## Decision

### 1. SIDs and names (`poly_security::appcontainer`)

- A package SID is `S-1-15-2` and the first seven little-endian words of the SHA-256 of the lower-cased name
  (unchanged); it is recognised by its eight sub-authorities. A capability name gives a group SID
  `S-1-5-32-<8 words>` and a capability SID: `S-1-15-3-<rid>` for the twelve well-known names
  (`internetClient` 1 ... `contacts` 12, case-insensitive) and `S-1-15-3-1024-<8 words>` for any other, the words
  being the first eight of the SHA-256 of the **upper-cased** name in UTF-16LE.
- Profile names are 1 to 64 units with no `\` or `/` (`ERROR_PATH_NOT_FOUND`) and no `:` (`ERROR_DIRECTORY`); the display name is
  below 512 units and the description below 2048.

### 2. Lowbox tokens

`Token::lowbox` makes a primary token from any token: the user, groups, session and elevation type of the source,
the integrity label Low, mandatory policy 1, at most `SeChangeNotifyPrivilege` and `SeIncreaseWorkingSetPrivilege`,
the container SID, an **AppContainerNumber** and the capabilities as given. The system numbers container SIDs in the order of
their first use (`NtSystem::lowbox_number`, 1, 2, ...: the same SID always has the same number, also from a copy
and after the profile went). A copy (`DuplicateTokenEx`, a child's token) keeps all of it; a lowbox token is not made into
another one. `NtQueryInformationToken` answers `TokenIsAppContainer`, `TokenIsSandboxed`, `TokenAppContainerSid`,
`TokenAppContainerNumber` and `TokenCapabilities` for it.

The service is `NtCreateLowBoxToken(NewToken*, ExistingToken, DesiredAccess, ObjectAttributes*, PackageSid*,
CapabilityCount, Capabilities*, HandleCount, Handles*)`, number 0x190 of the security services. The sheet
leaves its arguments open; the first seven are what `CreateAppContainerToken` needs, in the order such a call
takes them; the last two are accepted and not read. The source needs `TOKEN_DUPLICATE | TOKEN_QUERY`, an
impersonation source at least level 2 (`STATUS_BAD_IMPERSONATION_LEVEL`), the package SID eight sub-authorities, the
capabilities capability SIDs, each once (`STATUS_INVALID_PARAMETER`). `kernelbase` does the argument
checks first so that the Win32 errors are the sheet's (4250 for a SID that is no package SID, 2 for a child
container's SID of 12 sub-authorities, 87 for the capability list).

`CreateProcessW` with `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES` (0x20009) needs the container's profile
(a registered mapping) and gives error 2 without it; the child's token is a lowbox token of the creator's own,
made the way `CreateAppContainerToken` does. `CreateProcessAsUserW` with a lowbox token needs no profile.

### 3. Access checks

A token with a container is checked twice (`access::check`): the ordinary pass (user and groups), and a second pass whose
principals are the container's own SID, `ALL APPLICATION PACKAGES` (`S-1-15-2-1`), `ALL RESTRICTED APPLICATION PACKAGES`
(`S-1-15-2-2`) and the token's enabled capabilities; the grant is the bits both passes give. The sheet's cases decide
the details: Everyone alone is refused, a deny entry for the container's own SID does not match (the SID is no
ordinary group), a deny entry for `ALL APPLICATION PACKAGES` does. The mandatory label of a container counts as
Medium, so it is not stopped by a Medium no-write-up label, an object without a label or a Low one, but a High no-read-up label
still refuses a read (the sheet). The token itself keeps integrity Low.

### 4. The registry mapping, the object directory and the profile folder

`AppContainerRegisterSid(sid, moniker, displayName)` writes `HKCU\Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppContainer\Mappings\<SID>`
with the values `Moniker` and `DisplayName` (once: `0x800700B7` after), so every process of the user sees it;
`AppContainerLookupMoniker` reads it back (`0x80070002`, `0x8007109A`, `0x80072023` for more than 64 characters).
`CreateAppContainerProfile` (userenv) makes `%LOCALAPPDATA%\Packages\<name>\AC` and `AC\Temp`, registers the SID
and returns it; `DeleteAppContainerProfile` removes both and is `S_OK` for a name that never was a profile;
`GetAppContainerFolderPath` finds the name through the mapping (`0x80070002` without). `GetAppContainerNamedObjectPath`
answers `AppContainerNamedObjects\<SID>` (session 0, as the sheet saw it); Poly does not give a container's `Local\`
names a directory of their own yet, the names of its objects share the session's.

### 5. `LdrRegisterDllNotification`

ntdll keeps the registrations (their address is the cookie) and `LdrLoadDll` / `LdrUnloadDll` call them under the
loader lock: reason 1 for every module a load mapped, in initialisation order, after the entry points ran;
reason 2 for every module an unload takes away, after its detach and before it is unmapped. A library only counted again,
a failing load and a data-file load never reach the loader, so they tell nobody.

### 6. `mdmregistration.dll`

A new DLL whose `IsDeviceRegisteredWithManagement` is `S_OK` with FALSE and `IsManagementRegistrationAllowed` (no
sheet) `S_OK` with FALSE: Poly is not enrolled.

### 7. Prohibit-dynamic-code and image views

The `PAGE_POLY_LOADER` bit is gone. `NtProtectVirtualMemory` of a process with the policy refuses an executable protection unless
the whole range lies in one view of an image section (`VirtualMemory::is_image_view`): Windows maps modules as image sections,
which the mitigation lets be (Learn, "Process Mitigation Policies"), and since 0188 so does Poly's loader. A DLL that
makes no image section (the loader's fallback that reads the whole file) cannot be loaded into such a process, and private memory
can never become executable in it.

## To be confirmed by the spec agent

- `NtCreateLowBoxToken`'s own arguments, the handle list, LPAC containers (`ALL RESTRICTED APPLICATION PACKAGES` as the only
  application-package principal) and child containers with a parent.
- `IsManagementRegistrationAllowed`'s answer; `IsSandboxedProcess`, which the browser resolves by name and no DLL exports.
- A container's `Local\` namespace in an interactive session (`\Sessions\n\AppContainerNamedObjects\<SID>`), and the object directory's access rules.
- A mapping that exists without a profile folder (Poly: `CreateAppContainerProfile` accepts it, as the sheet saw).

## Consequences

- Chrome's broker can make profiles, register their SIDs, derive capabilities and start children with lowbox tokens; the
  object manager checks them as the sheet says.
- A process with prohibit-dynamic-code loads its DLLs through the same image views as any process and has no way to make private memory
  executable.
- The coverage map claims the entries for `impl-chrome-s2`; nothing is `green` until the held-out tests have run.
