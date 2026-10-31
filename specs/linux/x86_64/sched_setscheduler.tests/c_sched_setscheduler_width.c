/* Argument widths: pid and policy are 32-bit ints, the upper halves of
 * their registers are ignored: pid 1<<32 is the caller, policy
 * 1<<32 | SCHED_BATCH is SCHED_BATCH, policy 1<<32 | 0x80000000 is a
 * negative policy (EINVAL), pid 1<<32 | 0xffffffff is -1 (EINVAL).
 * poly-diff: syscalls=names (pid arguments are not masked for this call)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3

static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

int test_main(void) {
    i64 r = setsched(1L << 32, (1L << 32) | SCHED_BATCH, 0);
    check("high_bits_ignored", r == 0, r);
    r = syscall1(SYS_sched_getscheduler, 0);
    check("now_batch", r == SCHED_BATCH, r);
    r = setsched(0, (1L << 32) | 0x80000000L, 0);
    check("negative_policy_einval", r == -EINVAL, r);
    r = setsched((1L << 32) | 0xffffffffL, SCHED_OTHER, 0);
    check("negative_pid_einval", r == -EINVAL, r);
    r = setsched((9L << 36) | getpid(), (3L << 33) | SCHED_OTHER, 0);
    check("own_pid_high_bits_ok", r == 0, r);
    r = syscall1(SYS_sched_getscheduler, 0);
    check("now_other", r == SCHED_OTHER, r);
    return finish();
}
