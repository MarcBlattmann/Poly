/* sched_get_priority_min for every valid policy: SCHED_FIFO and SCHED_RR
 * report 1; SCHED_OTHER, SCHED_BATCH, SCHED_IDLE, SCHED_DEADLINE and
 * SCHED_EXT (7) report 0. The answer does not depend on the caller.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_get_priority_min 147
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_EXT 7

static i64 pmin(i64 policy) { return syscall1(SYS_sched_get_priority_min, policy); }

int test_main(void) {
    i64 r = pmin(SCHED_OTHER);
    check("other_0", r == 0, r);
    r = pmin(SCHED_FIFO);
    check("fifo_1", r == 1, r);
    r = pmin(SCHED_RR);
    check("rr_1", r == 1, r);
    r = pmin(SCHED_BATCH);
    check("batch_0", r == 0, r);
    r = pmin(SCHED_IDLE);
    check("idle_0", r == 0, r);
    r = pmin(SCHED_DEADLINE);
    check("deadline_0", r == 0, r);
    r = pmin(SCHED_EXT);
    check("ext_0", r == 0, r);

    int p = 0;
    r = syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &p);
    check("to_batch", r == 0, r);
    r = pmin(SCHED_RR);
    check("rr_1_from_batch", r == 1, r);
    r = syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, &p);
    check("back_to_other", r == 0, r);
    return finish();
}
