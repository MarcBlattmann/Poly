/* setpriority errors: a which other than 0, 1, 2 is EINVAL before who
 * is looked at (also with a who nothing has); nothing found is ESRCH: a
 * pid, process group or uid no task has, a negative who (-1 is not
 * special), the pid of a process that is not a group leader asked as a
 * group, and a reaped child. A failed call changes nothing. which and
 * who are 32-bit ints (upper halves ignored).
 * poly-diff: syscalls=off (a child process; its pid is an argument)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2
/* An id no task, group or user has: above PID_MAX_LIMIT (linux/threads.h). */
#define NO_ID 0x3ffffff0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

int test_main(void) {
    i64 r = setprio(3, 0, 1);
    check("which_3_einval", r == -EINVAL, r);
    r = setprio(-1, 0, 1);
    check("which_minus1_einval", r == -EINVAL, r);
    r = setprio(3, NO_ID, 1);
    check("einval_beats_esrch", r == -EINVAL, r);
    r = setprio((1L << 32) | 3, 0, 1);
    check("high_bits_which_3_einval", r == -EINVAL, r);
    r = setprio(PRIO_PROCESS, NO_ID, 1);
    check("process_esrch", r == -ESRCH, r);
    r = setprio(PRIO_PROCESS, -1, 1);
    check("process_minus1_esrch", r == -ESRCH, r);
    r = setprio(PRIO_PGRP, NO_ID, 1);
    check("pgrp_esrch", r == -ESRCH, r);
    r = setprio(PRIO_USER, NO_ID, 1);
    check("user_esrch", r == -ESRCH, r);
    r = setprio(PRIO_USER, -1, 1);
    check("user_minus1_esrch", r == -ESRCH, r);
    r = getprio(PRIO_PROCESS, 0);
    check("unchanged_20", r == 20, r);

    volatile unsigned *g = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(getprio(PRIO_PROCESS, 0) == 16 ? 0 : 1);
    }
    gate_wait(&g[0]);
    r = setprio(PRIO_PGRP, pid, 1);
    check("non_leader_pgrp_esrch", r == -ESRCH, r);
    r = setprio((7L << 32) | PRIO_PROCESS, (1L << 33) | pid, 4);
    check("child_nice_4_high_bits", r == 0, r);
    gate_open(&g[1]);
    int code = wait_exit(pid);
    check("child_saw_4", code == 0, code);
    r = setprio(PRIO_PROCESS, pid, 1);
    check("reaped_esrch", r == -ESRCH, r);
    return finish();
}
