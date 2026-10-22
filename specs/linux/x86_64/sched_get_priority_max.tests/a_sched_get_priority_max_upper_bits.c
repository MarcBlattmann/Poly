/* The policy is a 32-bit int: the upper 32 bits of the register are
 * ignored. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("hi1_fifo", pmax((i64)0x0000000100000001ULL), 99);
    expect("hi_all_rr", pmax((i64)0xffffffff00000002ULL), 99);
    expect("hi_other", pmax((i64)0x8000000000000000ULL), 0);
    expect("hi_batch", pmax((i64)0x1234567800000003ULL), 0);
    expect("hi_hole_einval", pmax((i64)0x0000000100000004ULL), -EINVAL);
    expect("low_minus1_einval", pmax((i64)0x00000000ffffffffULL), -EINVAL);
    return finish();
}
