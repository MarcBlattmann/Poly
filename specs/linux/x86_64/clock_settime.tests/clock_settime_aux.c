/* Clock ids 16..23 are the auxiliary clocks. While they are disabled
 * (the default after boot), clock_settime on them reads tp (EFAULT for
 * NULL or a bad address), checks it like CLOCK_REALTIME does (EINVAL for
 * a bad tv_nsec, a negative tv_sec, tv_sec 8277292036), and then fails
 * with ENODEV. 15 and 24 are not clocks: EINVAL without reading tp.
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
    struct a_timespec good = {1700000000, 0}, bad_ns = {1700000000, -1}, bad_sec = {-1, 0};
    struct a_timespec big = {CS_SEC_LIMIT, 0}, zero = {0, 0};
    char name[64];
    for (i64 id = 16; id <= 23; id++) {
        char num[24];
        a_utoa(num, (u64)id);
        str_cat2(name, "aux_", num);
        char *e = name + str_len(name);
        str_cpy(e, "_good_enodev");
        expect(name, cs(id, &good), -ENODEV);
        str_cpy(e, "_zero_enodev");
        expect(name, cs(id, &zero), -ENODEV);
        str_cpy(e, "_null_efault");
        expect(name, cs(id, 0), -EFAULT);
        str_cpy(e, "_addr_1_efault");
        expect(name, cs(id, (void *)1), -EFAULT);
        str_cpy(e, "_bad_nsec_einval");
        expect(name, cs(id, &bad_ns), -EINVAL);
        str_cpy(e, "_bad_sec_einval");
        expect(name, cs(id, &bad_sec), -EINVAL);
        str_cpy(e, "_sec_limit_einval");
        expect(name, cs(id, &big), -EINVAL);
    }
    expect("id_15_null_einval", cs(15, 0), -EINVAL);
    expect("id_24_null_einval", cs(24, 0), -EINVAL);
    return finish();
}
