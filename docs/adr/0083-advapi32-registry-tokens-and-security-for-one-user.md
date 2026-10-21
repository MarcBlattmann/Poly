# 0083: `advapi32` for one user: the registry API and security functions in `kernelbase`, `advapi32` forwarding through API sets, tokens as kernel objects

**Status:** Accepted. Builds on [ADR 0079](0079-our-kernelbase-and-kernel32.md)
(our `kernelbase`/`kernel32`, their workspace, build and cache, API-set
forwarding), [ADR 0071](0071-our-ntdll-a-real-pe-dll-built-from-rust.md)
(our `ntdll`, its loader and API-set table),
[ADR 0066](0066-nt-registry-hives-in-polys-own-format.md) (the registry
services and default hives, the single user `S-1-5-21-0-0-0-1000`) and
[ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md) (object
types, access masks).

## Context

The Phase 5 item is "`advapi32` basics: registry API, security
descriptors, tokens (single user)". Nearly every Windows program reads the
registry through `RegOpenKeyExW` / `RegQueryValueExW`, many ask who they
run as (`GetUserNameW`, `OpenProcessToken` + `GetTokenInformation`, "am I
elevated / an administrator") and build security descriptors to pass to
`Create*` functions even when they never check them.

Sources (clean room): Learn's reference pages of every function
implemented, including each page's metadata (its "APILocation" list:
the DLLs and API-set contracts that export it), "Predefined Keys",
"HKEY_CLASSES_ROOT Key", "Merged View of HKEY_CLASSES_ROOT", "Registry
Value Types", "SID Strings", "SID Components", "Well-known SIDs", "Access
Rights for Access-Token Objects", the WDK pages of `ZwOpenProcessTokenEx`,
`ZwOpenThreadTokenEx` and `ZwQueryInformationToken`, and the constants and
layouts of the public SDK headers (`winnt.h`, `winreg.h`, `winerror.h`,
`ntstatus.h`, `sddl.h`, `processthreadsapi.h`, `minwinbase.h`, `wdm.h`).
Export names come from the coverage map. No Windows binary, Wine or
ReactOS source was looked at; there is still no Windows oracle VM, so what
Learn leaves open is a labelled guess (below).

## Decision

### Where the functions live: `kernelbase`, reached through `advapi32`'s forwarders

Learn's API locations put the registry functions in
`api-ms-win-core-registry-l1-1-0`, `OpenProcessToken`/`OpenThreadToken` in
`api-ms-win-core-processthreads-l1-1-0` and the SID, ACL,
security-descriptor and token-information functions in
`api-ms-win-security-base-l1-1-0`, each also listing `KernelBase.dll`. So,
as ADR 0079 did for `kernel32`:

- `kernelbase.dll` implements them (`kernelbase/src/registry.rs`,
  `kernelbase/src/security.rs`).
- A new `advapi32.dll` (`personalities/nt/win32/advapi32`, in the same
  workspace, built, cached and verified by `cargo xtask win32` with the
  others) exports each as a forwarder string `<contract>.<name>`
  (`poly_kernelbase::advapi::ADVAPI32_FORWARDS`, written by its `build.rs`);
  `ntdll`'s loader maps the contracts to `kernelbase` (`poly_ldr::apiset`).
- `advapi32` implements the functions whose locations name no `kernelbase`
  contract (`ADVAPI32_OWN`): `RegDeleteKeyW/A` (`api-ms-win-core-registry-l2`),
  `ConvertSidToStringSidW/A` and `ConvertStringSidToSidW/A`
  (`api-ms-win-security-sddl-l1-1-0`, which Poly's API-set table already
  gives to `advapi32`), `GetUserNameW/A`, `LookupPrivilegeValueW` and
  `LookupPrivilegeNameW`.
- The kernel embeds `advapi32.dll` like the other modules and the layout
  writes it to `C:\Windows\System32` (it was already a known DLL).

### Tokens are kernel objects; one user means one token's contents

A new object type, `Token` (`poly-ntobj`, with `winnt.h`'s `TOKEN_*`
rights and generic mapping), and a new service table,
`SECURITY_SERVICES` (`0x180`–`0x1BF`, `personalities/nt/src/security.rs`):

| Number | Service | Arguments (Learn's `Zw*` order) |
|---|---|---|
| `0x180` | `NtOpenProcessToken` | `ProcessHandle, DesiredAccess, TokenHandle*` |
| `0x181` | `NtOpenProcessTokenEx` | `ProcessHandle, DesiredAccess, HandleAttributes, TokenHandle*` |
| `0x182` | `NtOpenThreadToken` | `ThreadHandle, DesiredAccess, OpenAsSelf, TokenHandle*` |
| `0x183` | `NtOpenThreadTokenEx` | `ThreadHandle, DesiredAccess, OpenAsSelf, HandleAttributes, TokenHandle*` |
| `0x184` | `NtQueryInformationToken` | `TokenHandle, TokenInformationClass, TokenInformation, Length, ReturnLength*` |

Each open makes a new unnamed `Token` object (cookie: the process id) held
by its handle, so `CloseHandle`, `DuplicateHandle` and access checks are
the object manager's. The process handle needs
`PROCESS_QUERY_LIMITED_INFORMATION`; querying needs `TOKEN_QUERY`
(`TOKEN_QUERY_SOURCE` for `TokenSource`). Threads never impersonate, so
`NtOpenThreadToken` is `STATUS_NO_TOKEN` (`ERROR_NO_TOKEN`, Learn's
documented case) once the thread handle checks out. The pseudo-handles of
`processthreadsapi.h` work without opening anything:
`GetCurrentProcessToken()` (-4) and `GetCurrentThreadEffectiveToken()` (-6)
are the process token, `GetCurrentThreadToken()` (-5) is `STATUS_NO_TOKEN`.

The token's contents are `poly_security::token::Token::poly_user()`, the
same for every process: user `S-1-5-21-0-0-0-1000`; groups `S-1-5-21-0-0-0-513`,
Everyone, Administrators (`SE_GROUP_OWNER`), Users, INTERACTIVE, CONSOLE
LOGON, Authenticated Users, This Organization, the logon SID
`S-1-5-5-0-256000`, LOCAL and the high integrity label; **elevated**
(`TokenIsElevated` 1) with `TokenElevationTypeDefault` (no UAC split
token: the single user owns the machine, so programs that need
administrator rights proceed); the usual administrator privileges with
`SeChangeNotify`, `SeImpersonate`, `SeCreateGlobal` enabled; owner the
user; a default DACL granting `GENERIC_ALL` to the user and SYSTEM;
session 1. `NtQueryInformationToken` lays each structure out at the
caller's buffer address (pointers into the buffer, SIDs after the fixed
part) and follows Learn's rule: a buffer smaller than the structure gets
nothing (`STATUS_BUFFER_TOO_SMALL`), `ReturnLength` gets the size either
way. Served classes: user, groups, privileges, owner, primary group,
default DACL, source, type, statistics, session id, sandbox-inert, origin,
elevation type, elevation, has-restrictions, virtualization
allowed/enabled, integrity level, UI access, mandatory policy, logon SID,
is-app-container; `TokenLinkedToken` is `STATUS_NO_SUCH_LOGON_SESSION`
(added to `poly_rtl::status`: `ERROR_NO_SUCH_LOGON_SESSION`); the rest
`STATUS_INVALID_INFO_CLASS`.

`GetUserNameW` maps the token's user to its name: the single user's
`poly` (anything else would be `ERROR_NONE_MAPPED`). `CheckTokenMembership`
with a null token checks the effective token's user and enabled groups;
a token handle passed in must be an impersonation token (Learn), which
Poly never has, so that is `ERROR_NO_IMPERSONATION_TOKEN`.

### SIDs, ACLs and security descriptors: stored and returned, never enforced

`crates/poly-security` (`no_std` + `alloc`, host-tested, shared by the
kernel and the DLLs) holds `SID` (layout, validity, equality, well-known
SIDs), SID strings (`S-R-I-S...`, authorities of 2^32 and more in hex, the
two-letter constants of "SID Strings" that need no domain controller, plus
`LA`/`LG` relative to the machine domain `S-1-5-21-0-0-0`), ACLs on the
caller's bytes (`InitializeAcl`, validation, add/insert/delete/find ACEs,
`ACL_REVISION_INFORMATION`/`ACL_SIZE_INFORMATION`), security descriptors
(control bits, the absolute x64 layout of 40 bytes with four pointers, the
self-relative layout of 20 bytes with offsets, conversion and validation)
and the token. `kernelbase` exports the `winnt.h`-level functions over it:
`AllocateAndInitializeSid` (process heap; `FreeSid`), `EqualSid`,
`EqualPrefixSid`, `GetLengthSid`, `IsValidSid`, `CopySid`, `InitializeSid`,
`GetSidLengthRequired`, `GetSid{IdentifierAuthority,SubAuthority,SubAuthorityCount}`,
`InitializeSecurityDescriptor`, `IsValidSecurityDescriptor`,
`GetSecurityDescriptorLength`, `Get/SetSecurityDescriptor{Control,Dacl,Sacl,Owner,Group}`,
`MakeSelfRelativeSD`, `InitializeAcl`, `IsValidAcl`, `GetAclInformation`,
`AddAccessAllowedAce(Ex)`, `AddAccessDeniedAce(Ex)`, `AddAce`, `GetAce`,
`DeleteAce`, `FindFirstFreeAce`. Nothing checks a descriptor against a
token: Poly has one user and no per-object security (ADR 0066); an `SD`
given to `Create*` or `RegCreateKeyExW` is ignored.

### The registry API

Over the `Nt*Key` services (ADR 0066), with the decisions host-tested in
`poly_kernelbase::registry`:

- **Predefined keys** (`winreg.h`'s sign-extended values; zero-extended
  ones accepted) are opened once per process and cached: `HKLM` →
  `\REGISTRY\MACHINE`, `HKU` → `\REGISTRY\USER`, `HKCU` →
  `\REGISTRY\USER\<the token user's SID>`, `HKCC` →
  `HKLM\SYSTEM\CurrentControlSet\Hardware Profiles\Current`,
  `HKEY_CURRENT_USER_LOCAL_SETTINGS` → `HKCU\Software\Classes\Local
  Settings`. The performance keys and `HKEY_DYN_DATA` are
  `ERROR_INVALID_HANDLE`. `RegCloseKey` of a predefined key succeeds and
  keeps it; `RegOpenKeyExW` of a predefined key with an empty subkey
  returns that key (Learn).
- **`HKEY_CLASSES_ROOT` is a view** ("Merged View of HKEY_CLASSES_ROOT"):
  opening a subkey tries `HKCU\Software\Classes` first, then
  `HKLM\Software\Classes`; creating one uses an existing key under the
  user's classes, else creates under `HKLM\Software\Classes`; the root
  itself (its values, enumerating it) is `HKLM\Software\Classes`.
- **Every function returns its Win32 error** (`RtlNtStatusToDosError` of
  the status) and leaves the last error alone. `KEY_WOW64_*` bits are
  dropped (one view). Subkey paths lose empty components.
- `RegCreateKeyExW` creates every missing key on the path (Learn), with the
  same options; the class goes to the last key; `bInheritHandle` of the
  security attributes becomes `OBJ_INHERIT`.
- Value buffers (`RegQueryValueExW`, `RegEnumValueW`, `RegGetValueW`):
  no buffer → the size and success; a short buffer → `ERROR_MORE_DATA` and
  the size; a buffer without its size → `ERROR_INVALID_PARAMETER`.
  `RegGetValueW` applies the `RRF_RT_*` restriction
  (`ERROR_UNSUPPORTED_TYPE`), checks `RRF_RT_DWORD`/`QWORD` binary sizes
  (`ERROR_DATATYPE_MISMATCH`), expands `REG_EXPAND_SZ` with the process
  environment unless `RRF_NOEXPAND` (the type is then `REG_SZ`), adds the
  missing NUL(s) Learn promises and zeroes the buffer on failure with
  `RRF_ZEROONFAILURE`.
- Names (`RegEnumKeyExW`, `RegEnumValueW`, the class of
  `RegQueryInfoKeyW`): they fit when the buffer holds them and a NUL; the
  count then is the length without it; otherwise `ERROR_MORE_DATA`.
- The `A` functions convert names and string data with the ANSI code page
  (1252, ADR 0079); `RegQueryValueExA` reports the converted size.

## To be confirmed by the spec agent against real Windows

1. The contracts `advapi32` forwards to (from Learn's API locations) and
   which functions it keeps; whether `kernel32` also exports the registry
   functions (Poly's does not yet).
2. The token's contents: groups and attributes, privileges and which are
   enabled, owner, primary group, default DACL, LUIDs, session, the
   statistics' dynamic sizes, `TokenSource`'s name (ours: `Poly`), and that
   the single user is elevated with `TokenElevationTypeDefault`.
3. `GetTokenInformation`: a too-small buffer for a fixed-size class (ours:
   `ERROR_INSUFFICIENT_BUFFER` like the others), the classes not served
   (ours: `ERROR_INVALID_PARAMETER`), `TokenImpersonationLevel` of a primary
   token, `TokenLinkedToken` without a linked token.
4. The access `OpenProcessToken` needs on the process handle and
   `OpenThreadToken` on the thread handle; whether a new handle refers to
   one shared token object (ours: a new object per open).
5. SID strings: the hex form of large authorities, case of `S` and of the
   constants, a SID with no sub-authority, `EqualSid`'s last error when the
   SIDs differ (ours: cleared).
6. ACLs: `InitializeAcl` limits (below 8: `ERROR_INSUFFICIENT_BUFFER`; above
   0xFFFF: `ERROR_INVALID_PARAMETER`; rounding down to 4), raising the ACL
   revision when a newer ACE is added, the validity rules.
7. Security descriptors: `MakeSelfRelativeSD`'s part order (ours: owner,
   group, SACL, DACL) and whether it updates `*lpdwBufferLength` on success
   (ours: no), `GetSecurityDescriptorLength` of an absolute descriptor.
8. Registry: the last error after `Reg*` calls (ours: untouched), empty
   path components, `RegGetValueW` with `RRF_RT_REG_EXPAND_SZ` and no
   `RRF_NOEXPAND` (ours: `ERROR_INVALID_PARAMETER`) and with no type flag,
   a `REG_DWORD` of the wrong size, `RegEnumKeyExW` leaving the count alone
   on `ERROR_MORE_DATA`, `RegQueryInfoKeyW`'s security descriptor size
   (ours: 0), `RegSetValueExW` with a non-zero `Reserved`, the unserved
   predefined keys, the `HKEY_CLASSES_ROOT` merge rules beyond Learn's
   summary.
9. `LookupPrivilegeNameW`'s counts.

## Consequences

- Host tests: `cargo test -p poly-security` (SIDs, strings, ACLs,
  descriptors, token layouts), `-p poly-kernelbase` (registry rules, the
  `advapi32` export table against `poly_ldr::apiset`), `-p
  poly-personality-nt security` (the token services through the dispatcher)
  and `-p poly-ntobj`.
- The ring-3 test `nt::test_advapi32` runs `a32test.exe` (imports from
  `advapi32.dll` and `kernel32.dll`) from `C:\Program Files\PolyA32Test`:
  196 checks of forwarding (`GetProcAddress` through `advapi32` lands in
  `kernelbase`), reading `HKLM`/`HKU`/`HKCU`, creating a deep key with a
  class, values of every common type, `RegGetValueW`'s restrictions,
  expansion and termination, enumeration, `RegQueryInfoKeyW`, deleting
  (refused while subkeys remain), the `HKEY_CLASSES_ROOT` view, the token
  (user, elevation, integrity, groups, membership, access, thread tokens),
  `GetUserNameW`, SIDs and SID strings, ACLs and absolute and
  self-relative descriptors, ending with `ExitProcess(0x41)`.
- Build cost: two more crates in the Win32 workspace (`advapi32.dll`
  ~21 KB, `a32test.exe` ~15 KB); `kernelbase.dll` grows to ~88 KB.
- Edits to shared files are additive: the services table (a new table
  chained into `all()`), dispatch lines, `poly-ntobj`'s type list and
  token rights, two `pub(crate)` helpers in `threads.rs`, one status in
  `poly_rtl`, `kernelbase_exports()` chaining the `advapi32` list,
  `kernelbase`'s module list, the Win32 workspace members, `win32.rs`'s
  outputs and checks, the kernel's embed list and the layout's install.
- Left for later items: `AdjustTokenPrivileges` / `NtAdjustPrivilegesToken`
  (needs per-process token state), impersonation, `DuplicateToken(Ex)`,
  `LookupAccountSidW`/`LookupAccountNameW`, `CreateWellKnownSid`,
  `ConvertStringSecurityDescriptorToSecurityDescriptorW` and the other SDDL
  descriptor strings, `SetEntriesInAcl`/`GetExplicitEntriesFromAcl`,
  `GetSecurityInfo`/`SetSecurityInfo` and `RegGetKeySecurity`, a merged
  enumeration of `HKEY_CLASSES_ROOT`, `RegNotifyChangeKeyValue`,
  `RegSetKeyValue`/`RegDeleteTree`/`RegCopyTree`, `RegLoadKey`/`RegSaveKey`,
  the remaining `A` forms (`RegEnumKeyExA`, `RegEnumValueA`,
  `RegQueryInfoKeyA`) and the legacy `RegOpenKey`/`RegCreateKey`/
  `RegQueryValue`, the performance keys, services (`OpenSCManager`),
  event logging and crypto (`CryptAcquireContext`, which belongs with the
  cryptography item).
- The coverage entries of the exports here are claimed by `advapi32` in
  `advapi32.dll`, `kernelbase.dll` and `ntdll.dll` (the five token
  services and their `Zw` aliases); they turn green when the Windows
  oracle and held-out tests exist.
