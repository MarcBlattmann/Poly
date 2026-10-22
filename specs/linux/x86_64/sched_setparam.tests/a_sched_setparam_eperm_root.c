/* An unprivileged process may not set the parameters of a process owned
 * by another user: EPERM (1). The priority is checked first (an invalid one
 * is EINVAL even then), and NULL/bad pointers come before both. Its own
 * parameters it may still set. A root parent forks a child that drops to
 * uid 65534 and targets the parent.
 * poly-diff: needs=root (switching to another uid needs root)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_setresuid 117

int test_main(void) {
    a_block_sigchld();
    i64 parent = syscall0(SYS_getpid);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i32 zero = 0, five = 5;
        expect("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
        expect("parent_prio_0", syscall2(SYS_sched_setparam, parent, &zero), -EPERM);
        expect("parent_prio_5", syscall2(SYS_sched_setparam, parent, &five), -EINVAL);
        expect("parent_null", syscall2(SYS_sched_setparam, parent, 0), -EINVAL);
        expect("parent_bad_ptr", syscall2(SYS_sched_setparam, parent, 1), -EFAULT);
        expect("self_prio_0", syscall2(SYS_sched_setparam, 0, &zero), 0);
        expect("self_by_pid", syscall2(SYS_sched_setparam, syscall0(SYS_getpid), &zero), 0);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    i32 zero = 0;
    expect("root_self", syscall2(SYS_sched_setparam, 0, &zero), 0);
    return finish();
}
