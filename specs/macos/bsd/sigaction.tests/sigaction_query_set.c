/* sigaction(2), BSD 46 (signal, new action, old action): a signal starts
 * with the default action (handler 0, mask 0, flags 0); an action set is
 * read back with its handler, mask and flags; SIG_IGN is 1. The kernel's
 * new-action layout is {handler, trampoline, mask, flags}, the old-action
 * layout {handler, mask, flags}. Darwin's SIGUSR1 is 30 and SIGUSR2 31. */
#include "poly_mtest.h"

static void caught(int sig, void *info, void *uctx) {
    (void)sig;
    (void)info;
    (void)uctx;
}

int test_main(void) {
    struct poly_usigaction o = {(void *)-1, 0xff, -1};
    sysret r = bsd3(SYS_sigaction, SIGUSR1, 0, &o);
    check("the default: success", r.err == 0 && r.ret == 0, r.err);
    check("handler 0, mask 0, flags 0", o.handler == SIG_DFL && o.mask == 0 && o.flags == 0, (i64)o.handler);
    r = poly_sigaction(SIGUSR1, (void *)caught, SIGBIT(SIGUSR2), SA_SIGINFO | SA_RESTART, &o);
    check("setting an action returns the old one (default)", r.err == 0 && o.handler == SIG_DFL, r.err);
    r = bsd3(SYS_sigaction, SIGUSR1, 0, &o);
    check("the handler reads back", r.err == 0 && o.handler == (void *)caught, r.err);
    check("the mask reads back", o.mask == SIGBIT(SIGUSR2), o.mask);
    check("the flags read back (SA_SIGINFO|SA_RESTART)", o.flags == (SA_SIGINFO | SA_RESTART), o.flags);
    poly_sigaction(SIGUSR1, SIG_IGN, 0, 0, &o);
    check("replacing returns the previous handler", o.handler == (void *)caught, 0);
    bsd3(SYS_sigaction, SIGUSR1, 0, &o);
    check("SIG_IGN reads back as 1", o.handler == SIG_IGN, (i64)o.handler);
    poly_sigaction(SIGUSR1, SIG_DFL, 0, 0, 0);
    bsd3(SYS_sigaction, SIGUSR1, 0, &o);
    check("and SIG_DFL as 0", o.handler == SIG_DFL, (i64)o.handler);
    value("SIGUSR1", SIGUSR1);
    value("SIGUSR2", SIGUSR2);
    return finish();
}
