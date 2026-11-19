# 0227: Chrome's own setup on Poly: shell links, hive files of programs, the event log and the system's lists

**Status:** Accepted (2026-11-19). Track `impl-chrome-installer` (batch B4 of
`docs/windows/chrome-installer-plan.md`). Builds on ADR 0066 (the NT registry and its hive
files), ADR 0067 (NT file I/O over the VFS) and ADR 0223 (Poly's own COM classes). Inputs: the
44 sheets of spec-win-chrome-setup (`docs/windows/chrome-setup-plan.md`, section 4) with their
visible tests; Microsoft Learn ("IShellLinkW", "IPersistFile", "IPropertyStore", "ZwSaveKey",
"RegLoadKeyW", "RegUnLoadKeyW", "Registry Hives", "Privilege Constants", "EVENTLOGRECORD",
"ReadEventLogW", "Event Logging", "MoveFileExW", "CreateToolhelp32Snapshot",
"ReadDirectoryChangesW", "INetFwPolicy2"); the open specifications MS-SHLLINK and MS-PROPSTORE.
No third-party source.

Chrome's `setup.exe --system-level` writes its shortcuts with `CLSID_ShellLink`, reaches other
users' `ntuser.dat` with `RegLoadKeyW`/`RegUnLoadKeyW`, logs through `RegisterEventSourceA` /
`ReportEventA`, adds a firewall rule, and on uninstall walks the process list and deletes its own
running image at the next boot. All of these were stubs. Most of the batch is plain sheet work;
the decisions are below.

# Part 1: shell links (`.lnk`)

## Decision

- **One crate for both sides.** The MS-SHLLINK format (writing what `IPersistFile::Save` writes,
  reading per the sheet's `load_*` and `reading_a_link` rules, the ID list, LinkInfo, StringData,
  the extra blocks and the MS-PROPSTORE property store block `0xA0000009`) is the host-tested,
  `no_std` crate `poly-lnk`. `shell32` keeps only the COM object (`shell32/src/link.rs`:
  `IShellLinkW/A`, `IPersistFile`, `IPersistStream`, `IPropertyStore` and the `QueryInterface`
  list of the sheet), served through `poly_com::inproc` (ADR 0223). Poly's launcher (batch B5)
  imports links with the same crate: `poly_lnk::parse(bytes)` gives target, arguments, working
  directory, description, icon, show command, AppUserModelID and toast activator CLSID.
- **Where links live.** A link is an ordinary file with the exact name the caller saves it
  under. Chrome's setup writes `C:\Users\Public\Desktop\Google Chrome.lnk` and
  `C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Google Chrome.lnk` (the known folders of
  `KnownFolders.Common`, which exist as real directories in Poly's layout). The launcher reads
  the common Start Menu `Programs` folder (and the user's own) rather than keeping a copy: an
  uninstalled app disappears with its link.
- **Poly's own file-entry items.** The shell's ID list item format is not public; Poly writes
  items made to the sheet (drive item `0x2F`, file-entry items `0x31`/`0x32` with the ANSI name at
  offset 14 and a `0xBEEF0004` block with the Unicode name), and the reader takes the Unicode name
  when there is one. `System.VolumeId` is a GUID made from the volume serial (Poly's volumes have
  no GUID names); the tracker block's object id is a hash of the path; a UNC target's `0xA000000C`
  block holds Poly's own one-item list.
- `CLSCTX_LOCAL_SERVER` alone also gives a shell link (the sheet); Poly serves it from shell32
  in the caller's process.

# Part 2: the desktop's classes and the firewall

- `Shell.Application` is an in-process class of shell32. `ShellWindows`, a local server of
  Explorer on Windows, is served by shell32 in the caller's process when a local server is asked
  for (`poly_com::inproc::desktop_server_of`): Poly's desktop is no Windows process. "Start from
  the desktop" (`IShellDispatch2::ShellExecute`) therefore starts the program as the caller's
  child; starting it with the desktop user's token and the desktop as parent is B5's.
- `hnetcfg.dll` serves `NetFwPolicy2` and `NetFwRule` over a rule table in the process that
  enforces nothing (the sheet allows it): two built-in "no host firewall" rules, the firewall
  reported off, the private profile current. `IDispatch::Invoke` is `DISP_E_MEMBERNOTFOUND`.

# Part 3: the system's lists (processes, short names, pending renames, directory changes)

- **Processes.** A private service `PolyProcessList` (in the style of ADR 0206) lists the live NT
  processes (image name, parent, threads, base priority); `CreateToolhelp32Snapshot` copies it
  into a section with the walk position. A process the kernel started shows parent 4 (`System`).
- **8.3 names** are generated from a directory's current entries
  (`poly_kernelbase::shortname`), not stored: the NT path walk resolves them, so a path from
  `GetShortPathNameW` opens the same file; a number can change after a delete.
- **`MOVEFILE_DELAY_UNTIL_REBOOT`** records `PendingFileRenameOperations` exactly as the sheet
  observes; Poly does not carry the list out at the next boot yet (a later step in the session
  start can).
- **`ReadDirectoryChangesW`** is the new `NtNotifyChangeDirectoryFile` over per-handle watches
  fed by the NT file services; changes made outside them (Linux programs) are not seen.

# Part 4: hive files of programs

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

# Part 5: the event log

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
