/* The minimum is the lowest priority sched_setscheduler accepts for the
 * policy: SCHED_FIFO or SCHED_RR with minimum - 1 (0) is EINVAL for any
 * caller (range before permission), SCHED_OTHER, SCHED_BATCH and
 * SCHED_IDLE with minimum - 1 (-1) are EINVAL, and SCHED_OTHER and
 * SCHED_BATCH at the minimum 0 are accepted. sched_setparam checks the
 * same range against the caller's current policy.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_sched_get_priority_min 147
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5

static i64 pmin(i64 policy) { return syscall1(SYS_sched_get_priority_min, policy); }
static i64 setsched(i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, 0, policy, &p);
}

int test_main(void) {
    i64 r = setsched(SCHED_FIFO, (int)pmin(SCHED_FIFO) - 1);
    check("fifo_below_min_einval", r == -EINVAL, r);
    r = setsched(SCHED_RR, (int)pmin(SCHED_RR) - 1);
    check("rr_below_min_einval", r == -EINVAL, r);
    r = setsched(SCHED_OTHER, (int)pmin(SCHED_OTHER) - 1);
    check("other_below_min_einval", r == -EINVAL, r);
    r = setsched(SCHED_IDLE, (int)pmin(SCHED_IDLE) - 1);
    check("idle_below_min_einval", r == -EINVAL, r);
    r = setsched(SCHED_BATCH, (int)pmin(SCHED_BATCH));
    check("batch_at_min_ok", r == 0, r);
    int p = (int)pmin(SCHED_BATCH) - 1;
    r = syscall2(SYS_sched_setparam, 0, &p);
    check("setparam_below_min_einval", r == -EINVAL, r);
    r = setsched(SCHED_OTHER, (int)pmin(SCHED_OTHER));
    check("other_at_min_ok", r == 0, r);
    r = syscall1(SYS_sched_getscheduler, 0);
    check("other_again", r == SCHED_OTHER, r);
    return finish();
}
