/* clock_settime(CLOCK_REALTIME, tp) rejects a malformed time with EINVAL
 * for anyone, before the permission check: tv_nsec (all 64 bits) outside
 * 0..999999999, tv_sec negative or 8277292036 or more. The neighbouring
 * good values get EPERM (no CAP_SYS_TIME).
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
    struct a_timespec t;
    static const i64 bad_ns[] = {-1, 1000000000, 1000000001, 0x100000005LL, (i64)0x8000000000000000ULL,
                                 0x7fffffffffffffffLL};
    static const char *bad_ns_names[] = {"nsec_minus_1", "nsec_1e9", "nsec_1e9_plus_1", "nsec_2p32_plus_5",
                                         "nsec_int64_min", "nsec_int64_max"};
    for (int i = 0; i < 6; i++) {
        t.tv_sec = 1700000000;
        t.tv_nsec = bad_ns[i];
        expect(bad_ns_names[i], cs(CLOCK_REALTIME, &t), -EINVAL);
    }
    t.tv_nsec = 999999999;
    expect("nsec_999999999_eperm", cs(CLOCK_REALTIME, &t), -EPERM);
    t.tv_nsec = 0;
    t.tv_sec = -1;
    expect("sec_minus_1", cs(CLOCK_REALTIME, &t), -EINVAL);
    t.tv_sec = (i64)0x8000000000000000ULL;
    expect("sec_int64_min", cs(CLOCK_REALTIME, &t), -EINVAL);
    t.tv_sec = CS_SEC_LIMIT;
    expect("sec_limit", cs(CLOCK_REALTIME, &t), -EINVAL);
    t.tv_sec = 0x7fffffffffffffffLL;
    expect("sec_int64_max", cs(CLOCK_REALTIME, &t), -EINVAL);
    t.tv_sec = CS_SEC_LIMIT - 1;
    expect("sec_limit_minus_1_eperm", cs(CLOCK_REALTIME, &t), -EPERM);
    t.tv_sec = -1;
    t.tv_nsec = -1;
    expect("both_bad", cs(CLOCK_REALTIME, &t), -EINVAL);
    return finish();
}
