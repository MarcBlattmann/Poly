/* rt_sigaction flags: the signal mask while a handler runs (the signal
 * itself plus sa_mask, unless SA_NODEFER), the mask restored afterwards,
 * SA_NODEFER letting the handler be re-entered, SA_RESETHAND restoring
 * SIG_DFL on entry (and keeping the flags; the signal is still blocked
 * inside that handler). */
#include "poly_ipc.h"

static volatile u64 mask_in_handler;
static volatile int depth, max_depth, calls;

static void record_mask(int sig) {
    (void)sig;
    mask_in_handler = sig_current_mask();
}

static void reenter(int sig) {
    calls++;
    depth++;
    if (depth > max_depth) max_depth = depth;
    if (calls < 3) raise_sig(sig);
    depth--;
}

static void reset_handler(int sig) {
    struct k_sigaction now;
    (void)sig;
    calls++;
    syscall4(SYS_rt_sigaction, SIGUSR2, 0, &now, 8);
    mask_in_handler = now.handler; /* the action seen from inside */
}

int test_main(void) {
    /* Default: the signal is blocked while its handler runs, with sa_mask. */
    sig_action(SIGUSR1, (u64)record_mask, 0, SIGBIT(SIGTERM));
    raise_sig(SIGUSR1);
    check("handler_blocks_own_signal", (mask_in_handler & SIGBIT(SIGUSR1)) != 0, (i64)mask_in_handler);
    check("handler_blocks_sa_mask", (mask_in_handler & SIGBIT(SIGTERM)) != 0, (i64)mask_in_handler);
    check("mask_restored_after_handler", sig_current_mask() == 0, (i64)sig_current_mask());

    /* SA_NODEFER: not blocked inside, sa_mask still is. */
    sig_action(SIGUSR1, (u64)record_mask, SA_NODEFER, SIGBIT(SIGTERM));
    raise_sig(SIGUSR1);
    check("nodefer_leaves_signal_unblocked", (mask_in_handler & SIGBIT(SIGUSR1)) == 0, (i64)mask_in_handler);
    check("nodefer_still_blocks_sa_mask", (mask_in_handler & SIGBIT(SIGTERM)) != 0, (i64)mask_in_handler);

    /* Without SA_NODEFER a raise inside the handler waits until it returns. */
    calls = depth = max_depth = 0;
    sig_handler(SIGUSR1, reenter, 0);
    raise_sig(SIGUSR1);
    check("deferred_signal_runs_after_return", calls == 3, calls);
    check("deferred_signal_not_nested", max_depth == 1, max_depth);

    /* With SA_NODEFER it nests. */
    calls = depth = max_depth = 0;
    sig_handler(SIGUSR1, reenter, SA_NODEFER);
    raise_sig(SIGUSR1);
    check("nodefer_signal_nests", calls == 3 && max_depth == 3, max_depth);

    /* SA_RESETHAND: one delivery, then the default action again. */
    calls = 0;
    sig_handler(SIGUSR2, reset_handler, SA_RESETHAND);
    raise_sig(SIGUSR2);
    check("resethand_handler_ran", calls == 1, calls);
    check("resethand_default_inside_handler", mask_in_handler == SIG_DFL, (i64)mask_in_handler);
    struct k_sigaction now;
    syscall4(SYS_rt_sigaction, SIGUSR2, 0, &now, 8);
    check("resethand_action_now_default", now.handler == SIG_DFL, (i64)now.handler);
    /* Only the handler is reset: the flags stay as they were. */
    check("resethand_flags_kept", now.flags == (SA_RESETHAND | SA_RESTORER), (i64)now.flags);

    /* SA_RESETHAND does not imply SA_NODEFER: the signal is blocked inside. */
    sig_action(SIGUSR2, (u64)record_mask, SA_RESETHAND, 0);
    raise_sig(SIGUSR2);
    check("resethand_still_blocks_signal", (mask_in_handler & SIGBIT(SIGUSR2)) != 0, (i64)mask_in_handler);

    /* The flags read back as set (SA_RESTORER included). */
    sig_action(SIGUSR1, (u64)record_mask, SA_RESTART | SA_NODEFER | SA_SIGINFO | SA_ONSTACK, 0);
    syscall4(SYS_rt_sigaction, SIGUSR1, 0, &now, 8);
    check("flags_read_back",
          now.flags == (SA_RESTART | SA_NODEFER | SA_SIGINFO | SA_ONSTACK | SA_RESTORER), (i64)now.flags);
    return finish();
}
