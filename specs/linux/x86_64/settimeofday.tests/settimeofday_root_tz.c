/* As root, settimeofday(NULL, tz) sets the kernel time zone that
 * gettimeofday reports, dsttime as given; minuteswest must be within
 * -900..900 (15 hours) else EINVAL and nothing changes. The original
 * time zone is put back at the end.
 * poly-diff: needs=root (only CAP_SYS_TIME may set the time zone)
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

static struct a_timezone now_tz(void) {
    struct a_timezone z = {-1, -1};
    syscall2(SYS_gettimeofday, 0, &z);
    return z;
}

int test_main(void) {
    struct a_timezone tz0 = now_tz();
    struct a_timezone z = { tz0.tz_minuteswest, tz0.tz_dsttime ^ 1 };
    expect("set_dsttime_ok", stod(0, &z), 0);
    struct a_timezone got = now_tz();
    expect("reads_back_minuteswest", got.tz_minuteswest, tz0.tz_minuteswest);
    expect("reads_back_dsttime", got.tz_dsttime, tz0.tz_dsttime ^ 1);
    z.tz_minuteswest = 901;
    expect("minuteswest_901_einval", stod(0, &z), -EINVAL);
    z.tz_minuteswest = -901;
    expect("minuteswest_minus_901_einval", stod(0, &z), -EINVAL);
    z.tz_minuteswest = (i32)0x80000000u;
    expect("minuteswest_int_min_einval", stod(0, &z), -EINVAL);
    got = now_tz();
    expect("einval_left_minuteswest", got.tz_minuteswest, tz0.tz_minuteswest);
    z.tz_minuteswest = 900;
    z.tz_dsttime = 0;
    i64 r900 = stod(0, &z);
    got = now_tz();
    z.tz_minuteswest = -900;
    i64 rm900 = stod(0, &z);
    struct a_timezone gotm = now_tz();
    expect("restore_ok", stod(0, &tz0), 0);
    expect("minuteswest_900_ok", r900, 0);
    expect("minuteswest_900_reads_back", got.tz_minuteswest, 900);
    expect("minuteswest_minus_900_ok", rm900, 0);
    expect("minuteswest_minus_900_reads_back", gotm.tz_minuteswest, -900);
    got = now_tz();
    check("restored", got.tz_minuteswest == tz0.tz_minuteswest && got.tz_dsttime == tz0.tz_dsttime, got.tz_dsttime);
    return finish();
}
