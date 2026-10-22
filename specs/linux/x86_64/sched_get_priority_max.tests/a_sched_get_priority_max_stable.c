/* The values never change: many calls in a row, mixing valid and invalid
 * policies, give the same answers each time. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    static const i64 pol[] = {0, 1, 2, 3, 4, 5, 6, 8, -1};
    static const i64 want[] = {0, 99, 99, 0, -EINVAL, 0, 0, -EINVAL, -EINVAL};
    i64 bad = 0;
    for (int round = 0; round < 200; round++)
        for (unsigned i = 0; i < sizeof pol / sizeof pol[0]; i++)
            if (pmax(pol[i]) != want[i]) bad++;
    expect("all_rounds_same", bad, 0);
    return finish();
}
