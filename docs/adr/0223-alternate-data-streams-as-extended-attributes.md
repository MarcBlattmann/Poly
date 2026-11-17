# 0223: Alternate data streams: a file per stream, named by an extended attribute

**Status:** Accepted (2026-11-17). Track `impl-chrome-dl`. Builds on ADR 0067 (NT file I/O over
the VFS) and ADR 0060 (NT attributes in the extended attribute `user.poly.ntattr`). Inputs: the
sheets of `kernel32.dll/FindFirstStreamW`, `FindNextStreamW`, `MoveFileExW` and
`GetVolumeInformationW` with their visible tests (spec-chrome-dl); Microsoft Learn ("File
Streams (Local File Systems)", "FILE_STREAM_INFORMATION", "CopyFile"). No third-party source.

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
