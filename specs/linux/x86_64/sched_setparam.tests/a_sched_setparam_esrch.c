/* A pid no thread has is ESRCH (3); the lookup comes before the priority
 * is checked, so a missing pid with priority 5 is still ESRCH. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

int test_main(void) {
    i32 prio = 0;
    expect("pid_int_max", syscall2(SYS_sched_setparam, 0x7fffffff, &prio), -ESRCH);
    expect("pid_big", syscall2(SYS_sched_setparam, 0x3fffffff, &prio), -ESRCH);
    expect("pid_4194304", syscall2(SYS_sched_setparam, 4194304, &prio), -ESRCH);
    prio = 5;
    expect("missing_bad_prio", syscall2(SYS_sched_setparam, 0x3fffffff, &prio), -ESRCH);
    prio = -1;
    expect("missing_neg_prio", syscall2(SYS_sched_setparam, 0x7fffffff, &prio), -ESRCH);
    return finish();
}
