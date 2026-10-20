# 0043: `/dev`, `/proc` and `/sys`: a ChaCha20 generator, procfs over a kernel-provided source, a static sysfs; the Linux file system calls behind one services trait

**Status:** Accepted. Builds on [ADR 0023](0023-vfs-tmpfs-and-cpio.md) and
[ADR 0037](0037-vfs-and-initramfs-in-the-kernel.md) (the VFS in the
kernel), [ADR 0025](0025-linux-syscall-table-and-kernel-services-trait.md)
(the `LinuxKernel` trait), [ADR 0040](0040-page-cache-and-ext2-on-virtio-blk.md)
(file mappings) and [ADR 0041](0041-processes-threads-fork-exec-wait.md)
(the process tree, fork and exec).

## Context

Phase 4 asks for "`/dev` (null, zero, random, urandom, tty, console),
`/proc` (self, maps, status, cpuinfo), minimal `/sys`", and programs can
only use them through file system calls the personality did not have:
`openat`, the `stat` family, `getdents64`, `readlinkat`, `fcntl`, ... Static
glibc programs got `ENOSYS` for `readlinkat` and `prlimit64` at startup.
Random bytes came from RDRAND or a mixed timestamp counter, not from a
cryptographic generator; `/dev/random` and `/dev/urandom` did not exist.

Sources: RFC 8439 (ChaCha20, its test vectors), D. J. Bernstein's
"Fast-key-erasure random-number generators" (2017), the manual pages
proc(5) and its per-file pages (`proc_pid_stat(5)`, `proc_pid_status(5)`,
`proc_pid_maps(5)`, `proc_pid_fd(5)`, `proc_pid_fdinfo(5)`,
`proc_pid_mountinfo(5)`, `proc_meminfo(5)`, `proc_stat(5)`, ...), sysfs(5),
random(4), fstab(5), the section 2 pages of every call below, the public
UAPI headers (`asm-generic/statfs.h`, `linux/stat.h`, `linux/fcntl.h`,
`asm-generic/resource.h`, `linux/sysinfo.h`, `linux/magic.h`,
`linux/mount.h`) and the Intel SDM / AMD APM CPUID tables. No Linux or C
library source was read and no Linux system was observed.

## Decision

**Randomness: `crates/poly-random`.** ChaCha20 exactly as RFC 8439
specifies it (tested against sections 2.1.1, 2.3.2, 2.4.2 and appendix
A.1) and `Csprng`, a fast-key-erasure generator: each request takes block
0 of the current key's keystream, keeps its first half as the next key and
hands out the rest and the following blocks (at most 4064 bytes per key),
so the state never reveals earlier output. Input is folded in as
`key = F(key) XOR chunk` 32 bytes at a time plus a length step, with `F`
a block under a nonce reserved for mixing; data chosen without knowing the
key (a write to `/dev/urandom`) cannot weaken it. The generator counts
credited entropy and is "ready" at 256 bits. The kernel (`random.rs`)
seeds it at first use from RDSEED and RDRAND where the CPU has them,
4096 samples of timestamp-counter jitter (`arch/x86_64/entropy.rs`) and
the clocks, crediting the jitter with 256 bits so that the generator is
always ready and `getrandom(2)` never blocks (on QEMU's default CPU, which
has neither instruction, that is a weaker guarantee; the boot log says
which sources were used). It reseeds from the same sources once a minute.
`getrandom`, `/dev/random` (1, 8: at most 512 bytes per read),
`/dev/urandom` (1, 9: at most 32 MiB) and `/proc/sys/kernel/random` use
it; writes to the devices are mixed in without credit (random(4)).
`AT_RANDOM` at exec still uses the old RDRAND/TSC source (in `exec`, not
touched here).

**`/dev`.** The nodes stay in the root file system, made at boot if the
initramfs did not bring them (ADR 0037): `console`, `tty`, `null`,
`zero`, `full`, `random`, `urandom`, the links `fd` -> `/proc/self/fd` and
`stdin`/`stdout`/`stderr` -> `/proc/self/fd/0`..`2` (proc_pid_fd(5)), and
a tmpfs on `/dev/shm` (mode 01777). A separate devtmpfs mount was not made:
it would hide device nodes an initramfs brings.

**procfs: `crates/poly-procfs`.** `ProcFs` is a `poly_vfs::FileSystem`
whose contents are made on every read from a `ProcSource`, a trait the
kernel implements (`kernel/src/fs/proc.rs`, `KernelProc`) and the crate's
tests fake. The crate formats (`format.rs`, one function per file, per its
manual page); the kernel only reports facts (`ProcessInfo`, `MapEntry`,
`FdInfo`, `MountEntry`, `SystemInfo`, `CpuInfo`, `KernelInfo`). Inode
numbers encode what a node is (pid in bits 32-63, kind in 24-31, index or
descriptor in 0-23), so nothing is stored per node; directory positions
are stable (fixed entries from 2, processes at 2^32 + pid, descriptors at
2 + fd). Files report size 0 and are read to the end; a read regenerates
the text, so a read in pieces of a file that changes meanwhile may tear
(Linux makes the text once per open; `FileSystem` has no per-open state).
Everything is read-only (`EROFS`). Tree:

- `/proc/self` (the caller's pid; `ENOENT` for kernel threads, so the
  shell's `cat /proc/mounts` fails as on Linux), `/proc/mounts` ->
  `self/mounts`;
- `/proc/<pid>/{stat,statm,status,maps,cmdline,environ,comm,exe,cwd,root,
  fd/,fdinfo/,mounts,mountinfo}`;
- `/proc/{cpuinfo,meminfo,uptime,loadavg,version,stat,filesystems}`;
- `/proc/sys/kernel/{ostype,osrelease,version,hostname,domainname,pid_max,
  random/{boot_id,uuid,entropy_avail,poolsize}}`, `/proc/sys/fs/nr_open`.

`exe`, `cwd`, `root` and `fd/<n>` are symbolic links whose text the
kernel computes; following them resolves that text like any link (unlike
Linux's "magic" links, a deleted file or a pipe cannot be reopened through
them). `cpuinfo`'s flags are the SDM's (leaves 01H, 07H) and the AMD
APM's (80000001H) mnemonics, lower-cased; a few may differ from Linux's
names, for the spec agent to confirm. Not tracked yet and reported as 0:
CPU time (all of it counts as idle in `/proc/stat` and `/proc/uptime`),
load averages, context switches, page faults.

**Naming open files.** `/proc/<pid>/fd` and `/proc/<pid>/maps` need a
file's path. The VFS now records, in each `OpenFile`, the directory (a
weak mount reference and inode) and name `open` found it by
(`Walk::resolve_named`), and `Vfs::file_path` builds the path from the
directory's (walking `..`) and that name; if the name no longer leads to
the file it looks for the file's new name in that directory, appends
`" (deleted)"` (proc(5)) when the file has no links left, and otherwise
keeps the old path. `FileSource` (file mappings and exec) keeps its
`OpenFile`, and `PageSource` has `file()` and `label()` (for `[vdso]`-like
mappings), so `maps` names every mapping. `OpenFile::offset_now` reads
the offset without waiting: reading `fdinfo` of the very description being
read reports 0 instead of deadlocking on the offset lock.

**Per-process facts.** The kernel keeps one record per program it starts
(`fs::proc::record_exec`, called by `program::prepare` and `execve`; fork
copies the parent's, `record_fork`): the executable (weakly, and its path),
`comm`, the memory (weakly), where the argument and environment strings
lie, the start time and stack. `cmdline` and `environ` are read from the
process's memory, as proc_pid_cmdline(5) says. A process's memory map is
its record's, or the caller's own. Parent, group and session come from
the process tree (ADR 0041); zombies are found through it too.

**Locks.** procfs calls back into the kernel during path walks
(`/proc/self/cwd` needs the caller's working directory), so `Files` no
longer holds its context lock across a walk: `at()`, `chdir` and `getcwd`
work on a copy of the context (two `NodeRef` clones), and procfs asks for
a process's owner and start time (`ProcSource::owner`) instead of
anything that takes the context lock; the umask in `status` is read with
`try_lock`. Nothing else in the VFS changed: file systems are still called
without VFS locks except the offset lock.

**sysfs** (`kernel/src/fs/sysfs.rs`) is a `poly_procfs::StaticFs`, a
read-only tree built once whose files a closure fills: `devices/system/
cpu/{online,possible,present}` ("0": one CPU until SMP), `devices/
virtual/{mem,tty}/<name>/dev` ("major:minor"), `dev/char/<maj>:<min>` and
`class/{mem,tty}/<name>` links (sysfs(5)'s example), and empty `block`,
`bus`, `firmware`, `fs`, `kernel`, `module`, `power`. procfs, sysfs and
`/dev/shm` are mounted by `fs::init` (`nosuid,nodev`, procfs and sysfs
`noexec`).

**The Linux file system calls.** One method on `LinuxKernel`,
`fs() -> Option<&mut dyn LinuxFs>` (`None` by default: `ENOSYS`), reaches
a second trait, `LinuxFs` (`personalities/linux/src/fs.rs`), with the
operations on kernel buffers; `system_info()` serves sysinfo(2). This
keeps the shared trait small and the new code in new files. The handlers
(`sys/path.rs`, `sys/fd.rs`, `sys/limits.rs`, registered in a second list,
`table::FILE_SYSTEM`) check flags first (`EINVAL`), copy paths (at most
`PATH_MAX`, `ENAMETOOLONG`; empty paths `ENOENT` unless `AT_EMPTY_PATH`),
call the kernel, and encode `struct stat`, `struct statx`,
`struct statfs`, `linux_dirent64` / `linux_dirent` and `struct sysinfo`.
The kernel side (`kernel/src/linux/files.rs`) maps them onto
`fs::Files` (whose new methods are in `fs/files/more.rs`). Calls:
`open`, `openat`, `creat`, `stat`, `lstat`, `newfstatat`, `statx`,
`readlink`, `readlinkat`, `access`, `faccessat`, `faccessat2`, `getcwd`,
`chdir`, `fchdir`, `lseek`, `pread64`, `pwrite64`, `dup`, `dup2`,
`dup3`, `fcntl` (`F_DUPFD`, `F_DUPFD_CLOEXEC`, `F_GETFD`, `F_SETFD`,
`F_GETFL`, `F_SETFL`; locks, owners, leases, pipe sizes and seals are
`EINVAL` for now), `getdents`, `getdents64`, `mkdir`, `mkdirat`,
`unlink`, `unlinkat`, `rmdir`, `rename`, `renameat`, `renameat2`,
`symlink`, `symlinkat`, `link`, `linkat`, `chmod`, `fchmod`, `fchmodat`,
`truncate`, `ftruncate`, `fsync`, `fdatasync` (the same: file systems sync
whole, ADR 0040), `sync`, `syncfs`, `statfs`, `fstatfs` (`FileSystem` has
a `statfs` default method; tmpfs, ext2, the page cache, procfs and sysfs
report theirs, magic numbers from `linux/magic.h`), `umask`, `prlimit64`,
`getrlimit`, `setrlimit`, `sysinfo`. Permission checks are the VFS's
(everyone is root until users arrive; `AT_EACCESS` changes nothing yet).

**Resource limits** live in the personality: `LinuxProcess::limits`
(`rlimit.rs`), inherited by fork and kept by exec. Defaults are Poly's
choice (the manual pages give none): `RLIMIT_NOFILE` 1024 soft / 4096
hard, `RLIMIT_STACK` 8 MiB / unlimited, `RLIMIT_CORE` 0 / unlimited,
`RLIMIT_MEMLOCK` 8 MiB, `RLIMIT_NPROC` and `RLIMIT_SIGPENDING` 4096,
`RLIMIT_MSGQUEUE` 819200 (mq_overview(7)), nice and real-time priority 0,
the rest unlimited. Only `RLIMIT_NOFILE` is enforced (the descriptor
table's limit, `LinuxFs::set_fd_limit`; `EPERM` above `nr_open`,
1048576) and `RLIMIT_SIGPENDING` sets the signal queue limit; raising a
hard limit needs root.

**`rseq` stays `ENOSYS`.** Registering and never aborting would be wrong
whenever a thread is preempted inside a critical section, which the
manual's contract forbids; C libraries treat `ENOSYS` as "no rseq".

## Consequences

- A static glibc program's startup no longer gets `ENOSYS` but for `rseq`;
  `cat`, `ls`, `stat` and the like have the calls they need; programs can
  read `/proc/self/maps` (Rust's std uses it for the stack guard),
  `/proc/self/exe`, `/proc/cpuinfo`, `/proc/meminfo` and `/dev/urandom`.
- Host tests: RFC 8439 vectors and the generator's construction
  (`poly-random`); every procfs and sysfs file and directory through a
  real VFS over a fake source (`poly-procfs`); `file_path`, `offset_now`
  and `statfs` (`poly-vfs`); every new call against a `LinuxFs` over a real
  tmpfs (`personalities/linux/src/tests/files.rs`). Kernel tests: ring-3
  processes reading `/proc/self/{maps,status,stat,exe,cmdline,environ,fd,
  fdinfo}`, `/proc/cpuinfo`, `/proc/mounts`, `/sys`, `/dev/urandom`,
  `prlimit64` enforcing `RLIMIT_NOFILE`, the `*at` calls; and the kernel's
  own view (`fs::proc_tests`).
- `OpenFile` is a little larger (the name it was opened by). A file
  renamed into another directory keeps its old path in `/proc`.
- Not done yet: writable `/proc` files (`comm`, `sys/kernel/hostname`),
  `/proc/<pid>/task`, `smaps`, `limits`, `auxv`, `/proc/thread-self`,
  per-process CPU time and load averages, `utimensat`, `chown`, `mknod`,
  file locks, `O_TMPFILE`, `ioctl(FIOCLEX)`, `preadv`/`pwritev`.
