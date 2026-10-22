/* sched_setparam under SCHED_OTHER and SCHED_BATCH leaves the nice value
 * alone. The test only raises nice (any user may), relative to where it
 * started, clamped at 19. getpriority returns 20 - nice. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_getpriority 140
#define SYS_setpriority 141

/* linux/resource.h, linux/sched.h */
#define PRIO_PROCESS 0
#define SCHED_BATCH 3

int test_main(void) {
    i64 nice0 = 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0);
    i64 want = nice0 + 2 > 19 ? 19 : nice0 + 2;
    expect("raise_nice", syscall3(SYS_setpriority, PRIO_PROCESS, 0, want), 0);
    i32 zero = 0, one = 1;
    expect("setparam", syscall2(SYS_sched_setparam, 0, &zero), 0);
    expect("nice_kept", 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0), want);
    expect("setparam_einval", syscall2(SYS_sched_setparam, 0, &one), -EINVAL);
    expect("nice_kept_einval", 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0), want);
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    expect("batch_setparam", syscall2(SYS_sched_setparam, 0, &zero), 0);
    expect("batch_nice_kept", 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0), want);
    return finish();
}
