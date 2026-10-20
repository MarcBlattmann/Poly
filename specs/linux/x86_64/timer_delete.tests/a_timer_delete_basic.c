/* timer_delete: a deleted timer is gone (EINVAL for every timer call);
 * deleting twice is EINVAL; deleting an armed timer stops it, so its
 * signal never comes. */
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
    struct a_itimerspec its = {{0, 0}, {0, 50000000}};
    syscall4(SYS_timer_settime, id, 0, &its, 0);
    expect("delete_armed", syscall1(SYS_timer_delete, id), 0);
    struct a_timespec req = {0, 100000000};
    syscall2(SYS_nanosleep, &req, 0);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("no_signal_after_delete", pending & set, 0);
    expect("delete_twice_einval", syscall1(SYS_timer_delete, id), -EINVAL);
    struct a_itimerspec cur;
    expect("gettime_einval", syscall2(SYS_timer_gettime, id, &cur), -EINVAL);
    expect("never_created_einval", syscall1(SYS_timer_delete, 1234), -EINVAL);
    return finish();
}
