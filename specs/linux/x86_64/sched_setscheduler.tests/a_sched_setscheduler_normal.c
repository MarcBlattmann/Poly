/* sched_setscheduler moves the calling thread between SCHED_OTHER (0),
 * SCHED_BATCH (3) and SCHED_IDLE (5) with priority 0: it returns 0 (never
 * the previous policy), sched_getscheduler reports the new policy and
 * sched_getparam still reports 0. Setting the current policy again is
 * also 0. pid 0, getpid() and gettid() all name the caller. SCHED_IDLE is
 * entered last, in a child, because an unprivileged thread may not leave it. */
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
    i64 self = syscall0(SYS_getpid);
    i64 tid = syscall0(SYS_gettid);
    expect("start_other", getsch(0), SCHED_OTHER);
    expect("other_again", setsch(0, SCHED_OTHER, 0), 0);
    expect("to_batch", setsch(0, SCHED_BATCH, 0), 0);
    expect("is_batch", getsch(0), SCHED_BATCH);
    expect("is_batch_by_pid", getsch(self), SCHED_BATCH);
    expect("batch_prio", getprio(0), 0);
    expect("batch_again", setsch(self, SCHED_BATCH, 0), 0);
    expect("to_other_by_tid", setsch(tid, SCHED_OTHER, 0), 0);
    expect("is_other", getsch(0), SCHED_OTHER);
    expect("other_prio", getprio(0), 0);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("to_idle", setsch(0, SCHED_IDLE, 0), 0);
        expect("is_idle", getsch(0), SCHED_IDLE);
        expect("idle_prio", getprio(0), 0);
        expect("idle_again", setsch(0, SCHED_IDLE, 0), 0);
        expect("still_idle", getsch(0), SCHED_IDLE);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    expect("parent_still_other", getsch(0), SCHED_OTHER);
    return finish();
}
