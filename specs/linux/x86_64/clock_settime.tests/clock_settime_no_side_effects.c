/* Failed clock_settime calls change nothing: after a run of EPERM, EINVAL,
 * ENODEV and EFAULT results on every kind of clock id, CLOCK_REALTIME
 * still keeps its offset to CLOCK_MONOTONIC, CLOCK_MONOTONIC keeps
 * running forward, the CPU-time clock keeps counting, and the caller's tp
 * buffers hold what was put there. A forked child gets the same results.
 */
#include "a_sys.h"

#define SYS_capget 125
#define SYS_capset 126

/* First tv_sec that CLOCK_REALTIME refuses: 2^63 ns in seconds, less 30
 * years of 365 days (observed boundary on x86-64). */
#define CS_SEC_LIMIT 8277292036LL

static A_UNUSED i64 cs(i64 clock, const void *tp) { return syscall2(SYS_clock_settime, clock, tp); }

/* CPU-time clock id of a process (type 0..2) or thread (type 4..6). */
static A_UNUSED i64 cpu_clock(i64 pid, int type) { return (i64)(i32)(u32)((~(u64)pid << 3) | (u64)type); }

/* Clock id of an open file descriptor ("dynamic" clock). */
static A_UNUSED i64 fd_clock(i64 fd) { return (i64)(i32)(u32)((~(u64)fd << 3) | 3); }

static A_UNUSED void fill(void *p, u8 v, size_t n) {
    u8 *b = p;
    for (size_t i = 0; i < n; i++) b[i] = v;
}

/* Empty all three capability sets (any user may) and confirm the
 * effective set is empty: root then fails the privilege check too, and
 * can never change the clock by accident. Returns 0 when that worked. */
static A_UNUSED int drop_caps(void) {
    struct { u32 version; i32 pid; } h = { 0x20080522, 0 };
    struct { u32 effective, permitted, inheritable; } d[2];
    fill(d, 0, sizeof d);
    if (syscall2(SYS_capset, &h, d) != 0) return -1;
    fill(d, 0xff, sizeof d);
    if (syscall2(SYS_capget, &h, d) != 0) return -1;
    return (d[0].effective | d[1].effective) == 0 ? 0 : -1;
}

/* Two pages, the second inaccessible: returns the start of the first. */
static A_UNUSED u8 *edge_pages(void) {
    u8 *p = (u8 *)syscall6(SYS_mmap, 0, 2 * A_PAGE, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    syscall3(SYS_mprotect, (i64)(p + A_PAGE), A_PAGE, PROT_NONE);
    return p;
}

/* CLOCK_REALTIME minus CLOCK_MONOTONIC, in microseconds. */
static A_UNUSED i64 real_minus_mono_us(void) {
    struct a_timespec r, m;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    return (r.tv_sec - m.tv_sec) * 1000000 + (r.tv_nsec - m.tv_nsec) / 1000;
}

static int calls(void) {
    struct a_timespec a = {1, 2}, b = {3, 1000000000}, c = {0, 0};
    int bad = 0;
    bad |= cs(CLOCK_REALTIME, &a) != -EPERM;
    bad |= cs(CLOCK_REALTIME, &b) != -EINVAL;
    bad |= cs(CLOCK_REALTIME, &c) != -EPERM;
    bad |= cs(CLOCK_MONOTONIC, &a) != -EINVAL;
    bad |= cs(CLOCK_BOOTTIME, &c) != -EINVAL;
    bad |= cs(CLOCK_TAI, &c) != -EINVAL;
    bad |= cs(17, &a) != -ENODEV;
    bad |= cs(cpu_clock(0, 2), &c) != -EPERM;
    bad |= cs(cpu_clock(0, 6), &b) != -EPERM;
    bad |= cs(CLOCK_REALTIME, 0) != -EFAULT;
    bad |= a.tv_sec != 1 || a.tv_nsec != 2 || b.tv_sec != 3 || b.tv_nsec != 1000000000;
    bad |= c.tv_sec != 0 || c.tv_nsec != 0;
    return bad;
}

int test_main(void) {
    if (drop_caps() != 0) {
        check("drop_caps", 0, -1);
        return finish();
    }
    a_block_sigchld();
    i64 off0 = real_minus_mono_us();
    struct a_timespec m0, m1, c0, c1;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m0);
    syscall2(SYS_clock_gettime, CLOCK_PROCESS_CPUTIME_ID, &c0);
    check("calls_and_buffers", calls() == 0, 0);
    a_spin_ticks(50000000UL);
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m1);
    syscall2(SYS_clock_gettime, CLOCK_PROCESS_CPUTIME_ID, &c1);
    i64 d = real_minus_mono_us() - off0;
    check("realtime_offset_unchanged", d > -200000 && d < 200000, d);
    check("monotonic_forward", m1.tv_sec * 1000000000 + m1.tv_nsec > m0.tv_sec * 1000000000 + m0.tv_nsec, 0);
    check("cpu_clock_forward", c1.tv_sec * 1000000000 + c1.tv_nsec > c0.tv_sec * 1000000000 + c0.tv_nsec, 0);
    i64 pid = a_fork_sync();
    if (pid == 0) syscall1(SYS_exit_group, calls());
    i32 status = -1;
    expect("child_waited", syscall4(SYS_wait4, pid, &status, 0, 0), pid);
    check("child_same_results", A_WIFEXITED(status) && A_WEXITSTATUS(status) == 0, status);
    return finish();
}
