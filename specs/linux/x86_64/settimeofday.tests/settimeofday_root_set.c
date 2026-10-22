/* As root, settimeofday(tv, ...) sets CLOCK_REALTIME: NULL, NULL is 0;
 * setting the current time is 0 and keeps the realtime-monotonic offset
 * within 0.2 s; setting 2 s ahead moves CLOCK_REALTIME (not
 * CLOCK_MONOTONIC) forward by about 2 s, seen by gettimeofday too. The
 * test then sets the clock back by the same 2 s.
 * poly-diff: needs=root (only CAP_SYS_TIME may set the clock)
 * poly-diff: syscalls=names (times read differ between runs)
 */
#include "a_sys.h"

#define SYS_capset 126
#define SYS_settimeofday 164

/* First tv_sec that is too large to set: 2^63 ns in seconds, less 30
 * years of 365 days (observed boundary on x86-64). */
#define SETTOD_SEC_LIMIT 8277292036LL

static A_UNUSED void fill(void *p, u8 v, size_t n) {
    u8 *b = p;
    for (size_t i = 0; i < n; i++) b[i] = v;
}

static A_UNUSED i64 stod(void *tv, void *tz) { return syscall2(SYS_settimeofday, tv, tz); }

/* Empty all three capability sets (any user may): root then fails the
 * privilege check too, and can never change the clock by accident. */
static A_UNUSED i64 drop_caps(void) {
    struct { u32 version; i32 pid; } h = { 0x20080522, 0 };
    struct { u32 effective, permitted, inheritable; } d[2];
    fill(d, 0, sizeof d);
    return syscall2(SYS_capset, &h, d);
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
    struct a_timezone tz0 = {0, 0};
    syscall2(SYS_gettimeofday, 0, &tz0);
    expect("null_null_ok", stod(0, 0), 0);
    i64 off0 = real_minus_mono_us();
    struct a_timespec r, m0, m1;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    struct a_timeval tv = { r.tv_sec, r.tv_nsec / 1000 };
    expect("set_now_ok", stod(&tv, 0), 0);
    i64 d = real_minus_mono_us() - off0;
    check("set_now_keeps_offset", d > -200000 && d < 200000, d);
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m0);
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    tv.tv_sec = r.tv_sec + 2;
    tv.tv_usec = r.tv_nsec / 1000;
    i64 rj = stod(&tv, &tz0);
    struct a_timeval g;
    syscall2(SYS_gettimeofday, &g, 0);
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &m1);
    i64 jumped = real_minus_mono_us() - off0;
    /* Put the clock back: now minus the 2 s added. */
    if (rj == 0) {
        syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
        tv.tv_sec = r.tv_sec - 2;
        tv.tv_usec = r.tv_nsec / 1000;
        expect("set_back_ok", stod(&tv, 0), 0);
    }
    expect("jump_with_tz_ok", rj, 0);
    check("realtime_jumped_2s", jumped > 1800000 && jumped < 2200000, jumped);
    i64 gd = g.tv_sec - (r.tv_sec - 2);
    check("gettimeofday_sees_jump", gd >= 1 && gd <= 3, gd);
    i64 md = (m1.tv_sec - m0.tv_sec) * 1000000 + (m1.tv_nsec - m0.tv_nsec) / 1000;
    check("monotonic_not_jumped", md >= 0 && md < 200000, md);
    d = real_minus_mono_us() - off0;
    check("restored_offset", d > -200000 && d < 200000, d);
    return finish();
}
