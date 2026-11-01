/* pause() (syscall 34) sleeps until a signal runs a handler (or ends the
 * process) and then returns EINTR (4), after the handler has run. It is
 * never restarted: also with SA_RESTART on the handler, and with an
 * SA_SIGINFO handler, it returns EINTR. A periodic ITIMER_REAL of 100 ms
 * supplies SIGALRM; the timer is stopped as soon as pause returns.
 */
#include "a_sys.h"

#define SYS_pause 34
/* asm-generic/signal-defs.h */
#define SA_SIGINFO 0x00000004
#define SA_RESTART 0x10000000

void ld_restorer(void);
__asm__(".text\n"
        ".global ld_restorer\n"
        "ld_restorer:\n"
        "    mov $15, %eax\n"
        "    syscall\n"
        "    hlt\n");

static volatile i32 alarms, info_signo, info_code;
static void on_alrm(int s) {
    (void)s;
    alarms++;
}
static void on_alrm_info(int s, struct a_siginfo *si, void *uc) {
    (void)uc;
    (void)s;
    alarms++;
    info_signo = si->si_signo;
    info_code = si->si_code;
}

static i64 paused(u64 handler, u64 flags) {
    struct a_sigaction sa = {handler, SA_RESTORER | flags, (u64)ld_restorer, 0};
    syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8);
    struct a_itimerval on = {{0, 100000}, {0, 100000}}, off = {{0, 0}, {0, 0}};
    syscall3(SYS_setitimer, ITIMER_REAL, &on, 0);
    i64 r = syscall0(SYS_pause);
    syscall3(SYS_setitimer, ITIMER_REAL, &off, 0);
    return r;
}

int test_main(void) {
    expect("plain_handler_eintr", paused((u64)on_alrm, 0), -EINTR);
    check("handler_ran_before_return", alarms >= 1, alarms);
    i32 before = alarms;
    expect("sa_restart_still_eintr", paused((u64)on_alrm, SA_RESTART), -EINTR);
    check("restart_handler_ran", alarms > before, alarms);
    before = alarms;
    expect("siginfo_handler_eintr", paused((u64)on_alrm_info, SA_SIGINFO), -EINTR);
    check("siginfo_handler_ran", alarms > before, alarms);
    expect("siginfo_signo", info_signo, SIGALRM);
    expect("siginfo_code_si_kernel", info_code, 0x80); /* SI_KERNEL, asm-generic/siginfo.h */
    return finish();
}
