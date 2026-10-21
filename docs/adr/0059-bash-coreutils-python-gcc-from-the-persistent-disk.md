# 0059: bash, coreutils, Python and gcc from a persistent ext4 root disk made at test time; the system calls they needed

**Status:** Accepted. Builds on [0040](0040-page-cache-and-ext2-on-virtio-blk.md)
(the page cache), [0043](0043-dev-proc-sys-and-file-system-calls.md) (the
file system calls, `LinuxFs`), [0049](0049-dynamic-linking-program-interpreter.md)
(glibc's own `ld.so`, the host's libraries in test images, the `mremap`
and `rseq` decisions), [0051](0051-busybox-init-mount-and-power-off.md)
(BusyBox as init, mount(2), `poly.strace`), [0052](0052-ext4-extents-checksums-and-journal.md)
(ext4) and [0053](0053-smp-per-cpu-data-scheduling-and-tlb-shootdowns.md) (SMP).

## Context

The last line of roadmap Phase 4 asks for "bash, coreutils, Python and a C
compiler run from the persistent disk". Dynamic linking, ext2/3/4 on
virtio-blk behind a page cache, mount(2), chroot(2), BusyBox as init with
job control and about 200 system calls were there; nothing had run a real
interpreter or compiler, and no disk held a root tree.

These programs are GPL (bash, coreutils, grep, sed, gcc, binutils) or
PSF-licensed (Python): run as test programs, never committed. As for
BusyBox (ADR 0051), what they need was found only on Poly: the kernel's
`not implemented` lines, `poly.strace`, and the programs' own error
messages (`ls: /etc/hostname: Function not implemented`, Python's
`OSError: [Errno 38]`, gcc's `execv: Cannot allocate memory`). No
Linux, C library, bash, Python or gcc source was read, and nothing was run,
traced or observed on a real Linux system.

Sources: mremap(2), msync(2), fcntl(2) (record locks, open file
description locks), flock(2), close_range(2), copy_file_range(2),
posix_fadvise(2), getxattr(2), setxattr(2), listxattr(2), removexattr(2),
xattr(7), getrusage(2), wait4(2), read(2), signal(7), proc(5), chroot(2),
getcwd(2), ld.so(8), mke2fs(8), mke2fs.conf(5); the UAPI headers
`asm-generic/fcntl.h`, `linux/fcntl.h`, `linux/fs.h`, `linux/mman.h`,
`asm-generic/mman-common.h`, `linux/close_range.h`, `linux/fadvise.h`,
`linux/xattr.h`, `linux/limits.h`, `linux/resource.h`,
`asm-generic/errno-base.h`, `asm-generic/errno.h`.

## Decision

**The disk is made at test time from the host's own files**
(`tools/xtask/src/rootdisk.rs`). `cargo xtask root-disk` (and the boot
test, into a separate image) collects: bash; GNU coreutils (`ls`, `cat`,
`cp`, `mv`, `rm`, `mkdir`, `sort`, `wc`, `head`, `tail`, `env`, `date`,
`seq`, `dd`, `stty`, `sync` and a few more); grep and sed; Python 3 (the
interpreter `/usr/bin/python3` resolves to, with its standard library
directory, less tests, GUI, installer and documentation parts and the
static `config-*` build); the C compiler (the driver `gcc`/`cc` resolve
to, `cc1`, `collect2`, `lto-wrapper` and the LTO plugin from
`/usr/libexec/gcc/<triple>/<version>`, the start files, libgcc and its
linker script from `/usr/lib/gcc/<triple>/<version>`, `as` and `ld`, the C
library's link-time files `libc.so`, `libc_nonshared.a`, `crt1.o`,
`Scrt1.o`, `crti.o`, `crtn.o` and what the `libc.so` linker script names),
and the headers an ordinary C program includes: the closure of
`#include`/`#include_next` lines from `stdio.h`, `stdlib.h`, `string.h`,
`unistd.h` and others, resolved in the compiler's include directories
without evaluating conditionals (a superset). Shared objects come from the
`DT_NEEDED` closure of every ELF file in the tree (Python's extension
modules and the LTO plugin included) and `PT_INTERP`, searched as ld.so(8)
describes with the lookup `libsearch.rs` shares with the pipeline;
nothing is run on the host to find anything (no `ldd`, no `gcc
-print-*`): gcc's directories are found by their layout, its version from
the driver's name. Every file keeps its host path, with the directory's
symbolic links resolved (`/lib/x86_64-linux-gnu/libc.so.6` lands in
`/usr/lib/x86_64-linux-gnu` when `/lib` is a link), symbolic links stay
links and the host's top-level links (`/bin` -> `usr/bin`, ...) are made,
so the loader's default search path finds everything without
`/etc/ld.so.cache`. A missing program is left out with a note and the boot
test skips what needs it.

The image is written with `mke2fs -t ext4 -d` from a staging directory of
hard links (copies across file systems) when e2fsprogs are installed, with
a `mke2fs.conf` of its own that turns on exactly the ext4 features
`poly-ext2` supports (the host's defaults may add `orphan_file` and the
like); otherwise (or with `POLY_ROOTDISK_FORMAT=poly-ext2`) with
`poly-ext2`'s `format` and the driver itself. Its size is the contents plus
a quarter plus 160 MiB (about 280 MiB, 110 MiB on disk as a sparse file);
nothing of it is committed.

**Booting from it: BusyBox's init and a chroot.** Rather than a new
"root on disk" mode in the kernel, the initramfs is BusyBox's
(`initramfs::disk_root_image`): `rcS` as before, then
`/etc/init.d/rootdisk` mounts `/dev/vda` (ext4, the only virtio-blk disk
of these boots) on `/mnt/root` and binds `/dev`, `/dev/pts`, `/dev/shm`,
`/proc`, `/sys` and `/tmp` into it, and init respawns `cttyhack chroot
/mnt/root /bin/bash -l` on the console. The disk has its own `/etc`
(`passwd`, `group`, `profile`, `inputrc`, ...), and `/usr/local/sbin/
poweroff` and `reboot` (scripts that send init `SIGUSR2` / `SIGTERM`, as
BusyBox's own applets do). Guests get 1 GiB. `cargo xtask run
--disk-root` boots it interactively; the image is kept between runs
(`--fresh-disk` makes it again), so what one session writes the next one
finds.

**What the programs needed**, each found on Poly:

- **fadvise64** (`sort`, `cat`): checked (`EBADF`, `ESPIPE` for pipes,
  `EINVAL` for unknown advice) and accepted; the page cache's read-ahead
  does not take hints.
- **copy_file_range** (`cp`): through a kernel buffer like `sendfile`,
  with copy_file_range(2)'s file-offset-or-pointer rules; `EXDEV` across
  file systems (callers fall back to read and write), `EBADF` for
  `O_APPEND` outputs, `EINVAL` for overlapping ranges of one file.
- **The extended attribute calls** (`ls -l` asks for ACLs and labels):
  the twelve `*xattr` calls look the file up and answer `EOPNOTSUPP`, what
  the pages give for a file system without extended attributes; names are
  checked (`ERANGE`), `setxattr`'s flags too (`EINVAL`). Poly stores none
  yet.
- **close_range** (Python's `subprocess`): a `FdTable` operation;
  `CLOSE_RANGE_CLOEXEC` marks instead of closing; `CLOSE_RANGE_UNSHARE`
  is nothing to do for one thread and `EINVAL` with others, as unshare(2)
  with `CLONE_FILES` (ADR 0054).
- **mremap** (glibc's `realloc` of large blocks, Python's `mmap.resize`;
  ADR 0049 had left it for the program that needs it): the region map
  decides in a host-tested `VmMap::remap` (shrink, grow in place into free
  space, move with `MREMAP_MAYMOVE` to a hole, `MREMAP_FIXED` replacing
  what is there; `EFAULT` for a range that is not inside one mapping, the
  break area and the stack are not remapped; nothing changes on error, it
  works on a copy of the map). The kernel then moves the pages without
  copying (`user/memory/remap.rs`): its own frames are taken out of the
  page tables and mapped at the new address with the rights they had,
  parked frames and shared frames (copy on write after fork, shared
  anonymous memory) are filed under the new address (the next fault maps
  a shared one), page-cache pages are let go (the moved region maps the
  same file pages). `MREMAP_DONTUNMAP` and duplicating a shared mapping
  (`old_size` 0) are `EINVAL`.
- **msync** (Python's `mmap.flush`): the range must be mapped (`ENOMEM`);
  `MS_SYNC` syncs the files of the shared mappings in it; `MS_ASYNC` and
  `MS_INVALIDATE` have nothing to do with one page cache.
- **Record locks and flock** (SQLite, Python's `fcntl.lockf` and
  `fcntl.flock`): `F_GETLK`, `F_SETLK`, `F_SETLKW` and the open file
  description forms `F_OFD_*`, and flock(2). The rules are a host-tested
  `poly_vfs::locks::LockTable` (byte ranges per owner, split and merged,
  conflicts between owners when one writes; flock's whole-file locks,
  which never interact with record locks); the kernel keeps one table for
  every file (keyed by device and inode, `fs/locks.rs`). A process's
  locks (owner: its descriptor table) go when it closes any descriptor of
  the file, as fcntl(2) requires: every close path of `fs::Files` (close,
  `dup2`/`dup3` over a descriptor, close-on-exec, `close_range`, exit)
  tells the table, and does nothing until the first lock is ever taken.
  A description's locks go once the description is released (the table
  keeps weak references and drops dead owners at each close and lock
  call). `F_SETLKW` and blocking flock wait on the process event queue
  for a lock generation to move, interrupted by signals, kills and exits
  (`RESTART_IF_SA_RESTART`: signal(7) lists file lock waits as restarted
  under `SA_RESTART`). A conflict without waiting is `EAGAIN` (fcntl(2)
  allows `EACCES` or `EAGAIN`). Deadlocks are not detected.
- **Children's CPU time** (`resource.getrusage(RUSAGE_CHILDREN)`, bash's
  `time`): the accumulator existed but was never filled. When a waiter
  reaps a child, what the child and its own waited-for children used is
  added to the waiter's children's time and kept for wait4(2)'s and
  waitid(2)'s `rusage` (`CpuTimesOf::Reaped`); a process reaped as it ends
  (its parent ignores `SIGCHLD`) was not waited for and counts for nobody.
- **Whole reads** (`dd bs=1M count=32` read 2 MiB): a read of a regular
  file or a memory device (major 1: `/dev/zero`, `/dev/urandom`, ...) now
  goes on chunk by chunk until it has what was asked (or a short chunk),
  as Linux gives; dd counts a short read of `/dev/zero` as a partial
  record. Pipes and terminals still get one read.
- **exec reads only the headers** (`execv` of the 30 MiB `cc1` failed with
  `ENOMEM`): exec read the whole file into the kernel heap to parse it.
  `poly_elf::Elf::parse_prefix(head, file_len)` validates the headers
  from the first bytes (a page, then 64 KiB, the whole file only if the
  program header table or interpreter path lies further in) with the
  segments checked against the file's length; the segments are mapped
  from the file as before.
- **`/proc` paths from the reader's root** (`readlink /proc/self/exe` in
  the chroot read `/mnt/root/usr/bin/readlink`): procfs names files,
  directories and mappings as seen from the reading process's root
  directory (chroot(2)); kernel threads see the system's root, and a file
  outside the reader's root is named from the system's root.

`rseq` stays `ENOSYS` (ADR 0043, 0049): every program here treats it as
"no rseq".

**The boot test** (`rootdisk::boot_test`, in `cargo xtask test`, alone
`cargo xtask root-disk-test`) makes a fresh image, boots it and types into
the disk's bash (prompts and the session code are the BusyBox test's,
`busybox::Session`, with generous per-step deadlines): `stty`; bash as a
login shell (`$BASH_VERSINFO`, `login_shell`), its window size
(`$COLUMNS` / `$LINES` from `TIOCGWINSZ`), command substitution,
arithmetic, here-strings; `bash -c` pipelines of coreutils (`ls / | sort |
head -3`, `seq | grep | wc -l`, `seq | sed | tr`); `mkdir`, `cp`, `mv`,
`cat`, `wc`, `rm` and `ls` of a missing file (status 2) on the disk;
`env -i`, `date`, `tail`, `head`; `stat`, `ls -l`, `readlink
/proc/self/exe`, `ulimit -n`; 32 MiB through `dd`, `wc`, `cat` and
`sha256sum`; a background job, `jobs`, `kill %1` and `wait` (143);
Ctrl-C ending a foreground `sleep` (130); two Python scripts (arithmetic,
json, re, os, collections, file I/O on the disk, `subprocess` running
`sort` and a shell pipeline; then threads, a 48 MiB growing buffer, a
file mapping written, resized and flushed, shared memory across `fork`,
record locks and flock between processes, a lock waited for, SQLite,
temporary files, `shutil.copyfile`, hashlib against `sha256sum`, zlib, an
interval timer's signal, the terminal's size, resource limits and
children's CPU time, `os.cpu_count`, `statvfs`, `/proc/self/exe`, 24 MiB
written and read back, `posix_spawn`); gcc compiling `hello.c` onto the
disk and running it (output and exit status 42), and two files compiled
apart and linked with libm into a program that forks and runs `echo`
through a pipe; a file written and synced; `poweroff`. Every step's output
and exit status are compared exactly (expected values come from the
programs' documentation, not from running them on Linux), every failure is
reported together, and any `not implemented` line but `rseq` fails the
test. Then `e2fsck -fn` must pass on the image, the compiled program is
read back from it on the host with `poly-ext2`, and a second boot from
the same image must find the file and run the program. The whole test
takes about a minute.

## Consequences

- The roadmap line is done: bash, coreutils, grep, sed, Python 3 and gcc
  run unchanged from an ext4 disk, and what they write persists.
- New: `VmMap::remap`, `Error::Fault` (`EFAULT`) in `poly-vm`;
  `poly_vfs::locks`, `FdTable::close_range`; `Elf::parse_prefix`;
  `LinuxKernel::mremap`, `msync` and `CpuTimesOf::Reaped`;
  `LinuxFs::close_range`, `test_lock`, `set_lock`, `flock` (all with
  `ENOSYS` defaults); `sys::fadvise`, `sys::xattr`; `Files::lock_owner`;
  `kernel/src/fs/locks.rs`, `kernel/src/user/memory/remap.rs`.
- Host tests for every new call (`personalities/linux/src/tests/
  userland.rs`), the lock table, remapping and `parse_prefix`; kernel
  tests for the lock table's release on close and on the last description,
  and for moving own, shared and shrunk pages.
- Not done yet: extended attributes stored on ext4 and tmpfs,
  `MREMAP_DONTUNMAP`, deadlock detection for `F_SETLKW`, lock information
  in `/proc/locks`, read-ahead hints, `/proc/mounts` as seen from a
  chroot (it lists every mount with the system's paths), a root file
  system on disk without a chroot (`switch_root`, `MS_MOVE`), the
  `rusage` fields other than the two times, C++ (`cc1plus` and libstdc++
  headers are not in the image).
