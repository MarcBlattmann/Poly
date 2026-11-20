# 0246: The XNU BSD layer: files, memory, sysctl, signals and posix_spawn

**Status:** Accepted. Track `impl-macos-bsd`, Roadmap Phase 9 "BSD layer" (first part; the
milestone stays open). Builds on ADR 0237 (Mach-O loading and Poly's own dyld) and runs beside
ADR 0238 (the Mach core, which owns the Mach traps, `mprotect` and the other memory calls, ulock,
psynch and `bsdthread_create`). Inputs: the spec sheets `specs/macos/bsd/*.yaml` of batches 1 to 3
(open, sys_close, read, write, sys_fstat64, stat64, lstat64, fstatat64, getdirentries64,
sys_fcntl, sys_dup2, sys_chdir, pipe, mmap, munmap, sysctl, sys_sysctlbyname, getpid,
issetugid, csops, getentropy, sigaction, sigprocmask, kill, posix_spawn, wait4), the macOS SDK
headers for numbers and layouts (`<sys/syscall.h>`, `<sys/errno.h>`, `<sys/fcntl.h>`,
`<sys/stat.h>`, `<sys/dirent.h>`, `<sys/mman.h>`, `<sys/sysctl.h>`, `<sys/signal.h>`,
`<sys/wait.h>`), and the pread(2) and lseek(2) manual pages (no sheets yet). No XNU or Darwin
source was read.

## Decisions

1. **Where the code lives.** `personalities/xnu/src/bsd.rs` and `bsd/` decode the calls, check
   them as the sheets record, lay out Darwin's structures (`struct stat64`, the 64-bit
   `struct dirent` records, `struct __sigaction`, wait status words) and keep the sysctl table and
   the signal rules; it is host-tested against a fake kernel. The kernel's side is the
   `BsdKernel` implementation in `kernel/src/xnu.rs`, on the process's descriptor table, its
   `Memory` and the VFS. The two tables (this one and the Mach core's `crate::call`) are split by
   call (agreed with the Mach core's track): the kernel tries the BSD layer first, then the Mach
   core, then "not implemented".
2. **Errors** come back from the VFS and memory code as Linux numbers and are mapped to Darwin's
   (`bsd::errno::from_linux`: 1 to 34 are the same; `EAGAIN`, `ENAMETOOLONG`, `ELOOP` and the
   others differ).
3. **Files:** `open`/`openat` with Darwin's flag values, `close`, `read`/`pread` (blocking reads
   on pipes), `write`/`pwrite`, `lseek`, `stat64`/`lstat64`/`fstat64`/`fstatat64` (`AT_FDCWD` is
   -2), `getdirentries64` (records of at least 32 bytes, a 64-byte minimum buffer, `basep` 0 while
   records come and `0x7fffffff` at the end), `fcntl` (`F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`,
   `F_SETFD`, `F_GETFL`, `F_SETFL`, `F_GETPATH`; other commands `ENODEV`), `dup2`,
   `chdir`/`fchdir`, `pipe` (the write end in `rdx`). Device numbers are given in Darwin's
   encoding (major in the top byte) but are Poly's devices.
4. **Memory:** `mmap` (anonymous private and shared, regular files private and shared; length 0
   returns 0; an unaligned file offset returns an address that far into a page; devices are
   `ENODEV`) and `munmap`, on the process's `Memory` (the same `VmMap` the Mach core's
   `mprotect` works on). Pages are 4 KiB for x86-64 programs.
5. **sysctl and sysctlbyname:** a table of the values the sheets list (kern.ostype, osrelease,
   osrevision, version, osversion, osproductversion, hostname, boottime, argmax, maxproc,
   maxfiles, maxfilesperproc; hw.machine, ncpu, logicalcpu, physicalcpu, byteorder, pagesize
   (4 bytes by number, 8 by name), pagesize32, memsize, cachelinesize, tbfrequency; vm.loadavg;
   sysctl.proc_translated) and the name-to-numbers node {0, 3}. Poly answers as Darwin 27.0.0 /
   macOS 27.0 (the oracle's release). Values are read-only (`EPERM`). `KERN_PROC` and
   `KERN_PROCARGS2` are not there yet (`ENOENT`).
6. **Ids:** `getpid`, `getppid`, `getuid`/`geteuid`, `getgid`/`getegid`, `issetugid` (0),
   `thread_selfid`, `getentropy` (up to 256 bytes), and `csops` operation 0, which reports the
   flags of an ad hoc signed program (Poly checks no signature; other operations are `EINVAL`).
7. **Signals, first level:** `sigaction` stores and reads back actions (`SIGKILL`/`SIGSTOP`
   refused), `sigprocmask` keeps the mask (`SIGKILL`/`SIGSTOP` bits dropped), `sigpending`, and
   `kill` with the sheets' checks (the signal number first, `ESRCH`) and Darwin's default
   actions: terminating signals end the target as killed by that signal, ignored ones and
   `SIG_IGN` are dropped, blocked ones wait in the pending set. **Handlers are not called yet**:
   a caught signal stays pending (delivery through the trampoline and `sigreturn` is the next
   step); stopping and continuing are not done.
8. **posix_spawn and wait4:** `posix_spawn` with no attributes or file actions starts the program
   (Mach-O, or any format Poly runs) as a child that inherits the descriptors without
   close-on-exec, the mask and ignored signals; failures come back from the call. The attributes
   pointer is ignored (its layout is libSystem's, with no public header). `wait4` waits for one or
   any child, `WNOHANG`, the wait status word, `ECHILD`, `EFAULT` for an unwritable status before
   a child is reaped; `rusage` is zero.

## Tests

- `personalities/xnu/src/bsd/tests.rs` and the module tests: every call above against a fake
  kernel, with the sheets' error cases.
- Boot test group `macho`: `/bin/mbsd` (`crates/poly-dyld/tests/data/src/bsd.c`, built by
  `generate.sh` against our libSystem stand-in) opens and reads a file, lists a directory, maps
  memory and the file, asks sysctl, blocks and sends signals, and starts itself with
  `posix_spawn` as a child that writes through an inherited pipe and is reaped by `wait4`.

## Not yet

Signal handlers and `sigreturn`, stop/continue, `fork`/`vfork`/`execve`, `kqueue`/`kevent`,
`getattrlist`, `proc_info`, `ioctl`, extended attributes, `KERN_PROC`, posix_spawn attributes and
file actions, and the batch 4 sheets (PR #341).

## Addendum (2026-11-20): processes, names, attributes, and the spec tests on Poly

What the macOS integrator's first round (int-macos-1, PR #433) found missing or different.

1. **The spec tests run on Poly, both architectures.** Boot test group `macho-specs`
   (`tools/xtask/src/macho_specs.rs`, shard `userland`): every visible test of
   `specs/macos/{bsd,mach}` is built with clang (`-target <arch>-apple-macos13.0`; the raw test
   header needs no SDK) and rust-lld against Poly's stand-in libSystem, for x86-64 and for arm64
   (the translator, ADR 0257), into `target/`; without clang the group skips. Each runs as the
   program of a traced boot (`poly.test.capture`): its standard output comes from the kernel's
   `POLY-OUT` records, exactly, and is compared with the arm64 recording of `macos.json`
   (x86-64: the check lines and the exit code; `value` lines may depend on the architecture). It
   runs as an ordinary user (uid 501, gid 20) in an empty scratch directory of its own
   (`poly.test.cwd`, new: the test init makes the directory and gives it to that user), on 2 GiB,
   as the oracle ran. `personalities/xnu/tests/data/specs-passing.txt` is the ratchet: a listed
   run that fails fails the group.
2. **Processes.** `fork` (the memory copied on write, the descriptors shared, handlers and mask
   kept, nothing pending; the child returns its pid with 1 in the second register, and for arm64
   the translator now passes the second result back in `x1` for the calls that have one), `vfork`
   (no such call on macOS: the caller dies of `SIGSYS`), `execve` (the file's checks first:
   EACCES for a directory or a file without execute permission, then the loader's ENOEXEC; the
   new program replaces the memory and registers in place, close-on-exec descriptors close,
   handled signals reset), `waitid` (options first, the siginfo written before the child is
   reaped, `WNOWAIT`), process groups (`getpgrp`, `getpgid`, `setpgid`; `wait4` 0 and -pgid),
   `kill` of an unreaped zombie, `getppid` of a process without a parent is launchd's (1).
   `minherit(VM_INHERIT_NONE)` ranges are left out of a fork child. A process runs as a user and
   group of its own (`XnuState::ids`, inherited; the VFS's permission checks use them).
3. **Names and attributes.** `mkdir(at)`, `rmdir`, `unlink(at)`, `link(at)`, `symlink(at)`,
   `readlink(at)`, `rename`, `renameat`, `renameatx_np` (`RENAME_SWAP`, `RENAME_EXCL`; other bits
   `EINVAL`), `chmod`, `fchmod(at)`, `umask`, `mknod`, `truncate`, `ftruncate`, and the eight
   extended attribute calls: macOS names (UTF-8, 1 to 127 bytes) are the VFS's `user.<name>`;
   `ENOATTR` for a missing one; a size query; the resource fork read and written at a position;
   `com.apple.FinderInfo` exactly 32 bytes, absent when all zero.
4. **Descriptors.** `ioctl` (`FIONREAD`, `FIOCLEX`, `FIONCLEX`, `FIONBIO`; `TIOCGWINSZ` of what
   is no terminal), `fcntl`'s `F_FULLFSYNC`, `F_BARRIERFSYNC`, `F_NOCACHE`, `F_GLOBAL_NOCACHE`,
   `F_RDAHEAD`, `F_GETPATH_NOFIRMLINK`; a `read` whose buffer cannot be written takes nothing
   from the file.
5. **sysctl by architecture.** An arm64 program sees an arm64 Mac: `hw.pagesize` 16384,
   `hw.machine` arm64, `hw.tbfrequency` 24 MHz.

Still open: `kqueue` / `kevent`, signal handlers and `sigreturn` frames, stop and continue,
`getattrlist` and friends, `clonefileat`, `proc_info`, case-insensitive names (APFS),
attributes on symbolic links and values above 64 KiB (the VFS's limits), the `#!` path of
`execve`.
