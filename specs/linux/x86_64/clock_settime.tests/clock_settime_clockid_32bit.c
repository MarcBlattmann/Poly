/* The clock id is a 32-bit int: the upper 32 bits of the register are
 * ignored. 0x1_00000000 acts as CLOCK_REALTIME (EFAULT for NULL, EINVAL
 * for a bad tv_nsec, EPERM without CAP_SYS_TIME), 0xffffffff_00000001 as
 * CLOCK_MONOTONIC (EINVAL, tp not read), 0x5_00000010 as auxiliary clock
 * 16 (ENODEV while disabled), and a CPU clock id with garbage in the upper
 * half as that CPU clock (EPERM). 0x80000000 is INT_MIN, a negative id
 * (descriptor clock of a huge fd: EINVAL after reading tp).
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
    struct a_timespec good = {1700000000, 0}, bad = {1700000000, -1};
    i64 rt = (i64)0x100000000LL;
    expect("realtime_high_bits_eperm", cs(rt, &good), -EPERM);
    expect("realtime_high_bits_null_efault", cs(rt, 0), -EFAULT);
    expect("realtime_high_bits_bad_nsec_einval", cs(rt, &bad), -EINVAL);
    i64 rt2 = (i64)0xffffffff00000000ULL;
    expect("realtime_all_high_bits_eperm", cs(rt2, &good), -EPERM);
    i64 mono = (i64)0xffffffff00000001ULL;
    expect("monotonic_high_bits_einval", cs(mono, &good), -EINVAL);
    expect("monotonic_high_bits_null_einval", cs(mono, 0), -EINVAL);
    i64 aux = (i64)0x500000010LL;
    expect("aux16_high_bits_enodev", cs(aux, &good), -ENODEV);
    i64 own = (i64)(0x1200000000ULL | (u64)(u32)cpu_clock(0, 2));
    expect("cpu_clock_high_bits_eperm", cs(own, &good), -EPERM);
    i64 pos = (i64)(u64)(u32)cpu_clock(0, 6);
    expect("cpu_clock_zero_extended_eperm", cs(pos, &good), -EPERM);
    expect("int_min_zero_extended_einval", cs((i64)0x80000000LL, &good), -EINVAL);
    expect("int_min_zero_extended_null_efault", cs((i64)0x80000000LL, 0), -EFAULT);
    return finish();
}
