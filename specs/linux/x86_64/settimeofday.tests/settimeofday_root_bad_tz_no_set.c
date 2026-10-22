/* As root, settimeofday(tv, tz) with minuteswest out of range (901) is
 * EINVAL and the time in tv (2 s ahead) is not set; tv_usec 1000000 and
 * a negative tv_sec are EINVAL for root as well; with CAP_SYS_TIME
 * dropped root gets EPERM like anyone.
 * poly-diff: needs=root (only CAP_SYS_TIME gets past the permission check)
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
    struct a_timezone tz0 = {-1, -1}, got = {-1, -1};
    syscall2(SYS_gettimeofday, 0, &tz0);
    i64 off0 = real_minus_mono_us();
    struct a_timespec r;
    syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
    struct a_timeval tv = { r.tv_sec + 2, 0 };
    struct a_timezone z = { 901, 0 };
    i64 rb = stod(&tv, &z);
    i64 d = real_minus_mono_us() - off0;
    if (d > 1000000) { /* the clock moved after all: undo it */
        syscall2(SYS_clock_gettime, CLOCK_REALTIME, &r);
        struct a_timeval back = { r.tv_sec - 2, r.tv_nsec / 1000 };
        stod(&back, 0);
    }
    expect("bad_tz_einval", rb, -EINVAL);
    check("time_not_set", d > -200000 && d < 200000, d);
    syscall2(SYS_gettimeofday, 0, &got);
    check("tz_unchanged", got.tz_minuteswest == tz0.tz_minuteswest && got.tz_dsttime == tz0.tz_dsttime, got.tz_minuteswest);
    tv.tv_sec = r.tv_sec;
    tv.tv_usec = 1000000;
    expect("usec_1000000_einval", stod(&tv, 0), -EINVAL);
    tv.tv_sec = -1;
    tv.tv_usec = 0;
    expect("sec_negative_einval", stod(&tv, 0), -EINVAL);
    expect("drop_caps", drop_caps(), 0);
    expect("dropped_null_null_eperm", stod(0, 0), -EPERM);
    expect("dropped_tz_eperm", stod(0, &tz0), -EPERM);
    return finish();
}
