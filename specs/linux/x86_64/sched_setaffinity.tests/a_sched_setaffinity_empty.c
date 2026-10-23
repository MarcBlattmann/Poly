/* A mask that names no CPU the thread may use is EINVAL (22): all zero,
 * only CPU numbers the system does not have (bit 63), or a length of 0
 * (nothing is read, even with a NULL pointer). A failed call leaves the
 * mask as it was. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

static u64 mask_of(i64 pid) {
    u64 m[16];
    memset(m, 0, sizeof m);
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start;
    u64 v = lo;
    expect("set_lowest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);

    v = 0;
    expect("zero_mask", syscall3(SYS_sched_setaffinity, 0, 8, &v), -EINVAL);
    v = 1UL << 63;
    expect("only_cpu63", syscall3(SYS_sched_setaffinity, 0, 8, &v), -EINVAL);
    u64 two[2] = {0, 0};
    expect("zero_mask_len16", syscall3(SYS_sched_setaffinity, 0, 16, two), -EINVAL);
    v = ~0UL;
    expect("len0_all_ones", syscall3(SYS_sched_setaffinity, 0, 0, &v), -EINVAL);
    expect("len0_null", syscall3(SYS_sched_setaffinity, 0, 0, 0), -EINVAL);
    expect("len0_bad_pointer", syscall3(SYS_sched_setaffinity, 0, 0, 1), -EINVAL);
    unsigned char three[3] = {0, 0, 0};
    expect("len3_zero", syscall3(SYS_sched_setaffinity, 0, 3, three), -EINVAL);
    check("mask_unchanged", mask_of(0) == lo, 0);

    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    check("restored", mask_of(0) == start, 0);
    return finish();
}
