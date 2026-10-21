/* timer_getoverrun: while a periodic timer's signal stays pending, later
 * expiries are not queued again but counted as overruns. The count is
 * fixed when the signal is taken: the expiries up to that moment less the
 * one that queued the signal. How many that is depends on timing, so the
 * test measures the time around arming the timer and around taking the
 * signal and accepts exactly the counts those times allow.
 * poly-diff: syscalls=names (the count returned depends on timing)
 */
#include "a_sys.h"

#define PERIOD 10000000L /* 10 ms, the first expiry and the interval */

static i64 now_ns(void) {
    struct a_timespec t;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &t);
    return t.tv_sec * 1000000000L + t.tv_nsec;
}

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
    struct a_itimerspec its = {{0, PERIOD}, {0, PERIOD}};
    i64 arm_lo = now_ns();
    syscall4(SYS_timer_settime, id, 0, &its, 0);
    i64 arm_hi = now_ns();
    /* About 20 periods pass while the first signal is pending. (Re-arming
     * or disarming the timer first would make the pending signal stale.) */
    struct a_timespec req = {0, 205000000};
    syscall2(SYS_nanosleep, &req, 0);
    expect("unchanged_while_pending", syscall1(SYS_timer_getoverrun, id), 0);
    struct a_siginfo si;
    struct a_timespec zero = {0, 0};
    i64 take_lo = now_ns();
    expect("take_signal", syscall4(SYS_rt_sigtimedwait, &set, &si, &zero, 8), SIGUSR1);
    i64 take_hi = now_ns();
    i64 ov = syscall1(SYS_timer_getoverrun, id);
    /* The timer was armed at some time A in [arm_lo, arm_hi] and
     * the signal taken at some T in [take_lo, take_hi]; expiries
     * are at A + k * PERIOD (k >= 1), so the count is
     * floor((T - A) / PERIOD) - 1. */
    i64 lo = (take_lo - arm_hi) / PERIOD - 1;
    i64 hi = (take_hi - arm_lo) / PERIOD - 1;
    int in_range = ov >= lo && ov <= hi;
    check("overruns_match_elapsed_time", in_range, ov);
    if (!in_range) {
        /* Only on failure: the range the measured times allowed. */
        check("allowed_from", 0, lo);
        check("allowed_to", 0, hi);
    }
    check("about_20_periods", ov >= 19, ov);
    /* si_overrun (after the timer id in the siginfo) holds the same count. */
    check("si_overrun_matches", si.si_uid == ov, si.si_uid);
    /* The count stays as it is while the next signal is pending again. */
    struct a_timespec more = {0, 30000000};
    syscall2(SYS_nanosleep, &more, 0);
    check("fixed_until_next_taken", syscall1(SYS_timer_getoverrun, id) == ov, 0);
    /* Setting the timer (here: disarming it) resets the count to 0. */
    struct a_itimerspec stop = {{0, 0}, {0, 0}};
    syscall4(SYS_timer_settime, id, 0, &stop, 0);
    expect("disarm_resets_to_0", syscall1(SYS_timer_getoverrun, id), 0);
    syscall1(SYS_timer_delete, id);
    return finish();
}
