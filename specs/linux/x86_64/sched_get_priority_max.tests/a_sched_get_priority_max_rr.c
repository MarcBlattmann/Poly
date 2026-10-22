/* sched_get_priority_max(SCHED_RR = 2) returns 99, the same as SCHED_FIFO. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("rr_max_is_99", pmax(2), 99);
    expect("rr_equals_fifo", pmax(2), pmax(1));
    return finish();
}
