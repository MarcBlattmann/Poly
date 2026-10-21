/* pause: blocks until a signal runs a handler, then fails with EINTR
 * after the handler has run. Here a one-shot ITIMER_REAL of 100 ms raises
 * SIGALRM; pause does not return before the timer fired.
 */
#include "poly_ipc.h"

struct itv {
    struct k_timeval interval;
    struct k_timeval value;
};

static volatile int hits, got;

static void on_alrm(int sig) {
    got = sig;
    hits++;
}

int test_main(void) {
    sig_handler(SIGALRM, on_alrm, 0);
    struct itv t = { { 0, 0 }, { 0, 100000 } };
    i64 start = now_ns();
    syscall3(SYS_setitimer, 0, &t, 0);
    i64 r = syscall0(SYS_pause);
    i64 ms = (now_ns() - start) / 1000000;
    check("returns_eintr", r == -EINTR, r);
    check("handler_ran_once", hits == 1, hits);
    check("handler_got_sigalrm", got == SIGALRM, got);
    check("not_before_timer", ms >= 90, ms);
    check("returns_soon_after_timer", ms < 5000, ms);
    check("nothing_pending", sig_pending() == 0, (i64)sig_pending());
    return finish();
}
