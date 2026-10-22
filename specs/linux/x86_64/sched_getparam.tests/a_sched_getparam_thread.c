/* pid may be the tid of another thread of the process. The main thread
 * reads a worker's parameters by tid, the worker reads its own with pid 0,
 * its tid and the main thread's pid; all are 0 under SCHED_OTHER and
 * SCHED_BATCH. */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_BATCH 3

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid, stage;
static volatile i64 own0, own_tid, own_main, own_tid_val;
static volatile i64 main_pid;

static i64 getp(i64 pid) {
    i32 prio = 0x55;
    i64 r = syscall2(SYS_sched_getparam, pid, &prio);
    return r < 0 ? r : prio;
}

static int worker(void *arg) {
    (void)arg;
    a_spin_until(&stage, 1, A_SPIN_LIMIT);
    own0 = getp(0);
    own_tid = getp(syscall0(SYS_gettid));
    own_main = getp(main_pid);
    stage = 2;
    a_spin_until(&stage, 3, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    ctid = -1;
    stage = 0;
    main_pid = syscall0(SYS_getpid);
    i32 tid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID | CLONE_PARENT_SETTID, a_stacks[0] + A_STACK_SIZE, &tid, (i32 *)&ctid,
            0, worker, 0);
    check("tid_differs", tid > 0 && tid != main_pid, tid);
    expect("thread", getp(tid), 0);
    i32 zero = 0;
    expect("thread_to_batch", syscall3(SYS_sched_setscheduler, tid, SCHED_BATCH, &zero), 0);
    expect("thread_batch", getp(tid), 0);
    expect("thread_bad_ptr", syscall2(SYS_sched_getparam, tid, 1), -EFAULT);
    stage = 1;
    int seen = a_spin_until(&stage, 2, A_SPIN_LIMIT);
    check("worker_ran", seen, stage);
    expect("own_pid0", own0, 0);
    expect("own_tid", own_tid, 0);
    expect("own_main", own_main, 0);
    expect("main_still_other", syscall1(SYS_sched_getscheduler, 0), 0);
    stage = 3;
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    return finish();
}
