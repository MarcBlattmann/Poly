# 0064: Test runs as a tracer sees them (call entries, threads, a kernel init, output by open file); tmpfs behind the page cache; the behaviours of spec-a's entries

**Status:** Accepted. Amends the test-run protocol of
[0027](0027-agent-pipeline-tools-and-test-run-protocol.md) and its kernel side in
[0038](0038-linux-processes-in-the-kernel-and-the-first-program.md) (additive: old
records still parse), [0062](0062-signals-seen-as-a-tracer-sees-them-fifos-loopback-and-shared-futexes.md)
(signal records); builds on [0040](0040-page-cache-and-ext2-on-virtio-blk.md) (the page cache),
[0041](0041-processes-threads-fork-exec-wait.md), [0043](0043-dev-proc-sys-and-file-system-calls.md)
and [0044](0044-time-syscalls-timers-and-the-vdso.md).

## Context

The runner's diff of spec-a's 61 entries (visible tests, and counts of held-out
tests) showed two kinds of failure. Most were in how a Poly test run is *recorded*,
not in what the program saw: the parent's `clone`/`fork`/`vfork` record came out
when the parent returned, after the child's calls, while a ptrace trace has the
call at its entry; calls of threads came out under the process's pid, while a
tracer sees each thread as a task; the program was pid 1 with `getppid() == 0`,
while under the tracer it has a parent; output was taken by descriptor number, so
writes to fd 1 after `dup2` put a file there still counted as standard output, and
writes through a copy of fd 1 did not. The rest were behaviours: missing calls
(`mlock`, `munlock`, `mincore`, `utimes`), `ENODEV` for `MAP_SHARED` of a tmpfs
file, `EFAULT` where Linux returns a short count, clock ids, open flags and a
dozen single points. Sources: the spec sheets and visible tests in
`specs/linux/x86_64/`, the man pages (open(2), fcntl(2), linkat(2), chown(2),
mmap(2), mremap(2), mlock(2), mincore(2), madvise(2), read(2), write(2),
readv(2), getrlimit(2), getrusage(2), clock_getres(2), clock_nanosleep(2),
timer_create(2), timer_delete(2), setitimer(2), utimes(2), ptrace(2), tmpfs(5))
and the UAPI headers for numbers. No Linux or C library source was read and no
Linux system was run or traced.

## Decision

**Test-run records (kernel `testrun.rs`, parser `tools/pipeline/src/poly.rs`).**

- `POLY-SYSCALL` and `POLY-SIGNAL` carry `tid=` (the thread); the parser numbers
  tasks by `tid` (else `pid`), in creation order as before (thread ids come from
  the pid counter), so a thread is a task of its own, as for the tracer.
- `POLY-ENTER: pid= tid= nr= args=` is written for `clone`, `fork`, `vfork` and
  `clone3` at their entry, before the new task exists; the parser puts the call
  there and the task's next `POLY-SYSCALL` of that number completes it. Other calls
  keep one record at their return (writing two records per call would double the
  serial traffic for nothing: only calls that start tasks reorder the trace).
- `POLY-SIGNAL` for `SIGKILL` makes no event in the parser (a tracer never sees
  it; ADR 0062 stopped the kernel writing it too).
- **A kernel init.** With `poly.test.capture` the kernel makes pid 1 an init
  without threads of its own (`testinit.rs`): a root of the process tree whose
  work the booting kernel thread does. The program becomes its child (exit
  signal `SIGCHLD`, init's session) in a process group of its own, as under the
  tracer, which starts the program in its own group. The init reaps every child it
  gets, the program and the orphans. It is not a process record (`kill(1, ...)`
  finds nothing). Untraced `poly.init` runs (BusyBox as init, the boot tests) are
  unchanged: the program is pid 1.
- **Output by open file.** A traced process starts with fd 0 on the log console
  (reads end of file) and fds 1 and 2 on two write-only capture files whose bytes
  are written to the console and reported as `POLY-OUT fd=1` / `fd=2`
  (`tty::log_console_files`), wherever they are duplicated, as with the two pipes
  of a traced program on Linux. The descriptor-number capture in `write` and
  `pwrite64` is gone. App tests send their output into regular files
  (`/poly-app/stdout`, `/poly-app/stderr`); the pipeline now passes
  `poly.test.stdout=` / `poly.test.stderr=` for them and the kernel reports those
  files' contents when the program has ended, which is what Linux's run reports.

**tmpfs behind the page cache (`fs/memfs.rs`).** The root, `/dev/shm` and mounted
tmpfs instances are mounted as the page cache's `CachedFs` over a `Tmpfs`, like a
disk file system, so reads, writes and mappings share the cache's pages and
`MAP_SHARED` of a tmpfs file works (tmpfs(5)); the writeback thread writes dirty
pages back into the tmpfs, after which they can be evicted. The initramfs is
unpacked straight into the tmpfs before it is wrapped (no second copy while
unpacking). Each instance gets tmpfs(5)'s default limits, half of RAM for data and
half of RAM's pages as inodes, so `statfs` reports fixed totals with free counts
within them (`Tmpfs::with_limits`). The root tmpfs takes its times from the wall
clock, so new files have current ctimes and mtimes. Writing back through a second
copy costs memory a native page-cache tmpfs would not; it was the change that
needed no new mapping code.

**Behaviours** (each as its spec sheet describes, host-tested in
`personalities/linux/src/tests/spec_a.rs` and the crates' tests):

- `mlock`, `mlock2`, `munlock`, `mlockall`, `munlockall` (`sys/mlock.rs`): Poly
  never pages out, so locking brings the range in (`Memory::populate`) and records
  nothing; `ENOMEM` for unmapped or (to lock) `PROT_NONE` pages, `EINVAL` for a
  range that wraps. `mincore` (`Memory::residency`): resident if the page has a
  frame, or is a page of a file within the file whose source keeps the page cache.
- `utimes`, `futimesat`, `utime` (`sys/utimes.rs`) through `utimensat`'s service.
- Reads and writes copy page by page when a whole copy fails: `read`/`readv`
  return the bytes delivered and move a file's offset back over the rest (`EFAULT`
  and the offset unchanged when nothing was delivered); `write`/`writev` write what
  could be read before the bad page.
- Clock ids (`time/cpuclock.rs`): negative ids name CPU-time clocks of processes
  and threads (`(~pid << 3) | type`, a thread only of the caller's process; a new
  `CpuTimesOf::OtherProcess`); `CLOCK_TAI` (offset 0) and the `*_ALARM` clocks read
  and sleep as the wall clock and boottime; `clock_nanosleep` sleeps on process CPU
  clocks (looking once a tick) and answers `EOPNOTSUPP` for the thread CPU, raw and
  coarse clocks; `timer_create` the same for raw and coarse clocks.
- Open flags: every x86-64 open is a large-file open (`O_LARGEFILE` in `F_GETFL`
  but for `O_PATH`); `O_DIRECTORY` and `O_NOFOLLOW` are kept and reported;
  `F_SETFL` on `O_PATH` is `EBADF`; access mode 3 opens for neither reading nor
  writing but needs both permissions; `O_TMPFILE` makes an unnamed tmpfs file
  (`FileSystem::create_tmpfile`) that `linkat(..., AT_EMPTY_PATH)` can name
  (`Vfs::link_node`); a file that lost its names cannot get one back.
- Every successful `chown` clears the set-id bits (even with `-1`, `-1`); `fchmod`
  of an `O_PATH` descriptor is `EBADF`; a write of 0 bytes changes nothing.
- `RLIMIT_FSIZE` (`sys/fsize.rs`): writes are cut at the limit, writes at or past
  it fail with `EFBIG` and raise `SIGXFSZ`, truncations past it are `EFBIG`.
- `MADV_WIPEONFORK` / `MADV_KEEPONFORK`: a flag on private anonymous regions
  (`Vma::wipe_on_fork`, regions with different flags do not merge); `Memory::fork`
  does not share such regions' pages, so the child's copy starts zero-filled.
- `mremap` with `old_size` 0 and `MREMAP_MAYMOVE`: a second mapping of a shared
  mapping's pages (`EINVAL` for a private one).
- `setitimer` with a null new value disarms; `timer_delete` discards the timer's
  queued signal; POSIX timer ids go up and a deleted id is not given out again at
  once; `getrusage` reports `ru_maxrss` (the peak of resident pages seen at faults
  and calls) and `ru_minflt` (resolved page faults).

## Consequences

- Traced programs are pid 2; diff traces are unaffected (tasks are numbered),
  but serial logs and debugging notes that assumed pid 1 for a test run change.
- Timing-dependent orders the reference itself would have (a parent racing its
  child without `CLONE_VFORK`) remain; the tests avoid them.
- tmpfs data lives in the tmpfs and, for pages in use, in the cache: up to twice
  the memory for recently used files, bounded by the cache's limit for clean pages.
- Not done: Poly's monotonic clock starts at boot and a test run starts well under
  a second after it, so an absolute timer "in the past" at 1 s monotonic is still
  in the future on Poly (`timer_settime`'s visible test); a children's `ru_maxrss`;
  `O_TMPFILE` on ext2; `mlockall(MCL_CURRENT)` populating everything.
