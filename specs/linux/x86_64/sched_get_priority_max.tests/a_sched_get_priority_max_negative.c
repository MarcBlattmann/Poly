/* Negative policies fail with EINVAL. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("minus_1_einval", pmax(-1), -EINVAL);
    expect("minus_2_einval", pmax(-2), -EINVAL);
    expect("minus_100_einval", pmax(-100), -EINVAL);
    expect("int_min_einval", pmax((i64)(i32)0x80000000u), -EINVAL);
    return finish();
}
