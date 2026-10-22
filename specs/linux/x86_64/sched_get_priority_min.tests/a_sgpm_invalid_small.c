/* Unassigned small policy numbers (4, 8, 9, 10, 16, 257, INT_MAX) fail
 * with EINVAL, and a valid call right after still works. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

int test_main(void) {
    expect("policy_4_einval", syscall1(SYS_sched_get_priority_min, 4), -A_EINVAL);
    expect("policy_8_einval", syscall1(SYS_sched_get_priority_min, 8), -A_EINVAL);
    expect("policy_9_einval", syscall1(SYS_sched_get_priority_min, 9), -A_EINVAL);
    expect("policy_10_einval", syscall1(SYS_sched_get_priority_min, 10), -A_EINVAL);
    expect("policy_16_einval", syscall1(SYS_sched_get_priority_min, 16), -A_EINVAL);
    expect("policy_257_einval", syscall1(SYS_sched_get_priority_min, 257), -A_EINVAL);
    expect("policy_intmax_einval", syscall1(SYS_sched_get_priority_min, 0x7fffffff), -A_EINVAL);
    expect("valid_after_error", syscall1(SYS_sched_get_priority_min, 1), 1);
    return finish();
}
