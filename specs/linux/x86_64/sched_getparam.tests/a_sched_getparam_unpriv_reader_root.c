/* Reading needs no permission: a child that has dropped to uid/gid 65534
 * still reads its root parent's SCHED_RR priority, and the parent reads
 * the unprivileged child's. Setting the parent's is EPERM for the child,
 * and the value read afterwards is unchanged.
 * poly-diff: needs=root (only root may become real-time and change uid)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_setresuid 117
#define SYS_setresgid 119
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_RR 2

static i64 getp(i64 pid) {
    i32 prio = 0x55;
    i64 r = syscall2(SYS_sched_getparam, pid, &prio);
    return r < 0 ? r : prio;
}

int test_main(void) {
    a_block_sigchld();
    i32 p = 33;
    expect("to_rr_33", syscall3(SYS_sched_setscheduler, 0, SCHED_RR, &p), 0);
    i64 parent = syscall0(SYS_getpid);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i32 zero = 0, five = 5;
        /* Back to SCHED_OTHER first, while still root. */
        expect("child_to_other", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, &zero), 0);
        expect("drop_gid", syscall3(SYS_setresgid, 65534, 65534, 65534), 0);
        expect("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
        expect("child_reads_parent", getp(parent), 33);
        expect("child_self", getp(0), 0);
        expect("child_sets_parent", syscall2(SYS_sched_setparam, parent, &five), -EPERM);
        expect("child_parent_unchanged", getp(parent), 33);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    char info[128];
    expect("zombie_wait", syscall5(SYS_waitid, P_PID, pid, info, WEXITED | WNOWAIT, 0), 0);
    expect("parent_reads_child", getp(pid), 0);
    i32 st = -1;
    i64 r = syscall4(SYS_wait4, pid, &st, 0, 0);
    check("child_ok", r == pid && A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    expect("parent_33", getp(0), 33);
    return finish();
}
