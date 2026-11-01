/* restart_syscall resumes the sleep that a signal interrupted when it is
 * called from the signal's handler, before the handler returns: a
 * nanosleep of 300 ms interrupted after 50 ms (it has already written the
 * remaining time, about 250 ms, and will return EINTR) is continued by
 * restart_syscall in the handler for the rest of the time, which returns
 * 0; the whole call takes at least 300 ms. The same holds for a relative
 * clock_nanosleep. An absolute clock_nanosleep (TIMER_ABSTIME) and pause
 * leave nothing to restart: restart_syscall in their handler returns
 * EINTR (4) at once, and the absolute sleep returns EINTR well before its
 * deadline.
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

static volatile i64 in_handler;
static void on_alrm(int s) {
    (void)s;
    in_handler = syscall0(SYS_restart_syscall);
}

static i64 now_ns(void) {
    struct a_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}

static void arm_50ms(void) {
    struct a_itimerval it = {{0, 0}, {0, 50000}};
    in_handler = 12345;
    syscall3(SYS_setitimer, ITIMER_REAL, &it, 0);
}

int test_main(void) {
    struct a_sigaction sa = {(u64)on_alrm, SA_RESTORER, (u64)ld_restorer, 0};
    syscall4(SYS_rt_sigaction, SIGALRM, &sa, 0, 8);
    struct a_timespec req = {0, 300000000}, rem = {99, 99};

    arm_50ms();
    i64 t0 = now_ns();
    expect("nanosleep_still_eintr", syscall2(SYS_nanosleep, &req, &rem), -EINTR);
    i64 el = now_ns() - t0;
    expect("restart_in_handler_returns_0", in_handler, 0);
    check("slept_whole_300ms", el >= 300000000L, el);
    check("not_much_longer", el < 2000000000L, el);
    check("remaining_from_interruption", rem.tv_sec == 0 && rem.tv_nsec > 150000000 && rem.tv_nsec < 260000000,
          rem.tv_nsec);

    arm_50ms();
    t0 = now_ns();
    expect("clock_nanosleep_still_eintr", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, 0, &req, &rem), -EINTR);
    el = now_ns() - t0;
    expect("clock_restart_returns_0", in_handler, 0);
    check("clock_slept_whole_300ms", el >= 300000000L, el);

    struct a_timespec deadline;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &deadline);
    i64 dl = deadline.tv_sec * 1000000000L + deadline.tv_nsec + 300000000L;
    deadline.tv_sec = dl / 1000000000L;
    deadline.tv_nsec = dl % 1000000000L;
    arm_50ms();
    expect("abstime_still_eintr", syscall4(SYS_clock_nanosleep, CLOCK_MONOTONIC, TIMER_ABSTIME, &deadline, 0), -EINTR);
    expect("abstime_restart_in_handler_eintr", in_handler, -EINTR);
    check("abstime_ended_before_deadline", now_ns() < dl, 0);

    arm_50ms();
    expect("pause_eintr", syscall0(SYS_pause), -EINTR);
    expect("pause_restart_in_handler_eintr", in_handler, -EINTR);
    return finish();
}
