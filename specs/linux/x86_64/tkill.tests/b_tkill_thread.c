/* tkill to a second thread of the process: its handler runs on that
 * thread. A signal sent to a thread that blocks it is pending for that
 * thread only (the main thread's rt_sigpending does not show it) and is
 * handled by that thread once it unblocks it. tkill to the main thread's
 * tid (the pid) is taken by the main thread even though the other thread
 * does not block the signal.
 * poly-diff: syscalls=off (two threads)
 */
#include "poly_ipc.h"

static char stack[65536] __attribute__((aligned(16)));
static volatile unsigned started, go_block, blocked, go_check, checked, go_exit;
static volatile int handler_tid, hits, thread_saw_pending, thread_hits_after_unblock;
static volatile int ctid;

static void on_sig(int sig) {
    (void)sig;
    handler_tid = (int)gettid();
    __atomic_add_fetch(&hits, 1, __ATOMIC_SEQ_CST);
}

static int thread_main(void *arg) {
    (void)arg;
    gate_open(&started);
    gate_wait(&go_block);
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1), 0);
    gate_open(&blocked);
    gate_wait(&go_check);
    thread_saw_pending = sig_pending() == SIGBIT(SIGUSR1);
    int before = hits;
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR1), 0);
    thread_hits_after_unblock = hits - before;
    gate_open(&checked);
    gate_wait(&go_exit);
    return 0;
}

int test_main(void) {
    i64 pid = getpid();
    sig_handler(SIGUSR1, on_sig, 0);
    sig_handler(SIGUSR2, on_sig, 0);
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&ctid, (int *)&ctid, thread_main, 0);
    check("clone_thread", t > 0, t);
    gate_wait(&started);

    i64 r = syscall2(SYS_tkill, t, 0);
    check("thread_signal_zero", r == 0, r);
    r = syscall2(SYS_tkill, t, SIGUSR2);
    check("tkill_thread_returns_zero", r == 0, r);
    while (__atomic_load_n(&hits, __ATOMIC_SEQ_CST) < 1) syscall0(SYS_sched_yield);
    check("handler_on_target_thread", handler_tid == t, 0);

    /* Main thread's own tid: handled here, before tkill returns. */
    r = syscall2(SYS_tkill, pid, SIGUSR2);
    check("tkill_main_tid", r == 0, r);
    check("handled_on_main_thread", hits == 2 && handler_tid == pid, hits);

    gate_open(&go_block);
    gate_wait(&blocked);
    r = syscall2(SYS_tkill, t, SIGUSR1);
    check("tkill_blocking_thread", r == 0, r);
    check("not_pending_on_main_thread", sig_pending() == 0, (i64)sig_pending());
    check("not_handled_yet", hits == 2, hits);
    gate_open(&go_check);
    gate_wait(&checked);
    check("pending_on_target_thread", thread_saw_pending, 0);
    check("handled_on_unblock", thread_hits_after_unblock == 1, thread_hits_after_unblock);
    check("handled_by_target_thread", handler_tid == t, 0);
    gate_open(&go_exit);
    while (__atomic_load_n(&ctid, __ATOMIC_SEQ_CST) != 0) futex((volatile unsigned *)&ctid, FUTEX_WAIT, (unsigned)t, 0, 0, 0);
    return finish();
}
