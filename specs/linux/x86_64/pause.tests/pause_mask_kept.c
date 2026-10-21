/* pause: does not change the signal mask. The handler runs with the mask
 * the caller had plus its own signal and its sa_mask; after pause the mask
 * is exactly what it was before.
 */
#include "poly_ipc.h"

struct itv {
    struct k_timeval interval;
    struct k_timeval value;
};

static volatile u64 mask_in_handler;
static volatile int hits;

static void on_alrm(int sig) {
    (void)sig;
    hits++;
    mask_in_handler = sig_current_mask();
}

int test_main(void) {
    sig_action(SIGALRM, (u64)on_alrm, 0, SIGBIT(SIGHUP));
    u64 before = SIGBIT(SIGUSR2) | SIGBIT(SIGTERM);
    sig_mask(SIG_SETMASK, before, 0);
    struct itv t = { { 0, 0 }, { 0, 50000 } };
    syscall3(SYS_setitimer, 0, &t, 0);
    i64 r = syscall0(SYS_pause);
    check("returns_eintr", r == -EINTR, r);
    check("handler_ran", hits == 1, hits);
    check("handler_mask_is_caller_mask_plus_signal_and_sa_mask",
          mask_in_handler == (before | SIGBIT(SIGALRM) | SIGBIT(SIGHUP)), (i64)mask_in_handler);
    check("mask_unchanged_after", sig_current_mask() == before, (i64)sig_current_mask());
    return finish();
}
