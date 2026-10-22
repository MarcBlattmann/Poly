/* Policy 4 is not a scheduling policy (the gap between SCHED_BATCH and
 * SCHED_IDLE): EINVAL, while its neighbours succeed. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("policy_3_ok", pmax(3), 0);
    expect("policy_4_einval", pmax(4), -EINVAL);
    expect("policy_5_ok", pmax(5), 0);
    return finish();
}
