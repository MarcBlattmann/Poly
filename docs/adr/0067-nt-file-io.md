# 0067: NT file I/O: file and device objects over the VFS, drive letters, share modes, byte-range locks, IRP-style async I/O

**Status:** Accepted. Builds on [ADR 0063](0063-nt-processes-object-manager-and-memory.md)
(NT processes, the object manager, memory and sections in the kernel),
[ADR 0030](0030-nt-object-manager-and-dispatcher-in-a-crate.md) (`poly-ntobj`:
the object manager, its `ParseHook` and signalable opaque objects),
[ADR 0065](0065-nt-threads-synchronisation-and-apcs.md) (threads, waits and
user APCs), [ADR 0023](0023-vfs-tmpfs-and-cpio.md) (`poly-vfs`: the VFS the
file objects run over) and [ADR 0060](0060-extended-attributes-and-file-ioctls.md)
(extended attributes, where the NT-only attribute bits are stored).

## Context

The next Phase 5 roadmap item is "NT file I/O: case-insensitive lookup,
drive letters, share modes, byte-range locks, IRP-style async I/O". ADR 0063
left the object manager with opaque `File` and `Device` placeholders and a
`ParseHook` for objects that own a sub-namespace, and the kernel with a
`Sections` store that held "minimal file objects" (a file's bytes) only so a
section could be mapped from a data file; nothing opened a real file.

As in the earlier NT milestones there is no Windows oracle VM, so the
clean-room rules ([CLEAN_ROOM.md](../../CLEAN_ROOM.md)) allow only public
Microsoft documentation (learn.microsoft.com: `NtCreateFile`/`ZwCreateFile`,
`ZwOpenFile`, `ZwReadFile`, `ZwWriteFile`, `ZwQueryInformationFile`/
`ZwSetInformationFile` and the documented `FILE_INFORMATION_CLASS`
structures, `ZwQueryDirectoryFile`, `ZwQueryVolumeInformationFile`,
`ZwFlushBuffersFile`, `ZwLockFile`/`ZwUnlockFile`, `ZwDeleteFile`,
`NtCancelIoFile`, `IO_STATUS_BLOCK`, the "Creating and Opening Files"
sharing table, "FsRtlIsNameInExpression"), public SDK/WDK header *names,
layouts and constants* (`winnt.h`, `wdm.h`, `ntifs.h`, `ntstatus.h`) and the
VFS's own behaviour. Nothing is disassembled, decompiled or debugged, and no
Wine or ReactOS source is read. Where documentation does not settle a
behaviour the code picks one and says "to be confirmed by the spec agent
against real Windows".

## Decision

**The hardware-free logic is a new module `personalities/nt/src/io`,
host-tested over a real `poly-vfs`; the kernel (`kernel/src/nt/io.rs`) only
acts.** The module owns an `IoState` that lives in `NtSystem` behind the one
NT lock (ADR 0030): the volumes, the file objects (keyed by their object's
cookie), each open node's share access and delete-pending state, the
byte-range locks, the pipes and the I/O request packets. The kernel gives it
the VFS and carries out what it decides: the transfers, the worker threads,
the blocking and the completion delivery.

### Objects and names

- **A device per VFS mount.** Each mount is an opaque `Device` object
  `\Device\PolyFs<N>` created with `parse: true`; a namespace walk that
  reaches it hands the rest of the path to the create hook. **Drive letters
  are symbolic links in `\??`**: `\??\C:` → `\Device\PolyFs1`, the VFS path
  `/nt/c`, the NT *system root*; `\??\Z:` → `\Device\PolyFs0`, `/`, the whole
  host-visible VFS; and one unnamed-letter device per other mount, by mount
  point. So `\??\C:\Windows\win.ini` reaches `PolyFs1` through the link and
  becomes `/nt/c/Windows/win.ini`. The mount table is read when the NT
  personality first attaches; later mounts show under `Z:` but get no device
  of their own (watching the mount table is left for later).
- **A file object is itself a parse object**, so an open relative to a
  directory handle (`RootDirectory`) continues from that directory.
- **Case-insensitive, case-preserving lookup over the case-sensitive VFS.**
  Each component is looked up first by its exact UTF-8 bytes (one VFS call),
  else by scanning the directory for a name equal without case (ADR 0030's
  upcase rule). **Names that differ only in case** (a Linux program made
  `Readme` and `README`): an exact match wins, otherwise the candidate whose
  bytes are smallest, so the choice is deterministic whatever order the
  directory lists in; enumeration shows both in their own case, and a create
  that matches one without case collides with it. NT names are UTF-16;
  a name the VFS holds that is not UTF-8 is shown with U+FFFD and cannot be
  opened by that name. `.` and `..` and the characters `< > : " / \ | ? *`
  and 1–31 are rejected in a component (`STATUS_OBJECT_NAME_INVALID`).

### Create, read, write, information

- **`NtCreateFile`/`NtOpenFile`** apply the six dispositions (SUPERSEDE,
  OPEN, CREATE, OPEN_IF, OVERWRITE, OVERWRITE_IF) and the options
  (DIRECTORY_FILE, NON_DIRECTORY_FILE, DELETE_ON_CLOSE,
  SYNCHRONOUS_IO_ALERT/NONALERT, OPEN_REPARSE_POINT), map generic access
  rights, and enforce share modes across handles by the documented
  "Creating and Opening Files" table (`STATUS_SHARING_VIOLATION`). Every
  check runs before anything changes on disk, so a refused open never
  truncates. `FILE_ATTRIBUTE_READONLY` is the absence of write permission
  and is enforced by NT (the VFS lets root write anything); `DIRECTORY` and
  `REPARSE_POINT` come from the node type; `HIDDEN`, `SYSTEM`, `ARCHIVE`,
  `TEMPORARY`, `OFFLINE`, `NOT_CONTENT_INDEXED` and the birth time are stored
  in the extended attribute `user.poly.ntattr` (ADR 0060; a file system
  without xattrs keeps none). Times are 100 ns since 1601.
- **Read and write** support a file position (synchronous handles), an
  explicit `ByteOffset`, and `FILE_APPEND_DATA`. The documented information
  classes are answered (`FileBasic`, `Standard`, `Position`, `Name`,
  `Internal`, `EndOfFile`, `Disposition`, `Rename`, `AllInformation`,
  `NetworkOpen`, `AttributeTag`, `Mode`, `Access`, `Alignment`), the
  directory classes (`FileDirectory`, `FileFullDirectory`,
  `FileBothDirectory`, `FileIdBothDirectory`, `FileIdFullDirectory`,
  `FileNames`) and the basics of `NtQueryVolumeInformationFile`
  (`Volume`, `Size`, `FullSize`, `Device`, `Attribute`). Our volumes report
  a file system named `PolyFS`, case-preserving but not case-sensitive.

### IRP-style asynchronous I/O

- **Synchronous vs. asynchronous.** A handle opened with
  `FILE_SYNCHRONOUS_IO_*` keeps a current byte offset and each call waits
  for its request; a volume transfer then runs in the calling thread,
  outside the NT lock. Without those options the handle is asynchronous:
  `ByteOffset` is required and a read or write becomes an **I/O request
  packet** (`Irp`) queued on the volume's device; the call returns
  `STATUS_PENDING` and completes later through the `IO_STATUS_BLOCK`, the
  `Event` (reset when the request is made), or the file object when there is
  no event, and the APC (`ApcRoutine(ApcContext, IoStatusBlock, 0)`, ADR 0065,
  delivered at an alertable wait).
- **A per-device worker.** The kernel runs one `nt-io` worker thread per
  volume device: it takes the next request (`next_work`, under the NT lock),
  runs the transfer on the VFS with no NT lock held, records the result and
  completes it. A request is thus asynchronous and cancellable.
- **Completion order.** The kernel writes the caller's buffer and the
  `IO_STATUS_BLOCK` into the requesting process's memory *first* (outside the
  NT lock — a user page fault takes the NT lock, so it is never held across
  user-memory access), then signals the event or file object and drains the
  dispatcher's wakes, then queues the APC. A thread woken by the event or
  running the APC therefore always sees the status block filled in.
- **Cancellation.** `NtCancelIoFile` (`STATUS_CANCELLED`) cancels the
  calling thread's queued or waiting requests on a handle; `NtCancelIoFileEx`
  the process's, all or the one named by an `IO_STATUS_BLOCK`; a thread's
  requests are all cancelled when it exits (without their APCs). A request
  already running on the worker finishes on its own.
- **`\Device\PolyPipe\<name>`** is Poly's own slow byte-stream device, a
  stand-in until named pipes exist: a read with no data waits until a write
  to the same name feeds it, which is what the overlapped-I/O and
  cancellation tests need. **I/O completion ports are deferred**
  (no `NtCreateIoCompletion` yet): a request completes through its event,
  file object and APC.

### Byte-range locks

`NtLockFile`/`NtUnlockFile` keep a per-file table of locks owned by a
(file object, `Key`) pair. Locks are **mandatory and never merged**: an
exclusive lock excludes all, a shared lock excludes writes; a read
overlapping another owner's exclusive lock and a write overlapping any lock
that is not the owner's own exclusive one fail with
`STATUS_FILE_LOCK_CONFLICT`; a lock that cannot be granted is
`STATUS_LOCK_NOT_GRANTED` (fail-immediately) or waits as an IRP; an unlock
must name an exact locked range (`STATUS_RANGE_NOT_LOCKED`). **This is not
the POSIX `poly_vfs::locks` table** (ADR 0059): POSIX locks merge, split and
convert and are advisory, NT locks do not and are mandatory, so the two are
separate and a Linux process's `fcntl` locks and an NT process's locks on one
file do not conflict yet (joining them is left for later).

### Services (Poly's own numbering, `services.rs`, ADR 0063's layout)

A new executive block `0x0C0`–`0x0FF` (file I/O): `NtCreateFile` (0x0C0),
`NtOpenFile` (0x0C1), `NtReadFile` (0x0C2), `NtWriteFile` (0x0C3),
`NtQueryInformationFile` (0x0C4), `NtSetInformationFile` (0x0C5),
`NtQueryDirectoryFile` (0x0C6), `NtQueryVolumeInformationFile` (0x0C7),
`NtFlushBuffersFile` (0x0C8), `NtLockFile` (0x0C9), `NtUnlockFile` (0x0CA),
`NtDeleteFile` (0x0CB), `NtCancelIoFile` (0x0CC), `NtCancelIoFileEx` (0x0CD),
`NtQueryAttributesFile` (0x0CE), `NtQueryFullAttributesFile` (0x0CF). Numbers
are never reused.

## To be confirmed by the spec agent against real Windows

In addition to the earlier NT lists:

1. The `NTSTATUS` for each VFS error (`io::status_of`), the name-component
   rules (`.`/`..`, stream names, the invalid characters), and whether an
   odd `FileName` length in `FileRenameInformation` is
   `STATUS_INVALID_PARAMETER`.
2. The exact order of the create checks (type options, disposition,
   delete-pending, read-only, share access) and whether an overwrite of a
   `HIDDEN`/`SYSTEM` file without that attribute is `STATUS_ACCESS_DENIED`.
3. Whether a superseded file keeps its file id; whether a file object's
   share access is released at cleanup (last handle) or at close (last
   reference) — Poly releases it when the last reference goes, so a section
   mapping a file holds it until the section is gone.
4. The `FILE_DIRECTORY_INFORMATION` short-name field (always empty here),
   `RestartScan` with a new `FileName`, and the status for a first-call
   buffer that holds the fixed part but not the first name
   (`STATUS_BUFFER_OVERFLOW`, entry consumed).
5. Zero-length lock semantics, which of two identical exclusive/shared locks
   an unlock removes first, and whether an alertable synchronous wait is cut
   short by an APC (Poly waits non-alertably).
6. The volume `FileSystemAttributes`, serial numbers and the reported file
   system name (`PolyFS`); whether applications probe NTFS features rather
   than the name.
7. `FILE_USE_FILE_POINTER_POSITION`/`FILE_WRITE_TO_END_OF_FILE` on an
   asynchronous handle, and whether `ByteOffset` is truly required there.

## Consequences

- The path parsing, case folding, the share-mode table, the lock table, the
  wildcard matcher and every information-class layout are host-tested in
  `personalities/nt/src/io` over a real tmpfs (`cargo test -p
  poly-personality-nt io`), with the kernel played by the fake of the other
  NT host tests and the worker and completion delivery played by test
  helpers. The kernel side proves the ring-3 path, the worker threads and
  the asynchronous completion with one `kernel_test!`
  (`nt::tests::nt_file_io_from_ring3`) driving a self-assembled PE: a file
  created, written, read and closed under `C:\`; a mixed-case open; a
  sharing violation; a wildcard directory enumeration; overlapped reads
  completing through an event and through an APC; a cancelled pending read on
  a pipe; a lock conflict; and a delete-on-close.
- The edits to shared NT files stay small and additive: `services.rs` gains a
  block, `lib.rs` a dispatch arm per service, `system.rs` an `io` field and a
  reap hook, and `kernel/src/nt/mod.rs` the attach/register/after-call/drain
  calls — so the milestone merges cleanly beside the registry work running in
  parallel.
- The coverage map is unchanged: these services cannot go green without the
  Windows oracle, which does not exist yet.
- What is left: I/O completion ports, named pipes and mailslots, reparse
  points beyond the symbolic-link tag, alternate data streams, extended
  attributes (`EaBuffer` is `STATUS_EAS_NOT_SUPPORTED`), oplocks, and joining
  the NT and POSIX lock tables so the personalities see each other's locks.
