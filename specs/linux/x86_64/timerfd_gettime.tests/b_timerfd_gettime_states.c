/* timerfd_gettime in each state: a one-shot timer that expired reads all
 * zero (even before its count is read); an interval with a zero value is
 * not armed (value 0, it never expires) but the interval is still
 * reported; an absolute deadline is
 * reported as the time left; after disarming all zero. */
#include "poly_ipc.h"

int test_main(void) {
    int fd = (int)syscall2(SYS_timerfd_create, CLOCK_MONOTONIC, TFD_NONBLOCK);
    struct k_itimerspec one = { { 0, 0 }, { 0, 5000000 } }, cur;
    syscall4(SYS_timerfd_settime, fd, 0, &one, 0);
    sleep_ms(15);
    syscall2(SYS_timerfd_gettime, fd, &cur);
    check("expired_one_shot_zero", cur.value.tv_sec == 0 && cur.value.tv_nsec == 0, cur.value.tv_nsec);
    u64 n;
    check("count_still_readable", read(fd, &n, 8) == 8 && n == 1, 0);

    struct k_itimerspec ionly = { { 1, 0 }, { 0, 0 } };
    syscall4(SYS_timerfd_settime, fd, 0, &ionly, 0);
    syscall2(SYS_timerfd_gettime, fd, &cur);
    check("interval_without_value_not_armed", cur.value.tv_sec == 0 && cur.value.tv_nsec == 0, cur.value.tv_sec);
    check("interval_still_reported", cur.interval.tv_sec == 1 && cur.interval.tv_nsec == 0, cur.interval.tv_sec);
    sleep_ms(5);
    check("never_expires", read(fd, &n, 8) == -EAGAIN, 0);

    struct k_timespec now;
    syscall2(SYS_clock_gettime, CLOCK_MONOTONIC, &now);
    struct k_itimerspec ab = { { 0, 0 }, { now.tv_sec + 50, now.tv_nsec } };
    syscall4(SYS_timerfd_settime, fd, TFD_TIMER_ABSTIME, &ab, 0);
    syscall2(SYS_timerfd_gettime, fd, &cur);
    check("absolute_reported_as_left", cur.value.tv_sec == 49 || cur.value.tv_sec == 50, cur.value.tv_sec);
    struct k_itimerspec off;
    memset(&off, 0, sizeof off);
    syscall4(SYS_timerfd_settime, fd, 0, &off, 0);
    syscall2(SYS_timerfd_gettime, fd, &cur);
    check("disarmed_zero", cur.value.tv_sec == 0 && cur.value.tv_nsec == 0, cur.value.tv_sec);
    return finish();
}
