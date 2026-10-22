/* An unprivileged SCHED_IDLE thread may leave SCHED_IDLE when its
 * RLIMIT_NICE soft limit allows its nice value (20 - nice <= limit): at
 * nice 0 a limit of 20 allows it, a limit of 10 does not (EPERM, 1) until
 * the nice value is raised to 10. A root child sets the limit and drops to
 * uid 65534.
 * poly-diff: needs=root (only root may raise RLIMIT_NICE)
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
#define SYS_setpriority 141

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        struct { u64 cur, max; } lim = {20, 20};
        expect("nice_0", syscall3(SYS_setpriority, 0, 0, 0), 0);
        expect("nicelim_20", syscall2(SYS_setrlimit, RLIMIT_NICE, &lim), 0);
        expect("drop_uid", syscall3(SYS_setresuid, 65534, 65534, 65534), 0);
        expect("to_idle", setsch(0, SCHED_IDLE, 0), 0);
        expect("leave_to_other", setsch(0, SCHED_OTHER, 0), 0);
        expect("is_other", getsch(0), SCHED_OTHER);
        lim.cur = 10;
        expect("nicelim_10", syscall2(SYS_setrlimit, RLIMIT_NICE, &lim), 0);
        expect("to_idle2", setsch(0, SCHED_IDLE, 0), 0);
        expect("leave_denied", setsch(0, SCHED_BATCH, 0), -EPERM);
        expect("still_idle", getsch(0), SCHED_IDLE);
        expect("nice_10", syscall3(SYS_setpriority, 0, 0, 10), 0);
        expect("leave_to_batch", setsch(0, SCHED_BATCH, 0), 0);
        expect("is_batch", getsch(0), SCHED_BATCH);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    return finish();
}
