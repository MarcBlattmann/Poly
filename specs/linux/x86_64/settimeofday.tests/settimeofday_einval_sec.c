/* settimeofday: tv_sec negative (-1, INT64_MIN) or 8277292036 and above
 * is EINVAL for anyone, before the permission check; 0, 1 and
 * 8277292035 (also with tv_usec 999999) get past it (EPERM).
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
    struct a_timeval tv = {-1, 0};
    expect("sec_minus_1_einval", stod(&tv, 0), -EINVAL);
    tv.tv_sec = (i64)0x8000000000000000ULL;
    expect("sec_int64_min_einval", stod(&tv, 0), -EINVAL);
    tv.tv_sec = SETTOD_SEC_LIMIT;
    expect("sec_limit_einval", stod(&tv, 0), -EINVAL);
    tv.tv_sec = SETTOD_SEC_LIMIT + 1;
    expect("sec_limit_plus_1_einval", stod(&tv, 0), -EINVAL);
    tv.tv_sec = 0x7fffffffffffffffLL;
    expect("sec_int64_max_einval", stod(&tv, 0), -EINVAL);
    tv.tv_sec = 0;
    expect("sec_0_eperm", stod(&tv, 0), -EPERM);
    tv.tv_sec = 1;
    expect("sec_1_eperm", stod(&tv, 0), -EPERM);
    tv.tv_sec = SETTOD_SEC_LIMIT - 1;
    expect("sec_limit_minus_1_eperm", stod(&tv, 0), -EPERM);
    tv.tv_usec = 999999;
    expect("sec_limit_minus_1_usec_999999_eperm", stod(&tv, 0), -EPERM);
    tv.tv_usec = 1000000;
    expect("sec_limit_minus_1_usec_1000000_einval", stod(&tv, 0), -EINVAL);
    return finish();
}
