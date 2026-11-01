/* The real-time priority of another process is visible to its parent:
 * a child at SCHED_RR 42 reads as 42 through its pid, and a thread of
 * that child at SCHED_FIFO 7 reads as 7 through its thread id while the
 * child's main thread still reads 42.
 * poly-diff: needs=root (only a privileged caller may choose a real-time policy without RLIMIT_RTPRIO)
 * poly-diff: syscalls=off (a thread and a child process)
 */
#include "poly_ipc.h"

/* asm/unistd_64.h */
#define SYS_sched_getparam 143
#define SYS_sched_setscheduler 144
/* linux/sched.h */
#define SCHED_FIFO 1
#define SCHED_RR 2

static i64 setsched(i64 pid, i64 policy, int prio) {
    int p = prio;
    return syscall3(SYS_sched_setscheduler, pid, policy, &p);
}
static i64 getparam(i64 pid) {
    int p = -77;
    i64 r = syscall2(SYS_sched_getparam, pid, &p);
    return r < 0 ? r : p;
}

static char stack[65536] __attribute__((aligned(16)));
static volatile int tid_word;

static int thread_main(void *arg) {
    volatile unsigned *g = arg;
    setsched(0, SCHED_FIFO, 7);
    gate_open(&g[2]);
    gate_wait(&g[3]);
    return 0;
}

int test_main(void) {
    volatile unsigned *g = shared_page(4096);
    volatile int *tid = (volatile int *)&g[16];
    i64 pid = fork();
    if (pid == 0) {
        if (setsched(0, SCHED_RR, 42) != 0) {
            gate_open(&g[0]); /* do not leave the parent waiting */
            exit_group(1);
        }
        i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&tid_word, (int *)&tid_word, thread_main,
                           (void *)g);
        if (t <= 0) {
            gate_open(&g[0]);
            exit_group(2);
        }
        *tid = (int)t;
        gate_wait(&g[2]);
        gate_open(&g[0]);
        gate_wait(&g[1]);
        gate_open(&g[3]);
        while (__atomic_load_n(&tid_word, __ATOMIC_SEQ_CST) != 0)
            futex((volatile unsigned *)&tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);
        exit_group(0);
    }
    gate_wait(&g[0]);
    i64 r = getparam(pid);
    check("child_rr_42", r == 42, r);
    r = *tid ? getparam(*tid) : -1;
    check("child_thread_fifo_7", r == 7, r);
    r = getparam(0);
    check("parent_0", r == 0, r);
    gate_open(&g[1]);
    int code = wait_exit(pid);
    check("child_ok", code == 0, code);
    return finish();
}
