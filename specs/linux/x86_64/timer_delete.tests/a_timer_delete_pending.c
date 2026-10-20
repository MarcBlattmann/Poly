/* timer_delete with the timer's signal already pending: that signal is
 * never delivered afterwards (taking it finds nothing, EAGAIN), and no
 * signal is left pending. */
#include "a_sys.h"

int test_main(void) {
    u64 set = A_SIGBIT(SIGUSR2);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_SIGNAL;
    ev.sigev_signo = SIGUSR2;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id);
    struct a_itimerspec its = {{0, 0}, {0, 10000000}};
    syscall4(SYS_timer_settime, id, 0, &its, 0);
    struct a_timespec req = {0, 50000000};
    syscall2(SYS_nanosleep, &req, 0);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("signal_pending", pending & set, set);
    expect("delete", syscall1(SYS_timer_delete, id), 0);
    struct a_siginfo si;
    struct a_timespec zero = {0, 0};
    expect("stale_signal_not_delivered", syscall4(SYS_rt_sigtimedwait, &set, &si, &zero, 8), -EAGAIN);
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("nothing_left_pending", pending & set, 0);
    return finish();
}
