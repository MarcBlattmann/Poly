/* The policy is per thread: sched_setscheduler(tid, SCHED_BATCH) from
 * the main thread changes the second thread only, and the thread's own
 * call with pid 0 changes itself only; the main thread (pid 0, process
 * id) stays SCHED_OTHER. A process may change its own children (same
 * user) too.
 * poly-diff: syscalls=off (a thread and a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_setscheduler 144
#define SYS_sched_getscheduler 145
/* linux/sched.h */
#define SCHED_OTHER 0
#define SCHED_BATCH 3
#define SCHED_IDLE 5

static i64 getsched(i64 pid) { return syscall1(SYS_sched_getscheduler, pid); }
static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}

static char stack[65536] __attribute__((aligned(16)));
static volatile int tid_word;
static volatile unsigned started, step2, step2_done, may_exit;
static volatile i64 own_set, seen_after_main_set;

static int thread_main(void *arg) {
    (void)arg;
    gate_open(&started);
    gate_wait(&step2);
    seen_after_main_set = getsched(0);
    own_set = setsched(0, SCHED_IDLE, 0);
    gate_open(&step2_done);
    gate_wait(&may_exit);
    return 0;
}

int test_main(void) {
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&tid_word, (int *)&tid_word, thread_main, 0);
    check("thread_started", t > 0, t);
    gate_wait(&started);
    i64 r = setsched(t, SCHED_BATCH, 0);
    check("set_thread_batch", r == 0, r);
    r = getsched(t);
    check("thread_batch", r == SCHED_BATCH, r);
    r = getsched(0);
    check("main_other", r == SCHED_OTHER, r);
    gate_open(&step2);
    gate_wait(&step2_done);
    check("thread_saw_batch", seen_after_main_set == SCHED_BATCH, seen_after_main_set);
    check("thread_set_itself_idle", own_set == 0, own_set);
    r = getsched(t);
    check("thread_idle", r == SCHED_IDLE, r);
    r = getsched(getpid());
    check("process_still_other", r == SCHED_OTHER, r);
    gate_open(&may_exit);
    while (__atomic_load_n(&tid_word, __ATOMIC_SEQ_CST) != 0)
        futex((volatile unsigned *)&tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);

    volatile unsigned *g = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(getsched(0) == SCHED_BATCH ? 0 : 1);
    }
    gate_wait(&g[0]);
    r = setsched(pid, SCHED_BATCH, 0);
    check("set_child_batch", r == 0, r);
    gate_open(&g[1]);
    int code = wait_exit(pid);
    check("child_saw_batch", code == 0, code);
    return finish();
}
