/* The policy is per thread: pid may be the tid of another thread, which
 * alone changes. A thread may change its own policy with pid 0, which
 * leaves the main thread alone. A thread created later inherits the
 * creating thread's policy. */
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

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid1, ctid2, stage;
static volatile i64 w_before, w_set, w_after, w2_policy;

static int worker(void *arg) {
    (void)arg;
    a_spin_until(&stage, 1, A_SPIN_LIMIT);
    w_before = getsch(0);
    w_set = setsch(0, SCHED_IDLE, 0);
    w_after = getsch(0);
    stage = 2;
    a_spin_until(&stage, 3, A_SPIN_LIMIT);
    return 0;
}

static int worker2(void *arg) {
    (void)arg;
    w2_policy = getsch(0);
    return 0;
}

int test_main(void) {
    ctid1 = -1;
    stage = 0;
    i32 tid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID | CLONE_PARENT_SETTID, a_stacks[0] + A_STACK_SIZE, &tid,
            (i32 *)&ctid1, 0, worker, 0);
    expect("thread_to_batch", setsch(tid, SCHED_BATCH, 0), 0);
    expect("thread_is_batch", getsch(tid), SCHED_BATCH);
    expect("main_other", getsch(0), SCHED_OTHER);
    expect("main_by_pid_other", getsch(syscall0(SYS_getpid)), SCHED_OTHER);
    stage = 1;
    check("worker_ran", a_spin_until(&stage, 2, A_SPIN_LIMIT), stage);
    expect("worker_saw_batch", w_before, SCHED_BATCH);
    expect("worker_own_idle", w_set, 0);
    expect("worker_now_idle", w_after, SCHED_IDLE);
    expect("thread_seen_idle", getsch(tid), SCHED_IDLE);
    expect("main_still_other", getsch(0), SCHED_OTHER);
    stage = 3;
    a_spin_until(&ctid1, 0, A_SPIN_LIMIT);
    expect("main_batch", setsch(0, SCHED_BATCH, 0), 0);
    ctid2 = -1;
    w2_policy = -100;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[1] + A_STACK_SIZE, 0, (i32 *)&ctid2, 0, worker2, 0);
    a_spin_until(&ctid2, 0, A_SPIN_LIMIT);
    expect("new_thread_batch", w2_policy, SCHED_BATCH);
    return finish();
}
