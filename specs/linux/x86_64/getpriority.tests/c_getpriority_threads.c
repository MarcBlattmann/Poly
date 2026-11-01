/* The nice value is per thread: a second thread that sets its own nice
 * to 7 (setpriority PRIO_PROCESS, who 0) reads 13 for itself and
 * through its thread id, while the main thread reads 20 through who 0
 * and through the process id. A child process's value is visible to
 * the parent, also while it is a zombie.
 * poly-diff: syscalls=off (a thread and a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_getpriority 140
#define SYS_setpriority 141
#define SYS_waitid 247
/* linux/wait.h */
#define P_PID 1
#define WEXITED 0x00000004
#define WNOWAIT 0x01000000
/* linux/resource.h */
#define PRIO_PROCESS 0

static i64 getprio(i64 which, i64 who) { return syscall2(SYS_getpriority, which, who); }
static i64 setprio(i64 which, i64 who, i64 nice) { return syscall3(SYS_setpriority, which, who, nice); }

static char stack[65536] __attribute__((aligned(16)));
static volatile int tid_word;
static volatile unsigned ready, may_exit;
static volatile i64 t_set, t_own;

static int thread_main(void *arg) {
    (void)arg;
    t_set = setprio(PRIO_PROCESS, 0, 7);
    t_own = getprio(PRIO_PROCESS, 0);
    gate_open(&ready);
    gate_wait(&may_exit);
    return 0;
}

int test_main(void) {
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&tid_word, (int *)&tid_word, thread_main, 0);
    check("thread_started", t > 0, t);
    gate_wait(&ready);
    check("thread_set_7", t_set == 0, t_set);
    check("thread_reads_13", t_own == 13, t_own);
    i64 r = getprio(PRIO_PROCESS, t);
    check("by_tid_13", r == 13, r);
    r = getprio(PRIO_PROCESS, 0);
    check("main_20", r == 20, r);
    r = getprio(PRIO_PROCESS, getpid());
    check("process_id_is_main_20", r == 20, r);
    gate_open(&may_exit);
    while (__atomic_load_n(&tid_word, __ATOMIC_SEQ_CST) != 0)
        futex((volatile unsigned *)&tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);

    i64 pid = fork();
    if (pid == 0) {
        setprio(PRIO_PROCESS, 0, 9);
        exit_group(0);
    }
    struct k_siginfo info;
    memset(&info, 0, sizeof info);
    r = syscall5(SYS_waitid, P_PID, pid, &info, WEXITED | WNOWAIT, 0);
    check("child_exited", r == 0, r);
    r = getprio(PRIO_PROCESS, pid);
    check("zombie_11", r == 11, r);
    check("reaped", wait_exit(pid) == 0, 0);
    return finish();
}
