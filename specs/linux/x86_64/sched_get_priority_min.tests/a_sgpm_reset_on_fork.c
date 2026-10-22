/* SCHED_RESET_ON_FORK (0x40000000, linux/sched.h) is not accepted here:
 * OR-ed into any valid policy, or alone, the call fails with EINVAL. */
#include "a_sys.h"

#define SYS_sched_get_priority_min 147 /* arch/x86/entry/syscalls/syscall_64.tbl */
#define A_EINVAL 22                    /* asm-generic/errno-base.h */

#define RESET_ON_FORK 0x40000000

int test_main(void) {
    expect("flag_alone_einval", syscall1(SYS_sched_get_priority_min, RESET_ON_FORK), -A_EINVAL);
    expect("flag_fifo_einval", syscall1(SYS_sched_get_priority_min, RESET_ON_FORK | 1), -A_EINVAL);
    expect("flag_rr_einval", syscall1(SYS_sched_get_priority_min, RESET_ON_FORK | 2), -A_EINVAL);
    expect("flag_batch_einval", syscall1(SYS_sched_get_priority_min, RESET_ON_FORK | 3), -A_EINVAL);
    expect("flag_idle_einval", syscall1(SYS_sched_get_priority_min, RESET_ON_FORK | 5), -A_EINVAL);
    expect("flag_deadline_einval", syscall1(SYS_sched_get_priority_min, RESET_ON_FORK | 6), -A_EINVAL);
    return finish();
}
