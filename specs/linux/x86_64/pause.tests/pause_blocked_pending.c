/* pause: a signal that is pending but blocked does not end pause, and is
 * still pending afterwards (its handler has not run). pause ends when an
 * unblocked signal (SIGALRM from a 100 ms timer) runs its handler.
 */
#include "poly_ipc.h"

struct itv {
    struct k_timeval interval;
    struct k_timeval value;
};

static volatile int usr1, alrm;

static void on_usr1(int sig) {
    (void)sig;
    usr1++;
}
static void on_alrm(int sig) {
    (void)sig;
    alrm++;
}

int test_main(void) {
    sig_handler(SIGUSR1, on_usr1, 0);
    sig_handler(SIGALRM, on_alrm, 0);
    sig_mask(SIG_BLOCK, SIGBIT(SIGUSR1), 0);
    raise_sig(SIGUSR1);
    check("usr1_pending_before", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    struct itv t = { { 0, 0 }, { 0, 100000 } };
    i64 start = now_ns();
    syscall3(SYS_setitimer, 0, &t, 0);
    i64 r = syscall0(SYS_pause);
    i64 ms = (now_ns() - start) / 1000000;
    check("returns_eintr", r == -EINTR, r);
    check("ended_by_alarm", alrm == 1, alrm);
    check("blocked_handler_not_run", usr1 == 0, usr1);
    check("waited_for_timer", ms >= 90, ms);
    check("usr1_still_pending", sig_pending() == SIGBIT(SIGUSR1), (i64)sig_pending());
    sig_mask(SIG_UNBLOCK, SIGBIT(SIGUSR1), 0);
    check("usr1_delivered_on_unblock", usr1 == 1, usr1);
    return finish();
}
