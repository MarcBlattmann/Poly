/* A child forked by a SCHED_FIFO process reports the parent's priority for
 * itself and for the parent; with SCHED_RESET_ON_FORK set the child is
 * back at SCHED_OTHER and reports 0 while the parent still reports its
 * own priority. The parent reads the child's value while it is a zombie.
 * poly-diff: needs=root (only a privileged process may become real-time)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144

/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_RESET_ON_FORK 0x40000000

static i64 getp(i64 pid) {
    i32 prio = 0x55;
    i64 r = syscall2(SYS_sched_getparam, pid, &prio);
    return r < 0 ? r : prio;
}

/* Fork a child that checks its own and its parent's priority; returns the
 * child's pid (not yet reaped), or -1. */
static i64 run_child(i64 want_self, i64 want_parent) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("child_self", getp(0), want_self);
        expect("child_parent", getp(syscall0(SYS_getppid)), want_parent);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    return pid;
}

static void reap(const char *name, i64 pid) {
    i32 st = -1;
    i64 r = syscall4(SYS_wait4, pid, &st, 0, 0);
    check(name, r == pid && A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
}

int test_main(void) {
    a_block_sigchld();
    i32 p = 40;
    expect("to_fifo_40", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &p), 0);
    i64 c1 = run_child(40, 40);
    char info[128];
    expect("zombie_wait", syscall5(SYS_waitid, P_PID, c1, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_40", getp(c1), 40);
    reap("child_inherits", c1);
    p = 25;
    expect("to_fifo_rof_25", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO | SCHED_RESET_ON_FORK, &p), 0);
    i64 c2 = run_child(0, 25);
    expect("zombie_rof_wait", syscall5(SYS_waitid, P_PID, c2, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_0", getp(c2), 0);
    reap("child_reset", c2);
    expect("parent_25", getp(0), 25);
    return finish();
}
