/* sched_setparam errors and their order (as sched_setscheduler with the
 * policy unchanged): EINVAL for a NULL param or a negative pid first;
 * then EFAULT for an unreadable param (also with a pid no task has);
 * then ESRCH; then EINVAL for a priority outside the current policy's
 * range (only 0 under SCHED_OTHER). A failed call changes nothing.
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
/* An id no task has: above PID_MAX_LIMIT (linux/threads.h). */
#define NO_ID 0x3ffffff0

static i64 sp(i64 pid, const void *param) { return syscall2(SYS_sched_setparam, pid, param); }

int test_main(void) {
    int zero = 0, one = 1, minus1 = -1;
    i64 r = sp(0, 0);
    check("null_einval", r == -EINVAL, r);
    r = sp(-1, &zero);
    check("negative_pid_einval", r == -EINVAL, r);
    r = sp(NO_ID, 0);
    check("null_beats_esrch", r == -EINVAL, r);
    r = sp(-5, (void *)1);
    check("negative_pid_beats_efault", r == -EINVAL, r);
    r = sp(0, (void *)1);
    check("bad_param_efault", r == -EFAULT, r);
    r = sp(NO_ID, (void *)1);
    check("efault_beats_esrch", r == -EFAULT, r);
    r = sp(NO_ID, &zero);
    check("no_such_pid_esrch", r == -ESRCH, r);
    r = sp(NO_ID, &one);
    check("esrch_beats_bad_priority", r == -ESRCH, r);
    r = sp(0, &one);
    check("other_prio_1_einval", r == -EINVAL, r);
    r = sp(0, &minus1);
    check("other_prio_minus1_einval", r == -EINVAL, r);
    r = sp(0xffffffffL, &zero);
    check("low_half_minus1_einval", r == -EINVAL, r);

    r = syscall1(SYS_sched_getscheduler, 0);
    check("policy_unchanged", r == SCHED_OTHER, r);
    int p = 9;
    r = syscall2(SYS_sched_getparam, 0, &p);
    check("priority_unchanged", r == 0 && p == 0, p);
    r = sp(0, &zero);
    check("other_0_ok", r == 0, r);
    r = sp(1L << 32, &zero);
    check("high_bits_pid0_ok", r == 0, r);
    return finish();
}
