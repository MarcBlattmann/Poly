/* pause: SA_RESTART on the handler's signal does not restart pause; it
 * still fails with EINTR once the handler has run.
 */
#include "poly_ipc.h"

struct itv {
    struct k_timeval interval;
    struct k_timeval value;
};

static volatile int hits;

static void on_alrm(int sig) {
    (void)sig;
    hits++;
}

int test_main(void) {
    sig_handler(SIGALRM, on_alrm, SA_RESTART);
    struct itv t = { { 0, 0 }, { 0, 50000 } };
    syscall3(SYS_setitimer, 0, &t, 0);
    i64 r = syscall0(SYS_pause);
    check("sa_restart_still_eintr", r == -EINTR, r);
    check("handler_ran_once", hits == 1, hits);
    return finish();
}
