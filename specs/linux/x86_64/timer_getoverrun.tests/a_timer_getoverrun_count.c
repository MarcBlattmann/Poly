/* timer_getoverrun: while a periodic timer's signal stays pending, later
 * expiries are counted as overruns (read after the signal is taken). */
#include "a_sys.h"

int test_main(void) {
    u64 set = A_SIGBIT(SIGUSR1);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_SIGNAL;
    ev.sigev_signo = SIGUSR1;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id);
    expect("no_overrun_before", syscall1(SYS_timer_getoverrun, id), 0);
    struct a_itimerspec its = {{0, 10000000}, {0, 10000000}};
    syscall4(SYS_timer_settime, id, 0, &its, 0);
    /* About 20 periods pass while the first signal is pending; the
     * count is fixed when the signal is taken. (Re-arming or disarming
     * the timer first would make the pending signal stale.) */
    struct a_timespec req = {0, 205000000};
    syscall2(SYS_nanosleep, &req, 0);
    struct a_siginfo si;
    struct a_timespec zero = {0, 0};
    expect("take_signal", syscall4(SYS_rt_sigtimedwait, &set, &si, &zero, 8), SIGUSR1);
    i64 ov = syscall1(SYS_timer_getoverrun, id);
    struct a_itimerspec stop = {{0, 0}, {0, 0}};
    syscall4(SYS_timer_settime, id, 0, &stop, 0);
    check("overruns_counted", ov >= 10 && ov <= 25, ov);
    /* si_overrun (after the timer id in the siginfo) holds the same count. */
    expect("si_overrun_matches", si.si_uid, ov);
    syscall1(SYS_timer_delete, id);
    return finish();
}
