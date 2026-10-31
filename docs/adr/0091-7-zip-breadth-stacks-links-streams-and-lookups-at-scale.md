# 0091: 7-Zip in breadth: 99 boot-test steps, every thread's stack grows, a fault no longer overflows the kernel stack, hard links, streams and reparse points, and case-insensitive lookups at scale

**Status:** Accepted. Extends [ADR 0087](0087-7-zip-command-line-on-poly.md)
(the `7zip` boot test, whose five steps it keeps) without changing its
decisions. Builds on [ADR 0067](0067-nt-file-io.md) (NT file I/O),
[ADR 0068](0068-nt-structured-exception-handling-and-x64-unwinding.md) (exception dispatch) and
[ADR 0061](0061-per-cpu-interrupt-stacks.md) (kernel stack budget).

## Context

ADR 0087's boot test ran `7za.exe` five ways (usage, `i`, `a`, `t`, `x` of
one small file). Whether 7-Zip runs *nicely* needs much more: every format
`7za` reads and writes, archive updates, directories and wildcards, large
files and many files, threads, passwords, volumes, standard output, names
beyond ASCII and past `MAX_PATH`, attributes, hashes, character sets,
NTFS features and errors, and how long each takes under TCG.

Sources (clean room): Microsoft Learn's pages for `CreateHardLinkW`,
`FindFirstStreamW`, `FindNextStreamW`, `FindClose`, "File Streams (Local
File Systems)", `FILE_LINK_INFORMATION`, `FILE_STREAM_INFORMATION`,
`FSCTL_GET_REPARSE_POINT`, `FSCTL_SET_REPARSE_POINT`, "Thread Stack Size",
`CreateThread`; the public SDK headers for layouts and constants
(`ntifs.h`: `FILE_LINK_INFORMATION`, `FILE_STREAM_INFORMATION`,
`REPARSE_DATA_BUFFER`, `SYMLINK_FLAG_RELATIVE`; `wdm.h`/`ntifs.h`: the
information class numbers; `minwinbase.h`: `WIN32_FIND_STREAM_DATA`;
`winioctl.h`: the `FSCTL_*` codes; `winnt.h`: `IO_REPARSE_TAG_SYMLINK`;
`ntstatus.h`), and 7-Zip's documented command line and exit codes
(0, 1, 2, 7, 8, 255). 7-Zip was only run, on Poly. No Windows binary,
Wine, ReactOS or 7-Zip source was looked at.

## Decision

### The boot test: 99 steps, the inputs made by the same release's Linux `7zz`

`cargo xtask win fetch-7zip` now keeps the Linux `7zz` it already
downloaded (`target/7zip/26.05/linux/7zz`) instead of deleting it; CI's
cache key became `7zip-26.05-7zz` so the cache holds it. The boot test
writes its inputs deterministically (`sevenzip::sources`: an xorshift
generator, fixed times): `mid.txt` (64 KiB of text), `big.txt` (3 MiB),
`rand.bin` (300 KiB that does not compress), `tree/` (225 small files in
nine directories and five `.log` files), `many/` (3,000 tiny files in one
directory), `uni/` (Latin, CJK, Greek, an emoji outside the BMP, a space),
`long/` (a path of 300 characters below the program's directory), a
read-only `ro.txt`, a UTF-8 list file and a symbolic link. `7zz` makes the
archives the extraction steps read: 7z with LZMA, LZMA2, PPMd, BZip2,
Deflate, Deflate64 and Copy; zip, tar, gzip, bzip2, xz; passwords (7z with
encrypted headers, zip AES-256); 100 KiB volumes; hard links and a
symbolic link in tar; the tree, the big file, the 3,000 files. A damaged
and a truncated copy are made by the test. Each step is still its own boot
of one initramfs (7.5 MB) with 512 MiB of memory; a step may ask for more
CPUs (`-smp 4` for the threaded ones) and a longer timeout (the benchmark).
The report gives each step's exit code against the one it expects, what it
printed, and the program's own run time (from `POLY-INIT: exec` to
`POLY-EXIT`, as the serial lines arrive) beside the boot's. Every step must
pass except four reported only (`NOT_YET`), all ending in a C++ exception
(below). `wim` is not a format of `7za` (`7za i`), so it has no step.

The whole group takes about 190 s on TCG (95 required steps of about one
second each; the benchmark `b -mmt1 -md18` 20–40 s: about 1,500 MIPS).

### Fixes the steps found

1. **Every thread's stack grows** (`exec::make_stack`). Only the first
   thread's stack allocation was marked as a stack, so a new thread
   touching its guard page got `STATUS_GUARD_PAGE_VIOLATION` delivered as
   an exception instead of a grown stack. 7-Zip's worker threads (1 MiB
   reservations, one page committed) hit it at once: `a -tzip` of a tree
   and `-tbzip2 -mmt4` died. Learn's "Thread Stack Size" describes the
   guard-page growth for every thread.

2. **A user fault cannot overflow the kernel stack.** The exception above
   could not be delivered (the dispatcher's frame ran past the stack), so
   the kernel walked the user stack for the unhandled-exception backtrace;
   that walk reads user memory that page-faults into `resolve_fault`, and
   the chain `fault` → backtrace → `poly_seh::dispatch::walk` → `step` →
   page fault → `nt::with` went past the 16 KiB kernel stack in the debug
   kernel (a double fault: user mode could panic the kernel). In ADR
   0061's way the frames were trimmed, not the stack raised: `poly-seh`'s
   walks keep their `CONTEXT`s boxed (`walk` 3,968 → 272 bytes, `step`
   1,440 → 224), the pending unhandled exception in the kernel's `Kernel`
   is boxed (it is in every service's frame), the faulting thread's
   `CONTEXT` is boxed, the unhandled path is out of line, and the NT
   state's first construction left `nt::with` (2,544 → 176 bytes). The
   whole path is now about 5 KiB.

3. **Hard links**: `NtSetInformationFile(FileLinkInformation)` (the
   `FILE_RENAME_INFORMATION` layout) gives a file another name on the same
   volume through the VFS's `link`; directories are
   `STATUS_FILE_IS_A_DIRECTORY`, an existing name is replaced only with
   `ReplaceIfExists` and as a rename replaces (not a directory, read-only or
   open file). `CreateHardLinkW` was already written over it; `x -snh` of a
   tar with hard links now works. Our volumes already claimed
   `FILE_SUPPORTS_HARD_LINKS`.

4. **Streams**: `FileStreamInformation` reports a file's default data
   stream `::$DATA` (its size and allocation) and nothing for a directory;
   `FindFirstStreamW`/`FindNextStreamW` are written over it (a find handle
   `FindClose` closes; `ERROR_HANDLE_EOF` at the end and for a directory),
   and opening `name::$DATA` opens `name`. Poly's volumes keep no named
   streams (they do not claim `FILE_NAMED_STREAMS`), so any other `:` is
   still an invalid name.

5. **Reparse points for symbolic links**: `FSCTL_GET_REPARSE_POINT` on a
   symbolic link opened with `FILE_OPEN_REPARSE_POINT` answers a
   `REPARSE_DATA_BUFFER` with `IO_REPARSE_TAG_SYMLINK` (a relative target
   with `SYMLINK_FLAG_RELATIVE`, an absolute one as `\??\Z:\…`, the VFS root
   being `Z:`); `FSCTL_SET_REPARSE_POINT` with one turns an empty file or
   directory into a VFS symbolic link (an absolute target on a drive letter
   becomes that volume's VFS path). A symbolic link reports size 0, as it
   holds no data of its own. `a -snl` and `x -snl` now work.

6. **Case-insensitive lookups at scale.** Extracting 3,000 files into one
   directory took 246 s: every file made was a lookup that found nothing,
   and each such lookup read and compared the whole directory, twice
   (`CreateFileW` of the new name, and 7-Zip's `FindFirstFileW` of the
   path, a directory query whose pattern names one file). Now 9 s:
   - `poly-vfs` counts every change of a name (`Vfs::namespace_generation`:
     create, mkdir, mknod, symlink, link, unlink, rmdir, rename, only when
     they succeed).
   - The NT walk keeps case-folded indexes of the last eight directories it
     missed in (`walk::FoldCache`, in `VfsRoot`). They are right while the
     generation has not moved; one step more is accepted when it is the
     name the last lookup missed now being there (the usual next step: the
     file made), which joins its directory's index; anything else drops
     them all. A name an index gives is checked to exist before use, so a
     rename seen only as its new name cannot find the old one. Only
     directories whose names change through the VFS alone are indexed
     (file systems that let the VFS cache lookups, and tmpfs), not procfs
     or devpts.
   - A directory query whose pattern has no wildcard lists only the
     spellings of that name (`walk::find_all`), without reading the
     directory.

### Seen, not fixed here

- **C++ exceptions** (`0xE06D7363`): 7-Zip reports a missing archive
  (`x missing.7z`), a bad command (`q`) and the failures behind `a -sns`
  and `a -sni` (after `GetFileSecurityW`'s `ERROR_NOT_SUPPORTED` warnings)
  by throwing; without C++ exception handling (ADR 0087's stub) the process
  ends there. Another workstream owns that; the four steps are reported
  only until then.
- `-si` (standard input) is not run: the boot's standard input is the
  serial console, with nothing to type. A full disk is not run (Poly's
  tmpfs has no size limit to reach).
- File security descriptors (`GetFileSecurityW`, `SetFileSecurityW`) stay
  stubs: Poly's files keep none.

## To be confirmed by the spec agent against real Windows

1. `FindFirstStreamW` on a volume without named streams (FAT): ours lists
   `::$DATA` for a file, `ERROR_HANDLE_EOF` for a directory; and
   `FileStreamInformation` of a console or pipe (ours: none).
2. `FileLinkInformation` onto a name that already is one of the file's own
   (ours: success with `ReplaceIfExists`, else
   `STATUS_OBJECT_NAME_COLLISION`); `CreateHardLinkW`'s error for an
   existing new name (ours: `ERROR_ALREADY_EXISTS`).
3. `FSCTL_SET_REPARSE_POINT` on a file with data and with malformed data
   (ours: `STATUS_NOT_SUPPORTED`, `STATUS_INVALID_PARAMETER`; Windows may
   say `STATUS_IO_REPARSE_DATA_INVALID`), and what the handle sees after it.
4. The size `FindFirstFileW` reports for a symbolic link (ours: 0).

## Consequences

- The `7zip` group is about 190 s of the `userland` shard (was 4 s); it
  needs the Linux `7zz` in the cache (`fetch-7zip` gets it), and skips
  without it as without `7za.exe`.
- Host tests: `poly-personality-nt` (a new thread's stack grows; hard
  links; streams and `::$DATA`; reparse points both ways; lookups while a
  directory changes; literal directory patterns), `poly-vfs` (the
  generation counts every change of a name and nothing else),
  `poly-kernelbase` (`WIN32_FIND_STREAM_DATA`), `xtask` (the inputs are
  deterministic, step names unique, the run time); `k32test.exe` checks
  `CreateHardLinkW` and the stream functions for real.
- Shared code changed: `poly-vfs` (`Vfs` gains the counter), `poly-seh`
  (boxed contexts in its walks), `VfsRoot::new` (every construction site),
  `kernel/src/nt` (frames), `NtDeviceIoControlFile` (file-system controls
  for files on a volume, before the console's), `xtask`
  (`with_boot_timeout`).
- Coverage: `CreateHardLinkW`, `FindFirstStreamW` and `FindNextStreamW`
  (`kernel32`, `kernelbase`) went from `stub` to claimed by
  `impl-7zip-breadth`; docs/windows/7zip-gaps.md has four stubs left.
