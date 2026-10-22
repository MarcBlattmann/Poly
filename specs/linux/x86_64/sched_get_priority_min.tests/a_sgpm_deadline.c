/* sched_get_priority_min(SCHED_DEADLINE = 6) returns 0. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

int test_main(void) {
    expect("deadline_min_is_0", syscall1(SYS_sched_get_priority_min, 6), 0);
    return finish();
}
