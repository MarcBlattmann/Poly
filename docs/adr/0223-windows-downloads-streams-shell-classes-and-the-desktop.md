# 0223: Chrome's downloads: alternate data streams, Poly's own COM classes, and Windows shell requests reaching the desktop

**Status:** Accepted (2026-11-17). Track `impl-chrome-dl`. Builds on ADR 0067 (NT file I/O over
the VFS) and ADR 0060 (NT attributes in the extended attribute `user.poly.ntattr`). Inputs: the
sheets of `kernel32.dll/FindFirstStreamW`, `FindNextStreamW`, `MoveFileExW` and
`GetVolumeInformationW`, `shell32.dll/SHCreateItemFromParsingName`, `IFileOperation.*`,
`SHParseDisplayName`, `ShellExecuteExW`, `ITaskbarList3.*`, `ICustomDestinationList.*` and
`comdlg32.dll/IFileSaveDialog.*` with their visible tests (spec-chrome-dl); Microsoft Learn
("File Streams (Local File Systems)", "FILE_STREAM_INFORMATION", "CopyFile", "CoGetClassObject",
"DllGetClassObject", "InprocServer32", "IFileOperation", "SHOpenFolderAndSelectItems",
"ShellExecuteExW", "DialogBoxW"). No third-party source.

Chrome writes a download to `<guid>.tmp` and renames it twice through the shell
(`SHCreateItemFromParsingName` and `CLSID_FileOperation`); before this, both were stubs on Poly
and the file never landed. The decisions below are what that path, and the download bubble's
"Open" and "Show in folder" and Chrome's Save As, needed beyond implementing the sheets.

# Part 1: alternate data streams

## Context

Chrome for Windows writes `<download>:Zone.Identifier` (`[ZoneTransfer] ZoneId=3 ...`) with
`CreateFileW` after each download, and renames the file afterwards; programs also expect a
volume that calls itself `NTFS` to keep named streams. Poly's NT layer refused every `:` in a
name (`STATUS_OBJECT_NAME_INVALID`). The VFS under it (tmpfs, ext2/3/4) has one data stream per
inode, and extended attributes that are small (one block on ext2).

## Decision

- `file:stream[:$DATA]` is split off the last component of an NT path; `file::$DATA` stays the
  file itself; an empty stream name or another type is an invalid name.
- Each named stream is a **regular file of its own** in a **stream directory** of the file. The
  directories live in `.poly-streams-<device>` beside the volume's root directory (the root's
  parent: outside the volume, so neither NT nor Linux listings of the volume see them). The
  file's extended attribute `user.poly.streams` names its directory (`<inode hex>-<n>`, made with
  an exclusive `mkdir`, so a directory left by a deleted file is never taken over; an attribute
  whose inode number is not the file's, a copy made with its attributes, is ignored).
- A stream handle is an ordinary disk file object on that backing file: reading, writing, size,
  end of file, locks and share access (per stream) need nothing new.
- Rename keeps the streams (same inode, same attribute); hard links share them;
  `FILE_SUPERSEDE`/`FILE_OVERWRITE[_IF]` of the file drops them; when the file's last name goes
  (delete, or replaced by a rename) its stream directory is removed. A stream itself cannot be
  renamed (`STATUS_INVALID_PARAMETER`).
- `FileStreamInformation` lists `::$DATA` (files only) and then the named streams by name
  without case; `CopyFileW` copies the named streams; the volume reports `FILE_NAMED_STREAMS`
  and the file system name `NTFS` (it was `PolyFS`).

## Consequences

- Streams of any size work, and persist where the volume does (on the ext2/3/4 disk the store
  is `<disk>/poly/nt/.poly-streams-HarddiskVolume1`).
- A volume whose root is the VFS root (`Z:`) has no place beside it and so no named streams
  (and no `FILE_NAMED_STREAMS`).
- A file deleted by a Linux program leaves its stream directory behind (never reused, only
  space); a file copied by a Linux program has no streams.
- Considered: the stream bytes in an extended attribute of the file. Rejected: ext2 keeps one
  block of attributes per inode, and a stream handle would need its own read/write/size paths.

# Part 2: the classes of Poly's own DLLs

## Context

On Windows `CoCreateInstance` finds an in-process class in the registry
(`HKEY_CLASSES_ROOT\CLSID\{clsid}\InprocServer32`), loads that DLL and asks its
`DllGetClassObject` for the class object. Poly's registry starts with empty `Classes` keys,
and `ole32` registered no class but the global interface table.

## Decision

- Poly's own DLLs list their classes in code (`poly_com::inproc`): `CLSID_FileOperation`,
  `CLSID_TaskbarList` and `CLSID_DestinationList` in `shell32`, `CLSID_FileSaveDialog` in
  `comdlg32`. They are part of the system, not installed programs, so they are not written into
  a registry hive.
- `CoCreateInstance[Ex]` and `CoGetClassObject` take the documented path for those classes:
  load the DLL, call its exported `DllGetClassObject` for `IClassFactory`, then
  `CreateInstance`. A context without `CLSCTX_INPROC_SERVER` finds no class.
- A class a program registers itself (`InprocServer32` in the registry) is still not looked
  up: that is the COM activation work of its own track (`spec-win-com-oop`).

# Part 3: Windows shell requests reach Poly's desktop

## Context

"Show in folder" (`SHOpenFolderAndSelectItems`) opens an Explorer window on Windows, and
"Open" (`ShellExecuteExW`) on a type no Windows program is registered for asks the user which
program to use. On Poly the file manager is Files, a native program, and a Windows process
had no way to reach the desktop but its windows (the win32k link, ADR 0128/0131).

## Decision

- `shell32` opens the file (`FILE_READ_ATTRIBUTES`) and sends `Request::ShellOpen { handle,
  show }` through `PolyUserCall`. The NT personality turns the handle into the object's path in
  Poly's file system (the volume's VFS path plus the case-correct names it was opened by) and
  sends the link record `ToCompositor::Shell { show, path }`. A file handle, not a DOS path,
  goes into the kernel: the personality already knows where its files are, so the mapping of
  drive letters and case to the VFS is not repeated anywhere.
- The compositor starts Files with `--show PATH` (the folder, with the item selected) or
  `--open PATH` (the program Files opens such a file with, as a double click would; with none,
  Files shows the file selected in its folder with the reason in its status line).
- `ShellExecuteExW` keeps Windows' order: a program runs itself, a document opens with the
  command of its type in `HKEY_CLASSES_ROOT`; only a folder, or a type nothing there opens, goes
  to the desktop, and only when the caller allows a user interface (`SEE_MASK_FLAG_NO_UI` is
  the sheet's failure, `0x80004021`).
- The record is not replayed when the compositor reconnects; without a compositor the request
  fails with error 50.

# Part 4: the Save dialog

`IFileSaveDialog::Show` runs Poly's one file dialog (the one `GetSaveFileNameW` shows) with the
object's settings; Poly's dialog looks like Poly's, while what the object answers before and
after `Show` is the sheet's. Two dialog fixes it needed: `DialogBox*` now shows a modal dialog
whose template has no `WS_VISIBLE` when its queue first runs empty (Learn, "DialogBoxW": it
"displays the dialog box regardless of whether the template specifies the WS_VISIBLE style"),
and the file dialog keeps the focus in its file name field, so Enter means Save.

## Consequences

- Chrome's downloads land in `Downloads` under their names with their `Zone.Identifier`
  stream; the bubble's "Show in folder" opens Files with the file selected, "Open" opens it with
  Poly's program for its type; Save As shows a dialog and saves.
- Other Windows programs get the same: any caller of these shell functions and classes.
- Taskbar progress and jump lists succeed without effect until Poly's panel has them.
