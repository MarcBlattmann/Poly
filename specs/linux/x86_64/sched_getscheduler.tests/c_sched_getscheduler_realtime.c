/* Real-time policies as sched_getscheduler reports them: SCHED_FIFO (1),
 * SCHED_RR (2), and SCHED_RR with SCHED_RESET_ON_FORK as 0x40000002.
 * A fork child of a real-time parent with the flag runs SCHED_OTHER;
 * without the flag it inherits SCHED_FIFO. All of it in a child, which
 * the parent also observes from outside.
 * poly-diff: needs=root (only a privileged caller may choose a real-time policy without RLIMIT_RTPRIO)
 * poly-diff: syscalls=off (child processes)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_RESET_ON_FORK 0x40000000

static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

int test_main(void) {
    volatile unsigned *g = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        int bad = 0;
        if (setsched(0, SCHED_FIFO, 10) != 0 || getsched(0) != SCHED_FIFO) bad |= 1;
        i64 c = fork();
        if (c == 0) exit_group(getsched(0) == SCHED_FIFO ? 0 : 1);
        if (wait_exit(c) != 0) bad |= 2;
        if (setsched(0, SCHED_RR, 20) != 0 || getsched(0) != SCHED_RR) bad |= 4;
        if (setsched(0, SCHED_RR | SCHED_RESET_ON_FORK, 20) != 0 ||
            getsched(0) != (SCHED_RR | SCHED_RESET_ON_FORK))
            bad |= 8;
        c = fork();
        if (c == 0) exit_group(getsched(0) == SCHED_OTHER ? 0 : 1);
        if (wait_exit(c) != 0) bad |= 16;
        gate_open(&g[0]);
        gate_wait(&g[1]);
        if (setsched(0, SCHED_OTHER, 0) != 0 || getsched(0) != SCHED_OTHER) bad |= 32;
        exit_group(bad);
    }
    gate_wait(&g[0]);
    i64 r = getsched(pid);
    check("parent_sees_rr_reset", r == (SCHED_RR | SCHED_RESET_ON_FORK), r);
    gate_open(&g[1]);
    int code = wait_exit(pid);
    check("child_steps", code == 0, code);
    r = getsched(0);
    check("parent_other", r == SCHED_OTHER, r);
    return finish();
}
