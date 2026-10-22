/* Under the non-real-time policies the priority read back is always 0:
 * SCHED_OTHER, SCHED_BATCH, SCHED_IDLE, also with SCHED_RESET_ON_FORK
 * set (an unprivileged caller may not clear it again), and at a raised
 * nice value. sched_getparam changes nothing: the policy and nice value
 * read afterwards are the ones set. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/resource.h, linux/sched.h */
#define PRIO_PROCESS 0
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000

static i64 getp(void) {
    i32 prio = 0x55;
    i64 r = syscall2(SYS_sched_getparam, 0, &prio);
    return r < 0 ? r : prio;
}

int test_main(void) {
    i32 zero = 0;
    i64 nice0 = 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0);
    i64 want = nice0 + 1 > 19 ? 19 : nice0 + 1;
    expect("raise_nice", syscall3(SYS_setpriority, PRIO_PROCESS, 0, want), 0);
    expect("nice_other", getp(), 0);
    expect("nice_kept", 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0), want);
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    expect("batch", getp(), 0);
    expect("batch_kept", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    expect("to_other_rof", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER | SCHED_RESET_ON_FORK, &zero), 0);
    expect("other_rof", getp(), 0);
    expect("other_rof_kept", syscall1(SYS_sched_getscheduler, 0), SCHED_OTHER | SCHED_RESET_ON_FORK);
    expect("to_idle", syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE | SCHED_RESET_ON_FORK, &zero), 0);
    expect("idle", getp(), 0);
    expect("idle_kept", syscall1(SYS_sched_getscheduler, 0), SCHED_IDLE | SCHED_RESET_ON_FORK);
    expect("idle_nice_kept", 20 - syscall2(SYS_getpriority, PRIO_PROCESS, 0), want);
    return finish();
}
