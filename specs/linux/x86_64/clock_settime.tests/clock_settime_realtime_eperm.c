/* clock_settime(CLOCK_REALTIME, tp) without CAP_SYS_TIME (a normal user,
 * or root after emptying its capability sets) is EPERM for every
 * well-formed tp: tv_sec 0, 1, the current time, the largest accepted
 * value (8277292035 s + 999999999 ns). The clock does not change and tp
 * is not written.
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
    i64 off0 = real_minus_mono_us();
    struct a_timespec t0 = {0, 0}, t1 = {1, 0}, t2 = {1700000000, 123456789};
    struct a_timespec tmax = {CS_SEC_LIMIT - 1, 999999999};
    expect("sec_0_eperm", cs(CLOCK_REALTIME, &t0), -EPERM);
    expect("sec_1_eperm", cs(CLOCK_REALTIME, &t1), -EPERM);
    expect("ordinary_time_eperm", cs(CLOCK_REALTIME, &t2), -EPERM);
    expect("largest_time_eperm", cs(CLOCK_REALTIME, &tmax), -EPERM);
    struct a_timespec now;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &now);
    struct a_timespec copy = now;
    expect("current_time_eperm", cs(CLOCK_REALTIME, &now), -EPERM);
    check("tp_not_written", now.tv_sec == copy.tv_sec && now.tv_nsec == copy.tv_nsec, now.tv_nsec);
    check("t2_not_written", t2.tv_sec == 1700000000 && t2.tv_nsec == 123456789, t2.tv_nsec);
    i64 d = real_minus_mono_us() - off0;
    check("clock_unchanged", d > -200000 && d < 200000, d);
    return finish();
}
