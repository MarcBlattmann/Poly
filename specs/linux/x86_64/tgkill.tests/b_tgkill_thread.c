/* Thread-directed signals: tgkill to a second thread runs the handler on
 * that thread (gettid inside the handler); a signal the target thread
 * blocks is pending for that thread only, not for the process or for the
 * main thread; a process-directed kill is taken by a thread that does not
 * block it. Threads share the actions.
 * poly-diff: syscalls=off (two threads)
 */
#include "poly_ipc.h"

static char stack[65536] __attribute__((aligned(16)));
static volatile unsigned started, go_block, blocked_ok, go_check, done;
static volatile int handler_tid, thread_tid, thread_pending_ok, thread_hits;
static volatile int child_tid_word;

static void on_usr1(int sig) {
    (void)sig;
    handler_tid = (int)gettid();
}

static void on_usr2(int sig) {
    (void)sig;
    handler_tid = (int)gettid();
    __atomic_store_n(&thread_hits, 1, __ATOMIC_SEQ_CST);
}

static int thread_main(void *arg) {
    (void)arg;
    thread_tid = (int)gettid();
    gate_open(&started);
    gate_wait(&go_block);
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1), 0);
    gate_open(&blocked_ok);
    gate_wait(&go_check);
    thread_pending_ok = sig_pending() == SIGBIT(SIGUSR1);
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR1), 0); /* handled here, now */
    gate_open(&done);
    return 0;
}

int test_main(void) {
    i64 pid = getpid();
    sig_handler(SIGUSR1, on_usr1, 0);
    sig_handler(SIGUSR2, on_usr2, 0);
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&child_tid_word, (int *)&child_tid_word,
                       thread_main, 0);
    check("clone_thread", t > 0, t);
    gate_wait(&started);
    check("parent_tid_written", child_tid_word == t, 0);
    check("thread_tid_distinct", t != pid && thread_tid == t, 0);
    check("thread_shares_pid", syscall3(SYS_tgkill, pid, t, 0) == 0, 0);

    /* Directed at the thread: handled on the thread. */
    handler_tid = 0;
    i64 r = syscall3(SYS_tgkill, pid, t, SIGUSR2);
    check("tgkill_thread_returns_zero", r == 0, r);
    while (__atomic_load_n(&thread_hits, __ATOMIC_SEQ_CST) == 0) syscall0(SYS_sched_yield);
    check("handler_ran_on_target_thread", handler_tid == t, 0);

    /* The thread blocks SIGUSR1; a signal sent to it waits for it. */
    gate_open(&go_block);
    gate_wait(&blocked_ok);
    syscall3(SYS_tgkill, pid, t, SIGUSR1);
    check("main_thread_not_pending", sig_pending() == 0, (i64)sig_pending());
    gate_open(&go_check);
    gate_wait(&done);
    check("thread_saw_own_pending", thread_pending_ok, 0);
    check("pending_signal_handled_by_thread", handler_tid == t, 0);

    /* The thread is gone: CLONE_CHILD_CLEARTID zeroes the word and wakes. */
    while (__atomic_load_n(&child_tid_word, __ATOMIC_SEQ_CST) != 0)
        futex((volatile unsigned *)&child_tid_word, FUTEX_WAIT, (unsigned)t, 0, 0, 0);
    check("exited_thread_tid_cleared", child_tid_word == 0, 0);

    /* The main thread takes process-directed signals now. */
    handler_tid = 0;
    raise_sig(SIGUSR1);
    check("process_signal_on_main_thread", handler_tid == pid, 0);
    return finish();
}
