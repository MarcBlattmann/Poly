/* sched_rr_get_interval of other tasks needs no permission: a child, a
 * second thread and a zombie child report a fair-policy interval in the same
 * range as the caller (tv_sec 0, at most 100 ms; a task that is not
 * running may report 0 where the running caller reports its slice); a
 * reaped child is ESRCH.
 * poly-diff: syscalls=off (a thread and a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_rr_get_interval 148
#define SYS_waitid 247
/* linux/wait.h */
#define P_PID 1
#define WEXITED 0x00000004
#define WNOWAIT 0x01000000

static i64 interval(i64 pid) {
    i64 t[2] = { 77, 77 };
    i64 r = syscall2(SYS_sched_rr_get_interval, pid, t);
    if (r < 0) return r;
    if (t[0] != 0) return -1000;
    return t[1];
}

static char stack[65536] __attribute__((aligned(16)));
static volatile int tid_word;
static volatile unsigned started, may_exit;

static int thread_main(void *arg) {
    (void)arg;
    gate_open(&started);
    gate_wait(&may_exit);
    return 0;
}

int test_main(void) {
    i64 mine = interval(0);
    check("self_ok", mine >= 0, mine);
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&tid_word, (int *)&tid_word, thread_main, 0);
    check("thread_started", t > 0, t);
    gate_wait(&started);
    i64 r = interval(t);
    check("thread_in_range", r >= 0 && r <= 100000000, r);
    gate_open(&may_exit);
    while (__atomic_load_n(&tid_word, __ATOMIC_SEQ_CST) != 0)
        futex((volatile unsigned *)&tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);

    volatile unsigned *g = shared_page(4096);
    i64 pid = fork();
    if (pid == 0) {
        gate_open(&g[0]);
        gate_wait(&g[1]);
        exit_group(0);
    }
    gate_wait(&g[0]);
    r = interval(pid);
    check("child_in_range", r >= 0 && r <= 100000000, r);
    gate_open(&g[1]);
    struct k_siginfo info;
    memset(&info, 0, sizeof info);
    r = syscall5(SYS_waitid, P_PID, pid, &info, WEXITED | WNOWAIT, 0);
    check("child_exited", r == 0, r);
    r = interval(pid);
    check("zombie_ok", r >= 0, r);
    check("reaped", wait_exit(pid) == 0, 0);
    r = interval(pid);
    check("reaped_esrch", r == -ESRCH, r);
    return finish();
}
