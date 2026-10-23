/* clock_settime check order without CAP_SYS_TIME:
 * CLOCK_REALTIME: tp readable (EFAULT), then tv_nsec/tv_sec range
 * (EINVAL), then EPERM. Ids 1..15 and >= 24: EINVAL before tp is read.
 * Aux clocks 16..23 (disabled): EFAULT, EINVAL, then ENODEV.
 * Negative ids (CPU and descriptor clocks): tp read first (EFAULT), then
 * the id (EINVAL); CPU clocks never look at the time values (EPERM).
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

int test_main(void) {
    if (drop_caps() != 0) {
        check("drop_caps", 0, -1);
        return finish();
    }
    u8 *p = edge_pages();
    struct a_timespec *half = (struct a_timespec *)(p + A_PAGE - 8);
    half->tv_sec = -1;
    struct a_timespec bad = {-1, 1000000000}, good = {5, 0};
    expect("realtime_efault_before_einval", cs(CLOCK_REALTIME, half), -EFAULT);
    expect("realtime_einval_before_eperm", cs(CLOCK_REALTIME, &bad), -EINVAL);
    expect("realtime_eperm", cs(CLOCK_REALTIME, &good), -EPERM);
    expect("monotonic_einval_before_efault", cs(CLOCK_MONOTONIC, half), -EINVAL);
    expect("tai_einval_before_efault", cs(CLOCK_TAI, (void *)1), -EINVAL);
    expect("id_24_einval_before_efault", cs(24, half), -EINVAL);
    expect("aux_efault_before_einval", cs(16, half), -EFAULT);
    expect("aux_einval_before_enodev", cs(16, &bad), -EINVAL);
    expect("aux_enodev", cs(16, &good), -ENODEV);
    expect("cpu_efault_before_eperm", cs(cpu_clock(0, 2), half), -EFAULT);
    expect("cpu_bad_values_still_eperm", cs(cpu_clock(0, 2), &bad), -EPERM);
    expect("cpu_missing_efault_before_einval", cs(cpu_clock(0x3ffffff, 2), half), -EFAULT);
    expect("cpu_missing_bad_values_einval", cs(cpu_clock(0x3ffffff, 2), &bad), -EINVAL);
    expect("fd_clock_efault_before_einval", cs(fd_clock(1000), half), -EFAULT);
    expect("fd_clock_bad_values_einval", cs(fd_clock(1000), &bad), -EINVAL);
    return finish();
}
