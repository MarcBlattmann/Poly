/* sigaction(2), BSD 46, flags: SA_RESETHAND returns the action to
 * SIG_DFL after one delivery; SA_NODEFER leaves the signal unblocked in its
 * handler; the action's mask adds signals to the blocked set while the
 * handler runs. */
#include "poly_mtest.h"

static volatile int hits;
static volatile u32 mask_in_handler;

static void handler(int sig, void *info, void *uctx) {
    (void)sig;
    (void)info;
    (void)uctx;
    hits++;
    mask_in_handler = poly_sigmask();
}

int test_main(void) {
    i64 me = bsd0(SYS_getpid).ret;
    struct poly_usigaction o;
    poly_sigaction(SIGUSR2, (void *)handler, 0, SA_SIGINFO | SA_RESETHAND, 0);
    bsd2(SYS_kill, me, SIGUSR2);
    check("SA_RESETHAND: the handler ran once", hits == 1, hits);
    bsd3(SYS_sigaction, SIGUSR2, 0, &o);
    check("and the action is back to SIG_DFL", o.handler == SIG_DFL, (i64)o.handler);
    poly_sigaction(SIGUSR2, (void *)handler, 0, SA_SIGINFO | SA_NODEFER, 0);
    bsd2(SYS_kill, me, SIGUSR2);
    check("SA_NODEFER: the signal is not blocked in its handler", mask_in_handler == 0, mask_in_handler);
    poly_sigaction(SIGUSR2, (void *)handler, SIGBIT(SIGUSR1), SA_SIGINFO, 0);
    bsd2(SYS_kill, me, SIGUSR2);
    check("the action's mask is blocked too", mask_in_handler == (SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2)), mask_in_handler);
    check("the mask is restored", poly_sigmask() == 0, poly_sigmask());
    return finish();
}
