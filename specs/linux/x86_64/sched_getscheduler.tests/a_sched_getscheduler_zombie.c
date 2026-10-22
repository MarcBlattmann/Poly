/* A child that has exited but is not yet reaped is still found and reports
 * the policy it had when it exited (SCHED_IDLE | SCHED_RESET_ON_FORK, set
 * by the child itself); after waitid with WNOWAIT it is still there, after
 * wait4 it is gone (ESRCH). */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000

int test_main(void) {
    a_block_sigchld();
    i64 c = a_fork_sync();
    if (c == 0) {
        i32 zero = 0;
        i64 r = syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE | SCHED_RESET_ON_FORK, &zero);
        syscall1(SYS_exit_group, r == 0 ? 0 : 1);
    }
    check("forked", c > 0, c);
    expect("zombie", syscall1(SYS_sched_getscheduler, c), 0x40000005);
    char info[128];
    expect("waitid_nowait", syscall5(SYS_waitid, P_PID, c, info, WEXITED | WNOWAIT, 0), 0);
    expect("zombie_after_nowait", syscall1(SYS_sched_getscheduler, c), 0x40000005);
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, c, &st, 0, 0), c);
    check("child_ok", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    expect("reaped", syscall1(SYS_sched_getscheduler, c), -ESRCH);
    expect("self_other", syscall1(SYS_sched_getscheduler, 0), 0);
    return finish();
}
