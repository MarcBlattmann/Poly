/* An unprivileged real-time thread may lower its priority freely, keep it,
 * or raise it up to RLIMIT_RTPRIO; raising above both its current
 * priority and the limit is EPERM (1). Invalid priorities are still EINVAL
 * (22). A root child becomes SCHED_FIFO 50, sets RLIMIT_RTPRIO to 30 and
 * drops to uid 65534.
 * poly-diff: needs=root (only a privileged process may become real-time)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
#define SYS_setresuid 117

/* linux/sched.h */
#define SCHED_FIFO 1

static i64 setp(i32 v) {
    i32 prio = v;
    return syscall2(SYS_sched_setparam, 0, &prio);
}

static i64 getp(void) {
    i32 prio = -1;
    i64 r = syscall2(SYS_sched_getparam, 0, &prio);
    return r < 0 ? r : prio;
}

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i32 fifty = 50;
        struct a_rlimit { u64 cur, max; } lim = {30, 30};
        expect("to_fifo_50", syscall3(SYS_sched_setscheduler, 0, SCHED_FIFO, &fifty), 0);
        expect("rtprio_limit", syscall2(SYS_setrlimit, RLIMIT_RTPRIO, &lim), 0);
        expect("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
        expect("lower_40", setp(40), 0);
        expect("get_40", getp(), 40);
        expect("same_40", setp(40), 0);
        expect("raise_45", setp(45), -EPERM);
        expect("get_still_40", getp(), 40);
        expect("lower_20", setp(20), 0);
        expect("raise_to_limit_30", setp(30), 0);
        expect("get_30", getp(), 30);
        expect("raise_31", setp(31), -EPERM);
        expect("invalid_0", setp(0), -EINVAL);
        expect("invalid_100", setp(100), -EINVAL);
        expect("policy_fifo", syscall1(SYS_sched_getscheduler, 0), SCHED_FIFO);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    return finish();
}
