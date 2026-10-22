/* The policy argument is a 32-bit int: bits 32-63 of the register are
 * ignored, only the low 32 bits select the policy. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

int test_main(void) {
    expect("hi1_other_is_0", syscall1(SYS_sched_get_priority_min, 0x100000000LL), 0);
    expect("hi1_fifo_is_1", syscall1(SYS_sched_get_priority_min, 0x100000001LL), 1);
    expect("hi1_rr_is_1", syscall1(SYS_sched_get_priority_min, 0x100000002LL), 1);
    expect("hiall_fifo_is_1", syscall1(SYS_sched_get_priority_min, (i64)0xffffffff00000001ULL), 1);
    expect("hiall_other_is_0", syscall1(SYS_sched_get_priority_min, (i64)0xffffffff00000000ULL), 0);
    expect("hi_policy_4_einval", syscall1(SYS_sched_get_priority_min, (i64)0x5a5a5a5a00000004ULL), -A_EINVAL);
    expect("low_minus_1_einval", syscall1(SYS_sched_get_priority_min, (i64)0x00000001ffffffffULL), -A_EINVAL);
    return finish();
}
