/* settimeofday check order: tv readable (EFAULT), then tv_usec below 0
 * or above 1000000 (EINVAL), then tz readable (EFAULT), then tv_usec
 * exactly 1000000 and the tv_sec range (EINVAL), then the permission
 * (EPERM). So with an unreadable tz, tv_usec -1 or 1000001 is EINVAL
 * while tv_usec 1000000, tv_sec -1 or a too-large tv_sec are EFAULT.
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
    void *badtz = (void *)1;
    struct a_timezone tz = {901, 0};
    expect("bad_tv_bad_tz_efault", stod((void *)1, badtz), -EFAULT);
    expect("bad_tv_good_tz_efault", stod((void *)1, &tz), -EFAULT);
    struct a_timeval tv;
    syscall2(SYS_gettimeofday, &tv, 0);
    tv.tv_usec = -1;
    expect("usec_minus_1_before_tz_fault", stod(&tv, badtz), -EINVAL);
    tv.tv_usec = 1000001;
    expect("usec_1000001_before_tz_fault", stod(&tv, badtz), -EINVAL);
    tv.tv_usec = 1000000;
    expect("tz_fault_before_usec_1000000", stod(&tv, badtz), -EFAULT);
    tv.tv_usec = 0;
    tv.tv_sec = -1;
    expect("tz_fault_before_sec_negative", stod(&tv, badtz), -EFAULT);
    tv.tv_sec = SETTOD_SEC_LIMIT;
    expect("tz_fault_before_sec_too_large", stod(&tv, badtz), -EFAULT);
    expect("null_tv_tz_fault_before_eperm", stod(0, badtz), -EFAULT);
    tv.tv_sec = -1;
    expect("sec_negative_before_eperm", stod(&tv, &tz), -EINVAL);
    tv.tv_sec = 0;
    tv.tv_usec = 1000000;
    expect("usec_1000000_before_eperm", stod(&tv, &tz), -EINVAL);
    return finish();
}
