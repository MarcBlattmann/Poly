/* Failed settimeofday calls (EPERM, EINVAL, EFAULT) change nothing: the
 * offset between CLOCK_REALTIME and CLOCK_MONOTONIC stays within 0.2 s,
 * the time zone gettimeofday reports stays the same, and the caller's
 * buffers are not written.
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
    struct a_timezone tz0 = {-1, -1}, tz1 = {-1, -1};
    syscall2(SYS_gettimeofday, 0, &tz0);
    i64 off0 = real_minus_mono_us();
    struct a_timeval far = {0, 0};
    struct a_timezone tz = {600, 1};
    i64 r1 = stod(&far, &tz);
    far.tv_sec = 2000000000;
    i64 r2 = stod(&far, 0);
    i64 r3 = stod(0, &tz);
    far.tv_usec = 1000000;
    i64 r4 = stod(&far, &tz);
    far.tv_usec = 0;
    i64 r5 = stod(&far, (void *)1);
    check("all_failed", r1 == -EPERM && r2 == -EPERM && r3 == -EPERM && r4 == -EINVAL && r5 == -EFAULT, r1);
    i64 off1 = real_minus_mono_us();
    i64 d = off1 - off0;
    check("realtime_not_moved", d > -200000 && d < 200000, d);
    syscall2(SYS_gettimeofday, 0, &tz1);
    expect("tz_minuteswest_same", tz1.tz_minuteswest, tz0.tz_minuteswest);
    expect("tz_dsttime_same", tz1.tz_dsttime, tz0.tz_dsttime);
    check("buffers_kept", far.tv_sec == 2000000000 && far.tv_usec == 0 && tz.tz_minuteswest == 600 && tz.tz_dsttime == 1, 0);
    return finish();
}
