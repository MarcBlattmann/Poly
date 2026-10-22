/* Without CAP_SYS_NICE, a nonzero RLIMIT_RTPRIO soft limit allows real-time
 * policies with priorities up to the limit; above both the limit and the
 * current priority is EPERM (1). With the limit back at 0 the thread may
 * lower its priority within its policy or switch to a normal policy, but
 * may not switch to another real-time policy (EPERM). A root child sets
 * RLIMIT_RTPRIO to 30 and drops to uid 65534.
 * poly-diff: needs=root (dropping privilege needs root)
 */
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

/* asm/unistd_64.h */
#define SYS_setresuid 117

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct { u64 cur, max; } lim = {30, 30};
        expect("rtprio_30", syscall2(SYS_setrlimit, RLIMIT_RTPRIO, &lim), 0);
        expect("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
        expect("fifo_31", setsch(0, SCHED_FIFO, 31), -EPERM);
        expect("still_other", getsch(0), SCHED_OTHER);
        expect("fifo_30", setsch(0, SCHED_FIFO, 30), 0);
        expect("fifo_30_prio", getprio(0), 30);
        expect("rr_20", setsch(0, SCHED_RR, 20), 0);
        expect("rr_20_seen", getsch(0), SCHED_RR);
        expect("rr_31", setsch(0, SCHED_RR, 31), -EPERM);
        expect("rr_100", setsch(0, SCHED_RR, 100), -EINVAL);
        expect("rr_prio_kept", getprio(0), 20);
        lim.cur = 0;
        lim.max = 0;
        expect("rtprio_0", syscall2(SYS_setrlimit, RLIMIT_RTPRIO, &lim), 0);
        expect("rr_lower_10", setsch(0, SCHED_RR, 10), 0);
        expect("rr_same_10", setsch(0, SCHED_RR, 10), 0);
        expect("rr_raise_11", setsch(0, SCHED_RR, 11), -EPERM);
        expect("to_fifo_5", setsch(0, SCHED_FIFO, 5), -EPERM);
        expect("still_rr_10", getprio(0), 10);
        expect("to_batch", setsch(0, SCHED_BATCH, 0), 0);
        expect("batch_seen", getsch(0), SCHED_BATCH);
        expect("batch_prio", getprio(0), 0);
        expect("fifo_again", setsch(0, SCHED_FIFO, 1), -EPERM);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    return finish();
}
