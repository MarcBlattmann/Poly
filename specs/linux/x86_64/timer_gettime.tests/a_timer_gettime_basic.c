/* timer_gettime: a new timer is disarmed (zeros); an armed timer's
 * remaining time decreases; the interval reads back as set. */
#include "a_sys.h"

static i64 ns(struct a_timespec t) { return t.tv_sec * 1000000000L + t.tv_nsec; }

int test_main(void) {
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_NONE;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id);
    struct a_itimerspec cur;
    memset(&cur, 0x5a, sizeof cur);
    expect("gettime_new", syscall2(SYS_timer_gettime, id, &cur), 0);
    check("new_is_zero", ns(cur.it_value) == 0 && ns(cur.it_interval) == 0, ns(cur.it_value));
    struct a_itimerspec set = {{2, 500}, {3, 0}};
    syscall4(SYS_timer_settime, id, 0, &set, 0);
    struct a_itimerspec a, b;
    syscall2(SYS_timer_gettime, id, &a);
    struct a_timespec req = {0, 30000000};
    syscall2(SYS_nanosleep, &req, 0);
    syscall2(SYS_timer_gettime, id, &b);
    check("remaining_decreases", ns(b.it_value) < ns(a.it_value), ns(a.it_value) - ns(b.it_value));
    check("decreased_by_sleep", ns(a.it_value) - ns(b.it_value) >= 30000000, ns(a.it_value) - ns(b.it_value));
    expect("interval_as_set", ns(b.it_interval), 2000000500L);
    check("nsec_in_range", b.it_value.tv_nsec >= 0 && b.it_value.tv_nsec < 1000000000, b.it_value.tv_nsec);
    syscall1(SYS_timer_delete, id);
    return finish();
}
