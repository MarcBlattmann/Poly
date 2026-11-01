/* restart_syscall() (syscall 219) called by a program with nothing to
 * restart returns EINTR (4); its arguments are ignored. That is also the
 * case after a nanosleep finished normally, after a nanosleep that a
 * handled signal interrupted (the handler has returned through
 * rt_sigreturn, which forgets the restart), and after pause.
 */
#include "a_sys.h"

#define SYS_pause 34
#define SYS_restart_syscall 219

void ld_restorer(void);
__asm__(".text\n"
        ".global ld_restorer\n"
        "ld_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static volatile i32 alarms;
static void on_alrm(int s) {
    (void)s;
    alarms++;
}

static void arm_50ms(void) {
    struct a_itimerval it = {{0, 0}, {0, 50000}};
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
}

int test_main(void) {
    expect("fresh_process_eintr", syscall0(SYS_restart_syscall), -EINTR);
    expect("arguments_ignored", syscall6(SYS_restart_syscall, 1, 2, 3, 4, 5, 6), -EINTR);
    expect("again_eintr", syscall0(SYS_restart_syscall), -EINTR);

    struct a_timespec short_sleep = {0, 1000000}, rem;
    expect("nanosleep_done", syscall2(SYS_nanosleep, &short_sleep, &rem), 0);
    expect("after_completed_sleep_eintr", syscall0(SYS_restart_syscall), -EINTR);

    struct a_sigaction sa = {(u64)on_alrm, SA_RESTORER, (u64)ld_restorer, 0};
    syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8);
    struct a_timespec long_sleep = {5, 0};
    arm_50ms();
    expect("nanosleep_interrupted", syscall2(SYS_nanosleep, &long_sleep, &rem), -EINTR);
    expect("handler_ran", alarms, 1);
    check("remaining_written", rem.tv_sec >= 3 && rem.tv_sec <= 4, rem.tv_sec);
    expect("after_handled_signal_eintr", syscall0(SYS_restart_syscall), -EINTR);

    arm_50ms();
    expect("clock_nanosleep_interrupted", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, &long_sleep, &rem), -EINTR);
    expect("after_clock_nanosleep_eintr", syscall0(SYS_restart_syscall), -EINTR);

    arm_50ms();
    expect("pause_interrupted", syscall0(SYS_pause), -EINTR);
    expect("after_pause_eintr", syscall0(SYS_restart_syscall), -EINTR);
    expect("three_alarms", alarms, 3);
    return finish();
}
