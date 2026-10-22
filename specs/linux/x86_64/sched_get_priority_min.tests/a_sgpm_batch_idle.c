/* sched_get_priority_min of SCHED_BATCH (3) and SCHED_IDLE (5) is 0. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

int test_main(void) {
    expect("batch_min_is_0", syscall1(SYS_sched_get_priority_min, 3), 0);
    expect("idle_min_is_0", syscall1(SYS_sched_get_priority_min, 5), 0);
    return finish();
}
