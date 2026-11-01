/* The maximum is the highest priority sched_setscheduler accepts for the
 * policy: for SCHED_OTHER and SCHED_BATCH the maximum 0 is accepted and
 * 1 is EINVAL; for SCHED_FIFO and SCHED_RR a priority of maximum + 1
 * (100) is EINVAL for any caller (the range check comes before the
 * permission check, so this holds without privilege too). The maximum
 * is never below the minimum.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_sched_get_priority_max 146
#define SYS_sched_get_priority_min 147
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }
static i64 pmin(i64 policy) { return syscall1(SYS_sched_get_priority_min, policy); }
static i64 setsched(i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, 0, policy, &p);
}

int test_main(void) {
    static const int pols[] = { SCHED_OTHER, SCHED_FIFO, SCHED_RR, SCHED_BATCH, SCHED_IDLE };
    int ordered = 1;
    for (unsigned i = 0; i < sizeof pols / sizeof pols[0]; i++)
        if (pmax(pols[i]) < pmin(pols[i])) ordered = 0;
    check("max_not_below_min", ordered, 0);

    i64 r = setsched(SCHED_OTHER, (int)pmax(SCHED_OTHER));
    check("other_at_max_ok", r == 0, r);
    r = setsched(SCHED_OTHER, (int)pmax(SCHED_OTHER) + 1);
    check("other_above_max_einval", r == -EINVAL, r);
    r = setsched(SCHED_BATCH, (int)pmax(SCHED_BATCH) + 1);
    check("batch_above_max_einval", r == -EINVAL, r);
    r = setsched(SCHED_FIFO, (int)pmax(SCHED_FIFO) + 1);
    check("fifo_above_max_einval", r == -EINVAL, r);
    r = setsched(SCHED_RR, (int)pmax(SCHED_RR) + 1);
    check("rr_above_max_einval", r == -EINVAL, r);
    r = syscall1(SYS_sched_getscheduler, 0);
    check("still_other", r == SCHED_OTHER, r);
    return finish();
}
