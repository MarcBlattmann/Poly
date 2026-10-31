/* sched_get_priority_max with a policy that is not one: 4 (unused),
 * 8, 100, -1, INT_MIN, and valid policies with SCHED_RESET_ON_FORK
 * (0x40000000) or-ed in: all EINVAL. The flag is accepted by
 * sched_setscheduler but not here.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_get_priority_max 146
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_RESET_ON_FORK 0x40000000

static i64 pmax(i64 policy) { return syscall1(SYS_sched_get_priority_max, policy); }

int test_main(void) {
    i64 r = pmax(4);
    check("policy_4_einval", r == -EINVAL, r);
    r = pmax(8);
    check("policy_8_einval", r == -EINVAL, r);
    r = pmax(100);
    check("policy_100_einval", r == -EINVAL, r);
    r = pmax(-1);
    check("policy_minus1_einval", r == -EINVAL, r);
    r = pmax(-2147483647L - 1);
    check("policy_int_min_einval", r == -EINVAL, r);
    r = pmax(0x7fffffff);
    check("policy_int_max_einval", r == -EINVAL, r);
    r = pmax(SCHED_RESET_ON_FORK | SCHED_OTHER);
    check("reset_on_fork_other_einval", r == -EINVAL, r);
    r = pmax(SCHED_RESET_ON_FORK | SCHED_FIFO);
    check("reset_on_fork_fifo_einval", r == -EINVAL, r);
    r = pmax(SCHED_RESET_ON_FORK | SCHED_RR);
    check("reset_on_fork_rr_einval", r == -EINVAL, r);
    return finish();
}
