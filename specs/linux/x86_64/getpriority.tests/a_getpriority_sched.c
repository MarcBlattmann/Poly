/* getpriority reports the nice value however it was set: a nice value
 * set with sched_setattr (SCHED_OTHER) is returned as 20 - nice, and
 * switching to SCHED_BATCH or SCHED_IDLE keeps the nice value
 * getpriority returns.
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_sched_setscheduler 144
#define SYS_sched_setattr 314
/* linux/resource.h */
#define PRIO_PROCESS 0
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5

/* struct sched_attr, linux/sched/types.h (SCHED_ATTR_SIZE_VER1, 56 bytes) */
struct sched_attr {
    u32 size;
    u32 sched_policy;
    u64 sched_flags;
    i32 sched_nice;
    u32 sched_priority;
    u64 sched_runtime, sched_deadline, sched_period;
    u32 sched_util_min, sched_util_max;
};

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }

int test_main(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    i64 n0 = 20 - g0;
    struct sched_attr a;
    memset(&a, 0, sizeof a);
    a.size = sizeof a;
    a.sched_policy = SCHED_OTHER;
    a.sched_nice = (i32)(n0 + 2);
    expect("setattr_nice", syscall3(SYS_sched_setattr, 0, &a, 0), 0);
    expect("setattr_seen", getprio(PRIO_PROCESS, 0), g0 - 2);
    a.sched_nice = (i32)(n0 + 5);
    expect("setattr_nice_2", syscall3(SYS_sched_setattr, 0, &a, 0), 0);
    expect("setattr_seen_2", getprio(PRIO_PROCESS, 0), g0 - 5);
    i32 param = 0;
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &param), 0);
    expect("batch_keeps", getprio(PRIO_PROCESS, 0), g0 - 5);
    expect("to_idle", syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE, &param), 0);
    expect("idle_keeps", getprio(PRIO_PROCESS, 0), g0 - 5);
    return finish();
}
