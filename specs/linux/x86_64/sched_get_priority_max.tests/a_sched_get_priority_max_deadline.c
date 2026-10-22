/* sched_get_priority_max(SCHED_DEADLINE = 6) returns 0. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("deadline_max_is_0", pmax(6), 0);
    return finish();
}
