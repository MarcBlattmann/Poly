/* tkill sent from a second thread to the main thread: the handler runs on
 * the main thread, and si_pid is the sender's process id (the thread
 * group), not the sending thread's tid; si_code is SI_TKILL.
 * poly-diff: syscalls=off (two threads)
 */
#include "poly_ipc.h"

static char stack[65536] __attribute__((aligned(16)));
static volatile unsigned sent;
static volatile int handler_tid, got_pid, got_code, hits, send_result, ctid;

static void on_usr1(int sig, struct k_siginfo *si, void *uc) {
    (void)sig;
    (void)uc;
    handler_tid = (int)gettid();
    got_pid = si->u.kill.pid;
    got_code = si->si_code;
    __atomic_store_n(&hits, 1, __ATOMIC_SEQ_CST);
}

static int thread_main(void *arg) {
    i64 main_tid = (i64)arg;
    /* Block it here, so only the named (main) thread can take it. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1), 0);
    send_result = (int)syscall2(SYS_tkill, main_tid, SIGUSR1);
    gate_open(&sent);
    return 0;
}

int test_main(void) {
    i64 pid = getpid();
    sig_action(SIGUSR1, (u64)on_usr1, SA_SIGINFO, 0);
    i64 t = poly_clone(THREAD_FLAGS, stack + sizeof stack, (int *)&ctid, (int *)&ctid, thread_main, (void *)pid);
    check("clone_thread", t > 0, t);
    gate_wait(&sent);
    check("send_returned_zero", send_result == 0, send_result);
    while (__atomic_load_n(&hits, __ATOMIC_SEQ_CST) == 0) syscall0(SYS_sched_yield);
    check("handled_on_main_thread", handler_tid == pid, 0);
    check("si_pid_is_process_id", got_pid == pid, 0);
    check("si_pid_not_thread_id", got_pid != t, 0);
    check("si_code_tkill", got_code == SI_TKILL, got_code);
    while (__atomic_load_n(&ctid, __ATOMIC_SEQ_CST) != 0) futex((volatile unsigned *)&ctid, FUTEX_WAIT, (unsigned)t, 0, 0, 0);
    return finish();
}
