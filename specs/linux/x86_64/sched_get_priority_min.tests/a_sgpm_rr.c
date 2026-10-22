/* sched_get_priority_min(SCHED_RR = 2) returns 1. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

int test_main(void) {
    expect("rr_min_is_1", syscall1(SYS_sched_get_priority_min, 2), 1);
    return finish();
}
