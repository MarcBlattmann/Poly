/* getitimer ITIMER_REAL shows a timer set by alarm (they are the same
 * timer); a periodic timer reloads its interval when its SIGALRM is
 * taken (while the signal waits blocked, the timer shows 0 left). */
#include "a_sys.h"

static i64 us(struct a_timeval t) { return t.tv_sec * 1000000L + t.tv_usec; }

int test_main(void) {
    u64 set = A_SIGBIT(SIGALRM);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    syscall1(SYS_alarm, 7);
    struct a_itimerval cur;
    syscall2(SYS_getitimer, ITIMER_REAL, &cur);
    check("alarm_visible", us(cur.it_value) > 6000000 && us(cur.it_value) <= 7000000, us(cur.it_value));
    expect("alarm_has_no_interval", us(cur.it_interval), 0);
    syscall1(SYS_alarm, 0);
    struct a_itimerval periodic = {{0, 20000}, {0, 20000}};
    syscall3(SYS_setitimer, ITIMER_REAL, &periodic, 0);
    struct a_siginfo si;
    struct a_timespec wait = {5, 0};
    expect("first_sigalrm", syscall4(SYS_rt_sigtimedwait, &set, &si, &wait, 8), SIGALRM);
    syscall2(SYS_getitimer, ITIMER_REAL, &cur);
    check("reloaded", us(cur.it_value) > 0 && us(cur.it_value) <= 20000, us(cur.it_value));
    expect("interval_kept", us(cur.it_interval), 20000);
    expect("second_sigalrm", syscall4(SYS_rt_sigtimedwait, &set, &si, &wait, 8), SIGALRM);
    struct a_itimerval off = {{0, 0}, {0, 0}};
    syscall3(SYS_setitimer, ITIMER_REAL, &off, 0);
    return finish();
}
