/* settimeofday: tv_usec (a full 64-bit field) outside 0..999999 is
 * EINVAL for anyone, before the permission check: -1, 1000000, 1000001,
 * 2^32 + 5, INT64_MIN, INT64_MAX. 0 and 999999 get past it (EPERM).
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
    expect("drop_caps", drop_caps(), 0);
    struct a_timeval tv;
    syscall2(SYS_gettimeofday, &tv, 0);
    i64 bad[] = { -1, 1000000, 1000001, 0x100000005LL, (i64)0x8000000000000000ULL, 0x7fffffffffffffffLL };
    const char *names[] = { "usec_minus_1_einval", "usec_1000000_einval", "usec_1000001_einval",
                            "usec_2pow32_plus_5_einval", "usec_int64_min_einval", "usec_int64_max_einval" };
    for (unsigned i = 0; i < 6; i++) {
        tv.tv_usec = bad[i];
        expect(names[i], stod(&tv, 0), -EINVAL);
    }
    tv.tv_usec = 1000000;
    struct a_timezone tz = {0, 0};
    expect("usec_1000000_with_tz_einval", stod(&tv, &tz), -EINVAL);
    tv.tv_usec = 0;
    expect("usec_0_eperm", stod(&tv, 0), -EPERM);
    tv.tv_usec = 999999;
    expect("usec_999999_eperm", stod(&tv, 0), -EPERM);
    return finish();
}
