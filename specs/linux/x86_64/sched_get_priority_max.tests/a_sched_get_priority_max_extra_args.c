/* Only the first argument matters: garbage in the other five argument
 * registers changes nothing. */
#include "a_sys.h"

#ifndef SYS_sched_get_priority_max
#define SYS_sched_get_priority_max 146
#endif

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    expect("fifo_garbage", syscall6(SYS_sched_get_priority_max, 1, -1, -1, -1, -1, -1), 99);
    expect("fifo_plain", pmax(1), 99);
    expect("other_garbage", syscall6(SYS_sched_get_priority_max, 0, 0x1000, 1, 2, 3, 4), 0);
    expect("bad_garbage", syscall6(SYS_sched_get_priority_max, 4, 1, 1, 1, 1, 1), -EINVAL);
    return finish();
}
