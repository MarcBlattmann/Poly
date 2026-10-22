/* The value is the policy last set with sched_setscheduler: SCHED_BATCH (3),
 * SCHED_OTHER (0) again, and SCHED_IDLE (5). SCHED_IDLE is entered in a
 * child (an unprivileged thread may not leave it); the parent reads the
 * child's policy by pid while the child waits. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5

static volatile i32 done, ready, release;
static volatile i64 child_set, child_self;

static int idle_child(void *arg) {
    (void)arg;
    i32 zero = 0;
    child_set = syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE, &zero);
    child_self = syscall1(SYS_sched_getscheduler, 0);
    ready = 1;
    a_spin_until(&release, 1, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    a_block_sigchld();
    i32 zero = 0;
    expect("start_other", syscall1(SYS_sched_getscheduler, 0), SCHED_OTHER);
    expect("to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    expect("batch", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    expect("batch_again", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    expect("batch_still", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    expect("to_other", syscall3(SYS_sched_setscheduler, 0, SCHED_OTHER, &zero), 0);
    expect("other", syscall1(SYS_sched_getscheduler, 0), SCHED_OTHER);

    long pid = a_spawn(0, &done, idle_child, 0);
    check("spawned", pid > 0, pid);
    int seen = a_spin_until(&ready, 1, A_SPIN_LIMIT);
    check("child_ready", seen, ready);
    expect("parent_sees_idle", syscall1(SYS_sched_getscheduler, pid), SCHED_IDLE);
    release = 1;
    a_spin_until(&done, 0, A_SPIN_LIMIT);
    i32 st = -1;
    expect("reap", syscall4(SYS_wait4, pid, &st, 0, 0), pid);
    expect("child_set_idle", child_set, 0);
    expect("child_self_idle", child_self, SCHED_IDLE);
    expect("parent_other", syscall1(SYS_sched_getscheduler, 0), SCHED_OTHER);
    return finish();
}
