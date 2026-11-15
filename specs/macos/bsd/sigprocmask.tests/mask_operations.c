/* sigprocmask, BSD 48 (how, set pointer, old-set pointer): the mask starts
 * empty; SIG_BLOCK adds, SIG_UNBLOCK removes, SIG_SETMASK replaces; the old
 * mask is stored through the third argument; a NULL set only reads. A mask
 * is a 32-bit set, signal n is bit n-1. */
#include "poly_mtest.h"

int test_main(void) {
    u32 old = 0xdead;
    check_ok("read the mask with a NULL set", bsd3(SYS_sigprocmask, SIG_BLOCK, 0, &old));
    check("it starts empty", old == 0, old);
    u32 a = SIGBIT(SIGUSR1), b = SIGBIT(SIGUSR2);
    check_ok("block SIGUSR1", bsd3(SYS_sigprocmask, SIG_BLOCK, &a, &old));
    check("the old mask was empty", old == 0, old);
    check_ok("block SIGUSR2", bsd3(SYS_sigprocmask, SIG_BLOCK, &b, &old));
    check("the old mask had SIGUSR1", old == a, old);
    check("the mask is both", poly_sigmask() == (a | b), poly_sigmask());
    check_ok("unblock SIGUSR1", bsd3(SYS_sigprocmask, SIG_UNBLOCK, &a, &old));
    check("the old mask had both", old == (a | b), old);
    check("the mask is SIGUSR2", poly_sigmask() == b, poly_sigmask());
    u32 c = SIGBIT(SIGTERM);
    check_ok("set the mask to SIGTERM", bsd3(SYS_sigprocmask, SIG_SETMASK, &c, &old));
    check("the mask is exactly SIGTERM", poly_sigmask() == c, poly_sigmask());
    u32 none = 0;
    bsd3(SYS_sigprocmask, SIG_SETMASK, &none, 0);
    check("and empty again", poly_sigmask() == 0, poly_sigmask());
    return finish();
}
