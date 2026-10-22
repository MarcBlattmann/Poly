/* A child forked by a SCHED_FIFO process reports SCHED_FIFO for itself and
 * its parent; if the parent had SCHED_RESET_ON_FORK set, the child starts
 * as SCHED_OTHER (0) while the parent still reports
 * SCHED_FIFO | SCHED_RESET_ON_FORK. The parent reads each zombie child.
 * poly-diff: needs=root (only a privileged process may become real-time)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_RESET_ON_FORK 0x40000000

static i64 run_child(i64 want_self, i64 want_parent) {
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("child_self", syscall1(SYS_sched_getscheduler, 0), want_self);
        expect("child_parent", syscall1(SYS_sched_getscheduler, syscall0(SYS_getppid)), want_parent);
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
    i32 p = 30;
    expect("to_fifo", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &p), 0);
    i64 c1 = run_child(SCHED_FIFO, SCHED_FIFO);
    expect("zombie_fifo", syscall1(SYS_sched_getscheduler, c1), SCHED_FIFO);
    reap("child_inherits", c1);
    expect("to_rr_rof", syscall3(SYS_sched_setscheduler, 0, SCHED_RR | SCHED_RESET_ON_FORK, &p), 0);
    i64 c2 = run_child(0, SCHED_RR | SCHED_RESET_ON_FORK);
    expect("zombie_other", syscall1(SYS_sched_getscheduler, c2), 0);
    reap("child_reset", c2);
    expect("parent_rr_rof", syscall1(SYS_sched_getscheduler, 0), SCHED_RR | SCHED_RESET_ON_FORK);
    return finish();
}
