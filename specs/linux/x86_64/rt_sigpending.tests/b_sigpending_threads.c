/* rt_sigpending with two threads that both block SIGUSR1 and SIGUSR2: a
 * process-directed signal (kill) is pending for both threads (each sees
 * it); a thread-directed one (tgkill to the second thread) only for that
 * thread.
 * poly-diff: syscalls=off (two threads)
 */
#include "poly_ipc.h"

static char stack[65536] __attribute__((aligned(16)));
static volatile unsigned go, done;
static volatile u64 seen;
static volatile int tid_word;

static int thread_main(void *arg) {
    (void)arg;
    gate_wait(&go);
    seen = sig_pending();
    gate_open(&done);
    return 0;
}

int test_main(void) {
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2), 0);
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&tid_word, (int *)&tid_word, thread_main, 0);
    check("thread_started", t > 0, t);
    raise_sig(SIGUSR1);
    syscall3(SYS_tgkill, getpid(), t, SIGUSR2);
    check("main_sees_process_signal_only", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    gate_open(&go);
    gate_wait(&done);
    check("thread_sees_both", seen == (SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2)), (i64)seen);
    while (__atomic_load_n(&tid_word, __ATOMIC_SEQ_CST) != 0)
        futex((volatile unsigned *)&tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);
    check("process_signal_still_pending_after_thread_exit", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    return finish();
}
