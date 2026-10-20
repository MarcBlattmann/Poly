/* rt_sigtimedwait is interrupted by a signal outside its set that runs a
 * handler: EINTR (not restarted, even with SA_RESTART). The interrupting
 * signal comes from a repeating ITIMER_REAL, so one arrives while the
 * call waits even if the first fired before it.
 * poly-diff: syscalls=off (timer signals: how many arrive before the call varies)
 */
#include "poly_ipc.h"

#define ITIMER_REAL 0
struct k_itimerval {
    i64 interval_sec, interval_usec, value_sec, value_usec;
};

static volatile int alarms;
static void on_alarm(int sig) {
    (void)sig;
    alarms++;
}

int test_main(void) {
    sig_handler(SIGALRM, on_alarm, SA_RESTART);
    sig_mask(SIG_SETMASK, SIGBIT(SIGUSR1), 0);
    struct k_itimerval it = { 0, 10000, 0, 10000 };
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    u64 set = SIGBIT(SIGUSR1);
    struct k_timespec two = { 2, 0 };
    i64 r = syscall4(SYS_rt_sigtimedwait, &set, 0, &two, 8);
    struct k_itimerval off = { 0, 0, 0, 0 };
    syscall3(SYS_setitimer, ITIMER_REAL, &off, 0);
    check("interrupted_eintr", r == -EINTR, r);
    check("handler_ran", alarms >= 1, alarms);

    /* The same with a NULL timeout. */
    alarms = 0;
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    r = syscall4(SYS_rt_sigtimedwait, &set, 0, 0, 8);
    syscall3(SYS_setitimer, ITIMER_REAL, &off, 0);
    check("null_timeout_interrupted_eintr", r == -EINTR, r);
    check("null_timeout_handler_ran", alarms >= 1, alarms);
    return finish();
}
