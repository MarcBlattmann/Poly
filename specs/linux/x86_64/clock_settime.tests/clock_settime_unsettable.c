/* clock_settime on a clock id 0 < id < 16 (CLOCK_MONOTONIC, the CPU-time
 * shorthands 2 and 3, MONOTONIC_RAW, the coarse clocks, BOOTTIME, the
 * alarm clocks, CLOCK_TAI, unused 10, 12-15) or on 24, 100, INT_MAX is
 * EINVAL for anyone, and tp is never read: NULL, a bad address and a bad
 * tv_nsec all give EINVAL too.
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
    struct a_timespec good = {1700000000, 0}, bad = {1700000000, 1000000000};
    static const i64 ids[] = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 24, 100, 0x7fffffff};
    char name[64];
    for (unsigned i = 0; i < sizeof ids / sizeof ids[0]; i++) {
        char num[24];
        a_utoa(num, (u64)ids[i]);
        str_cat2(name, "id_", num);
        char *e = name + str_len(name);
        str_cpy(e, "_good");
        expect(name, cs(ids[i], &good), -EINVAL);
        str_cpy(e, "_null");
        expect(name, cs(ids[i], 0), -EINVAL);
        str_cpy(e, "_addr_1");
        expect(name, cs(ids[i], (void *)1), -EINVAL);
        str_cpy(e, "_bad_nsec");
        expect(name, cs(ids[i], &bad), -EINVAL);
    }
    return finish();
}
