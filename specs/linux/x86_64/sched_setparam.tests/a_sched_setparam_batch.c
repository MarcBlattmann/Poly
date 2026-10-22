/* Under SCHED_BATCH, as under SCHED_OTHER, only priority 0 is valid.
 * sched_setparam keeps the policy: the thread stays SCHED_BATCH. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_BATCH 3

int test_main(void) {
    i32 zero = 0, one = 1, neg = -1;
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    expect("prio_0", syscall2(SYS_sched_setparam, 0, &zero), 0);
    expect("still_batch", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    expect("prio_1", syscall2(SYS_sched_setparam, 0, &one), -EINVAL);
    expect("prio_minus1", syscall2(SYS_sched_setparam, 0, &neg), -EINVAL);
    expect("tid_prio_0", syscall2(SYS_sched_setparam, syscall0(SYS_gettid), &zero), 0);
    expect("batch_after", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    i32 got = -1;
    expect("getparam", syscall2(SYS_sched_getparam, 0, &got), 0);
    expect("prio_is_0", got, 0);
    return finish();
}
