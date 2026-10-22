/* getpriority(PRIO_PROCESS, 0): the caller's nice value as 20 - nice,
 * so 1..40 and never negative; who 0 and who = own pid name the same
 * thread; the value follows setpriority. The test starts with whatever
 * nice value it inherited (0 on a fresh system, more under some
 * runners), so it checks values relative to that.
 * poly-diff: syscalls=names (the pid is an argument)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
/* linux/resource.h */
#define PRIO_PROCESS 0
#define PRIO_PGRP 1
#define PRIO_USER 2

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

int test_main(void) {
    i64 g0 = getprio(PRIO_PROCESS, 0);
    check("in_range", g0 >= 1 && g0 <= 40, g0);
    check("start_nice_at_most_10", g0 >= 10, g0);
    i64 n0 = 20 - g0;
    i64 pid = syscall0(SYS_getpid);
    expect("own_pid_same", getprio(PRIO_PROCESS, pid), g0);
    expect("repeatable", getprio(PRIO_PROCESS, 0), g0);

    expect("set_plus_1", setprio(PRIO_PROCESS, 0, n0 + 1), 0);
    expect("get_plus_1", getprio(PRIO_PROCESS, 0), g0 - 1);
    expect("get_plus_1_by_pid", getprio(PRIO_PROCESS, pid), g0 - 1);
    expect("set_plus_4_by_pid", setprio(PRIO_PROCESS, pid, n0 + 4), 0);
    expect("get_plus_4", getprio(PRIO_PROCESS, 0), g0 - 4);
    /* Setting the value it already has changes nothing. */
    expect("set_same", setprio(PRIO_PROCESS, 0, n0 + 4), 0);
    expect("get_same", getprio(PRIO_PROCESS, 0), g0 - 4);
    return finish();
}
