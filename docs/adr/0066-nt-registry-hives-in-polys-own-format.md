# 0066: The NT registry: logic in `poly-registry`, `\REGISTRY` as a parse object, hives in Poly's own file format

**Status:** Accepted. Builds on [ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md)
(`poly-ntobj`: parse objects, opaque `Key` objects, access masks),
[ADR 0063](0063-nt-processes-object-manager-and-memory.md) (the NT service
layer, Poly's own service numbers, the ring-3 test PE),
[ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (blocking waits) and
[ADR 0059](0059-bash-coreutils-python-gcc-from-the-persistent-disk.md) (the
persistent ext4 root disk).

## Context

Phase 5's registry item asks for hive storage, the `NtOpenKey` family of
services and default system hives. Almost every Windows program reads the
registry at start (its version, the environment, its own settings), and
`advapi32`'s registry API (a later Phase 5 item) is a thin layer over these
services.

Two clean-room constraints shape the design ([CLEAN_ROOM.md](../../CLEAN_ROOM.md)):

- Microsoft's on-disk hive format ("regf") is **not publicly documented**.
  What is known about it comes from reverse engineering, so Poly neither
  reads nor writes it. Applications never see hive files anyway: they use
  the documented services, and the file format is the kernel's business.
- The behaviour comes from Microsoft Learn only: the WDK pages for
  `ZwCreateKey`, `ZwOpenKey`, `ZwOpenKeyEx`, `ZwQueryKey`, `ZwEnumerateKey`,
  `ZwQueryValueKey`, `ZwEnumerateValueKey`, `ZwSetValueKey`, `ZwDeleteKey`,
  `ZwDeleteValueKey`, `ZwFlushKey`, `ZwNotifyChangeKey`, the
  `KEY_INFORMATION_CLASS` / `KEY_VALUE_INFORMATION_CLASS` /
  `KEY_SET_INFORMATION_CLASS` enumerations and their structures; the Win32
  pages for `RegCreateKeyExW`, `RegNotifyChangeKeyValue`, "Registry Value
  Types", "Registry Element Size Limits", "Predefined Keys", "Environment
  Variables"; and constants from `winnt.h`, `wdm.h`, `ntddk.h` and
  `ntstatus.h`. There is no Windows oracle VM yet, so every behaviour the
  pages leave open is a labelled guess (listed below).

## Decision

**The registry is a host-tested `no_std` crate, `crates/poly-registry`.**
It knows nothing of handles, user memory or files:

- `tree`: the `Registry` value: keys in a generational slab (`KeyId`),
  case-insensitive, case-preserving names (the object manager's case
  folding), values of any `REG_*` type, volatile keys, symbolic-link keys,
  last-write times, deletion while open. An open instance (`OpenId`) stands
  for one key object; a deleted key stays (unreachable) until its last
  instance ends. `resolve` walks a path and answers `Found`, `Missing`
  (the last component, for a create) or `Reparse` (a link on the way:
  start over at its target).
- `info`: the `KEY_*_INFORMATION` and `KEY_VALUE_*_INFORMATION` structures
  laid out as the headers declare them, and the buffer-size rule.
- `notify`: `NtNotifyChangeKey` watches.
- `hive`: Poly's hive file format (below).
- `defaults`: the default hives and the volatile keys made at every boot.

**`\REGISTRY` is a parse object in the NT namespace** (`personalities/nt/src/registry`).
At start the personality creates `\REGISTRY` as a permanent opaque `Key`
object with `parse: true`. A walk that reaches it hands the rest of the
path to the registry's `ParseHook` (`KeyParse`), which resolves it and
answers with a new unnamed `Key` object for the key (its cookie is the
open instance), creates the last component for `NtCreateKey`, or asks the
object manager to reparse at a link's target. Every key object is a parse
object too, so a key handle works as `OBJECT_ATTRIBUTES.RootDirectory`.
Handles, `NtClose`, `NtDuplicateObject` and `NtQueryObject` are the object
manager's; when it deletes a key object (its last handle closed) the
personality's `reap` ends the open instance. `\REGISTRY\MACHINE`
(`HKEY_LOCAL_MACHINE`) and `\REGISTRY\USER` (`HKEY_USERS`) are master
keys below it.

**Services, in Poly's own numbering** (ADR 0063; a table of their own,
`REGISTRY_SERVICES`, executive indices `0x100`–`0x13F`):

| Number | Service | Arguments (Learn's `Zw*` order) |
|---|---|---|
| `0x100` | `NtCreateKey` | `KeyHandle*, DesiredAccess, ObjectAttributes*, TitleIndex, Class*, CreateOptions, Disposition*` |
| `0x101` | `NtOpenKey` | `KeyHandle*, DesiredAccess, ObjectAttributes*` |
| `0x102` | `NtOpenKeyEx` | `KeyHandle*, DesiredAccess, ObjectAttributes*, OpenOptions` |
| `0x103` | `NtDeleteKey` | `KeyHandle` |
| `0x104` | `NtDeleteValueKey` | `KeyHandle, ValueName*` |
| `0x105` | `NtEnumerateKey` | `KeyHandle, Index, KeyInformationClass, KeyInformation, Length, ResultLength*` |
| `0x106` | `NtEnumerateValueKey` | `KeyHandle, Index, KeyValueInformationClass, KeyValueInformation, Length, ResultLength*` |
| `0x107` | `NtQueryKey` | `KeyHandle, KeyInformationClass, KeyInformation, Length, ResultLength*` |
| `0x108` | `NtQueryValueKey` | `KeyHandle, ValueName*, KeyValueInformationClass, KeyValueInformation, Length, ResultLength*` |
| `0x109` | `NtSetValueKey` | `KeyHandle, ValueName*, TitleIndex, Type, Data*, DataSize` |
| `0x10A` | `NtFlushKey` | `KeyHandle` |
| `0x10B` | `NtNotifyChangeKey` | `KeyHandle, Event, ApcRoutine, ApcContext, IoStatusBlock*, CompletionFilter, WatchTree, Buffer, BufferSize, Asynchronous` |
| `0x10C` | `NtSetInformationKey` | `KeyHandle, KeySetInformationClass, KeySetInformation, Length` |

Access each needs of the handle (Learn): `KEY_QUERY_VALUE` to query and
enumerate values and for `NtQueryKey` (not for `KeyNameInformation`),
`KEY_ENUMERATE_SUB_KEYS` to enumerate subkeys, `KEY_SET_VALUE` to set and
delete values, `DELETE` to delete the key, `KEY_NOTIFY` to watch it, and
`KEY_CREATE_SUB_KEY` on the `RootDirectory` handle a new key is created
directly below. **Security is single-user**: no security descriptors are
kept; every key's allowed mask (ADR 0030) is every key right, so the only
denials come from what a handle was opened with. The `KEY_WOW64_*` view
bits are dropped (Poly has one view). `REG_OPTION_BACKUP_RESTORE` changes
nothing.

**Information classes.** `NtQueryKey`: basic, node, full, name (the full
path), cached and virtualization (all bits clear); `NtEnumerateKey`: basic,
node, full (Learn: anything else is `STATUS_INVALID_PARAMETER`). Values:
basic, full, partial, the two `Align64` variants (an unaligned buffer is
`STATUS_DATATYPE_MISALIGNMENT`) and layer (all bits clear). The buffer
rule (Learn's two statuses): a buffer that holds the whole structure gets
it; one that holds the fixed part but not the names or data gets the fixed
part and `STATUS_BUFFER_OVERFLOW`; a smaller one gets nothing and
`STATUS_BUFFER_TOO_SMALL`; `ResultLength` always receives the full size.

**Notifications.** A watch belongs to an open instance; the first
`NtNotifyChangeKey` fixes its filter and `WatchTree` (Learn), later calls
queue another one-time request. `REG_NOTIFY_CHANGE_NAME` fires on a subkey
created or deleted, `LAST_SET` on a value created, changed or deleted (not
when the same type and data are written again, as Learn says),
`ATTRIBUTES` on `KeyWriteTimeInformation`; `SECURITY` never fires (no
security descriptors). Closing the instance completes its requests with
`STATUS_NOTIFY_CLEANUP` (Learn: "If the specified key is closed, the event
is signaled"); deleting the watched key with `STATUS_KEY_DELETED`. A
satisfied request signals its event and writes its `IO_STATUS_BLOCK`. That
block may be in another process, which the kernel can only write from
that process, so completions wait in a per-process queue and the
personality writes them after every service the process makes
(`registry::finish`, called from `dispatch_with`); for a change made by the
watching process itself that is the same call, and its event is signalled
only after the block is written. An asynchronous call returns
`STATUS_PENDING`; a synchronous one waits (not alertable) on an event of
its own through ADR 0065's blocking wait and returns the request's status.
An APC routine is `STATUS_NOT_IMPLEMENTED` for now: its routine could run
before another process's block is written.

**Poly's hive file format, version 1.0** (`crates/poly-registry/src/hive.rs`).
A hive file is a snapshot of one hive's non-volatile keys, written whole:

```text
header (32 bytes, little-endian)
   0  "POLYHIVE"            magic
   8  u16 major = 1, u16 minor = 0
  12  u32 header size (32)
  16  u64 sequence          +1 at every save
  24  u32 payload size
  28  u32 CRC-32 (IEEE, as zlib's crc32) of the payload
payload: the root key record
  key   = u16 name length (UTF-16 units), name, u16 class length, class,
          u64 last write (FILETIME), u32 flags (bit 0: symbolic link),
          u32 value count, values, u32 subkey count, subkeys (key records)
  value = u16 name length, name, u32 type, u32 data length, data
```

A reader refuses another major version and anything malformed
(`STATUS_REGISTRY_CORRUPT`): bad magic or checksum, truncation, unknown
flags, empty or over-long names, a `\` in a name, duplicate names (ignoring
case) among siblings or values, a link with subkeys, more than 512 levels.
A later minor version may only append data inside the payload after the
root record, which a 1.0 reader ignores. Why a snapshot and not a log or
a B-tree: the default hives are a few KiB, a save writes the whole file
and then renames it over the old one (so a crash leaves the old or the new
file, never half of one), and a reader is a page of code that is easy to
check and fuzz. If hives grow large, a version 2 can add an append-only
change log after the snapshot.

**Where hives live** (`kernel/src/nt/registry.rs`). The first writable disk
mount whose root has a `poly` directory keeps them in
`<mount>/poly/nt/registry/<NAME>.hive` (`cargo xtask root-disk` makes
`/poly/nt/registry` on the persistent root disk). With no such disk at the
first NT use, the registry lives in memory for the whole boot; the choice is
made once. Loading happens when the NT state is first made. `NtFlushKey`
writes its hive at once (`NtKernel::save_hive`: `<NAME>.hive.new`, sync,
rename); every hive with unsaved changes is written when an NT process ends
and before the machine goes down (`power::prepare_shutdown`). A corrupt
file is logged and replaced by the defaults (the next save overwrites it).

### The default hives

Made the first time a hive has no file (`crates/poly-registry/src/defaults.rs`).
The values are **Poly's own**; none was copied from a Windows installation.
"Learn" means the meaning is documented there; "name only" means Learn
names the key or value without defining it, so the value is what the name
means for Poly.

| Hive (file) | Mounted at | Learn |
|---|---|---|
| `SYSTEM` | `\REGISTRY\MACHINE\SYSTEM` | "Predefined Keys" (`HKEY_LOCAL_MACHINE`) |
| `SOFTWARE` | `\REGISTRY\MACHINE\SOFTWARE` | "Predefined Keys" |
| `DEFAULT` | `\REGISTRY\USER\.DEFAULT` | "Predefined Keys": `HKEY_CURRENT_USER` falls back to `HKEY_USERS\.Default` |
| `USER` | `\REGISTRY\USER\S-1-5-21-0-0-0-1000` | Poly's single user: a local-account SID shape ("Well-known SIDs") with Poly's own sub-authorities; `HKEY_CURRENT_USER` maps here |

| Key (below the hive) | Values | Source |
|---|---|---|
| `SYSTEM\Select` | `Current`, `Default`, `LastKnownGood` = 1, `Failed` = 0 (`REG_DWORD`) | name only: which `ControlSet00n` is current |
| `SYSTEM\ControlSet001\Control\ComputerName\ComputerName` | `ComputerName` = `POLY` | name only (`GetComputerName` returns the machine's name) |
| `SYSTEM\ControlSet001\Control\ProductOptions` | `ProductType` = `WinNT` (a workstation) | name only |
| `SYSTEM\ControlSet001\Control\Session Manager\Environment` | `ComSpec`, `Path`, `TEMP`, `TMP`, `windir` (`REG_EXPAND_SZ`, from `%SystemRoot%`), `OS` = `Windows_NT`, `PATHEXT` = `.COM;.EXE;.BAT;.CMD`, `PROCESSOR_ARCHITECTURE` = `AMD64` | Learn: "Environment Variables" (system variables live in this key); the variable names: "Recognized environment variables" |
| `SYSTEM\ControlSet001\Control\Nls\CodePage` | `ACP` = `1252`, `OEMCP` = `437` | Learn: "Code Page Identifiers" (the numbers); the key: name only |
| `SYSTEM\ControlSet001\Control\TimeZoneInformation` | `TimeZoneKeyName` = `UTC`, `Bias` = 0 | Learn: `DYNAMIC_TIME_ZONE_INFORMATION` (`TimeZoneKeyName`, `Bias`) |
| `SYSTEM\ControlSet001\Enum`, `...\Services` | (empty) | Learn: "Predefined Keys" (the Enum branch), "HKLM\SYSTEM\CurrentControlSet\Services Registry Tree" |
| `SYSTEM\ControlSet001\Hardware Profiles\Current` | (empty) | Learn: "Predefined Keys": `HKEY_CURRENT_CONFIG` is an alias for it |
| `SOFTWARE\Classes` | (empty) | Learn: "HKEY_CLASSES_ROOT Key" (merged with the user's `Software\Classes`) |
| `SOFTWARE\Microsoft\Windows NT\CurrentVersion` | `ProductName` = `Poly`; `CurrentMajorVersionNumber` = 10, `CurrentMinorVersionNumber` = 0 (`REG_DWORD`); `CurrentVersion` = `6.3`; `CurrentBuild`, `CurrentBuildNumber` = `19045`; `InstallationType` = `Client`; `SystemRoot` = `C:\Windows`; `RegisteredOwner` = `poly`; `RegisteredOrganization` = empty; `InstallDate` = the first boot, seconds since 1970 (`REG_DWORD`) | name only. The build number is the API level Poly aims at: build 19045 is Windows 10 22H2 (Learn: "Windows 10 release information"), which programs compare against to choose APIs; `RtlGetVersion` and the PEB must report the same when they exist |
| `SOFTWARE\Microsoft\Windows\CurrentVersion` | `ProgramFilesDir`, `ProgramW6432Dir` = `C:\Program Files`, `ProgramFilesDir (x86)` = `C:\Program Files (x86)`, `CommonFilesDir` = `C:\Program Files\Common Files` | name only (the folders `KNOWNFOLDERID`'s program-files ids name) |
| `SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths`, `...\Run`, `...\Uninstall` | (empty) | Learn: "App Paths", "Run and RunOnce Registry Keys", "Uninstall Registry Key" |
| `.DEFAULT\Environment`, `USER\Environment` | `TEMP`, `TMP` = `%USERPROFILE%\AppData\Local\Temp` (`REG_EXPAND_SZ`) | Learn: "Environment Variables" (user variables in `HKEY_CURRENT_USER\Environment`) |
| `.DEFAULT\Control Panel\International`, `USER\Control Panel\International` | `LocaleName` = `en-US` | Learn: "Locale Names" (the value's format); the key: name only |
| `.DEFAULT\Software\Classes`, `USER\Software\Classes` | (empty) | Learn: "HKEY_CLASSES_ROOT Key" |
| `USER\Console`, `USER\Software\Microsoft\Windows\CurrentVersion\Run` | (empty) | name only; Learn: "Run and RunOnce Registry Keys" |

Volatile keys, made at every boot and never saved:

| Key | Values | Source |
|---|---|---|
| `\REGISTRY\MACHINE\HARDWARE\DESCRIPTION\System`, `...\DEVICEMAP` | (empty until device detection fills them) | Learn: "ZwSetValueKey": the system writes the hardware description tree |
| `HKLM\SYSTEM\CurrentControlSet` | a symbolic link (`SymbolicLinkValue`, `REG_LINK`) to `\REGISTRY\MACHINE\SYSTEM\ControlSet001` | name only (`CurrentControlSet` is how Learn names the current control set throughout) |
| `HKU\S-1-5-21-0-0-0-1000\Volatile Environment` | `APPDATA`, `HOMEDRIVE`, `HOMEPATH`, `LOCALAPPDATA`, `LOGONSERVER`, `USERDOMAIN`, `USERNAME` = `poly`, `USERPROFILE` = `C:\Users\poly` | Learn: "Recognized environment variables" (each variable's meaning) |

## To be confirmed by the spec agent against real Windows

1. The master keys' names as `KeyNameInformation` reports them
   (`\REGISTRY\MACHINE`, upper case) and that `\REGISTRY` is a `Key`.
2. A key name component's limit (255 characters, per component) and the
   status for a longer one (`STATUS_OBJECT_NAME_INVALID`); a value name over
   16,383 characters (`STATUS_INVALID_PARAMETER`); more than 512 levels
   (`STATUS_INVALID_PARAMETER`); Poly's own 16 MiB value-data limit
   (`STATUS_INSUFFICIENT_RESOURCES`).
3. A missing key on the way of `NtCreateKey` / `NtOpenKey`:
   `STATUS_OBJECT_NAME_NOT_FOUND`; an empty component or trailing `\`:
   `STATUS_OBJECT_NAME_INVALID`.
4. Enumeration order: subkeys sorted by upcased name, values in the order
   first set; replacing a value keeps its place and the case of its first
   name.
5. Last-write times: a subkey created or deleted updates the parent's; a
   same-data write updates nothing; `KeyWriteTimeInformation` notifies
   `REG_NOTIFY_CHANGE_ATTRIBUTES`.
6. `STATUS_CHILD_MUST_BE_VOLATILE` for a non-volatile key below a master
   key; `STATUS_CANNOT_DELETE` for a master key or a hive root;
   `STATUS_ACCESS_DENIED` for a subkey of a link opened as itself.
7. Links: `REG_OPTION_CREATE_LINK` over an existing key is
   `STATUS_OBJECT_NAME_COLLISION`; a link without a usable target (missing,
   not `REG_LINK`, not absolute) is `STATUS_OBJECT_TYPE_MISMATCH`; a path
   below a link opened with `REG_OPTION_OPEN_LINK` goes below its target; a
   trailing NUL in `SymbolicLinkValue` is accepted.
8. The buffer rule's partial data (the fixed part only); `ClassOffset` when
   the class is empty; `KEY_VALUE_FULL_INFORMATION.DataOffset` right after
   the name (8-aligned in the `Align64` variant); `sizeof(KEY_CACHED_INFORMATION)`
   = 40; the reserved key classes (flags, handle tags, trust, layer) are
   `STATUS_INVALID_PARAMETER`; `NtSetInformationKey`'s statuses
   (`STATUS_INVALID_INFO_CLASS`, `STATUS_INFO_LENGTH_MISMATCH`) and its
   access (`KEY_SET_VALUE`); `NtFlushKey` needs no access.
9. Unknown `CreateOptions` bits: `STATUS_INVALID_PARAMETER`;
   `NtCreateKey` with a name outside `\REGISTRY`:
   `STATUS_OBJECT_TYPE_MISMATCH`; `NtOpenEvent` (or another non-key open)
   through `\REGISTRY`: `STATUS_OBJECT_PATH_NOT_FOUND` (the object services
   have no parse procedure).
10. Notifications: the status of a satisfied request (`STATUS_SUCCESS`,
    `Information` 0), of a deleted watched key (`STATUS_KEY_DELETED`), the
    event reset when a request starts, changes while no request is queued
    being forgotten, a filter with no change bit or unknown bits being
    `STATUS_INVALID_PARAMETER`, and the key handle becoming signalled when
    there is no event (not done: key objects are not waitable).

## Consequences

- The registry, its information classes, notifications, hive format and
  default hives are host-tested in `poly-registry` (`cargo test -p
  poly-registry`), the services against the fake kernel in
  `personalities/nt` (`tests/registry.rs`), and the kernel side by two
  `kernel_test!`s: a self-assembled PE that creates a key under
  `\Registry\Machine\SOFTWARE`, sets four values of four types, enumerates
  and queries them (with a buffer-overflow status), reads `ProductName` and
  a value through the `CurrentControlSet` link, waits for a notification,
  and deletes its key; and a hive file round trip through the VFS.
- Edits to shared NT files are additions: a `registry` field and a `Key`
  arm in `NtSystem::reap`, two default methods on `NtKernel` (`system_time`,
  `save_hive`), one call in `dispatch_with`, a separate service table, and
  in the kernel the NT state's constructor, `process_exit` and
  `power::prepare_shutdown`.
- Every system call now ends with a short look at the caller's pending
  notification completions (one extra lock of the NT state).
- `advapi32` (a later item) maps `HKEY_*` to these paths and Win32 errors;
  per-user `HKEY_CLASSES_ROOT` merging, registry virtualization, WOW64
  views, security descriptors, `NtLoadKey`/`NtSaveKey`/`NtRenameKey` and
  `NtQueryMultipleValueKey` are left for when a program needs them.
- The coverage map is unchanged: these services cannot go green without the
  Windows oracle.
