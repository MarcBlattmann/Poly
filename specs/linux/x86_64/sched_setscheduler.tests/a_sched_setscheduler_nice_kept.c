/* Switching between SCHED_OTHER, SCHED_BATCH and SCHED_IDLE keeps the
 * nice value (getpriority, which reports 20 - nice). The test only raises
 * its nice value, relative to where it started, clamped at 19. Runs in a
 * child. */
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
#define SYS_getpriority 140
#define SYS_setpriority 141

int test_main(void) {
    a_block_sigchld();
    i64 pid = a_fork_sync();
    if (pid == 0) {
        i64 start = 20 - syscall2(SYS_getpriority, 0, 0);
        i64 target = start + 3 > 19 ? 19 : start + 3;
        expect("to_batch", setsch(0, SCHED_BATCH, 0), 0);
        expect("batch_nice", 20 - syscall2(SYS_getpriority, 0, 0), start);
        expect("raise", syscall3(SYS_setpriority, 0, 0, target), 0);
        expect("to_other", setsch(0, SCHED_OTHER, 0), 0);
        expect("other_nice", 20 - syscall2(SYS_getpriority, 0, 0), target);
        expect("to_batch2", setsch(0, SCHED_BATCH, 0), 0);
        expect("batch2_nice", 20 - syscall2(SYS_getpriority, 0, 0), target);
        expect("to_idle", setsch(0, SCHED_IDLE, 0), 0);
        expect("idle_nice", 20 - syscall2(SYS_getpriority, 0, 0), target);
        expect("failed_call", setsch(0, SCHED_IDLE, 1), -EINVAL);
        expect("failed_nice", 20 - syscall2(SYS_getpriority, 0, 0), target);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_passed", st, 0);
    return finish();
}
