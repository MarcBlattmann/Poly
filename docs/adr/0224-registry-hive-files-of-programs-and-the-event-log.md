# 0224: Hive files of programs (RegSaveKeyW, RegLoadKeyW, RegUnLoadKeyW) and the event log

**Status:** Accepted (2026-11-19). Track `impl-chrome-installer` (B4, group R). Builds on ADR
0066 (the NT registry and its hive files) and ADR 0067 (NT file I/O over the VFS). Inputs: the
sheets of `advapi32.dll/RegSaveKeyW`, `RegLoadKeyW`, `RegUnLoadKeyW`, `RegEnumValueA`,
`RegisterEventSourceA`, `ReportEventA`, `EventLogSourceRegistration` and of
`ntdll.dll/NtOpenKeyEx`, `NtQueryValueKey`, `RtlFormatCurrentUserKeyPath` with their visible tests
(spec-win-chrome-setup); Microsoft Learn ("ZwSaveKey", "RegLoadKeyW", "RegUnLoadKeyW",
"Registry Hives", "Privilege Constants", "EVENTLOGRECORD", "ReadEventLogW", "Event Logging").
No third-party source.

Chrome's installer reaches other users' `ntuser.dat` with `RegLoadKeyW`/`RegUnLoadKeyW`, enumerates
values with `RegEnumValueA`, and logs through `RegisterEventSourceA`/`ReportEventA`. All of these
were stubs.

# Part 1: hive files of programs

## Decision

- Three new NT services (Poly's numbers 0x10D–0x10F): `NtSaveKey(KeyHandle, FileHandle)`,
  `NtLoadKey(TargetKey*, SourceFile*)` and `NtUnloadKey(TargetKey*)`, the documented
  arguments of `ZwSaveKey` and of `RegLoadKeyW`/`RegUnLoadKeyW` as native names.
  `kernelbase` implements `RegSaveKeyW`, `RegLoadKeyW`, `RegUnLoadKeyW` (and `RegEnumValueA`,
  `RegDeleteTreeW/A`) over them; `advapi32` forwards to it through
  `api-ms-win-core-registry-l1-1-0`.
- The file format: the sheets observe a file that starts with `regf` and whose size is a
  multiple of 4096 and at least 8 KiB. Microsoft's hive format is not public, so the file is
  Poly's own container (`poly_registry::hive::encode_regf`): a 4096-byte header block with
  `regf` and `POLYHIVE`, then ADR 0066's hive file, padded to whole blocks. Windows' own hive
  files do not load (`ERROR_REGISTRY_IO_FAILED`), as any file that is not a Poly hive.
- A loaded hive is a hive of `poly_registry::Registry` marked *external*: it is never written
  to the kernel's hive store. Its file stays open in the kernel's handle table, read-write
  without sharing, until the unload (loading the same file under another name is
  `ERROR_SHARING_VIOLATION`, as the sheet observes); the unload and `NtFlushKey` write the
  whole hive to it. Changes of a hive still loaded at shutdown are lost (Windows writes them
  lazily too; a later step can flush external hives with the store's).
- `<file>.LOG1`/`.LOG2` are made empty beside a loaded hive (the sheets see them); Poly writes
  hives whole and needs no log.
- Saving needs `SeBackupPrivilege` enabled, loading and unloading `SeRestorePrivilege`
  (`STATUS_PRIVILEGE_NOT_HELD`, 1314). The checks are on the caller's effective token, so the
  standard token of ADR 0222 gets 1314 where the sheets' `*_std` tests do.
- `\REGISTRY\USER\<sid>_Classes`, the user's classes hive on Windows, is a volatile symbolic
  link to `<sid>\Software\Classes`, where Poly keeps the user's classes.

# Part 2: the event log

## Decision

- No service: a log is a file of Poly's own, `%ProgramData%\Poly\EventLog\<log>.evtlog`, the
  `EVENTLOGRECORD`s one after the other, numbered from 1. A writer opens it without sharing
  (others wait), so numbers are unique across processes; a reader reads it whole. The logic
  (record layout, numbering, `ReadEventLogW`'s sequential/seek reads in either direction, the
  handle table) is `poly_kernelbase::eventlog`, host-tested.
- Every source writes to the Application log, registered or not; `OpenEventLog` of `System`
  reads the System log, of any other name the Application log (the sheets).
- `%ProgramData%` rather than `System32\winevt`: without a service the writer is the program
  itself, and a standard user may create files there.

## Consequences

- Records stay in the file; nothing trims a log yet.
- A test that expects records of an earlier run (the oracle's log keeps them) sees only its own
  on a fresh Poly disk.
