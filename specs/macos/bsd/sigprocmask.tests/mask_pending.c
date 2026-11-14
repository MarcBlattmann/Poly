/* sigprocmask, BSD 48, with delivery: a blocked signal stays pending
 * (sigpending, BSD 52), several of the same kind count once, and
 * unblocking delivers it before sigprocmask returns, after which nothing
 * is pending. */
#include "poly_mtest.h"

static volatile int hits;

static void handler(int sig, void *info, void *uctx) {
    (void)sig;
    (void)info;
    (void)uctx;
    hits++;
}

int test_main(void) {
    poly_sigaction(SIGUSR1, (void *)handler, 0, SA_SIGINFO, 0);
    u32 m = SIGBIT(SIGUSR1), pending = 0;
    bsd3(SYS_sigprocmask, SIG_BLOCK, &m, 0);
    i64 me = bsd0(SYS_getpid).ret;
    bsd2(SYS_kill, me, SIGUSR1);
    bsd2(SYS_kill, me, SIGUSR1);
    bsd2(SYS_kill, me, SIGUSR1);
    check("blocked: no handler ran", hits == 0, hits);
    check_ok("sigpending", bsd1(SYS_sigpending, &pending));
    check("the signal is pending", pending == m, pending);
    bsd3(SYS_sigprocmask, SIG_UNBLOCK, &m, 0);
    check("unblocking ran the handler, once for three signals", hits == 1, hits);
    bsd1(SYS_sigpending, &pending);
    check("nothing is pending afterwards", pending == 0, pending);
    return finish();
}
