/* A child made by fork inherits the normal policy: SCHED_BATCH and
 * SCHED_IDLE stay, also when the parent has SCHED_RESET_ON_FORK set (the
 * flag only resets real-time policies and negative nice values). The flag
 * itself is not inherited: the child sees the bare policy and may switch
 * policy freely, while the parent keeps the flag. */
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
    i32 st;
    expect("batch", setsch(0, SCHED_BATCH, 0), 0);
    i64 pid = a_fork_sync();
    if (pid == 0) {
        expect("c1_batch", getsch(0), SCHED_BATCH);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    st = -1;
    expect("reap1", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child1_passed", st, 0);
    expect("batch_reset", setsch(0, SCHED_BATCH | SCHED_RESET_ON_FORK, 0), 0);
    pid = a_fork_sync();
    if (pid == 0) {
        expect("c2_batch_no_flag", getsch(0), SCHED_BATCH);
        expect("c2_to_other", setsch(0, SCHED_OTHER, 0), 0);
        expect("c2_other", getsch(0), SCHED_OTHER);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    st = -1;
    expect("reap2", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child2_passed", st, 0);
    expect("parent_keeps_flag", getsch(0), SCHED_BATCH | SCHED_RESET_ON_FORK);
    expect("idle_reset", setsch(0, SCHED_IDLE | SCHED_RESET_ON_FORK, 0), 0);
    pid = a_fork_sync();
    if (pid == 0) {
        expect("c3_idle_no_flag", getsch(0), SCHED_IDLE);
        expect("c3_prio", getprio(0), 0);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    st = -1;
    expect("reap3", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child3_passed", st, 0);
    expect("parent_idle_flag", getsch(0), SCHED_IDLE | SCHED_RESET_ON_FORK);
    return finish();
}
