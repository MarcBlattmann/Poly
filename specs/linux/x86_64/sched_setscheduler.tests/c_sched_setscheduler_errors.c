/* sched_setscheduler errors and their order, for any caller:
 * 1. EINVAL: param NULL or pid negative (before anything else: NULL with
 *    a pid no task has, or with an invalid policy, is EINVAL);
 * 2. EFAULT: param unreadable (before the pid lookup and the policy
 *    check);
 * 3. ESRCH: no task has the pid (before the policy check);
 * 4. EINVAL: policy not one of 0,1,2,3,5 (4, 6 = SCHED_DEADLINE, 8, -1,
 *    high flag bits other than SCHED_RESET_ON_FORK), or a priority out of
 *    the policy's range (fair policies: only 0; SCHED_FIFO/RR: 1..99).
 * A failed call leaves the policy as it was.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_RESET_ON_FORK 0x40000000
/* An id no task has: above PID_MAX_LIMIT (linux/threads.h). */
#define NO_ID 0x3ffffff0

static i64 ss(i64 pid, i64 policy, const void *param) { return syscall3(SYS_sched_setscheduler, pid, policy, param); }

int test_main(void) {
    int zero = 0, one = 1, hundred = 100, minus1 = -1;
    i64 r = ss(0, SCHED_OTHER, 0);
    check("null_einval", r == -EINVAL, r);
    r = ss(-1, SCHED_OTHER, &zero);
    check("negative_pid_einval", r == -EINVAL, r);
    r = ss(NO_ID, SCHED_OTHER, 0);
    check("null_beats_esrch", r == -EINVAL, r);
    r = ss(-1, SCHED_OTHER, (void *)1);
    check("negative_pid_beats_efault", r == -EINVAL, r);

    r = ss(0, SCHED_OTHER, (void *)1);
    check("bad_param_efault", r == -EFAULT, r);
    r = ss(NO_ID, SCHED_OTHER, (void *)1);
    check("efault_beats_esrch", r == -EFAULT, r);
    r = ss(0, 9, (void *)1);
    check("efault_beats_bad_policy", r == -EFAULT, r);

    r = ss(NO_ID, SCHED_OTHER, &zero);
    check("no_such_pid_esrch", r == -ESRCH, r);
    r = ss(NO_ID, 9, &zero);
    check("esrch_beats_bad_policy", r == -ESRCH, r);
    r = ss(NO_ID, SCHED_FIFO, &zero);
    check("esrch_beats_bad_priority", r == -ESRCH, r);

    r = ss(0, 4, &zero);
    check("policy_4_einval", r == -EINVAL, r);
    r = ss(0, SCHED_DEADLINE, &zero);
    check("deadline_einval", r == -EINVAL, r);
    r = ss(0, 8, &zero);
    check("policy_8_einval", r == -EINVAL, r);
    r = ss(0, -1, &zero);
    check("policy_minus1_einval", r == -EINVAL, r);
    r = ss(0, SCHED_RESET_ON_FORK | 4, &zero);
    check("reset_flag_policy_4_einval", r == -EINVAL, r);
    r = ss(0, 0x20000000 | SCHED_OTHER, &zero);
    check("other_flag_bit_einval", r == -EINVAL, r);
    r = ss(0, (i64)0x80000000UL | SCHED_OTHER, &zero);
    check("bit31_einval", r == -EINVAL, r);

    r = ss(0, SCHED_OTHER, &one);
    check("other_prio_1_einval", r == -EINVAL, r);
    r = ss(0, SCHED_OTHER, &minus1);
    check("other_prio_minus1_einval", r == -EINVAL, r);
    r = ss(0, SCHED_BATCH, &one);
    check("batch_prio_1_einval", r == -EINVAL, r);
    r = ss(0, SCHED_IDLE, &one);
    check("idle_prio_1_einval", r == -EINVAL, r);
    r = ss(0, SCHED_FIFO, &zero);
    check("fifo_prio_0_einval", r == -EINVAL, r);
    r = ss(0, SCHED_RR, &hundred);
    check("rr_prio_100_einval", r == -EINVAL, r);
    r = ss(0, SCHED_FIFO, &minus1);
    check("fifo_prio_minus1_einval", r == -EINVAL, r);

    r = syscall1(SYS_sched_getscheduler, 0);
    check("policy_unchanged", r == SCHED_OTHER, r);
    r = ss(0, SCHED_OTHER, &zero);
    check("other_0_ok", r == 0, r);
    return finish();
}
