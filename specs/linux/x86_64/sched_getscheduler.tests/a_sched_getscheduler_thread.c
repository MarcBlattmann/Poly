/* pid names a thread: the main thread reads a worker by its tid, the
 * worker reads itself with pid 0 and its tid and the main thread with the
 * process's pid. A worker that switches itself to SCHED_IDLE changes only
 * itself. Once the worker has ended its tid is ESRCH.
 * poly-diff: syscalls=off (the end of the worker is polled; two threads)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145

/* linux/sched.h */
#define SCHED_BATCH 3
#define SCHED_IDLE 5

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static volatile i32 ctid, stage;
static volatile i64 w_pid0, w_tid, w_main, w_set, w_after, w_main_after;
static i64 main_pid;

static int worker(void *arg) {
    (void)arg;
    i32 zero = 0;
    w_pid0 = syscall1(SYS_sched_getscheduler, 0);
    w_tid = syscall1(SYS_sched_getscheduler, syscall0(SYS_gettid));
    w_main = syscall1(SYS_sched_getscheduler, main_pid);
    w_set = syscall3(SYS_sched_setscheduler, 0, SCHED_IDLE, &zero);
    w_after = syscall1(SYS_sched_getscheduler, 0);
    w_main_after = syscall1(SYS_sched_getscheduler, main_pid);
    stage = 1;
    a_spin_until(&stage, 2, A_SPIN_LIMIT);
    return 0;
}

int test_main(void) {
    i32 zero = 0;
    main_pid = syscall0(SYS_getpid);
    expect("main_to_batch", syscall3(SYS_sched_setscheduler, 0, SCHED_BATCH, &zero), 0);
    ctid = -1;
    stage = 0;
    i32 tid = -1;
    a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID | CLONE_PARENT_SETTID, a_stacks[0] + A_STACK_SIZE, &tid, (i32 *)&ctid,
            0, worker, 0);
    check("tid_differs", tid > 0 && tid != main_pid, tid);
    int seen = a_spin_until(&stage, 1, A_SPIN_LIMIT);
    check("worker_ran", seen, stage);
    expect("worker_pid0", w_pid0, SCHED_BATCH);
    expect("worker_tid", w_tid, SCHED_BATCH);
    expect("worker_main", w_main, SCHED_BATCH);
    expect("worker_set_idle", w_set, 0);
    expect("worker_idle", w_after, SCHED_IDLE);
    expect("worker_main_after", w_main_after, SCHED_BATCH);
    expect("main_reads_worker", syscall1(SYS_sched_getscheduler, tid), SCHED_IDLE);
    expect("main_self", syscall1(SYS_sched_getscheduler, 0), SCHED_BATCH);
    expect("main_by_pid", syscall1(SYS_sched_getscheduler, main_pid), SCHED_BATCH);
    stage = 2;
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    /* The tid is cleared at the worker's exit; it may still be found for a
     * short moment after, so wait for ESRCH. */
    i64 r = 0;
    for (int i = 0; i < 100000; i++) {
        r = syscall1(SYS_sched_getscheduler, tid);
        if (r == -ESRCH) break;
        syscall0(SYS_sched_yield);
    }
    expect("ended_worker", r, -ESRCH);
    return finish();
}
