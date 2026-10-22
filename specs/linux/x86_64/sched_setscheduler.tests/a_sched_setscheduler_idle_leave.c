/* Leaving SCHED_IDLE needs privilege or an RLIMIT_NICE that allows the
 * thread's nice value (20 - nice <= RLIMIT_NICE). With RLIMIT_NICE lowered
 * to 0 an unprivileged thread gets EPERM (1) for SCHED_OTHER, SCHED_BATCH
 * and SCHED_FIFO; staying SCHED_IDLE (also adding SCHED_RESET_ON_FORK)
 * works; invalid arguments are still EINVAL (22). A privileged caller
 * (uid 0 here) may leave. Runs in a child. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setparam 142
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_FIFO 1
#define SCHED_RR 2
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_DEADLINE 6
#define SCHED_RESET_ON_FORK 0x40000000

static A_UNUSED i64 setsch(i64 pid, i64 policy, i32 prio) {
    i32 p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

static A_UNUSED i64 getsch(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }

static A_UNUSED i64 getprio(i64 pid) {
    i32 p = -1;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct { u64 cur, max; } lim = {0, 0};
        int priv = syscall0(SYS_geteuid) == 0;
        expect("nice_lim_0", syscall2(SYS_setrlimit, RLIMIT_NICE, &lim), 0);
        expect("rtprio_lim_0", syscall2(SYS_setrlimit, RLIMIT_RTPRIO, &lim), 0);
        expect("to_idle", setsch(0, SCHED_IDLE, 0), 0);
        expect("idle_again", setsch(0, SCHED_IDLE, 0), 0);
        expect("idle_bad_prio", setsch(0, SCHED_IDLE, 1), -EINVAL);
        expect("bad_policy", setsch(0, 8, 0), -EINVAL);
        expect("fifo_0", setsch(0, SCHED_FIFO, 0), -EINVAL);
        expect("idle_reset", setsch(0, SCHED_IDLE | SCHED_RESET_ON_FORK, 0), 0);
        expect("idle_reset_shown", getsch(0), SCHED_IDLE | SCHED_RESET_ON_FORK);
        expect("to_batch", setsch(0, SCHED_BATCH | SCHED_RESET_ON_FORK, 0), priv ? 0 : -EPERM);
        expect("after_batch", getsch(0), (priv ? SCHED_BATCH : SCHED_IDLE) | SCHED_RESET_ON_FORK);
        expect("back_idle", setsch(0, SCHED_IDLE | SCHED_RESET_ON_FORK, 0), 0);
        expect("to_other", setsch(0, SCHED_OTHER | SCHED_RESET_ON_FORK, 0), priv ? 0 : -EPERM);
        expect("back_idle2", setsch(0, SCHED_IDLE | SCHED_RESET_ON_FORK, 0), 0);
        expect("to_fifo", setsch(0, SCHED_FIFO | SCHED_RESET_ON_FORK, 5), priv ? 0 : -EPERM);
        expect("end", getsch(0), (priv ? SCHED_FIFO : SCHED_IDLE) | SCHED_RESET_ON_FORK);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    expect("parent_other", getsch(0), SCHED_OTHER);
    return finish();
}
