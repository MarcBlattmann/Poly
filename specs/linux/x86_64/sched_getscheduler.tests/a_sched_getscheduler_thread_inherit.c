/* A new thread starts with the policy of the thread that created it, but
 * without SCHED_RESET_ON_FORK: created by a SCHED_BATCH | RESET_ON_FORK
 * thread it reports SCHED_BATCH; created (in a child process) by a
 * SCHED_IDLE thread it reports SCHED_IDLE. The creator keeps its flag. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_BATCH 3
#define SCHED_IDLE 5
#define SCHED_RESET_ON_FORK 0x40000000

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid;
static volatile i64 seen_self;

static int worker(void *arg) {
    (void)arg;
    seen_self = syscall1(SYS_sched_getscheduler, 0);
    return 0;
}

static i64 run_thread(void) {
    ctid = -1;
    seen_self = -1000;
    i32 tid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID | CLONE_PARENT_SETTID, a_stacks[0] + A_STACK_SIZE, &tid, (i32 *)&ctid,
            0, worker, 0);
    if (!a_spin_until(&ctid, 0, A_SPIN_LIMIT)) return -2000;
    return seen_self;
}

int test_main(void) {
    a_block_sigchld();
    i32 zero = 0;
    expect("to_batch_rof", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH | SCHED_RESET_ON_FORK, &zero), 0);
    expect("thread_batch", run_thread(), SCHED_BATCH);
    expect("creator_keeps_flag", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH | SCHED_RESET_ON_FORK);
    i64 c = a_fork_sync();
    if (c == 0) {
        expect("child_to_idle", syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE, &zero), 0);
        expect("child_thread_idle", run_thread(), SCHED_IDLE);
        expect("child_still_idle", syscall1(SYS_sched_getscheduler, 0), SCHED_IDLE);
        syscall1(SYS_exit_group, failed ? 1 : 0);
    }
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, c, &st, 0, 0), c);
    check("child_ok", A_WIFEXITED(st) && A_WEXITSTATUS(st) == 0, st);
    return finish();
}
