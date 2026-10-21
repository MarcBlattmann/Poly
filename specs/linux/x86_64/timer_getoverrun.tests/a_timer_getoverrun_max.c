/* timer_getoverrun: the count saturates at DELAYTIMER_MAX, which is
 * INT_MAX (timer_getoverrun(2), BUGS: since Linux 4.19; limits.h). A 1 ns
 * periodic timer whose signal stays pending for 2.5 s misses about 2.5e9
 * expiries, more than INT_MAX. */
#include "a_sys.h"

#define DELAYTIMER_MAX 2147483647L

int test_main(void) {
    u64 set = A_SIGBIT(SIGUSR2);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_SIGNAL;
    ev.sigev_signo = SIGUSR2;
    i32 id;
    expect("create", syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id), 0);
    struct a_itimerspec its = {{0, 1}, {0, 1}};
    expect("arm_1ns_periodic", syscall4(SYS_timer_settime, id, 0, &its, 0), 0);
    struct a_timespec req = {2, 500000000};
    syscall2(SYS_nanosleep, &req, 0);
    expect("unchanged_while_pending", syscall1(SYS_timer_getoverrun, id), 0);
    struct a_siginfo si;
    struct a_timespec zero = {0, 0};
    expect("take_signal", syscall4(SYS_rt_sigtimedwait, &set, &si, &zero, 8), SIGUSR2);
    expect("capped_at_delaytimer_max", syscall1(SYS_timer_getoverrun, id), DELAYTIMER_MAX);
    expect("si_overrun_capped", si.si_uid, DELAYTIMER_MAX);
    struct a_itimerspec stop = {{0, 0}, {0, 0}};
    expect("disarm", syscall4(SYS_timer_settime, id, 0, &stop, 0), 0);
    expect("disarm_resets_to_0", syscall1(SYS_timer_getoverrun, id), 0);
    expect("delete", syscall1(SYS_timer_delete, id), 0);
    return finish();
}
