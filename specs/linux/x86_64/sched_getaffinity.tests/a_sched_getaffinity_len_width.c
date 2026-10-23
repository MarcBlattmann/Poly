/* len is a 32-bit unsigned int (upper 32 bits of the register ignored),
 * and its size in bits (len * 8) is also taken modulo 2^32. So:
 * 0x100000000 is 0 (EINVAL), 0x100000008 is 8 (returns 8), 0x100000001
 * is 1 (EINVAL), -8 is 0xfffffff8 (returns 8), -1 is 0xffffffff (not a
 * multiple of 8: EINVAL). A len whose bit count wraps to 0 (0x20000000,
 * 0x40000000, 0x80000000, 0xe0000000) counts as too small: EINVAL;
 * 0x20000008 returns 8. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getaffinity 204

int test_main(void) {
    u64 ref = 0;
    expect("len8", syscall3(SYS_sched_getaffinity, 0, 8, &ref), 8);

    u64 m[2];
    m[0] = 0; m[1] = 0x77;
    expect("len_4g_plus_8", syscall3(SYS_sched_getaffinity, 0, 0x100000008L, m), 8);
    check("len_4g_plus_8_mask", m[0] == ref && m[1] == 0x77, 0);
    m[0] = 0;
    expect("len_minus_8", syscall3(SYS_sched_getaffinity, 0, -8L, m), 8);
    check("len_minus_8_mask", m[0] == ref && m[1] == 0x77, 0);
    m[0] = 0;
    expect("len_20000008", syscall3(SYS_sched_getaffinity, 0, 0x20000008L, m), 8);
    check("len_20000008_mask", m[0] == ref, 0);

    m[0] = 0x1234;
    expect("len_4g", syscall3(SYS_sched_getaffinity, 0, 0x100000000L, m), -EINVAL);
    expect("len_4g_plus_1", syscall3(SYS_sched_getaffinity, 0, 0x100000001L, m), -EINVAL);
    expect("len_minus_1", syscall3(SYS_sched_getaffinity, 0, -1L, m), -EINVAL);
    expect("len_ffffffff", syscall3(SYS_sched_getaffinity, 0, 0xffffffffL, m), -EINVAL);
    expect("len_20000000", syscall3(SYS_sched_getaffinity, 0, 0x20000000L, m), -EINVAL);
    expect("len_40000000", syscall3(SYS_sched_getaffinity, 0, 0x40000000L, m), -EINVAL);
    expect("len_80000000", syscall3(SYS_sched_getaffinity, 0, 0x80000000L, m), -EINVAL);
    expect("len_e0000000", syscall3(SYS_sched_getaffinity, 0, 0xe0000000L, m), -EINVAL);
    check("failures_wrote_nothing", m[0] == 0x1234 && m[1] == 0x77, 0);
    return finish();
}
