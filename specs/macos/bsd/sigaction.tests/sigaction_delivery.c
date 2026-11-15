/* sigaction(2), BSD 46, delivery: a signal the process sends itself with
 * kill runs the handler before kill returns, once; the handler gets the
 * signal number and, with SA_SIGINFO, a siginfo with si_signo, si_code 0
 * (sent by kill) and the sender's pid; the signal is blocked while its
 * handler runs and the mask is restored afterwards; SIG_IGN discards it. */
#include "poly_mtest.h"

static volatile int hits, got_sig, got_code, got_signo, pid_ok;
static volatile u32 mask_in_handler;

static void handler(int sig, void *info, void *uctx) {
    (void)uctx;
    hits++;
    got_sig = sig;
    if (info) {
        i32 *p = info;
        got_signo = p[0];
        got_code = p[2];
        pid_ok = p[3] == bsd0(SYS_getpid).ret;
    }
    mask_in_handler = poly_sigmask();
}

int test_main(void) {
    i64 me = bsd0(SYS_getpid).ret;
    poly_sigaction(SIGUSR1, (void *)handler, 0, SA_SIGINFO, 0);
    sysret r = bsd2(SYS_kill, me, SIGUSR1);
    check("kill returns 0", r.err == 0 && r.ret == 0, r.err);
    check("the handler ran once before kill returned", hits == 1, hits);
    check("with the signal number", got_sig == SIGUSR1, got_sig);
    check("siginfo: si_signo is the signal", got_signo == SIGUSR1, got_signo);
    check("siginfo: si_code is 0 for kill", got_code == 0, got_code);
    check("siginfo: si_pid is the sender", pid_ok, 0);
    check("the signal was blocked inside the handler", mask_in_handler == SIGBIT(SIGUSR1), mask_in_handler);
    check("and the mask is restored afterwards", poly_sigmask() == 0, poly_sigmask());
    hits = 0;
    poly_sigaction(SIGUSR1, SIG_IGN, 0, 0, 0);
    bsd2(SYS_kill, me, SIGUSR1);
    check("SIG_IGN: no handler runs", hits == 0, hits);
    return finish();
}
