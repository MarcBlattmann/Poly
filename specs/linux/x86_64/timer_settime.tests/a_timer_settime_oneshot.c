/* timer_settime: arming a one-shot timer, reading the remaining time,
 * disarming with a zero value (no signal comes afterwards); the old value
 * is returned. The timer's signal is blocked. */
#include "a_sys.h"

static i64 ns(struct a_timespec t) { return t.tv_sec * 1000000000L + t.tv_nsec; }

int test_main(void) {
    u64 sigs = A_SIGBIT(SIGUSR1);
    syscall4(SYS_rt_sigprocmask, SIG_BLOCK, &sigs, 0, 8);
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_SIGNAL;
    ev.sigev_signo = SIGUSR1;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id);
    struct a_itimerspec set = {{0, 0}, {10, 0}}, old, cur;
    memset(&old, 0x5a, sizeof old);
    expect("arm_10s", syscall4(SYS_timer_settime, id, 0, &set, &old), 0);
    check("old_was_disarmed", ns(old.it_value) == 0 && ns(old.it_interval) == 0, ns(old.it_value));
    syscall2(SYS_timer_gettime, id, &cur);
    check("remaining_at_most_10s", ns(cur.it_value) > 9000000000L && ns(cur.it_value) <= 10000000000L,
          ns(cur.it_value));
    struct a_itimerspec zero = {{0, 0}, {0, 0}};
    expect("disarm", syscall4(SYS_timer_settime, id, 0, &zero, &old), 0);
    check("old_value_remaining", ns(old.it_value) > 9000000000L, ns(old.it_value));
    syscall2(SYS_timer_gettime, id, &cur);
    expect("disarmed_reads_zero", ns(cur.it_value), 0);
    struct a_itimerspec short_one = {{0, 0}, {0, 10000000}};
    syscall4(SYS_timer_settime, id, 0, &short_one, 0);
    struct a_siginfo si;
    struct a_timespec wait = {5, 0};
    expect("one_shot_signal", syscall4(SYS_rt_sigtimedwait, &sigs, &si, &wait, 8), SIGUSR1);
    syscall2(SYS_timer_gettime, id, &cur);
    expect("expired_one_shot_reads_zero", ns(cur.it_value), 0);
    /* Arm, disarm, wait past the old expiry: no signal. */
    syscall4(SYS_timer_settime, id, 0, &short_one, 0);
    syscall4(SYS_timer_settime, id, 0, &zero, 0);
    struct a_timespec req = {0, 30000000};
    syscall2(SYS_nanosleep, &req, 0);
    u64 pending = 0;
    syscall2(SYS_rt_sigpending, &pending, 8);
    expect("disarmed_never_fires", pending & sigs, 0);
    syscall1(SYS_timer_delete, id);
    return finish();
}
