/* setitimer ITIMER_VIRTUAL and ITIMER_PROF count CPU time: they expire
 * (SIGVTALRM, SIGPROF, blocked here) while the process spins, not while
 * it sleeps. */
#include "a_sys.h"

int test_main(void) {
    u64 set = A_SIGBIT(SIGVTALRM) | A_SIGBIT(SIGPROF);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &set, 0, 8);
    struct a_itimerval it = {{0, 0}, {0, 50000}};
    expect("arm_virtual", syscall3(SYS_setitimer, ITIMER_VIRTUAL, &it, 0), 0);
    expect("arm_prof", syscall3(SYS_setitimer, ITIMER_PROF, &it, 0), 0);
    struct a_timespec req = {0, 200000000};
    syscall2(SYS_nanosleep, &req, 0);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("sleeping_does_not_expire", pending & set, 0);
    a_spin_ticks(1500000000UL);
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("spinning_expires_both", pending & set, set);
    struct a_siginfo si;
    struct a_timespec zero = {0, 0};
    u64 one = A_SIGBIT(SIGVTALRM);
    expect("take_vtalrm", syscall4(SYS_rt_sigtimedwait, &one, &si, &zero, 8), SIGVTALRM);
    one = A_SIGBIT(SIGPROF);
    expect("take_prof", syscall4(SYS_rt_sigtimedwait, &one, &si, &zero, 8), SIGPROF);
    return finish();
}
