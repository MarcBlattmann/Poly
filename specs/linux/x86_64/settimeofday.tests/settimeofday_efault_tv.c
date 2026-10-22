/* settimeofday: tv must be 16 readable bytes, else EFAULT (address 1, a
 * kernel address, tv_usec on an inaccessible page), before the EPERM
 * check. A read-only tv, or one ending exactly at the end of accessible
 * memory, is fine (EPERM without CAP_SYS_TIME). tv is never written.
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
    u8 *p = edge_pages();
    struct a_timeval now;
    syscall2(SYS_gettimeofday, &now, 0);
    expect("addr_1_efault", stod((void *)1, 0), -EFAULT);
    expect("kernel_addr_efault", stod((void *)0xffffffff80000000ULL, 0), -EFAULT);
    expect("unmapped_page_efault", stod(p + A_PAGE, 0), -EFAULT);
    struct a_timeval *half = (struct a_timeval *)(p + A_PAGE - 8);
    half->tv_sec = now.tv_sec;
    expect("usec_inaccessible_efault", stod(half, 0), -EFAULT);
    struct a_timezone tz = {0, 0};
    expect("usec_inaccessible_with_tz_efault", stod(half, &tz), -EFAULT);
    struct a_timeval *end = (struct a_timeval *)(p + A_PAGE - 16);
    end->tv_sec = now.tv_sec;
    end->tv_usec = 0;
    expect("ends_at_page_end_eperm", stod(end, 0), -EPERM);
    syscall3(SYS_mprotect, (i64)p, A_PAGE, PROT_READ);
    expect("read_only_eperm", stod(end, &tz), -EPERM);
    check("not_written", end->tv_sec == now.tv_sec && end->tv_usec == 0, end->tv_usec);
    return finish();
}
