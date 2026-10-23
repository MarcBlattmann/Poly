/* tkill to the calling thread itself: signal 0 only checks the thread
 * exists; an unblocked signal runs its handler before tkill returns 0. The
 * handler's siginfo carries the signal, si_code SI_TKILL (-6), the sender's
 * pid and its real uid. The main thread's tid is the pid.
 */
#include "poly_ipc.h"

static volatile int hits, got_signo, got_errno, got_code, got_pid;
static volatile unsigned got_uid;

static void on_usr1(int sig, struct k_siginfo *si, void *uc) {
    (void)uc;
    hits++;
    got_signo = sig == si->si_signo ? sig : -1;
    got_errno = si->si_errno;
    got_code = si->si_code;
    got_pid = si->u.kill.pid;
    got_uid = si->u.kill.uid;
}

int test_main(void) {
    i64 pid = getpid();
    i64 tid = gettid();
    check("main_tid_is_pid", tid == pid, tid);
    check("sigaction", sig_action(SIGUSR1, (u64)on_usr1, SA_SIGINFO, 0) == 0, 0);

    i64 r = syscall2(SYS_tkill, tid, 0);
    check("signal_zero_returns_zero", r == 0, r);
    check("signal_zero_delivers_nothing", hits == 0 && sig_pending() == 0, hits);

    r = syscall2(SYS_tkill, tid, SIGUSR1);
    check("tkill_returns_zero", r == 0, r);
    check("handler_ran_before_return", hits == 1, hits);
    check("si_signo", got_signo == SIGUSR1, got_signo);
    check("si_errno_zero", got_errno == 0, got_errno);
    check("si_code_tkill", got_code == SI_TKILL, got_code);
    check("si_pid_is_sender", got_pid == pid, 0);
    check("si_uid_is_real_uid", (i64)got_uid == syscall0(SYS_getuid), 0);
    check("nothing_left_pending", sig_pending() == 0, (i64)sig_pending());

    /* A second one is delivered again (handlers stay installed). */
    r = syscall2(SYS_tkill, tid, SIGUSR1);
    check("second_tkill", r == 0 && hits == 2, hits);
    return finish();
}
