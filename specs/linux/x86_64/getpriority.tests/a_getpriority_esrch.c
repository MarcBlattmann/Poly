/* getpriority: ESRCH when which and who name no thread: a pid, a process
 * group or a uid nobody has; -1 for each kind; the pid of a child that
 * has been reaped; a process group id that is the pid of a process
 * that is not a group leader.
 * poly-diff: syscalls=off (fork; pids are arguments)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }

static int body(void) {
    expect("pid_unused", getprio(PRIO_PROCESS, 0x7fffffff), -ESRCH);
    expect("pid_minus_1", getprio(PRIO_PROCESS, -1), -ESRCH);
    expect("pid_int_min", getprio(PRIO_PROCESS, -0x80000000L), -ESRCH);
    expect("pgrp_unused", getprio(PRIO_PGRP, 0x7fffffff), -ESRCH);
    expect("pgrp_minus_1", getprio(PRIO_PGRP, -1), -ESRCH);
    expect("user_unused", getprio(PRIO_USER, 4000000), -ESRCH);
    expect("user_minus_1", getprio(PRIO_USER, -1), -ESRCH);

    /* A child in our process group: its pid is no process group. */
    volatile i32 *sh = a_shared();
    sh[0] = 0;
    sh[1] = 0;
    i64 pid = syscall2(SYS_fork, 0, 0);
    if (pid == 0) {
        sh[0] = 1;
        a_spin_until(&sh[1], 1, A_SPIN_LIMIT);
        syscall1(SYS_exit_group, 0);
    }
    check("fork_ok", pid > 0, pid);
    check("child_started", a_spin_until(&sh[0], 1, A_SPIN_LIMIT), sh[0]);
    i64 r = getprio(PRIO_PROCESS, pid);
    check("child_found", r >= 1 && r <= 40, r);
    expect("child_pid_not_a_pgrp", getprio(PRIO_PGRP, pid), -ESRCH);
    sh[1] = 1;
    i32 st = 0;
    expect("reaped", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("reaped_child_esrch", getprio(PRIO_PROCESS, pid), -ESRCH);
    return finish();
}

int test_main(void) { return a_in_session(body); }
