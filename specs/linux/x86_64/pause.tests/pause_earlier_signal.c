/* pause: a signal handled before pause is entered (raised while
 * unblocked, so its handler ran at once) does not make pause return; pause
 * waits for the next signal, here SIGALRM from a 100 ms timer.
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
    raise_sig(SIGUSR1);
    check("usr1_handled_before_pause", usr1 == 1, usr1);
    struct itv t = { { 0, 0 }, { 0, 100000 } };
    i64 start = now_ns();
    syscall3(SYS_setitimer, 0, &t, 0);
    i64 r = syscall0(SYS_pause);
    i64 ms = (now_ns() - start) / 1000000;
    check("returns_eintr", r == -EINTR, r);
    check("ended_by_alarm", alrm == 1, alrm);
    check("usr1_handler_once", usr1 == 1, usr1);
    check("waited_for_timer", ms >= 90, ms);
    return finish();
}
