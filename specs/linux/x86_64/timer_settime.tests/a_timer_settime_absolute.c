/* timer_settime with TIMER_ABSTIME: the value is a time on the timer's
 * clock; a time in the past expires at once; periodic timers keep their
 * interval and stay armed. */
#include "a_sys.h"

static i64 ns(struct a_timespec t) { return t.tv_sec * 1000000000L + t.tv_nsec; }

int test_main(void) {
    struct a_sigevent ev;
    memset(&ev, 0, sizeof ev);
    ev.sigev_notify = SIGEV_NONE;
    i32 id;
    syscall3(SYS_timer_create, CLOCK_MONOTONIC, &ev, &id);
    struct a_timespec now;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &now);
    struct a_itimerspec set = {{0, 0}, {now.tv_sec + 5, now.tv_nsec}}, cur;
    expect("arm_absolute", syscall4(SYS_timer_settime, id, TIMER_ABSTIME, &set, 0), 0);
    syscall2(SYS_timer_gettime, id, &cur);
    check("remaining_is_relative", ns(cur.it_value) > 4000000000L && ns(cur.it_value) <= 5000000000L,
          ns(cur.it_value));
    /* A time in the past: half of the clock's current reading. It is before
     * now however long the machine has been up (a fixed time such as 1 s is
     * not past yet right after boot), and not zero (zero would disarm). */
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &now);
    i64 half = ns(now) / 2;
    struct a_itimerspec past = {{0, 0}, {half / 1000000000L, half % 1000000000L}};
    expect("arm_in_past", syscall4(SYS_timer_settime, id, TIMER_ABSTIME, &past, 0), 0);
    syscall2(SYS_timer_gettime, id, &cur);
    expect("past_expired_at_once", ns(cur.it_value), 0);
    struct a_itimerspec periodic = {{0, 50000000}, {0, 50000000}};
    expect("arm_periodic", syscall4(SYS_timer_settime, id, 0, &periodic, 0), 0);
    struct a_timespec req = {0, 120000000};
    syscall2(SYS_nanosleep, &req, 0);
    syscall2(SYS_timer_gettime, id, &cur);
    expect("interval_kept", ns(cur.it_interval), 50000000);
    check("still_armed", ns(cur.it_value) > 0 && ns(cur.it_value) <= 50000000, ns(cur.it_value));
    syscall1(SYS_timer_delete, id);
    return finish();
}
