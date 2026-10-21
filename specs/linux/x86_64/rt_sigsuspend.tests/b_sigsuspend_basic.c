/* rt_sigsuspend: replaces the mask, waits for a signal that runs a
 * handler, returns EINTR after the handler, and puts the old mask back. A
 * signal already pending and unblocked by the new mask ends it at once;
 * signals still blocked by the new mask stay pending. sigsetsize other
 * than 8: EINVAL. */
#include "poly_ipc.h"

static volatile int got, hits;
static volatile u64 mask_in_handler;

static void on_sig(int sig) {
    got = sig;
    hits++;
    mask_in_handler = sig_current_mask();
}

int test_main(void) {
    sig_handler(SIGUSR1, on_sig, 0);
    sig_handler(SIGUSR2, on_sig, 0);
    sig_mask(SIG_SETMASK, SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2) | SIGBIT(SIGHUP), 0);
    raise_sig(SIGUSR1);
    raise_sig(SIGUSR2);

    /* The new mask unblocks SIGUSR1 only. */
    u64 m = SIGBIT(SIGUSR2) | SIGBIT(SIGTERM);
    i64 r = syscall2(SYS_rt_sigsuspend, &m, 8);
    check("returns_eintr", r == -EINTR, r);
    check("handler_ran", hits == 1 && got == SIGUSR1, got);
    check("handler_mask_is_suspend_mask_plus_signal",
          mask_in_handler == (SIGBIT(SIGUSR2) | SIGBIT(SIGTERM) | SIGBIT(SIGUSR1)), (i64)mask_in_handler);
    check("old_mask_restored", sig_current_mask() == (SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2) | SIGBIT(SIGHUP)),
          (i64)sig_current_mask());
    check("still_blocked_signal_pending", sig_pending() == SIGBIT(SIGUSR2), (i64)sig_pending());

    /* Empty mask: the other pending signal ends the next suspend. */
    m = 0;
    r = syscall2(SYS_rt_sigsuspend, &m, 8);
    check("second_returns_eintr", r == -EINTR && got == SIGUSR2 && hits == 2, r);
    check("nothing_pending", sig_pending() == 0, (i64)sig_pending());

    /* A pending signal whose action is to ignore does not end it, but the
     * test cannot wait forever: so only the errors here. */
    r = syscall2(SYS_rt_sigsuspend, &m, 4);
    check("sigsetsize_4_einval", r == -EINVAL, r);
    r = syscall2(SYS_rt_sigsuspend, &m, 16);
    check("sigsetsize_16_einval", r == -EINVAL, r);
    r = syscall2(SYS_rt_sigsuspend, 8, 8);
    check("bad_mask_pointer_efault", r == -EFAULT, r);
    check("errors_keep_mask", sig_current_mask() == (SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2) | SIGBIT(SIGHUP)),
          (i64)sig_current_mask());
    return finish();
}
