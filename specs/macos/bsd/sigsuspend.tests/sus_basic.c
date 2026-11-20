#include "poly_mtest.h"
/* sigsuspend, BSD 111 (mask, by value): replaces the signal mask with `mask`, waits for a signal
 * that is caught, runs its handler, restores the old mask and fails with EINTR (4). A signal already
 * pending and unblocked by the temporary mask is delivered at once. Several pending signals are all
 * delivered before it returns, the higher-numbered first: SIGUSR2 (31) before SIGUSR1 (30). Inside
 * the handler the mask is the temporary mask plus the signal itself (and the action's mask). */
static volatile i64 order, mask_seen;
static void h(int sig, void *info, void *uc) { (void)info; (void)uc; order = order * 100 + sig; mask_seen = poly_sigmask(); }
int test_main(void) {
    i64 me = bsd0(SYS_getpid).ret;
    poly_sigaction(SIGUSR1, (void *)h, 0, SA_SIGINFO, 0);
    poly_sigaction(SIGUSR2, (void *)h, 0, SA_SIGINFO, 0);
    u32 m = SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2);
    bsd3(SYS_sigprocmask, SIG_BLOCK, &m, 0);
    bsd2(SYS_kill, me, SIGUSR1);
    bsd2(SYS_kill, me, SIGUSR2);
    check("both are held back: no handler yet", order == 0, order);
    sysret r = bsd1(SYS_sigsuspend, 0);
    check_err("sigsuspend returns EINTR", r, EINTR);
    check("both handlers ran, SIGUSR2 first", order == 3130, order);
    check("in the last handler (SIGUSR1) the mask was just its own bit", mask_seen == SIGBIT(SIGUSR1), mask_seen);
    check("the old mask is restored (USR1 and USR2 blocked)", poly_sigmask() == m, poly_sigmask());
    u32 pend = 1;
    bsd1(SYS_sigpending, &pend);
    check("nothing is pending afterwards", pend == 0, pend);
    order = 0;
    sysret f = bsd0(SYS_fork);
    if (f.ret2 == 1) {
        for (volatile int i = 0; i < 20000000; i++) {}
        bsd2(SYS_kill, bsd0(SYS_getppid).ret, SIGUSR1);
        poly_exit(0);
    }
    r = bsd1(SYS_sigsuspend, SIGBIT(SIGUSR2));
    check_err("a signal sent while waiting wakes it: EINTR", r, EINTR);
    check("the handler ran once", order == 30, order);
    check("in the handler the mask was the temporary mask plus the signal", mask_seen == (SIGBIT(SIGUSR2) | SIGBIT(SIGUSR1)), mask_seen);
    check("restored afterwards", poly_sigmask() == m, poly_sigmask());
    poly_wait(f.ret);
    return finish();
}
