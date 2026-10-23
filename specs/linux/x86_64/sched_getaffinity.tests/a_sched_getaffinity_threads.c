/* The mask is per thread: sched_getaffinity(tid) reads that thread's
 * mask, pid 0 inside a thread reads the calling thread, and the pid of
 * the process (or 0 in the main thread) reads the main thread. A new
 * thread starts with its creator's mask.
 * poly-diff: syscalls=off (two threads' calls interleave by scheduling)
 */
#include "a_sys.h"

/* asm/unistd_64.h */
#define SYS_sched_setaffinity 203
#define SYS_sched_getaffinity 204

#define THREAD_FLAGS (CLONE_VM | CLONE_FS | CLONE_FILES | CLONE_SIGHAND | CLONE_THREAD | CLONE_SYSVSEM)

static u64 mask_of(i64 pid) {
    u64 m[2] = {0, 0};
    i64 r = syscall3(SYS_sched_getaffinity, pid, sizeof m, m);
    return r == 8 ? m[0] : 0;
}

static u64 highest(u64 m) {
    u64 h = 0;
    for (; m; m &= m - 1) h = m & -m;
    return h;
}

static volatile i32 ctid, step;
static volatile u64 t_first, t_own, t_main;
static volatile i64 t_tid;

static int worker(void *arg) {
    (void)arg;
    t_tid = syscall0(SYS_gettid);
    t_first = mask_of(0);
    step = 1;
    while (step != 2) a_relax();
    t_own = mask_of(0);
    t_main = mask_of(syscall0(SYS_getpid));
    step = 3;
    while (step != 4) a_relax();
    return 0;
}

int test_main(void) {
    u64 start = mask_of(0);
    u64 lo = start & -start, hi = highest(start);
    i64 pid = syscall0(SYS_getpid);
    u64 v = lo;
    expect("main_set_lowest", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);

    ctid = -1;
    long tid = a_clone(THREAD_FLAGS | CLONE_CHILD_CLEARTID, a_stacks[0] + A_STACK_SIZE, 0, (i32 *)&ctid, 0, worker, 0);
    check("thread_started", tid > 0, tid);
    a_spin_until(&step, 1, A_SPIN_LIMIT);
    check("thread_tid", t_tid == tid, t_tid);
    check("thread_inherits", t_first == lo, 0);

    v = hi;
    expect("set_thread", syscall3(SYS_sched_setaffinity, tid, 8, &v), 0);
    check("thread_by_tid", mask_of(tid) == hi, 0);
    check("main_by_0", mask_of(0) == lo, 0);
    check("main_by_pid", mask_of(pid) == lo, 0);
    step = 2;
    a_spin_until(&step, 3, A_SPIN_LIMIT);
    check("thread_by_0_in_thread", t_own == hi, 0);
    check("main_by_pid_in_thread", t_main == lo, 0);

    step = 4;
    a_spin_until(&ctid, 0, A_SPIN_LIMIT);
    v = start;
    expect("restore", syscall3(SYS_sched_setaffinity, 0, 8, &v), 0);
    return finish();
}
