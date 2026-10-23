/* As root, clock_settime(CLOCK_REALTIME, tp) sets the realtime clock:
 * setting the current time is 0 and keeps the realtime-monotonic offset
 * within 0.2 s; setting 2 s ahead moves CLOCK_REALTIME (and
 * gettimeofday), not CLOCK_MONOTONIC, forward by about 2 s; tv_nsec is
 * kept (a time set with tv_nsec 500000000 reads back with a fraction near
 * it). The test then sets the clock back by the same 2 s.
 * poly-diff: needs=root (only CAP_SYS_TIME may set the clock)
 * poly-diff: syscalls=names (times read differ between runs)
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
    if (syscall0(SYS_geteuid) != 0) return finish(); /* needs root: nothing to check */
    i64 off0 = real_minus_mono_us();
    struct a_timespec r, m0, m1;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    expect("set_now_ok", cs(CLOCK_REALTIME, &r), 0);
    i64 d = real_minus_mono_us() - off0;
    check("set_now_keeps_offset", d > -200000 && d < 200000, d);
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    struct a_timespec ahead = {r.tv_sec + 2, r.tv_nsec};
    i64 rj = cs(CLOCK_REALTIME, &ahead);
    struct a_timeval g;
    syscall2(SYS_gettimeofday, &g, 0);
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m1);
    i64 jumped = real_minus_mono_us() - off0;
    /* Put the clock back: now minus the 2 s added. */
    if (rj == 0) {
        syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
        struct a_timespec back = {r.tv_sec - 2, r.tv_nsec};
        expect("set_back_ok", cs(CLOCK_REALTIME, &back), 0);
    }
    expect("jump_ok", rj, 0);
    check("realtime_jumped_2s", jumped > 1800000 && jumped < 2200000, jumped);
    i64 gd = g.tv_sec - (r.tv_sec - 2);
    check("gettimeofday_sees_jump", gd >= 1 && gd <= 3, gd);
    i64 md = (m1.tv_sec - m0.tv_sec) * 1000000 + (m1.tv_nsec - m0.tv_nsec) / 1000;
    check("monotonic_not_jumped", md >= 0 && md < 200000, md);
    /* Nanoseconds are kept: set the current second + 0.5 s, read back. */
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    i64 before_us = real_minus_mono_us();
    struct a_timespec half = {r.tv_sec, 500000000};
    i64 rh = cs(CLOCK_REALTIME, &half);
    struct a_timespec h;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &h);
    i64 frac = (h.tv_sec - r.tv_sec) * 1000000000 + h.tv_nsec;
    /* Restore: undo the shift that setting half caused. */
    i64 shift_us = real_minus_mono_us() - before_us;
    if (rh == 0) {
        syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
        i64 ns = r.tv_sec * 1000000000 + r.tv_nsec - shift_us * 1000;
        struct a_timespec fix = {ns / 1000000000, ns % 1000000000};
        expect("restore_ok", cs(CLOCK_REALTIME, &fix), 0);
    }
    expect("set_half_ok", rh, 0);
    check("nsec_kept", frac >= 500000000 && frac < 700000000, frac);
    d = real_minus_mono_us() - off0;
    check("restored_offset", d > -200000 && d < 200000, d);
    return finish();
}
