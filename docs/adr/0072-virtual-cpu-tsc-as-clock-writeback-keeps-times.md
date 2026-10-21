# 0072: A virtual CPU's TSC is the clock (so the vDSO answers under QEMU); page-cache writeback keeps file times; removed directories

**Status:** Accepted. Amends [0013](0013-interrupt-controllers-and-time-base.md) (which
counter the clock reads), [0044](0044-time-syscalls-timers-and-the-vdso.md) (when the
vDSO reads the clock itself) and [0064](0064-test-runs-as-a-tracer-sees-them-and-spec-a-behaviours.md)
(tmpfs behind the page cache).

## Context

App spec cases (`specs/linux/apps/`) compare the system calls of unchanged programs on
Linux and on Poly. Three of them diverged in ways the programs' own calls did not
explain:

- `sed-i` and `ls-ld`: on Poly sed made a real `clock_gettime(CLOCK_MONOTONIC)` and ls
  a `clock_gettime(CLOCK_REALTIME)`; on Linux the C library gets both from the vDSO,
  without a system call. Poly's vDSO serves these clocks, but only while the kernel's
  clock reads an invariant TSC (ADR 0044). QEMU's CPU models do not report the
  invariant bit (CPUID.80000007H:EDX[8]), so under QEMU the clock read the HPET and the
  vDSO published "use the system call".
- `cp-p`: `cp -p` copies, then sets the copy's times with `utimensat(fd, NULL,
  times)`, which returned 0, yet `ls -l` showed the current time. tmpfs sits behind the
  page cache (ADR 0064): a write puts the data in the cache and sets the times; the
  writeback thread later writes the pages into the tmpfs with its ordinary `write`,
  which set the modification and change times again, after the program's utimensat.
  ext2 behind the cache did the same.

The runner's fourth pass also left `getdents64` and `symlinkat` red on held-out tests.
Their spec sheets say a directory removed while open lists as `ENOENT`, and a
trailing slash after a file name is "a non-directory used as a directory" (`ENOTDIR`);
Poly listed nothing (0) and answered `EEXIST`.

Sources: the spec sheets and visible tests in `specs/linux/x86_64/` and
`specs/linux/apps/`, the man pages vdso(7), clock_gettime(2), utimensat(2),
getdents64(2), symlink(2), the Intel SDM (CPUID leaf 1 ECX bit 31, leaf 80000007H) and
the UAPI headers for numbers. No Linux or C library source was read and no Linux
system was run or traced.

## Decision

**A virtual CPU's TSC runs at a constant rate.** `CpuFeatures` decodes the hypervisor
bit (CPUID.01H:ECX[31], 0 on real processors) and `constant_rate_tsc()` is "invariant
or virtual". The clock reads a calibrated TSC whenever it runs at a constant rate
(`MonotonicClock::new` gets `constant_rate_tsc()` instead of `invariant_tsc`). A
hypervisor gives its guests a TSC that does not follow the host's power states (under
QEMU, Poly's boot calibrates it against the HPET within a few tens of ppm, and the
kernel test `a_tsc_clock_agrees_with_the_reference` checks the rate again later). Real
CPUs without the invariant bit keep the HPET (or PM timer), as before. Because the
vDSO reads the TSC exactly when the clock does (ADR 0044), under QEMU it now answers
`CLOCK_REALTIME`, `CLOCK_MONOTONIC` (and `_RAW`, `BOOTTIME`, the coarse clocks),
`gettimeofday` and `time` without a system call, agreeing with the kernel to the
nanosecond; the data page is published before the first program runs. `CLOCK_TAI`
joins them (Poly's TAI offset is 0, so it reads as the wall clock, as the system
call does). The CPU-time and alarm clocks still go to the system call.

**Writeback keeps the times.** `FileSystem::write_back` writes data a page cache
holds back to the file without touching its times (the default is `write`); tmpfs and
ext2 implement it, and the cache's writeback uses it. The cache already sets the
modification and change times when the program writes (`CachedFs::write`), as Linux
does at write time, so a later utimensat survives the writeback.

**Removed directories.** tmpfs and ext2 answer a listing of a directory removed while
open with `ENOENT` (from any position; a directory removed in the middle of one call's
listing ends that call with what was listed). `symlink` onto an existing name with a
trailing slash resolves the name: a non-directory (or a link to one) is `ENOTDIR`,
anything else `EEXIST`.

## Consequences

- Under QEMU the clock is the TSC: reading the time costs an `rdtsc` instead of an
  emulated HPET access, in the kernel and in user mode. Boot logs say
  `time: clock source TSC (a virtual CPU's TSC: constant rate)` and `vdso: ...
  clock_gettime reads the TSC`.
- A hypervisor that does not keep its TSC at one rate (none we run under) would make
  the clock drift; `MonotonicClock::fall_back_to_reference` remains the way back.
- With several virtual CPUs the vDSO relies on their TSCs agreeing, as it does on real
  hardware with an invariant TSC (ADR 0044).
- File systems with times of their own that sit behind the page cache must implement
  `write_back`; the default would let the writeback reset the times.
