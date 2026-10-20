/* alarm: returns the seconds left on the previous alarm (rounded, at
 * least 1 while one is pending), 0 if none; alarm(0) cancels. */
#include "a_sys.h"

int test_main(void) {
    u64 set = A_SIGBIT(SIGALRM);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    expect("none_before", syscall1(SYS_alarm, 5), 0);
    i64 left = syscall1(SYS_alarm, 10);
    check("previous_about_5", left == 5 || left == 4, left);
    left = syscall1(SYS_alarm, 0);
    check("cancel_returns_about_10", left == 10 || left == 9, left);
    expect("cancelled_none_left", syscall1(SYS_alarm, 0), 0);
    /* A sub-second remainder still reports 1. */
    struct a_itimerval it = {{0, 0}, {0, 300000}};
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
    expect("subsecond_reports_1", syscall1(SYS_alarm, 0), 1);
    struct a_itimerval it2 = {{0, 0}, {2, 600000}};
    syscall3(SYS_setitimer, ITIMER_REAL, &it2, 0);
    expect("rounds_to_nearest_3", syscall1(SYS_alarm, 0), 3);
    return finish();
}
