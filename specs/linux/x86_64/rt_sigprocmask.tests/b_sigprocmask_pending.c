/* Blocked signals stay pending and are delivered when unblocked, before
 * rt_sigprocmask returns: a standard signal raised twice is delivered once,
 * a real-time signal raised twice is delivered twice; several pending
 * signals are taken lowest number first, each frame stacked on the last;
 * SIG_IGN or SIG_DFL (default ignore) discards a pending signal. */
#include "poly_ipc.h"

static volatile int order[16];
static volatile int n;

static void record(int sig) {
    if (n < 16) order[n] = sig;
    n++;
}

int test_main(void) {
    int sigs[] = { SIGUSR1, SIGUSR2, SIGTERM, SIGHUP, SIGRTMIN, SIGRTMIN + 1, SIGRTMAX };
    for (unsigned i = 0; i < sizeof sigs / sizeof sigs[0]; i++) sig_handler(sigs[i], record, 0);

    u64 all = SIGBIT(SIGUSR1) | SIGBIT(SIGUSR2) | SIGBIT(SIGTERM) | SIGBIT(SIGHUP) | SIGBIT(SIGRTMIN) |
              SIGBIT(SIGRTMIN + 1) | SIGBIT(SIGRTMAX);
    sig_mask(SIG_BLOCK, all, 0);
    raise_sig(SIGUSR1);
    check("blocked_signal_not_delivered", n == 0, n);
    check("blocked_signal_pending", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR1), 0);
    check("unblock_delivers_before_return", n == 1 && order[0] == SIGUSR1, n);
    check("delivered_signal_no_longer_pending", sig_pending() == 0, (i64)sig_pending());

    /* A standard signal does not queue. */
    n = 0;
    raise_sig(SIGUSR2);
    raise_sig(SIGUSR2);
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR2), 0);
    check("standard_signal_delivered_once", n == 1, n);

    /* A real-time signal queues. */
    n = 0;
    raise_sig(SIGRTMIN);
    raise_sig(SIGRTMIN);
    raise_sig(SIGRTMIN);
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGRTMIN), 0);
    check("rt_signal_delivered_three_times", n == 3, n);

    /* Order of delivery: ascending signal number. */
    sig_mask(SIG_SETMASK, all, 0);
    n = 0;
    raise_sig(SIGRTMAX);
    raise_sig(SIGRTMIN + 1);
    raise_sig(SIGRTMIN);
    raise_sig(SIGTERM);
    raise_sig(SIGUSR2);
    raise_sig(SIGHUP);
    raise_sig(SIGUSR1);
    check("all_seven_pending", sig_pending() == all, (i64)sig_pending());
    sig_mask(SIG_SETMASK, 0, 0);
    check("seven_delivered", n == 7, n);
    /* Every pending signal gets its frame before the return to user mode,
     * lowest number first, each on top of the last: so with handlers that
     * do not block each other, the highest number's handler runs first. */
    int want[] = { SIGRTMAX, SIGRTMIN + 1, SIGRTMIN, SIGTERM, SIGUSR2, SIGUSR1, SIGHUP };
    int same = 1;
    for (int i = 0; i < 7; i++) same &= order[i] == want[i];
    check("unmasked_handlers_run_highest_first", same, order[0] * 100 + order[1]);

    /* With handlers that block all the others, one frame at a time: lowest first. */
    for (unsigned i = 0; i < sizeof sigs / sizeof sigs[0]; i++) sig_action(sigs[i], (u64)record, 0, all);
    sig_mask(SIG_SETMASK, all, 0);
    n = 0;
    raise_sig(SIGRTMAX);
    raise_sig(SIGRTMIN + 1);
    raise_sig(SIGRTMIN);
    raise_sig(SIGTERM);
    raise_sig(SIGUSR2);
    raise_sig(SIGHUP);
    raise_sig(SIGUSR1);
    sig_mask(SIG_SETMASK, 0, 0);
    int want2[] = { SIGHUP, SIGUSR1, SIGUSR2, SIGTERM, SIGRTMIN, SIGRTMIN + 1, SIGRTMAX };
    same = n == 7;
    for (int i = 0; i < 7; i++) same &= order[i] == want2[i];
    check("masked_handlers_run_lowest_first", same, order[0] * 100 + order[1]);

    /* Ignoring a pending signal discards it. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1), 0);
    raise_sig(SIGUSR1);
    sig_action(SIGUSR1, SIG_IGN, 0, 0);
    check("ignore_discards_pending", sig_pending() == 0, (i64)sig_pending());

    /* A blocked signal whose action is SIG_IGN is still made pending. */
    raise_sig(SIGUSR1);
    check("blocked_ignored_signal_pending", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR1), 0);
    check("unblocked_ignored_signal_dropped", sig_pending() == 0, (i64)sig_pending());

    /* Setting SIG_DFL for a signal whose default is to ignore also discards. */
    sig_mask(SIG_BLOCK, SIGBIT(SIGURG), 0);
    raise_sig(SIGURG);
    check("blocked_sigurg_pending", sig_pending() == SIGBIT(SIGURG), (i64)sig_pending());
    sig_action(SIGURG, SIG_DFL, 0, 0);
    check("default_ignore_discards_pending", sig_pending() == 0, (i64)sig_pending());
    return finish();
}
