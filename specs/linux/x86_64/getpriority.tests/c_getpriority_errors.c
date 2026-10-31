/* getpriority errors: a which other than PRIO_PROCESS (0), PRIO_PGRP (1)
 * or PRIO_USER (2) is EINVAL, checked before who (EINVAL even with a
 * who nothing has). Nothing found is ESRCH: a pid or process group no
 * task has, a negative who (-1 is not special, ESRCH for every which),
 * a uid no process runs as, and the pid of a process that is not a
 * process group leader asked as a group.
 * poly-diff: syscalls=off (a child process; its pid is an argument)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2
/* An id no task, group or user has: above PID_MAX_LIMIT (linux/threads.h). */
#define NO_ID 0x3ffffff0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }

int test_main(void) {
    i64 r = getprio(3, 0);
    check("which_3_einval", r == -EINVAL, r);
    r = getprio(-1, 0);
    check("which_minus1_einval", r == -EINVAL, r);
    r = getprio(0x7fffffff, 0);
    check("which_int_max_einval", r == -EINVAL, r);
    r = getprio(3, NO_ID);
    check("einval_beats_esrch", r == -EINVAL, r);

    r = getprio(PRIO_PROCESS, NO_ID);
    check("process_esrch", r == -ESRCH, r);
    r = getprio(PRIO_PROCESS, -1);
    check("process_minus1_esrch", r == -ESRCH, r);
    r = getprio(PRIO_PGRP, NO_ID);
    check("pgrp_esrch", r == -ESRCH, r);
    r = getprio(PRIO_PGRP, -1);
    check("pgrp_minus1_esrch", r == -ESRCH, r);
    r = getprio(PRIO_USER, -1);
    check("user_minus1_esrch", r == -ESRCH, r);
    r = getprio(PRIO_USER, NO_ID);
    check("user_unused_esrch", r == -ESRCH, r);

    /* A child stays in the parent's group: its pid is no group id. */
    volatile unsigned *g = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(0);
    }
    gate_wait(&g[0]);
    r = getprio(PRIO_PGRP, pid);
    check("non_leader_pgrp_esrch", r == -ESRCH, r);
    r = getprio(PRIO_PROCESS, pid);
    check("child_process_20", r == 20, r);
    gate_open(&g[1]);
    check("child_done", wait_exit(pid) == 0, 0);
    r = getprio(PRIO_PROCESS, pid);
    check("reaped_esrch", r == -ESRCH, r);
    return finish();
}
