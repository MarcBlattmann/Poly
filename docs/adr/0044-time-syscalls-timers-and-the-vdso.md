# 0044: Time system calls, process timers, CPU-time accounting and a vDSO built from our own Rust

**Status:** Accepted. Builds on [0013](0013-interrupt-controllers-and-time-base.md)
(the clock and its TSC conversion), [0016](0016-interrupt-dispatch-tick-and-kernel-locks.md)
(the tick, sleeping), [0024](0024-elf-loader.md) (`AT_SYSINFO_EHDR`),
[0025](0025-linux-syscall-table-and-kernel-services-trait.md) and
[0031](0031-linux-signals-and-futexes.md) (the personality, signals,
restarts), [0036](0036-demand-paging-and-exec.md) and
[0040](0040-page-cache-and-ext2-on-virtio-blk.md) (memory maps, page-cache
pages in mappings) and [0038](0038-linux-processes-in-the-kernel-and-the-first-program.md).

## Context

Phase 4 needs the time calls every C library and runtime makes:
`clock_gettime` for every clock (CPU-time clocks included), `clock_getres`,
`clock_settime` / `settimeofday`, `gettimeofday`, `time`, `nanosleep` and
`clock_nanosleep` (with `TIMER_ABSTIME` and restarts after a stop),
`times`, `getrusage`, `alarm`, `setitimer` / `getitimer` and POSIX timers.
Programs call `clock_gettime` constantly, so Linux answers it in user mode
from a vDSO (vdso(7)): an ELF shared object mapped into every process and
announced with `AT_SYSINFO_EHDR`, exporting `__vdso_clock_gettime`,
`__vdso_gettimeofday`, `__vdso_time`, `__vdso_clock_getres` and
`__vdso_getcpu` in version `LINUX_2.6`; glibc and musl look for it and fall
back to the system call when it is missing.

The kernel had a monotonic clock (TSC when invariant, else HPET), a wall
clock read once from the RTC, a 1 kHz tick and `thread::sleep`, but no CPU
time per thread, no way to set the wall clock, and nothing that fires a
signal at a time in the future.

Inputs allowed by `CLEAN_ROOM.md`: the man pages (vdso(7), time(7),
clock_gettime(2), clock_nanosleep(2), nanosleep(2), gettimeofday(2),
time(2), times(2), getrusage(2), alarm(2), setitimer(2), timer_create(2),
timer_settime(2), timer_getoverrun(2), timer_delete(2), getcpu(2),
signal(7), restart_syscall(2), fork(2), execve(2)), the ELF gABI and x86-64
psABI, the Intel SDM, and the UAPI headers (`linux/time.h`,
`linux/time_types.h`, `linux/times.h`, `linux/resource.h`,
`asm-generic/siginfo.h`, `asm/auxvec.h`). No Linux or C library source was
read and no Linux system (or its vDSO) was observed.

## Decision

**The vDSO is our own Rust, built with the kernel.** `kernel/build.rs`
compiles `crates/poly-vdso` and `kernel/src/arch/x86_64/vdso/vdso.rs` with
the pinned `rustc` for `x86_64-unknown-none` (relocation model `pie` and
fat LTO, so every call is direct and nothing needs the GOT) and links them
with `rust-lld` as a shared object: our linker script puts the ELF header,
the dynamic tables (`DT_HASH` and `DT_GNU_HASH`, `.dynsym`, version
definitions), read-only data, unwind tables and code in one read-execute
`PT_LOAD` at address 0, our version script exports the five symbols in
`LINUX_2.6`, the soname is `linux-vdso.so.1` (vdso(7)). The build script
refuses an image with more than one `PT_LOAD`, a writable one, or any
relocation table: nothing relocates a vDSO. The kernel embeds the image
(`arch::vdso::IMAGE`, about 3.5 KiB). Hand-written assembly or a byte-level
ELF writer were the alternatives; they would duplicate the time arithmetic
the kernel tests on the host.

**The data page is a seqlock in a host-tested crate.** `poly-vdso` holds
`Vvar` (`#[repr(C)]`, all atomics: a sequence number, the mode, the TSC
conversion `origin`, `mult`, `shift`, `offset_ns`, the wall clock's offset
from the monotonic clock, the coarse clocks' resolution and the time zone)
and the arithmetic both sides use. The kernel is the only writer
(`Vvar::publish`, under a spinlock with interrupts off, so a reader on the
same CPU never waits for a preempted writer); readers retry while the
number is odd or changed (`Vvar::try_read`, Boehm's fence pattern). The
conversion is the formula `poly_irq::MonotonicClock` applies
(`offset + ((tsc - origin) * mult) >> shift`), read out with
`MonotonicClock::tsc_conversion`, so the vDSO and the system call agree to
the nanosecond. The layout is Poly's own (kernel and vDSO are built
together); clock ids are Linux's.

**The vDSO reads the TSC only when the kernel does.** In `Mode::Tsc` the
vDSO computes `CLOCK_REALTIME`, `CLOCK_MONOTONIC` (and `_RAW`,
`CLOCK_BOOTTIME`: Poly never slews and never suspends) and the coarse
clocks (which read the same, with the tick as their resolution) with
`lfence; rdtsc`. When the clock reads the HPET (no invariant TSC: QEMU's
TCG) the kernel publishes `Mode::Syscall` and every vDSO function makes
the system call; so do unknown and CPU-time clocks and null pointers (the
call returns `EFAULT`). `__vdso_getcpu` answers CPU 0, node 0 (one CPU).
Tests can force the TSC path with the calibrated rate
(`vdso::force_tsc_for_tests`), so it is exercised under TCG too.

**Mapping: the page cache shares the frame.** The data page must be the
same frame in every process. Memory maps already share kernel frames
through page-cache pages (ADR 0040), so the data page is a one-page file of
a private tmpfs registered with the page cache; `vdso.rs` holds its page
forever (the cache never evicts a referenced page) and writes it through
the direct map. At exec (`exec::load`) the image is mapped as a private
file mapping (read-execute, ceiling read-execute) wherever the layout puts
mappings, and the data page right below it as a shared one (read-only,
ceiling read-only: `mprotect` cannot make it writable), and `AT_SYSINFO_EHDR`
points at the image. The vDSO finds the page at `__ehdr_start - 4096`. A
new region kind in `poly-vm` was the alternative; fork and `/proc/<pid>/maps`
then would have had to learn it.

**A settable wall clock.** `time::realtime_ns()` is the RTC's time at boot
plus the monotonic clock, as before, plus an offset that `clock_settime` /
`settimeofday` move (`time::set_realtime_ns`, refusing a time below the
monotonic clock as clock_settime(2) documents); every change is published
to the vDSO. The time zone `settimeofday` stores lives in the data page
too. Effective uid 0 stands in for `CAP_SYS_TIME`.

**CPU time per thread in the scheduler.** `kernel/src/cputime.rs` measures
how long each thread-table slot ran with the monotonic clock at every
context switch, and splits it into user and system time in the ratio of
tick samples (the tick records whether it interrupted ring 3). A slot
knows its user thread's process and thread id; when a thread is reaped its
time moves to its `Process` (`ProcessCpu`), which also has room for
waited-for children (`add_child`, for `wait4`). `CLOCK_THREAD_CPUTIME_ID`,
`CLOCK_PROCESS_CPUTIME_ID`, `times` and `getrusage` read it through
`LinuxKernel::cpu_times`.

**Timers: state in the personality, waiting in one kernel thread.** A
process's timers (`time::timers::Timers` in `LinuxProcess`: the three
interval timers and the POSIX timers by id) are plain data the personality
arms and reads; it tells the kernel the earliest monotonic expiry and
whether any timer counts CPU time (`LinuxKernel::timers_changed`). The
kernel keeps one table of those wake-ups by pid and one thread,
`linux-timers`, that sleeps until the earliest (one tick at a time while
CPU-time timers exist) and then calls `time::fire` on the process's state
with the clocks' values: due timers queue their signals through the normal
generation rules (`LinuxProcess::send`), periodic ones are re-armed with
the expirations they missed counted, and the threads to notify are woken
as for any signal. Both the system calls and the thread store the wake-up
with the process's state locked, so neither stores a stale one. Interval
timers send `SIGALRM`, `SIGVTALRM` (user time) and `SIGPROF` (user and
system time) with `si_code` `SI_KERNEL`; POSIX timers send `sigev_signo`
with `SI_TIMER`, the timer id, the overrun count and `sigev_value`
(`_timer` in `asm-generic/siginfo.h`), to the process or, with
`SIGEV_THREAD_ID`, to one thread; `SIGEV_NONE` sends nothing. A timer's
signal is queued once: expirations while it is pending raise the overrun
count in the queued siginfo. fork(2) gives the child no timers; execve(2)
deletes POSIX timers and keeps interval timers.

**Sleeps.** `nanosleep` and relative `clock_nanosleep` block until a
monotonic deadline (a relative sleep is not affected by setting the wall
clock); absolute `clock_nanosleep` on `CLOCK_REALTIME` blocks until the
wall clock's deadline as it is set at the time, and sleeps on if the clock
was set back meanwhile. Interrupted by a handler they fail with `EINTR`
(signal(7): never restarted after a handler) and a relative one writes the
time left; after a stop a relative one continues through
restart_syscall(2) (`RestartBlock::Sleep`, the absolute deadline it had)
and an absolute one simply runs again.

**Handlers in their own list.** The time calls are registered in
`sys::time::CALLS`, which `table.rs` merges with `IMPLEMENTED` at compile
time, so subsystems added in parallel do not edit the same lines.

**Points the documentation leaves open are Poly's choice, marked "to be
confirmed by the diff runner":** the order of error checks, `alarm`'s
rounding of the time left (to the nearest second, at least 1 while
pending), a null `new_value` in `setitimer` (`EFAULT`), `SI_KERNEL` for
interval timers, the `settimeofday` time-zone range (±15 hours),
`ENOTSUP` for sleeping on CPU-time and raw/coarse clocks, a due but not
yet fired timer reporting 1 ns left, the 4096-timer limit.

## Consequences

- A static program's `clock_gettime` costs no system call wherever the
  TSC is invariant (KVM, hardware); under TCG it costs one, as before.
  glibc's dynamic loader will find the vDSO through `AT_SYSINFO_EHDR` once
  dynamic linking lands.
- Every Linux process maps two more regions (two pages of address space,
  the image's page private and copied on demand, the data page shared). A
  debugger's breakpoint in the vDSO copies its page; `mprotect` cannot
  open the data page for writing.
- `kernel/build.rs` now runs `rustc` twice for the vDSO; it needs nothing
  beyond the pinned toolchain (its `rust-lld`).
- Setting the wall clock does not move armed `CLOCK_REALTIME` timers
  (they were converted to the monotonic clock when armed) and does not
  wake absolute wall-clock sleeps early; Linux does both. Wait queues
  that need it can be added when a program depends on it.
- The CPU-time split is sampled: a thread that ran less than a tick shows
  all of its time as system time. `getrusage` fills only the two times;
  the counters stay 0. The children's times wait for `wait4`.
- While any process has a CPU-time timer armed, the timer thread runs once
  per tick. SMP needs per-CPU data in the vDSO (RDPID or `TSC_AUX` for
  `getcpu`, a check that the TSCs agree) and per-CPU accounting.
