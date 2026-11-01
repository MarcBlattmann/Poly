/* sched_getscheduler of the caller: pid 0, its own pid and its own
 * thread id all name the caller; a test program starts with SCHED_OTHER
 * (0). Changing the policy with sched_setscheduler to SCHED_BATCH (3)
 * shows at once, also through the pid; going back to SCHED_OTHER shows
 * 0 again. Only the low 32 bits of the pid argument count: 1<<32 is
 * pid 0, the caller.
 * poly-diff: syscalls=names (pid arguments are not masked for this call)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3

static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

int test_main(void) {
    i64 r = getsched(0);
    check("start_other", r == SCHED_OTHER, r);
    r = getsched(getpid());
    check("own_pid_other", r == SCHED_OTHER, r);
    r = getsched(gettid());
    check("own_tid_other", r == SCHED_OTHER, r);
    r = getsched(1L << 32);
    check("high_bits_pid0_other", r == SCHED_OTHER, r);

    r = setsched(0, SCHED_BATCH, 0);
    check("set_batch", r == 0, r);
    r = getsched(0);
    check("now_batch", r == SCHED_BATCH, r);
    r = getsched(getpid());
    check("pid_batch", r == SCHED_BATCH, r);
    r = getsched((7L << 40) | getpid());
    check("high_bits_pid_batch", r == SCHED_BATCH, r);

    r = setsched(0, SCHED_OTHER, 0);
    check("set_other", r == 0, r);
    r = getsched(0);
    check("other_again", r == SCHED_OTHER, r);
    return finish();
}
