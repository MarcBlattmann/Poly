/* sched_get_priority_max(SCHED_FIFO = 1) returns 99. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("fifo_max_is_99", pmax(1), 99);
    return finish();
}
