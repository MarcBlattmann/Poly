# 0171: The Windows security model: tokens, security descriptors, access checks and `CreateProcessAsUserW`

**Status:** Accepted. Replaces the "stored and returned, never enforced"
single-user model of [ADR 0083](0083-advapi32-registry-tokens-and-security-for-one-user.md)
for everything the spec sheets of `spec-win-sec` (`specs/windows/{advapi32,ntdll,kernel32}.dll/`,
149 entries) observed on Windows 11 25H2. Builds on [ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md)
(object manager), [ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (threads),
[ADR 0079](0079-our-kernelbase-and-kernel32.md) (`kernelbase`), [ADR 0114](0114-7-zip-trace-matched-privileges-processor-groups-and-last-errors.md)
(privilege adjustment) and the Chrome ladder of [ADR 0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)
(stage S2: Chrome's sandbox on).

## Context

Chrome for Windows runs every renderer, the GPU process and the network
service as children of the browser with a **restricted token** (groups made
deny-only, privileges removed, restricting SIDs, a lowered integrity level),
inside a job object, with an alternate desktop; the targets lower their own
token after start-up. That is only a sandbox if the kernel really evaluates
tokens: `AccessCheck` over the DACL for the groups, a second pass over the
restricting SIDs, the mandatory integrity label, and a refusal of what the
restricted token is not allowed to open. ADR 0083 had one token for one user
and no enforcement. Inputs (clean room, `CLEAN_ROOM.md`): the spec sheets and
visible tests named above, Learn ("Access Tokens", "How AccessCheck Works",
"Mandatory Integrity Control", "Security Descriptor String Format", the
function pages), the public headers `winnt.h`, `sddl.h`, `aclapi.h`,
`ntstatus.h`, `winerror.h`, `ntsecapi.h`. Nothing of Wine, ReactOS or
Chromium was read, nothing was run on Windows.

## Decision

### `poly-security` holds the model (host-tested)

- **SIDs, well-known SIDs, SID strings** (`sid`, `wellknown`, `sddl`): raw
  byte semantics (a SID is `8 + 4 * count` bytes, nothing validated unless
  the function validates), `CreateWellKnownSid` types 0 to 121,
  `S-R-I-S...` with hex and saturating forms, all the two-letter constants
  of `sddl.h`.
- **ACLs** (`acl`) on the caller's memory with the walk rules the sheets
  recorded: `InitializeAcl`'s size rounding, `IsValidAcl` (per ACE type:
  access layout, object layout only in revision 4), `AddAce` list
  checking, `GetAce` storing the pointer even for an ACE outside the ACL,
  `FindFirstFreeAce` with its three outcomes, `DeleteAce` validating first,
  `GetAclInformation` strict (Win32) or lenient (`RtlQueryInformationAcl`).
- **Security descriptors** (`sd`, `sdops`, `sdstr`): absolute (40 bytes, four
  pointers) and self-relative (20 bytes, SACL, DACL, owner, group with no
  padding); `Set/Get*`, `MakeSelfRelativeSD` (`RtlMakeSelfRelativeSD`
  copies a descriptor that is already self-relative), `MakeAbsoluteSD`,
  validity with a memory probe where Windows catches a fault, the
  `SECURITY_INFORMATION` selection and merge that `NtQuerySecurityObject` /
  `NtSetSecurityObject` do (labels, protection flags, owner rules) and SDDL in
  both directions (the irregular error codes of the sheets: 87, 1332, 1337,
  1336, 1004, 1804, 1705; object ACEs; `NO_ACCESS_CONTROL`; the length the
  API reports). Conditional ACEs (`XA`, `XD`, `XU`) and resource attributes are
  refused.
- **AccessCheck** (`access`): privileges first (`WRITE_OWNER` with
  `SeTakeOwnershipPrivilege`, `ACCESS_SYSTEM_SECURITY` with
  `SeSecurityPrivilege`, reported as used), the implicit owner rights
  (`READ_CONTROL | WRITE_DAC`, replaced by an `OWNER RIGHTS` ACE), a missing
  or NULL DACL grants everything, an empty one nothing, the ordered walk
  (allow for enabled non-deny-only groups, deny also for deny-only ones), the
  second walk for restricting SIDs (write access only for write-restricted
  tokens), and the mandatory label: below the object's label (Medium with
  no-write-up when it has none) the token keeps only read, execute,
  `READ_CONTROL` and `SYNCHRONIZE`, less what `NO_READ_UP` / `NO_EXECUTE_UP`
  take away. `MAXIMUM_ALLOWED` returns the union; a request is all or nothing.
- **Tokens** (`token`): user, groups and restricting SIDs with attributes,
  privileges, owner, primary group, default DACL, integrity label (a group
  with `SE_GROUP_INTEGRITY`), type and level, flags;
  `Token::poly_user()` is **the elevated administrator's token the sheets
  were observed with** (12 groups, 24 privileges all enabled, High integrity,
  owner Administrators, default DACL Administrators / SYSTEM / logon SID,
  session 0), so the ambient tests compare; `duplicate`, `filter`
  (`CreateRestrictedToken`), `adjust_privileges`, `adjust_groups`,
  `set_information`, `privilege_check`, `same_context`, and `query` for 45
  classes laid out at the caller's buffer.

`NtStatus` and its constants moved from `poly-ntobj` into `poly-security`
(`poly_ntobj::status` re-exports it) so the object manager can use the
security crate without a cycle.

### The object manager owns tokens and descriptors

`poly_ntobj::ObjectSecurity` (field `ObjectManager::security`): the tokens
(an object of type `Token` per token, cookie = token id), the primary token
of each process, the impersonation token of each thread, and the
self-relative security descriptor of each object that has one.

- A **new process** gets a copy of its creator's primary token (a new
  object, new id), or the primary token handed to `NtCreateUserProcess`
  with Poly's flag `PROCESS_CREATE_FLAGS_POLY_TOKEN` (`0x40000000`, the
  `CreateInfo` argument is the token handle; this is how
  `CreateProcessAsUserW` passes it). The first process has
  `Token::poly_user()`. Its process object and first thread get the default
  descriptor of the creator's token.
- The **effective token** of a request is the thread's impersonation token,
  else the process's. `NtOpenThreadToken`'s `OpenAsSelf`, the impersonation
  level rules (anonymous cannot be opened, identification cannot be used
  for the check) and `NtSetInformationThread(ThreadImpersonationToken)` are
  the kernel's; `ImpersonateSelf` cannot raise the level of an existing
  thread token (`RtlImpersonateSelf` checks it from user mode).
- **Opening an object that has a descriptor is checked** with the effective
  token: `open_existing` (every open and create of an existing object, and
  `open_by_pointer`) asks `access::check`; `MAXIMUM_ALLOWED` gets what the
  DACL grants, an explicit request all or nothing. Kernel-mode callers are
  not checked (Learn: `ObOpenObjectByPointer`). A new named event, mutex,
  semaphore, timer or section, and any object created with an explicit
  descriptor in `OBJECT_ATTRIBUTES`, gets one (the explicit parts completed
  with the creator's token owner, group and default DACL, generic rights in
  ACEs mapped by the type's mapping). Processes, threads and tokens get one
  when they are made, with the creator's integrity as a mandatory label
  (`S:AI(ML;;NW;;;HI)` on a token, `NRNW` on a process or thread). File and
  key objects get a fixed descriptor when the file system / registry makes
  the object (`DefaultSd`): owner Administrators, SYSTEM and Administrators
  full, Users and RESTRICTED read; files also Authenticated Users modify and
  Everyone read and execute (Windows gives a restricted token its system
  DLLs through `KnownDlls`; Poly's loader opens them, so the ACE stands in).
  Pipes, sockets and the console are not checked (Chrome's IPC pipes).
- **Services** (new, numbers `0x186`-`0x18F` in the security table):
  `NtDuplicateToken`, `NtFilterToken`, `NtAdjustGroupsToken`,
  `NtSetInformationToken`, `NtCompareTokens`, `NtPrivilegeCheck`,
  `NtAccessCheck`, `NtQuerySecurityObject`, `NtSetSecurityObject`,
  `NtSetInformationThread`. The token services of ADR 0083 and 0114 now work
  on the shared token object (every handle, the pseudo-handles and the
  process see one state); `NtSystem::token_privileges` is gone.
- `ObjectType` mappings for process, thread and token follow the default
  descriptors observed (read = `READ_CONTROL | VM_READ | QUERY_INFORMATION`,
  token read `0x2001A`, write `0x201E0`, execute `0x20005`).

### The Win32 layer

`kernelbase` (`security.rs`, `secdata.rs`) and `ntdll` (`rtlsec.rs`) are thin
over the crate: every SID/ACL/descriptor function of the 149 entries, the
`Rtl*` forms (their `NTSTATUS` differ from the Win32 errors where the sheets
say so: `STATUS_BUFFER_TOO_SMALL` for no room in `RtlAddAce`,
`STATUS_INVALID_PARAMETER` for bad flags), `RtlAdjustPrivilege`,
`RtlImpersonateSelf`; `advapi32` adds the SDDL conversions and
`Get/SetSecurityInfo`. `ADVAPI32_FORWARDS` lists the new names;
`RtlNtStatusToDosError`'s table has the security statuses.

### `CreateProcessAsUserW`

`kernelbase` builds the same `NtCreateUserProcess` call as `CreateProcessW`
with `PROCESS_CREATE_FLAGS_POLY_TOKEN` and the token handle; the token has
to be a primary token with `TOKEN_ASSIGN_PRIMARY | TOKEN_DUPLICATE |
TOKEN_QUERY`. Chrome's order (create suspended with the lockdown token,
`SetThreadToken(&thread, initial)` on the suspended thread, assign to the job,
resume) works because thread tokens are per thread and per process.

### What the sheets fixed that the Learn pages do not say

- A write-restricted token takes the restricting-SID pass even with no
  restricting SID (nothing matches, so writes fail); `READ_CONTROL` and
  `SYNCHRONIZE` are not writes for it (a write-restricted child loads its DLLs).
- `NtQuerySecurityObject` lays the descriptor out owner, group, SACL, DACL (the
  `Rtl*` self-relative functions keep SACL, DACL, owner, group); the label
  query answers control `0x8800` for an object without a SACL and the stored
  `SACL_AUTO_INHERITED` otherwise, a label set by the caller clears it; label
  ACLs are revision 2; an ACL is stored at its used size.
- A failed `GetSecurityInfo` nulls the outputs it was asked for.
- `NtAccessCheck` clears the `PRIVILEGE_SET` it is given when no privilege was
  used; `NtAdjustGroupsToken` writes only the count of an empty previous state
  and `*ReturnLength` 0 when it refuses; `NtOpenProcessTokenEx` ignores every
  attribute but `OBJ_INHERIT` (`OBJ_EXCLUSIVE` is refused); the previous state
  of a removed privilege reads 0; `TokenVirtualizationAllowed` needs
  `SeTcbPrivilege` (1314).
- A process started with a token of Medium integrity has 17 of its 24
  privileges enabled, Low and Untrusted 3
  (`Token::clamp_privileges_to_integrity`; which ones is our choice).
- The machine domain is now `S-1-5-21-520193847-884726105-1936024571` (the
  digit counts the sheets' string lengths imply) and Poly's user is RID 1002
  (the sheets' owner SID); the logon session is `S-1-5-5-0-123456789`.
  ADR 0066, 0073 and 0083 were updated to it. A `USER` hive saved before loads
  under the new name unchanged (hives hold their contents; the SID is the mount
  name), and the layout sync removes the old `ProfileList` key.
  `poly_registry::defaults::POLY_USER_SID` and `poly_security::MACHINE_DOMAIN`
  are the one place they are written.

### Accounts, trustees and security by name (the 12 follow-up entries)

- `poly_security::account`: the fixed accounts of a one-user machine
  (`LookupAccountSidW` / `LookupAccountNameW`: names, domains, `SID_NAME_USE`;
  Poly's user `poly` and group `None` in the domain `POLY`).
- `poly_security::explicit`: `SetEntriesInAclW` (old ACEs keep their order, new
  allows at the end, new denies and audits in front, `SET`/`REVOKE` drop the
  trustee's allows, the size of the old ACL is kept less what a removal freed)
  and `GetExplicitEntriesFromAclW` (inherited ACEs are skipped, an audit ACE
  gives two entries, a label ACE is an error); `advapi32` reads and writes the
  `accctrl.h` structures (`acctrl.rs`).
- `poly_security::inherit` and `ObjectSecurity::paths`: what was set on a file
  or key is kept **by path** (`SetNamedSecurityInfoW` goes through
  `NtSetSecurityObject`), and an object's descriptor is that plus the
  inheritable ACEs of its parents computed down the path, so changing a parent
  changes every child that did not protect its own DACL; a new file or key gets
  the inherited ACEs (`0x10`) at once. Inheritance flags mean nothing on a file
  and are dropped there. Nothing set on a path means the flat default.
- `GetNamedSecurityInfoW` / `SetNamedSecurityInfoW` open the file (backup
  semantics) or key (`MACHINE\SOFTWARE`, `CURRENT_USER`, `USERS`,
  `CLASSES_ROOT`) with the rights the information needs and call
  `Get`/`SetSecurityInfo`; a named kernel object is `ERROR_FILE_NOT_FOUND`.
- `Get`/`SetProcessMitigationPolicy` (`kernelbase/mitigation.rs`): the state of
  the 19 policies in the process; only the dynamic-code and win32k policies go
  to the kernel (`NtSetInformationProcess`, class 52, a Poly service `0x097`),
  `NoChildProcessCreation` stops `CreateProcessW`.
- `userenv.dll` (new): `DeriveAppContainerSidFromAppContainerName` is
  `S-1-15-2-` and the first 28 bytes of the SHA-256 of the lower-cased name in
  UTF-16LE (the sheets' vectors fix it), names of 1 to 64 units.
- `TokenAccessInformation` is the full 1972 bytes: the 88-byte header, three
  `SID_AND_ATTRIBUTES_HASH`, the privileges, the SID arrays, and 360 bytes of
  room whose purpose the sheets do not show. `RtlAdjustPrivilege(100)` keeps a
  flag of its own, as Windows does.

### Jobs

The job objects (`NtCreateJobObject`, ... `OpenJobObjectW`,
`TerminateJobObject`) are `impl-win-ipc`'s (ADR 0169). This branch adds
nothing for them except that `ObjectManager::assign_new_security` gives every
kind with a descriptor-bearing default (all but directories, links, files,
keys and devices) the creator's default descriptor when it is created named
or with an explicit one, so a merge with the job objects needs no change
there; a job's default label ACE (creator's integrity, no-write-up) is added
by the job code.

## Verification

- The 254 visible tests of the 149 entries, run on Poly: see the hand-off
  (`handoff/impl-win-sec.md`). What fails needs the job-object, named-pipe and
  `OpenProcess` exports of `impl-win-ipc`, or is `TokenAccessInformation`'s
  exact size, or `RtlAdjustPrivilege(100)`. The `CreateProcessAsUserW` tests
  were run with stand-ins for `IsProcessInJob` and `PeekNamedPipe` (and
  stubbed job calls in `cpau_low_integrity`): the output of
  `cpau_restricted_child` and `cpau_token_kinds` equals Windows' line by line,
  `cpau_low_integrity` up to the job lines.
- 7-Zip's trace still matches Windows' in all 15 steps; the self-test shards,
  `desktop-win32` and the other NT boot tests pass.
- Host tests: `a_restricted_token_cannot_open_what_the_process_can` is the
  sandbox scenario through the dispatcher (a token restricted by the SID
  S-1-0-0 is refused `\Registry\Machine\SOFTWARE`, which the process token and
  one restricted by Users open).

## To be confirmed by the spec agent

- Which privileges a Medium or Low token keeps enabled (only the counts, 17
  and 3, are in the sheets), and the default descriptors of threads and jobs.
- `RtlAdjustPrivilege` with LUID 100 succeeds on Windows (it flips a state);
  here it is `STATUS_PRIVILEGE_NOT_HELD`.

- `NtQuerySecurityObject` of a file or registry key: the default
  descriptors of the file system and the registry are our guess (SYSTEM and
  Administrators full, Users read and execute), as is the descriptor of a
  process pseudo-handle beyond the sheets.
- `TokenAccessInformation` (class 22) is a plausible 88-byte header plus the
  privileges; its exact size on Windows (1972 for the oracle token) is not
  reproduced.
- Conditional ACEs in SDDL, `SetEntriesInAcl`, `GetNamedSecurityInfo` and the
  `*ByType` access checks are not implemented.

## Consequences

- Host tests: `cargo test -p poly-security` (SIDs, SDDL, ACLs, descriptors,
  access check, tokens), `-p poly-ntobj`, `-p poly-personality-nt` (the token
  services through the dispatcher and the sandbox scenario: a restricted token
  cannot open what the unrestricted one can), `-p poly-kernelbase`.
- The 254 visible tests of the 149 entries run on Poly with
  `cargo xtask pipeline win poly` and are compared with their recorded
  Windows output; `docs/progress` shows the result per entry in the hand-off.
- Every process now has a token and every named kernel object a
  descriptor; the cost is one `Token` clone per `NtQueryInformationToken` and
  one descriptor parse per open of a secured object.
